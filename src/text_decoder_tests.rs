use super::*;

fn once(encoding: &str, ignore_bom: bool, fatal: bool, bytes: &[u8]) -> Option<String> {
    decode(new_id(), encoding, ignore_bom, fatal, false, bytes)
}

#[test]
fn labels_name_their_encoding_as_node_reports_it() {
    assert_eq!(encoding_name("latin1").as_deref(), Some("windows-1252"));
    assert_eq!(encoding_name("UTF-16").as_deref(), Some("utf-16le"));
    assert_eq!(encoding_name(" sjis\n").as_deref(), Some("shift_jis"));
    assert_eq!(
        encoding_name("x-user-defined").as_deref(),
        Some("x-user-defined")
    );
    assert_eq!(encoding_name("iso-2022-kr").as_deref(), Some("replacement"));
    assert_eq!(encoding_name("utf8").as_deref(), Some("utf-8"));
    assert_eq!(encoding_name("latin-1"), None);
    assert_eq!(encoding_name(""), None);
}

#[test]
fn ids_are_fresh() {
    let a = new_id();
    assert_eq!(new_id(), a + 1);
}

#[test]
fn decodes_replacing_or_failing_on_malformed_input() {
    assert_eq!(
        once("windows-1252", false, false, &[0x80, 0xe9]).unwrap(),
        "€é"
    );
    assert_eq!(
        once("utf-16le", false, false, &[0x41, 0, 0x42]).unwrap(),
        "A\u{fffd}"
    );
    assert_eq!(once("utf-16le", false, true, &[0x41, 0, 0x42]), None);
    assert_eq!(
        once("utf-16le", false, true, &[0x3d, 0xd8, 0, 0xde]).unwrap(),
        "😀"
    );
    assert_eq!(once("iso-8859-3", false, true, &[0xa5]), None);
    assert_eq!(once("no-such-encoding", false, false, &[0x41]), None);
}

#[test]
fn strips_the_encodings_own_bom_unless_ignored() {
    assert_eq!(
        once("utf-16le", false, false, &[0xff, 0xfe, 0x41, 0]).unwrap(),
        "A"
    );
    assert_eq!(
        once("utf-16le", true, false, &[0xff, 0xfe, 0x41, 0]).unwrap(),
        "\u{feff}A"
    );
    assert_eq!(
        once("utf-16be", false, false, &[0xfe, 0xff, 0, 0x41]).unwrap(),
        "A"
    );
    assert_eq!(
        once("windows-1252", false, false, &[0xef, 0xbb, 0xbf]).unwrap(),
        "ï»¿"
    );
}

#[test]
fn a_stream_keeps_its_decoder_until_the_final_call() {
    let id = new_id();
    assert_eq!(
        decode(id, "shift_jis", false, false, true, &[0x41, 0x82]).unwrap(),
        "A"
    );
    assert_eq!(
        decode(id, "shift_jis", false, false, true, &[0xa0]).unwrap(),
        "あ"
    );
    assert_eq!(
        decode(id, "shift_jis", false, false, true, &[0x82]).unwrap(),
        ""
    );
    assert_eq!(
        decode(id, "shift_jis", false, false, false, &[]).unwrap(),
        "\u{fffd}"
    );
    // closed: the next call starts afresh (a BOM is stripped again)
    let id = new_id();
    assert_eq!(
        decode(id, "utf-16le", false, false, true, &[0xff, 0xfe, 0x41]).unwrap(),
        ""
    );
    assert_eq!(
        decode(id, "utf-16le", false, false, false, &[0]).unwrap(),
        "A"
    );
    assert_eq!(
        decode(id, "utf-16le", false, false, false, &[0xff, 0xfe, 0x42, 0]).unwrap(),
        "B"
    );
}

#[test]
fn a_fatal_error_closes_the_stream() {
    let id = new_id();
    assert_eq!(
        decode(id, "shift_jis", false, true, true, &[0x82]).unwrap(),
        ""
    );
    assert_eq!(decode(id, "shift_jis", false, true, false, &[0x20]), None);
    assert_eq!(
        decode(id, "shift_jis", false, true, false, &[0x82, 0xa0]).unwrap(),
        "あ"
    );
}
