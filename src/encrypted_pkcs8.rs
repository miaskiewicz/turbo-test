//! Bounded encrypted PKCS8 import via RustCrypto PBES2 and PKCS12 SHA1/3DES.
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use pkcs8::der::Decode;
use zeroize::Zeroizing;
const MAX_BYTES: usize = 1_048_576;
const MAX_ROUNDS: u32 = 1_000_000;
fn bounds(rounds: u32, salt: &[u8]) -> Result<(), String> {
    if rounds == 0 || rounds > MAX_ROUNDS || salt.len() > 1024 {
        return Err("Encrypted PKCS8 KDF exceeds supported resource bounds".into());
    }
    Ok(())
}
fn decrypt_pbes2(der: &[u8], password: &[u8]) -> Result<Vec<u8>, String> {
    let encrypted =
        pkcs8::EncryptedPrivateKeyInfo::from_der(der).map_err(|_| "Invalid PBES2 PKCS8")?;
    let params = encrypted
        .encryption_algorithm
        .pbes2()
        .ok_or("Unsupported PBES encryption")?;
    let kdf = params
        .kdf
        .pbkdf2()
        .ok_or("Only PBKDF2 PBES2 imports are supported")?;
    bounds(kdf.iteration_count, kdf.salt)?;
    encrypted
        .decrypt(password)
        .map(|doc| doc.as_bytes().to_vec())
        .map_err(|_| "Unable to decrypt PKCS8 key".into())
}
fn pkcs12_parameters(
    info: &pkcs12::pbe_params::EncryptedPrivateKeyInfo,
) -> Result<(pkcs12::pbe_params::Pkcs12PbeParams, u32), String> {
    let params: pkcs12::pbe_params::Pkcs12PbeParams = info
        .encryption_algorithm
        .parameters
        .as_ref()
        .ok_or("Missing PKCS12 PBE parameters")?
        .decode_as()
        .map_err(|_| "Invalid PKCS12 PBE parameters")?;
    let rounds = u32::try_from(params.iterations).map_err(|_| "Invalid PKCS12 KDF iterations")?;
    bounds(rounds, params.salt.as_bytes())?;
    Ok((params, rounds))
}
fn decrypt_pkcs12(
    info: &pkcs12::pbe_params::EncryptedPrivateKeyInfo,
    password: &[u8],
) -> Result<Vec<u8>, String> {
    let (params, rounds) = pkcs12_parameters(info)?;
    let password = std::str::from_utf8(password).map_err(|_| "PKCS12 PBE password must be UTF8")?;
    let key = derive_pkcs12(
        password,
        params.salt.as_bytes(),
        rounds,
        pkcs12::kdf::Pkcs12KeyType::EncryptionKey,
        24,
    )?;
    let iv = derive_pkcs12(
        password,
        params.salt.as_bytes(),
        rounds,
        pkcs12::kdf::Pkcs12KeyType::Iv,
        8,
    )?;
    let cipher = cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
        .map_err(|_| "Invalid PKCS12 cipher parameters")?;
    cipher
        .decrypt_padded_vec_mut::<Pkcs7>(info.encrypted_data.as_bytes())
        .map_err(|_| "Unable to decrypt PKCS8 key".into())
}
fn derive_pkcs12(
    password: &str,
    salt: &[u8],
    rounds: u32,
    kind: pkcs12::kdf::Pkcs12KeyType,
    length: usize,
) -> Result<Zeroizing<Vec<u8>>, String> {
    pkcs12::kdf::derive_key_utf8::<sha1_legacy::Sha1>(password, salt, kind, rounds as i32, length)
        .map(Zeroizing::new)
        .map_err(|_| "Unsupported PKCS12 password encoding".into())
}
fn decrypt(der: &[u8], password: &[u8]) -> Result<Vec<u8>, String> {
    if der.len() > MAX_BYTES || password.len() > 1024 {
        return Err("Encrypted PKCS8 input exceeds supported resource bounds".into());
    }
    if pkcs8::PrivateKeyInfo::from_der(der).is_ok() {
        return Ok(der.to_vec());
    }
    let info = pkcs12::pbe_params::EncryptedPrivateKeyInfo::from_der(der)
        .map_err(|_| "Invalid encrypted PKCS8 DER")?;
    match info.encryption_algorithm.oid.to_string().as_str() {
        "1.2.840.113549.1.5.13" => decrypt_pbes2(der, password),
        "1.2.840.113549.1.12.1.3" => decrypt_pkcs12(&info, password),
        _ => Err("Unsupported encrypted PKCS8 algorithm".into()),
    }
}
fn bytes(args: &v8::FunctionCallbackArguments, index: i32) -> Result<Zeroizing<Vec<u8>>, String> {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(args.get(index))
        .map_err(|_| "PKCS8 inputs must be byte views")?;
    if view.byte_length() > MAX_BYTES {
        return Err("Encrypted PKCS8 input exceeds supported resource bounds".into());
    }
    let mut data = Zeroizing::new(vec![0; view.byte_length()]);
    view.copy_contents(&mut data);
    Ok(data)
}
fn request(args: &v8::FunctionCallbackArguments) -> Result<Vec<u8>, String> {
    decrypt(&bytes(args, 0)?, &bytes(args, 1)?)
}
pub fn native_decrypt_pkcs8(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match request(&args) {
        Ok(data) => {
            let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
            rv.set(v8::ArrayBuffer::with_backing_store(scope, &store).into());
        }
        Err(error) => {
            let text = v8::String::new(scope, &error).unwrap();
            let exception = v8::Exception::error(scope, text);
            scope.throw_exception(exception);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn expensive_or_invalid_kdfs_are_rejected_before_derivation() {
        assert!(super::bounds(0, &[0; 8]).is_err());
        assert!(super::bounds(1_000_001, &[0; 8]).is_err());
        assert!(super::bounds(1, &[0; 1025]).is_err());
        assert!(super::bounds(2048, &[0; 8]).is_ok());
    }
}
