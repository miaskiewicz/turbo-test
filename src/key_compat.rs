//! P-256 key generation with OS entropy, PKCS#8/SPKI keys and SHA-256 ECDSA.
//! Key bytes live with the JS KeyObject; there is no persistent host key registry.
use p256::ecdsa::{signature::Signer, signature::Verifier, Signature, SigningKey, VerifyingKey};
use p256::elliptic_curve::zeroize::Zeroizing;
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use p256::{PublicKey, SecretKey};

enum Output {
    Bytes(Vec<u8>),
    Pair(Vec<u8>, Vec<u8>),
    Verified(bool),
}

fn private_der(key: &SecretKey) -> Result<Vec<u8>, String> {
    key.to_pkcs8_der()
        .map(|document| document.as_bytes().to_vec())
        .map_err(|error| error.to_string())
}

fn public_der(key: &PublicKey) -> Result<Vec<u8>, String> {
    key.to_public_key_der()
        .map(|document| document.as_bytes().to_vec())
        .map_err(|error| error.to_string())
}

fn generate() -> Result<Output, String> {
    generate_with_entropy(|bytes| {
        getrandom::getrandom(bytes).map_err(|error| format!("OS entropy failed: {error}"))
    })
}

fn generate_with_entropy(
    mut fill: impl FnMut(&mut [u8]) -> Result<(), String>,
) -> Result<Output, String> {
    let mut scalar = Zeroizing::new([0u8; 32]);
    loop {
        fill(scalar.as_mut())?;
        if let Ok(key) = SecretKey::from_slice(scalar.as_ref()) {
            return Ok(Output::Pair(
                private_der(&key)?,
                public_der(&key.public_key())?,
            ));
        }
    }
}

fn import_private(bytes: &[u8]) -> Result<Output, String> {
    let key = SecretKey::from_pkcs8_der(bytes).map_err(|error| error.to_string())?;
    Ok(Output::Bytes(private_der(&key)?))
}

fn import_public(bytes: &[u8]) -> Result<Output, String> {
    let key = PublicKey::from_public_key_der(bytes).map_err(|error| error.to_string())?;
    Ok(Output::Bytes(public_der(&key)?))
}

fn derive_public(bytes: &[u8]) -> Result<Output, String> {
    let key = SecretKey::from_pkcs8_der(bytes).map_err(|error| error.to_string())?;
    Ok(Output::Bytes(public_der(&key.public_key())?))
}

fn sign(data: &[u8], key: &[u8]) -> Result<Output, String> {
    let secret = SecretKey::from_pkcs8_der(key).map_err(|error| error.to_string())?;
    let signer = SigningKey::from(secret);
    let signature: Signature = signer.try_sign(data).map_err(|error| error.to_string())?;
    Ok(Output::Bytes(signature.to_der().as_bytes().to_vec()))
}

fn verify(data: &[u8], key: &[u8], signature: &[u8]) -> Result<Output, String> {
    let verifier = VerifyingKey::from_public_key_der(key).map_err(|error| error.to_string())?;
    let Ok(signature) = Signature::from_der(signature) else {
        return Ok(Output::Verified(false));
    };
    Ok(Output::Verified(verifier.verify(data, &signature).is_ok()))
}

fn operation(name: &str, inputs: &[Vec<u8>; 3]) -> Result<Output, String> {
    match name {
        "generate" => generate(),
        "private" => import_private(&inputs[0]),
        "public" => import_public(&inputs[0]),
        "derive" => derive_public(&inputs[0]),
        "sign" => sign(&inputs[0], &inputs[1]),
        "verify" => verify(&inputs[0], &inputs[1], &inputs[2]),
        _ => Err("Unsupported EC key operation".into()),
    }
}

fn bytes(value: v8::Local<v8::Value>) -> Vec<u8> {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) else {
        return Vec::new();
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    bytes
}

fn view<'s>(scope: &mut v8::PinScope<'s, '_>, data: Vec<u8>) -> v8::Local<'s, v8::Uint8Array> {
    let length = data.len();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length).unwrap()
}

fn return_output(scope: &mut v8::PinScope, output: Output, rv: &mut v8::ReturnValue) {
    match output {
        Output::Bytes(data) => rv.set(view(scope, data).into()),
        Output::Pair(private, public) => {
            let private = view(scope, private);
            let public = view(scope, public);
            rv.set(v8::Array::new_with_elements(scope, &[private.into(), public.into()]).into());
        }
        Output::Verified(value) => rv.set_bool(value),
    }
}

/// `__ttEc(operation, firstBytes, secondBytes, thirdBytes)`: owned P-256 backend.
/// JS validates public API options and normalizes all byte views before entering this callback.
pub fn native_ec(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let inputs = [bytes(args.get(1)), bytes(args.get(2)), bytes(args.get(3))];
    match operation(&name, &inputs) {
        Ok(output) => return_output(scope, output, &mut rv),
        Err(message) => {
            let message = v8::String::new(scope, &message).unwrap();
            let error = v8::Exception::error(scope, message);
            scope.throw_exception(error);
        }
    }
}

#[cfg(test)]
#[path = "key_compat_tests.rs"]
mod tests;
