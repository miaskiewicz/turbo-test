//! Real EC key generation and ECDSA signing controls (#318), including independent Node parsing.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn ec_keys_are_usable_and_interoperate_with_node() {
    let p = Project::new("keys-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"keys-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("keys.test.cjs", include_str!("data/compat-084/keys.cjs"));
    let output_path = p.path("interop.json");
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_KEY_INTEROP_PATH", output_path.to_str().unwrap())],
    );
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 8, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
    let control = std::process::Command::new("node")
        .arg("-e")
        .arg(r#"
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const input = JSON.parse(require('node:fs').readFileSync(process.argv[1], 'utf8'));
const publicPem = crypto.createPublicKey(input.publicPem);
const privatePem = crypto.createPrivateKey(input.privatePem);
const publicDer = crypto.createPublicKey({key: Buffer.from(input.publicDer, 'base64'), format: 'der', type: 'spki'});
const privateDer = crypto.createPrivateKey({key: Buffer.from(input.privateDer, 'base64'), format: 'der', type: 'pkcs8'});
const message = Buffer.from(input.message, 'base64');
const signature = Buffer.from(input.signature, 'base64');
for (const publicKey of [publicPem, publicDer]) {
  assert.equal(crypto.verify('sha256', message, publicKey, signature), true);
  for (const privateKey of [privatePem, privateDer]) assert.equal(crypto.verify('sha256', message, publicKey, crypto.sign('sha256', message, privateKey)), true);
  assert.equal(crypto.verify('sha256', Buffer.from('changed'), publicKey, signature), false);
}
"#)
        .arg(&output_path)
        .output()
        .expect("Node is required for independent EC interoperability verification");
    assert!(
        control.status.success(),
        "Node EC control: {}",
        String::from_utf8_lossy(&control.stderr)
    );
}
