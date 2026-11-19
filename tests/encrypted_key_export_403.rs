//! KeyObject.export with `cipher` / `passphrase` (encrypted PKCS#8), found in a Flux spec that
//! exports a generated key encrypted with aes-256-cbc (turbo-test#403 follow-up). The spec's
//! expected values are Node 26.11's; Node itself then decrypts what turbo-test exported.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn encrypted_pkcs8_exports_match_node_and_decrypt_in_node() {
    let p = Project::new("encrypted-key-export-403");
    p.file("package.json", r#"{"name":"encrypted-key-export-403"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "export.test.cjs",
        include_str!("data/compat-084/encrypted-export.cjs"),
    );
    let out = p.path("exports.json");
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_KEY_EXPORT_PATH", out.to_str().unwrap())],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    let control = std::process::Command::new("node")
        .arg("-e")
        .arg(
            r#"
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const input = JSON.parse(require('node:fs').readFileSync(process.argv[1], 'utf8'));
const plain = Buffer.from(input.plain, 'base64');
const der = crypto.createPrivateKey({ key: Buffer.from(input.der, 'base64'), format: 'der', type: 'pkcs8', passphrase: 'pw' });
const pem = crypto.createPrivateKey({ key: input.pem, passphrase: 'pw' });
for (const key of [der, pem]) assert.ok(key.export({ format: 'der', type: 'pkcs8' }).equals(plain));
assert.throws(() => crypto.createPrivateKey({ key: input.pem, passphrase: 'wrong' }));
"#,
        )
        .arg(&out)
        .output()
        .expect("node");
    assert!(
        control.status.success(),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
}
