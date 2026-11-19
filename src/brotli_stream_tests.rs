use super::*;
use crate::zlib_stream::{open as open_handle, write as write_handle, FULL_FLUSH, SYNC_FLUSH};

const TEXT: &[u8] = b"brotli-085 stream brotli-085 stream brotli-085 stream brotli-085 stream \
brotli-085 stream brotli-085 stream brotli-085 stream brotli-085 stream brotli-085 stream";

/// Feeds `input` through a fresh handle of `mode` in `size`-byte writes, then finishes.
fn run(mode: u32, params: &[(u32, u32)], input: &[u8], size: usize) -> Result<Vec<u8>, ZlibError> {
    let id = open_handle(mode, 6, params)?;
    let mut out = Vec::new();
    for chunk in input.chunks(size.max(1)) {
        out.extend(write_handle(id, chunk, NO_FLUSH)?);
    }
    out.extend(write_handle(id, &[], FINISH)?);
    Ok(out)
}

#[test]
fn chunked_streams_round_trip_and_match_the_one_shot_decoder() {
    for size in [1, 7, TEXT.len()] {
        let packed = run(BROTLI_ENCODE, &[], TEXT, size).unwrap();
        assert!(packed.len() < TEXT.len());
        let mut plain = Vec::new();
        brotli::BrotliDecompress(&mut &packed[..], &mut plain).unwrap();
        assert_eq!(plain, TEXT);
        for split in [1, 3, packed.len()] {
            assert_eq!(run(BROTLI_DECODE, &[], &packed, split).unwrap(), TEXT);
        }
    }
}

#[test]
fn params_configure_the_encoder_and_out_of_range_values_clamp() {
    let quality = BrotliEncoderParameter::BROTLI_PARAM_QUALITY as u32;
    let lgwin = BrotliEncoderParameter::BROTLI_PARAM_LGWIN as u32;
    for params in [
        vec![(quality, 0)],
        vec![(quality, 12), (lgwin, 30)],
        vec![(quality, u32::MAX), (lgwin, 0)],
        vec![(0, 1), (5, 1000), (6, 1), (lgwin, 30)],
    ] {
        let packed = run(BROTLI_ENCODE, &params, TEXT, 9).unwrap();
        assert_eq!(
            run(BROTLI_DECODE, &[(1, 1)], &packed, 4).unwrap(),
            TEXT,
            "{params:?}"
        );
    }
}

#[test]
fn unknown_or_rejected_params_fail_initialisation() {
    for (mode, params) in [
        (BROTLI_ENCODE, vec![(7, 0)]),
        (BROTLI_ENCODE, vec![(4, 2)]),
        (BROTLI_DECODE, vec![(2, 0)]),
    ] {
        assert!(matches!(
            open_handle(mode, 6, &params),
            Err(ZlibError::Init)
        ));
    }
    assert!(open_handle(BROTLI_DECODE, 6, &[(0, 1)]).is_ok());
}

#[test]
fn flush_emits_a_decodable_prefix() {
    for flush in [SYNC_FLUSH, FULL_FLUSH] {
        let enc = open_handle(BROTLI_ENCODE, 6, &[]).unwrap();
        let dec = open_handle(BROTLI_DECODE, 6, &[]).unwrap();
        let prefix = write_handle(enc, b"first;", flush).unwrap();
        assert_eq!(write_handle(dec, &prefix, NO_FLUSH).unwrap(), b"first;");
        let tail = write_handle(enc, b"second", FINISH).unwrap();
        assert_eq!(write_handle(dec, &tail, FINISH).unwrap(), b"second");
    }
}

#[test]
fn large_output_needs_several_decoder_rounds() {
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let packed = run(BROTLI_ENCODE, &[(1, 1)], &big, 100_000).unwrap();
    assert_eq!(run(BROTLI_DECODE, &[], &packed, packed.len()).unwrap(), big);
}

#[test]
fn malformed_truncated_and_trailing_input() {
    assert_eq!(
        run(BROTLI_DECODE, &[], b"nope nope nope", 4),
        Err(ZlibError::Brotli {
            errno: -15,
            code: "ERR__ERROR_FORMAT_PADDING_2".into()
        })
    );
    let packed = run(BROTLI_ENCODE, &[], TEXT, 64).unwrap();
    assert_eq!(
        run(BROTLI_DECODE, &[], &packed[..packed.len() - 2], 3),
        Err(ZlibError::Truncated)
    );
    assert_eq!(run(BROTLI_DECODE, &[], &[], 1), Err(ZlibError::Truncated));
    let mut trailing = packed.clone();
    trailing.extend_from_slice(b"xyz");
    assert_eq!(run(BROTLI_DECODE, &[], &trailing, 5).unwrap(), TEXT);
    assert_eq!(
        run(
            BROTLI_DECODE,
            &[],
            &run(BROTLI_ENCODE, &[], b"", 1).unwrap(),
            1
        )
        .unwrap(),
        b""
    );
}
