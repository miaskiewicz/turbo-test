//! Authenticated AES-256-GCM and OS entropy regressions for real Flux encryption flows (#350).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn aes_gcm_authentication_and_node_interoperability() {
    let p = Project::new("aes-gcm-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"aes-gcm-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("aes.test.cjs", include_str!("data/compat-084/aes-gcm.cjs"));
    let receipt = p.path("interop.json");
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_AES_INTEROP_PATH", receipt.to_str().unwrap())],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 11, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(r#"
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const raw = JSON.parse(require('node:fs').readFileSync(process.argv[1], 'utf8'));
const value = Object.fromEntries(Object.entries(raw).map(([name, bytes]) => [name, Buffer.from(bytes, 'base64')]));
const decipher = crypto.createDecipheriv('aes-256-gcm', value.key, value.iv);
decipher.setAAD(value.aad); decipher.setAuthTag(value.tag);
assert.deepEqual(Buffer.concat([decipher.update(value.ciphertext), decipher.final()]), value.clear);
const cipher = crypto.createCipheriv('aes-256-gcm', value.key, value.iv);
cipher.setAAD(value.aad);
assert.deepEqual(Buffer.concat([cipher.update(value.clear), cipher.final()]), value.ciphertext);
assert.deepEqual(cipher.getAuthTag(), value.tag);
"#)
        .arg(receipt)
        .output()
        .expect("Node is required for independent AES-GCM verification");
    assert!(
        output.status.success(),
        "Node AES control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
