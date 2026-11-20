//! The persistent TS worker keeps a single `node` + `require("typescript")` alive
//! across many files, so a spec with hundreds of decorated models stops paying the
//! ~300 ms cold `require("typescript")` cost per file (turbo-test#431).

#![cfg(unix)]

use super::{
    decode_escape, decode_response, decode_unicode_escape, encode_request, json_quote, spawn_count,
    transform,
};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ts_dir() -> PathBuf {
    repo_root().join("node_modules/typescript")
}

const DECORATED: &str = r#"
function Tag<T extends Function>(ctor: T): T { return ctor; }
@Tag
export class Service {
  value = 1;
}
"#;

#[test]
fn transform_lowers_legacy_decorators_through_the_persistent_worker() {
    let root = repo_root();
    let out = transform(
        &root,
        &ts_dir(),
        &root.join("service.ts"),
        "commonjs",
        DECORATED,
    )
    .expect("worker transform returned None");
    assert!(
        out.contains("__decorate"),
        "decorator lowering missing: {out}"
    );
    assert!(
        out.contains("Reflect.metadata")
            || out.contains("__metadata")
            || out.contains("__decorate"),
        "decorator / metadata helper missing: {out}"
    );
    assert!(
        out.contains("exports.Service"),
        "commonjs export missing: {out}"
    );
}

#[test]
fn json_quote_escapes_every_json_control_sequence() {
    // Every JSON escape branch the worker emits: covers json_quote arms individually.
    let mut out = String::new();
    json_quote(&mut out, "a \"q\" b");
    assert_eq!(out, r#""a \"q\" b""#);
    out.clear();
    json_quote(&mut out, r"back\slash");
    assert_eq!(out, r#""back\\slash""#);
    out.clear();
    json_quote(&mut out, "line\n\r\tnext");
    assert_eq!(out, r#""line\n\r\tnext""#);
    out.clear();
    // U+0001 is below 0x20 and has no dedicated escape → \u0001
    json_quote(&mut out, "ctrl\u{0001}end");
    assert_eq!(out, "\"ctrl\\u0001end\"");
    // Non-ASCII above 0x7F is emitted verbatim (JSON allows raw UTF-8)
    out.clear();
    json_quote(&mut out, "☃");
    assert_eq!(out, "\"☃\"");
}

#[test]
fn decode_response_parses_each_json_string_escape() {
    // Every decode_escape branch, including \u for a code unit.
    let body = r#"{"code":"q\"\\/n\n\r\t\b\fA"}"#.as_bytes();
    assert_eq!(
        decode_response(body).as_deref(),
        Some("q\"\\/n\n\r\t\u{08}\u{0c}A")
    );
    // An unknown escape letter → decode_escape returns None, decode_response bails.
    let bad = r#"{"code":"x\z"}"#.as_bytes();
    assert_eq!(decode_response(bad), None);
    // The response's shape must carry the "code":"..." marker; other shapes → None.
    assert_eq!(decode_response(r#"{"error":"nope"}"#.as_bytes()), None);
    // Non-UTF-8 → None.
    assert_eq!(decode_response(&[0xffu8, 0xfeu8]), None);
    // decode_unicode_escape's short-sequence guard: fewer than 4 hex digits → None.
    let mut short = "12".chars();
    assert_eq!(decode_unicode_escape(&mut short), None);
    // Bare decode_escape: `None` on empty iterator.
    let mut empty = "".chars();
    assert_eq!(decode_escape(&mut empty), None);
}

#[test]
fn encode_request_builds_json_frame_shape_the_worker_decodes() {
    let body = encode_request(Path::new("/tmp/x.ts"), "commonjs", "const a = 1;\n");
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.starts_with(r#"{"file":"/tmp/x.ts","moduleKind":"commonjs","src":"#));
    assert!(text.ends_with(r#""}"#));
    assert!(text.contains(r#""src":"const a = 1;\n""#));
}

#[test]
fn many_files_reuse_a_single_worker_subprocess() {
    let root = repo_root();
    let ts_dir = ts_dir();
    let before = spawn_count();
    for i in 0..8 {
        let src = format!(
            "function Tag<T extends Function>(ctor: T): T {{ return ctor; }} @Tag export class K{i} {{ v = {i}; }}\n"
        );
        let file = root.join(format!("k{i}.ts"));
        let code = transform(&root, &ts_dir, &file, "commonjs", &src)
            .unwrap_or_else(|| panic!("worker transform returned None for k{i}"));
        assert!(code.contains(&format!("exports.K{i}")));
        assert!(code.contains("__decorate"));
    }
    let spawned = spawn_count() - before;
    assert!(
        spawned <= 1,
        "the persistent worker should serve many files from one node subprocess; spawned {spawned} times"
    );
}
