//! Native SHA-256 for the owned Node and Web Crypto compatibility APIs.
use sha2::{Digest, Sha256};

/// `__ttSha256(view)`: hash precisely the bytes of the supplied view, returning a fresh buffer.
/// Input normalization and algorithm validation belong to the public JS APIs.
pub fn native_sha256<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(args.get(0)) else {
        let message = v8::String::new(scope, "SHA-256 input must be an ArrayBufferView").unwrap();
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
        return;
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    let digest = Sha256::digest(&bytes).to_vec().into_boxed_slice();
    let store = v8::ArrayBuffer::new_backing_store_from_boxed_slice(digest).make_shared();
    rv.set(v8::ArrayBuffer::with_backing_store(scope, &store).into());
}

/// Native SHA-1 for Node createHash consumers such as HTTP etags.
pub fn native_sha1<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(args.get(0)) else {
        let message = v8::String::new(scope, "SHA-1 input must be an ArrayBufferView").unwrap();
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
        return;
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    let digest = <sha1::Sha1 as sha1::Digest>::digest(&bytes)
        .to_vec()
        .into_boxed_slice();
    let store = v8::ArrayBuffer::new_backing_store_from_boxed_slice(digest).make_shared();
    rv.set(v8::ArrayBuffer::with_backing_store(scope, &store).into());
}

fn keyed_digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match algorithm {
        "sha1" => Ok(<sha1::Sha1 as sha1::Digest>::digest(data).to_vec()),
        "sha256" => Ok(Sha256::digest(data).to_vec()),
        _ => Err("Unsupported keyed hash algorithm".into()),
    }
}

fn hmac_digest(algorithm: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let key = if key.len() > 64 {
        keyed_digest(algorithm, key)?
    } else {
        key.to_vec()
    };
    let mut inner = vec![0x36; 64];
    let mut outer = vec![0x5c; 64];
    for (index, byte) in key.iter().enumerate() {
        inner[index] ^= byte;
        outer[index] ^= byte;
    }
    inner.extend_from_slice(data);
    outer.extend_from_slice(&keyed_digest(algorithm, &inner)?);
    keyed_digest(algorithm, &outer)
}

fn validate_pbkdf2(iterations: u32, length: usize) -> Result<(), String> {
    if iterations == 0 || length == 0 || length > 1_048_576 {
        return Err(
            "PBKDF2 requires positive iterations and output length of 1 through 1048576 bytes"
                .into(),
        );
    }
    Ok(())
}

fn pbkdf2_sha1(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Result<Vec<u8>, String> {
    validate_pbkdf2(iterations, length)?;
    let mut output = Vec::with_capacity(length);
    for block in 1..=length.div_ceil(20) {
        let mut input = salt.to_vec();
        input.extend_from_slice(&(block as u32).to_be_bytes());
        let mut current = hmac_digest("sha1", password, &input)?;
        let mut accumulated = current.clone();
        for _ in 1..iterations {
            current = hmac_digest("sha1", password, &current)?;
            for (target, value) in accumulated.iter_mut().zip(&current) {
                *target ^= value;
            }
        }
        output.extend_from_slice(&accumulated);
    }
    output.truncate(length);
    Ok(output)
}

fn keyed_input(args: &v8::FunctionCallbackArguments, index: i32) -> Result<Vec<u8>, String> {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(args.get(index))
        .map_err(|_| "Keyed hash inputs must be byte views".to_owned())?;
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    Ok(bytes)
}

fn keyed_operation(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<Vec<u8>, String> {
    let name = args.get(0).to_rust_string_lossy(scope);
    let key = keyed_input(args, 1)?;
    let data = keyed_input(args, 2)?;
    match name.as_str() {
        "hmac-sha1" => hmac_digest("sha1", &key, &data),
        "hmac-sha256" => hmac_digest("sha256", &key, &data),
        "pbkdf2-sha1" => pbkdf2_sha1(
            &key,
            &data,
            args.get(3).uint32_value(scope).unwrap_or(0),
            args.get(4).uint32_value(scope).unwrap_or(0) as usize,
        ),
        _ => Err("Unsupported keyed hash operation".into()),
    }
}

/// Native keyed hashes; public JS normalizes and copies supplied byte views.
pub fn native_crypto_hashes(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match keyed_operation(scope, &args) {
        Ok(bytes) => {
            let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
            rv.set(v8::ArrayBuffer::with_backing_store(scope, &store).into());
        }
        Err(message) => {
            let message = v8::String::new(scope, &message).unwrap();
            let error = v8::Exception::error(scope, message);
            scope.throw_exception(error);
        }
    }
}
