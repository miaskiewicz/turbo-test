//! Authenticated AES-256-GCM for the owned Node cipher API and OS-backed entropy.
use aes_gcm::aead::{consts, Aead, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Nonce, TagSize};
use zeroize::Zeroizing;

struct Request {
    decrypt: bool,
    key: Zeroizing<Vec<u8>>,
    iv: Vec<u8>,
    data: Vec<u8>,
    aad: Vec<u8>,
}

fn bytes(value: v8::Local<v8::Value>) -> Result<Vec<u8>, String> {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
        .map_err(|_| "AES inputs must be byte views".to_owned())?;
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    Ok(bytes)
}

fn request(args: &v8::FunctionCallbackArguments) -> Result<Request, String> {
    let request = Request {
        decrypt: args.get(0).is_true(),
        key: Zeroizing::new(bytes(args.get(1))?),
        iv: bytes(args.get(2))?,
        data: bytes(args.get(3))?,
        aad: bytes(args.get(4))?,
    };
    if request.key.len() != 32 || request.iv.len() != 12 {
        return Err("AES-256-GCM requires a 32-byte key and a 12-byte IV".into());
    }
    Ok(request)
}

fn crypt<Tag: TagSize>(request: &Request) -> Result<Vec<u8>, String> {
    let cipher = AesGcm::<Aes256, consts::U12, Tag>::new_from_slice(&request.key)
        .map_err(|_| "Invalid AES key length".to_owned())?;
    let nonce = Nonce::from_slice(&request.iv);
    let payload = Payload {
        msg: &request.data,
        aad: &request.aad,
    };
    let output = if request.decrypt {
        cipher.decrypt(nonce, payload)
    } else {
        cipher.encrypt(nonce, payload)
    };
    output.map_err(|_| "Unsupported state or unable to authenticate data".into())
}

fn operation(request: &Request, tag_length: u32) -> Result<Vec<u8>, String> {
    match tag_length {
        12 => crypt::<consts::U12>(request),
        13 => crypt::<consts::U13>(request),
        14 => crypt::<consts::U14>(request),
        15 => crypt::<consts::U15>(request),
        16 => crypt::<consts::U16>(request),
        _ => Err("AES-GCM supports authentication tags of 12 through 16 bytes".into()),
    }
}

fn throw_error(scope: &mut v8::PinScope, message: &str) {
    let text = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::error(scope, text);
    scope.throw_exception(error);
}

fn return_bytes(scope: &mut v8::PinScope, data: Vec<u8>, rv: &mut v8::ReturnValue) {
    let length = data.len();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    rv.set(
        v8::Uint8Array::new(scope, buffer, 0, length)
            .unwrap()
            .into(),
    );
}

/// `__ttAesGcm(decrypt, key, iv, data, aad, tagLength)`; encrypted input/output includes its tag.
pub fn native_aes_gcm(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = request(&args)
        .and_then(|input| operation(&input, args.get(5).uint32_value(scope).unwrap_or(0)));
    match result {
        Ok(data) => return_bytes(scope, data, &mut rv),
        Err(message) => throw_error(scope, &message),
    }
}

fn random_with(
    size: usize,
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Zeroizing::new(vec![0; size]);
    fill(&mut bytes)?;
    Ok(bytes.to_vec())
}

/// OS entropy is independent of test clocks and the runtime's deterministic PRNG.
pub fn native_random_bytes(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let size = args.get(0).uint32_value(scope).unwrap_or(u32::MAX);
    if size > i32::MAX as u32 {
        return throw_error(scope, "Invalid random byte size");
    }
    let result = random_with(size as usize, |bytes| {
        getrandom::getrandom(bytes).map_err(|error| format!("OS entropy failed: {error}"))
    });
    match result {
        Ok(data) => return_bytes(scope, data, &mut rv),
        Err(message) => throw_error(scope, &message),
    }
}

#[cfg(test)]
#[path = "aes_compat_tests.rs"]
mod tests;
