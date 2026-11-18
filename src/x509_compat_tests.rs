use super::*;
use base64::Engine;
#[test]
fn serial_hex_is_byte_padded_for_odd_digit_serials() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/data/compat-084/x509/certificates.json"
    ))
    .unwrap();
    let mut der = base64::engine::general_purpose::STANDARD
        .decode(fixture["certificates"]["matching"].as_str().unwrap())
        .unwrap();
    let offset = {
        let (_, certificate) = X509Certificate::from_der(&der).unwrap();
        certificate.raw_serial().as_ptr() as usize - der.as_ptr() as usize
    };
    der[offset] = 1;
    let parsed = parse(&der).unwrap();
    assert!(parsed.serial.starts_with("01"));
    assert_eq!(parsed.serial.len() % 2, 0);
}
