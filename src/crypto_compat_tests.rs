use super::*;
#[test]
fn keyed_algorithms_and_kdf_bounds_reject_unsupported_requests() {
    assert!(keyed_digest("md5", b"message").is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 0, 20).is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 1, 0).is_err());
    assert!(pbkdf2_sha1(b"password", b"salt", 1, 1_048_577).is_err());
    assert_eq!(hmac_digest("sha1", b"key", b"message").unwrap().len(), 20);
}
