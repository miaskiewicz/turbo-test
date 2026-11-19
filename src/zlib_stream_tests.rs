use super::*;
use std::io::{Read, Write};

const TEXT: &[u8] = b"zip-404 stream zip-404 stream zip-404 stream zip-404 stream zip-404 stream \
zip-404 stream zip-404 stream zip-404 stream zip-404 stream zip-404 stream zip-404 stream";

/// Feeds `input` through a fresh handle of `mode` in `size`-byte writes, then finishes.
fn run(mode: u32, input: &[u8], size: usize) -> Result<Vec<u8>> {
    let id = open(mode, 6, &[]).unwrap();
    let mut out = Vec::new();
    for chunk in input.chunks(size.max(1)) {
        out.extend(write(id, chunk, NO_FLUSH)?);
    }
    out.extend(write(id, &[], FINISH)?);
    Ok(out)
}

fn gzip_member(text: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(text).unwrap();
    e.finish().unwrap()
}

/// A gzip member with FEXTRA, FNAME, FCOMMENT and FHCRC fields before the body.
fn gzip_with_fields(text: &[u8]) -> Vec<u8> {
    let plain = gzip_member(text);
    let mut out = vec![0x1f, 0x8b, 8, 2 | 4 | 8 | 16, 0, 0, 0, 0, 0, 255];
    out.extend_from_slice(&[3, 0, b'a', b'b', b'c']);
    out.extend_from_slice(b"name.txt\0comment\0");
    out.extend_from_slice(&[0xaa, 0xbb]);
    out.extend_from_slice(&plain[10..]);
    out
}

#[test]
fn every_mode_round_trips_chunk_by_chunk() {
    for (encode, decode) in [
        (DEFLATE, INFLATE),
        (DEFLATERAW, INFLATERAW),
        (GZIP, GUNZIP),
        (DEFLATE, UNZIP),
        (GZIP, UNZIP),
    ] {
        let packed = run(encode, TEXT, 7).unwrap();
        assert!(packed.len() < TEXT.len());
        for size in [1, 2, 5, packed.len()] {
            assert_eq!(
                run(decode, &packed, size).unwrap(),
                TEXT,
                "{encode}>{decode}/{size}"
            );
        }
    }
}

#[test]
fn output_is_readable_by_independent_decoders() {
    let mut text = Vec::new();
    flate2::read::ZlibDecoder::new(&run(DEFLATE, TEXT, 3).unwrap()[..])
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, TEXT);
    text.clear();
    flate2::read::DeflateDecoder::new(&run(DEFLATERAW, TEXT, 3).unwrap()[..])
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, TEXT);
    text.clear();
    flate2::read::GzDecoder::new(&run(GZIP, TEXT, 3).unwrap()[..])
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, TEXT);
}

#[test]
fn gzip_header_matches_nodes_level_and_os_bytes() {
    for (level, xfl) in [(6, 0), (9, 2), (1, 4), (0, 4)] {
        let id = open(GZIP, level, &[]).unwrap();
        let out = write(id, b"a", FINISH).unwrap();
        assert_eq!(out[..10], [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, GZIP_OS]);
        assert_eq!(out[out.len() - 4..], 1u32.to_le_bytes());
    }
}

#[test]
fn sync_and_full_flush_emit_a_decodable_prefix() {
    for flush in [SYNC_FLUSH, FULL_FLUSH] {
        let enc = open(DEFLATERAW, 6, &[]).unwrap();
        let dec = open(INFLATERAW, 6, &[]).unwrap();
        let prefix = write(enc, b"first;", flush).unwrap();
        assert_eq!(write(dec, &prefix, NO_FLUSH).unwrap(), b"first;");
        let tail = write(enc, b"second", FINISH).unwrap();
        assert_eq!(write(dec, &tail, FINISH).unwrap(), b"second");
    }
}

#[test]
fn empty_input_finishes_complete_streams_and_truncated_decoders_fail() {
    for (encode, decode) in [(DEFLATE, INFLATE), (DEFLATERAW, INFLATERAW), (GZIP, GUNZIP)] {
        let packed = run(encode, b"", 1).unwrap();
        assert_eq!(run(decode, &packed, 1).unwrap(), b"");
        assert_eq!(run(decode, &[], 1), Err(ZlibError::Truncated), "{decode}");
    }
    assert_eq!(run(UNZIP, &[], 1), Err(ZlibError::Truncated));
    assert_eq!(run(UNZIP, &[0x1f], 1), Err(ZlibError::Truncated));
}

#[test]
fn truncation_anywhere_in_a_member_is_an_unexpected_end() {
    let packed = run(GZIP, TEXT, 64).unwrap();
    for cut in [5, 12, packed.len() - 6, packed.len() - 1] {
        assert_eq!(
            run(GUNZIP, &packed[..cut], 3),
            Err(ZlibError::Truncated),
            "{cut}"
        );
    }
    let zlib = run(DEFLATE, TEXT, 64).unwrap();
    assert_eq!(
        run(INFLATE, &zlib[..zlib.len() / 2], 3),
        Err(ZlibError::Truncated)
    );
}

