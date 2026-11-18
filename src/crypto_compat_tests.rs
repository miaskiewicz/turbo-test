use super::*;
#[test]
fn keyed_algorithms_and_kdf_bounds_reject_unsupported_requests() {
    assert!(keyed_digest("md5", b"message").is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 0, 20).is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 1, 0).is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 1, 1_048_577).is_err());
    assert_eq!(hmac_digest("sha1", b"key", b"message").unwrap().len(), 20);
}

#[test]
fn pbkdf2_checks_total_block_work_without_deriving() {
    assert!(validate_pbkdf2(1_000_000, 20).is_ok());
    assert!(validate_pbkdf2(500_000, 40).is_ok());
    assert!(validate_pbkdf2(1_000_001, 20).is_err());
    assert!(validate_pbkdf2(500_001, 21).is_err());
    assert!(validate_pbkdf2(u32::MAX, 1_048_576).is_err());
}
