use super::*;

fn generated() -> (Vec<u8>, Vec<u8>) {
    let output = generate_with_entropy(|bytes| {
        bytes.fill(0);
        bytes[31] = 1;
        Ok(())
    })
    .unwrap();
    match output {
        Output::Pair(private, public) => (private, public),
        _ => panic!("generation must return a pair"),
    }
}

#[test]
fn generated_keys_export_import_sign_and_reject_changes() {
    let (private, public) = generated();
    assert!(matches!(import_private(&private), Ok(Output::Bytes(data)) if data == private));
    assert!(matches!(import_public(&public), Ok(Output::Bytes(data)) if data == public));
    assert!(matches!(derive_public(&private), Ok(Output::Bytes(data)) if data == public));
    let Output::Bytes(signature) = sign(b"message", &private).unwrap() else {
        panic!("signing must return DER bytes");
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
fn malformed_keys_and_unknown_operations_are_errors() {
    assert!(import_private(&[1, 2, 3]).is_err());
    assert!(import_public(&[1, 2, 3]).is_err());
    assert!(derive_public(&[1, 2, 3]).is_err());
    assert!(sign(b"message", &[1, 2, 3]).is_err());
    assert!(verify(b"message", &[1, 2, 3], &[1, 2, 3]).is_err());
    assert!(operation("unsupported", &[vec![], vec![], vec![]]).is_err());
}

#[test]
fn entropy_failures_propagate_and_invalid_scalars_are_retried() {
    let failure = generate_with_entropy(|_| Err("OS entropy unavailable".into()));
    assert!(matches!(failure, Err(message) if message == "OS entropy unavailable"));
    let mut attempts = 0;
    let generated = generate_with_entropy(|bytes| {
        attempts += 1;
        bytes.fill(0);
        if attempts > 1 {
            bytes[31] = 1;
        }
        Ok(())
    });
    assert!(generated.is_ok());
    assert_eq!(attempts, 2);
}
