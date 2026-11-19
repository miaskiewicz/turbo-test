//! Real zlib bytes, checked by an independent Node decoder.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::process::Command;

#[test]
fn deflate_and_inflate_exchange_real_bytes_with_node() {
    let project = Project::new("zlib-deflate-084");
    project.with_esbuild();
    project.file("package.json", r#"{"name":"zlib-deflate-084"}"#);
    project.file(
        "jest.config.js",
        "module.exports = {testEnvironment:'node'};",
    );
    let oracle = Command::new("node")
        .args(["-e", "const z=require('node:zlib');const b=Buffer.from('turbo-test-zlib-'.repeat(64)+'\\0☃');console.log(JSON.stringify({clear:b.toString('hex'),compressed:z.deflateSync(b).toString('hex')}));"])
        .output()
        .unwrap();
    assert!(oracle.status.success());
    let oracle_json: serde_json::Value = serde_json::from_slice(&oracle.stdout).unwrap();
    let output = project.dir.join("compressed.bin");
    let source = include_str!("data/compat-084/zlib-deflate.cjs")
        .replace(
            "__ORACLE__",
            String::from_utf8(oracle.stdout).unwrap().trim(),
        )
        .replace("__OUTPUT__", &serde_json::to_string(&output).unwrap());
    let fixture = project.file("zlib.test.cjs", &source);
    let control = Command::new("node")
        .args([
            "-e",
            "global.test=(_name,fn)=>fn();require(process.argv[1]);",
        ])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        control.status.success(),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
    std::fs::remove_file(&output).unwrap();
    let run = turbo_test(&project.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    let independent = Command::new("node")
        .args(["-e", "const fs=require('node:fs'),z=require('node:zlib'),a=require('node:assert/strict');a.equal(z.inflateSync(fs.readFileSync(process.argv[1])).toString('hex'),process.argv[2]);"])
        .arg(&output)
        .arg(oracle_json["clear"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        independent.status.success(),
        "{}",
        String::from_utf8_lossy(&independent.stderr)
    );
}

#[test]
fn unsupported_deflate_options_report_capability_errors() {
    let project = Project::new("zlib-deflate-bounds-084");
    project.with_esbuild();
    project.file("package.json", r#"{"name":"zlib-deflate-bounds-084"}"#);
    project.file(
        "jest.config.js",
        "module.exports = {testEnvironment:'node'};",
    );
    project.file("bounds.test.cjs", r#"
const assert = require('node:assert/strict');
const zlib = require('node:zlib');
test('unsupported options throw instead of silently ignoring them', () => {
  for (const name of ['dictionary', 'windowBits', 'memLevel', 'strategy', 'flush', 'finishFlush', 'maxOutputLength']) {
    assert.throws(() => zlib.deflateSync('input', {[name]: 1}), {code:'ERR_TURBO_UNSUPPORTED_ZLIB_OPTION'});
    assert.throws(() => zlib.createDeflateRaw({[name]: 1}), {code:'ERR_TURBO_UNSUPPORTED_ZLIB_OPTION'});
    assert.throws(() => zlib.gzip('input', {[name]: 1}, () => {}), {code:'ERR_TURBO_UNSUPPORTED_ZLIB_OPTION'});
  }
  assert.throws(() => new zlib.Gzip({level: 10}), RangeError);
});
"#);
    let run = turbo_test(&project.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
