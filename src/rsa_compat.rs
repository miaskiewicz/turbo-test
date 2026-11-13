//! Real RSA PKCS#1 v1.5 / SHA-256 for the Flux Assistant RS256 path (#349).
//! Key generation and private-operation blinding use a fresh OS-seeded CSPRNG.
use p256::elliptic_curve::zeroize::Zeroizing;
use rand_chacha::ChaCha20Rng;
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use rsa::rand_core::SeedableRng;
use rsa::signature::{RandomizedSigner, SignatureEncoding, Verifier};
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;

enum Output {
    Bytes(Vec<u8>),
    Pair(Vec<u8>, Vec<u8>),
    Verified(bool),
    Details(usize, String),
}

fn rng() -> Result<ChaCha20Rng, String> {
    rng_with_entropy(|seed| {
        getrandom::getrandom(seed).map_err(|error| format!("OS entropy failed: {error}"))
    })
}
fn rng_with_entropy(
    mut fill: impl FnMut(&mut [u8]) -> Result<(), String>,
) -> Result<ChaCha20Rng, String> {
    let mut seed = Zeroizing::new([0; 32]);
    fill(seed.as_mut())?;
    Ok(ChaCha20Rng::from_seed(*seed))
}
fn private_der(key: &RsaPrivateKey) -> Result<Vec<u8>, String> {
    key.to_pkcs8_der()
        .map(|value| value.as_bytes().to_vec())
        .map_err(|error| error.to_string())
}
fn public_der(key: &RsaPublicKey) -> Result<Vec<u8>, String> {
    key.to_public_key_der()
        .map(|value| value.as_bytes().to_vec())
        .map_err(|error| error.to_string())
}
fn private(bytes: &[u8]) -> Result<RsaPrivateKey, String> {
    let key = RsaPrivateKey::from_pkcs8_der(bytes).map_err(|error| error.to_string())?;
    key.validate().map_err(|error| error.to_string())?;
    Ok(key)
}
fn public(bytes: &[u8]) -> Result<RsaPublicKey, String> {
    RsaPublicKey::from_public_key_der(bytes).map_err(|error| error.to_string())
}
fn valid_parameters(bits: usize, exponent: u32) -> bool {
    (512..=4096).contains(&bits) && exponent >= 3 && exponent % 2 != 0
}
fn key_pair(key: &RsaPrivateKey) -> Result<Output, String> {
    Ok(Output::Pair(
        private_der(key)?,
        public_der(&RsaPublicKey::from(key))?,
    ))
}
fn generate(bits: usize, exponent: u32) -> Result<Output, String> {
    if !valid_parameters(bits, exponent) {
        return Err("Unsupported RSA modulus length or public exponent".into());
    }
    let key = RsaPrivateKey::new_with_exp(&mut rng()?, bits, &BigUint::from(exponent))
        .map_err(|error| error.to_string())?;
    key_pair(&key)
}
fn sign(data: &[u8], key: &[u8]) -> Result<Output, String> {
    let signer = SigningKey::<Sha256>::new(private(key)?);
    let signature: Signature = signer
        .try_sign_with_rng(&mut rng()?, data)
        .map_err(|error| error.to_string())?;
    Ok(Output::Bytes(signature.to_vec()))
}
fn verify(data: &[u8], key: &[u8], signature: &[u8]) -> Result<Output, String> {
    let verifier = VerifyingKey::<Sha256>::new(public(key)?);
    let Ok(signature) = Signature::try_from(signature) else {
        return Ok(Output::Verified(false));
    };
    Ok(Output::Verified(verifier.verify(data, &signature).is_ok()))
}
fn details(key: &RsaPublicKey) -> Output {
    Output::Details(key.n().bits(), key.e().to_string())
}
fn import_private(bytes: &[u8]) -> Result<Output, String> {
    private(bytes)
        .and_then(|key| private_der(&key))
        .map(Output::Bytes)
}
fn import_public(bytes: &[u8]) -> Result<Output, String> {
    public(bytes)
        .and_then(|key| public_der(&key))
        .map(Output::Bytes)
}
fn derive_public(bytes: &[u8]) -> Result<Output, String> {
    private(bytes)
        .and_then(|key| public_der(&RsaPublicKey::from(&key)))
        .map(Output::Bytes)
}
fn key_details(name: &str, bytes: &[u8]) -> Result<Output, String> {
    match name {
        "details-private" => private(bytes).map(|key| details(&RsaPublicKey::from(&key))),
        "details-public" => public(bytes).map(|key| details(&key)),
        _ => Err("Unsupported RSA key operation".into()),
    }
}
fn key_operation(name: &str, bytes: &[u8]) -> Result<Output, String> {
    match name {
        "private" => import_private(bytes),
        "public" => import_public(bytes),
        "derive" => derive_public(bytes),
        _ => key_details(name, bytes),
    }
}
fn operation(
    name: &str,
    inputs: &[Vec<u8>; 3],
    bits: usize,
    exponent: u32,
) -> Result<Output, String> {
    match name {
        "generate" => generate(bits, exponent),
        "sign" => sign(&inputs[0], &inputs[1]),
        "verify" => verify(&inputs[0], &inputs[1], &inputs[2]),
        _ => key_operation(name, &inputs[0]),
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
        Output::Details(bits, exponent) => {
            let bits = v8::Integer::new_from_unsigned(scope, bits as u32);
            let exponent = v8::String::new(scope, &exponent).unwrap();
            rv.set(v8::Array::new_with_elements(scope, &[bits.into(), exponent.into()]).into());
        }
    }
}
/// Internal byte-oriented RSA backend; the owned JS API validates public options.
pub fn native_rsa(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let inputs = [bytes(args.get(1)), bytes(args.get(2)), bytes(args.get(3))];
    let bits = args.get(4).uint32_value(scope).unwrap_or(0) as usize;
    let exponent = args.get(5).uint32_value(scope).unwrap_or(0);
    match operation(&name, &inputs, bits, exponent) {
        Ok(output) => return_output(scope, output, &mut rv),
        Err(message) => {
            let message = v8::String::new(scope, &message).unwrap();
            let error = v8::Exception::error(scope, message);
            scope.throw_exception(error);
        }
    }
}

#[cfg(test)]
#[path = "rsa_compat_tests.rs"]
mod tests;