#[test]
fn gunzip_parses_optional_header_fields_split_anywhere() {
    let member = gzip_with_fields(TEXT);
    for size in [1, 3, 11, member.len()] {
        assert_eq!(run(GUNZIP, &member, size).unwrap(), TEXT, "{size}");
    }
}

#[test]
fn gunzip_joins_members_and_ignores_trailing_bytes() {
    let mut two = gzip_member(b"one ");
    two.extend(gzip_member(b"two"));
    assert_eq!(run(GUNZIP, &two, 4).unwrap(), b"one two");
    let mut padded = gzip_member(b"padded");
    padded.extend([0, 0, 0, b'x']);
    assert_eq!(run(GUNZIP, &padded, 1).unwrap(), b"padded");
    assert_eq!(run(UNZIP, &padded, 2).unwrap(), b"padded");
}

#[test]
fn malformed_input_is_a_data_error_with_zlibs_message() {
    let header = Err(ZlibError::Data("incorrect header check"));
    assert_eq!(run(GUNZIP, b"nope", 1), header);
    assert_eq!(run(GUNZIP, &[0x1f, 0x00], 1), header);
    let mut method = gzip_member(TEXT);
    method[2] = 7;
    assert_eq!(
        run(GUNZIP, &method, 64),
        Err(ZlibError::Data("unknown compression method"))
    );
    let mut crc = gzip_member(TEXT);
    let at = crc.len() - 8;
    crc[at] ^= 1;
    assert_eq!(
        run(GUNZIP, &crc, 64),
        Err(ZlibError::Data("incorrect data check"))
    );
    let mut size = gzip_member(TEXT);
    let at = size.len() - 1;
    size[at] ^= 1;
    assert_eq!(
        run(GUNZIP, &size, 64),
        Err(ZlibError::Data("incorrect length check"))
    );
    let garbage = [0xff; 12];
    let invalid = Err(ZlibError::Data("invalid compressed data"));
    for mode in [INFLATE, INFLATERAW, UNZIP] {
        assert_eq!(run(mode, &garbage, 4), invalid, "{mode}");
    }
}

#[test]
fn bytes_after_a_finished_deflate_stream_are_ignored() {
    let mut packed = run(DEFLATE, b"hi", 8).unwrap();
    packed.extend_from_slice(b"xyz");
    assert_eq!(run(INFLATE, &packed, 8).unwrap(), b"hi");
}

#[test]
fn large_input_grows_the_output_through_many_rounds() {
    let big: Vec<u8> = (0..200_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for (encode, decode) in [(DEFLATERAW, INFLATERAW), (GZIP, GUNZIP)] {
        let packed = run(encode, &big, 70_000).unwrap();
        assert_eq!(run(decode, &packed, 90_000).unwrap(), big);
    }
}

#[test]
fn handles_close_on_finish_error_close_and_reset() {
    assert_eq!(open(99, 6, &[]), Err(ZlibError::Mode));
    let before = open_count();
    let id = open(DEFLATE, 6, &[]).unwrap();
    assert_eq!(open_count(), before + 1);
    write(id, b"x", FINISH).unwrap();
    assert_eq!(write(id, b"x", NO_FLUSH), Err(ZlibError::Closed));
    let failed = open(INFLATE, 6, &[]).unwrap();
    assert!(write(failed, b"garbage!", NO_FLUSH).is_err());
    assert_eq!(write(failed, b"", FINISH), Err(ZlibError::Closed));
    let closed = open(GZIP, 6, &[]).unwrap();
    close(closed);
    close(closed);
    assert_eq!(write(closed, b"", NO_FLUSH), Err(ZlibError::Closed));
    assert_eq!(open_count(), before);
    open(UNZIP, 20, &[]).unwrap();
    reset();
    assert_eq!(open_count(), 0);
}

#[test]
fn errors_map_to_nodes_message_code_and_errno() {
    let node = |error: ZlibError| {
        let (message, code, errno) = error.node();
        (message, code, errno)
    };
    let cases = [
        (
            ZlibError::Data("incorrect header check"),
            ("incorrect header check", "Z_DATA_ERROR", Some(-3)),
        ),
        (
            ZlibError::Truncated,
            ("unexpected end of file", "Z_BUF_ERROR", Some(-5)),
        ),
        (
            ZlibError::Closed,
            ("zlib binding closed", "ERR_ZLIB_BINDING_CLOSED", None),
        ),
        (
            ZlibError::Brotli {
                errno: -15,
                code: "ERR__ERROR_FORMAT_PADDING_2".into(),
            },
            (
                "Decompression failed",
                "ERR__ERROR_FORMAT_PADDING_2",
                Some(-15),
            ),
        ),
        (
            ZlibError::Init,
            (
                "Initialization failed",
                "ERR_ZLIB_INITIALIZATION_FAILED",
                None,
            ),
        ),
        (
            ZlibError::Mode,
            ("invalid zlib mode", "ERR_INVALID_ARG_VALUE", None),
        ),
    ];
    for (error, (message, code, errno)) in cases {
        assert_eq!(node(error), (message, code.to_string(), errno));
    }
}
