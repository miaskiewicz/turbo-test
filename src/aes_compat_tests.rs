use super::*;

fn input(decrypt: bool, data: Vec<u8>, aad: Vec<u8>) -> Request {
    Request {
        decrypt,
        key: Zeroizing::new(vec![0; 32]),
        iv: vec![0; 12],
        data,
        aad,
    }
}

#[test]
fn aes_known_answer_and_authentication_failures() {
    let encrypted = operation(&input(false, vec![0; 16], vec![]), 16).unwrap();
    assert_eq!(
        &encrypted[..16],
        &[
            0xce, 0xa7, 0x40, 0x3d, 0x4d, 0x60, 0x6b, 0x6e, 0x07, 0x4e, 0xc5, 0xd3, 0xba, 0xf3,
            0x9d, 0x18
        ]
    );
    assert_eq!(
        operation(&input(true, encrypted.clone(), vec![]), 16).unwrap(),
        vec![0; 16]
    );
    for index in [0, 16, 31] {
        let mut changed = encrypted.clone();
        changed[index] ^= 1;
        assert!(operation(&input(true, changed, vec![]), 16).is_err());
    }
    assert!(operation(&input(true, encrypted, vec![1]), 16).is_err());
}

#[test]
fn configured_tags_and_empty_payloads_round_trip() {
    for length in 12..=16 {
        let encrypted = operation(&input(false, vec![], vec![1, 2]), length).unwrap();
        assert_eq!(encrypted.len(), length as usize);
        assert_eq!(
            operation(&input(true, encrypted, vec![1, 2]), length).unwrap(),
            Vec::<u8>::new()
        );
    }
    assert!(operation(&input(false, vec![], vec![]), 11).is_err());
}

#[test]
fn entropy_errors_return_no_random_output() {
    assert_eq!(
        random_with(3, |bytes| {
            bytes.copy_from_slice(&[1, 2, 3]);
            Ok(())
        })
        .unwrap(),
        [1, 2, 3]
    );
    assert_eq!(
        random_with(3, |_| Err("entropy unavailable".into())).unwrap_err(),
        "entropy unavailable"
    );
}
