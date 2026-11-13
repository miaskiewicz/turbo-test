use super::*;

#[test]
fn rsa_key_material_exports_imports_derives_and_signs_real_bytes() {
    let mut rng = ChaCha20Rng::from_seed([7; 32]);
    let key = RsaPrivateKey::new(&mut rng, 1024).unwrap();
    let private = private_der(&key).unwrap();
    let public = public_der(&RsaPublicKey::from(&key)).unwrap();
    assert!(
        matches!(key_operation("private", &private), Ok(Output::Bytes(bytes)) if bytes == private)
    );
    assert!(
        matches!(key_operation("public", &public), Ok(Output::Bytes(bytes)) if bytes == public)
    );
    assert!(
        matches!(key_operation("derive", &private), Ok(Output::Bytes(bytes)) if bytes == public)
    );
    assert!(
        matches!(key_operation("details-private", &private), Ok(Output::Details(1024, exponent)) if exponent == "65537")
    );
    assert!(
        matches!(key_operation("details-public", &public), Ok(Output::Details(1024, exponent)) if exponent == "65537")
    );
    let Output::Bytes(signature) = sign(b"message", &private).unwrap() else {
        panic!("RSA sign must produce bytes");
    };
    assert!(matches!(
        verify(b"message", &public, &signature),
        Ok(Output::Verified(true))
    ));
    assert!(matches!(
        verify(b"changed", &public, &signature),
        Ok(Output::Verified(false))
    ));
    assert!(matches!(
        verify(b"message", &public, &[1, 2, 3]),
        Ok(Output::Verified(false))
    ));
}

#[test]
fn rsa_malformed_keys_invalid_parameters_and_unknown_operations_are_errors() {
    assert!(key_operation("private", &[1, 2, 3]).is_err());
    assert!(key_operation("public", &[1, 2, 3]).is_err());
    assert!(key_operation("derive", &[1, 2, 3]).is_err());
    assert!(key_operation("details-private", &[1, 2, 3]).is_err());
    assert!(key_operation("details-public", &[1, 2, 3]).is_err());
    assert!(key_operation("unknown", &[]).is_err());
    assert!(generate(256, 65537).is_err());
    assert!(generate(2048, 2).is_err());
    assert!(operation("unknown", &[vec![], vec![], vec![]], 2048, 65537).is_err());
}

#[test]
fn rsa_csprng_entropy_errors_are_propagated_instead_of_manufacturing_keys() {
    let error = rng_with_entropy(|_| Err("controlled OS entropy failure".into()));
    assert!(matches!(error, Err(message) if message == "controlled OS entropy failure"));
}
