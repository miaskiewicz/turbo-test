//! Real Flux Assistant RSA-SHA256 usage and independent Node interoperability (#349).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn node_control(p: &Project, source: &str, filename: &std::path::Path) {
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(source)
        .arg(filename)
        .current_dir(&p.dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Node RSA control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rsa_generation_exports_and_flux_signatures_interoperate_with_node() {
    let p = Project::new("rsa-flux-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"rsa-flux-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("rsa.test.cjs", include_str!("data/compat-084/rsa.cjs"));
    let input = p.path("node-input.json");
    node_control(
        &p,
        r#"
const c = require('node:crypto'); const fs = require('node:fs');
const keys = c.generateKeyPairSync('rsa', { modulusLength: 2048, publicKeyEncoding: { type: 'spki', format: 'pem' }, privateKeyEncoding: { type: 'pkcs8', format: 'pem' } });
fs.writeFileSync(process.argv[1], JSON.stringify({ ...keys, signature: c.sign('RSA-SHA256', Buffer.from('flux assistant RS256 input'), keys.privateKey).toString('base64') }));
"#,
        &input,
    );
    let output = p.path("turbo-output.json");
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[
            ("TURBO_RSA_NODE_INPUT", input.to_str().unwrap()),
            ("TURBO_RSA_NODE_OUTPUT", output.to_str().unwrap()),
        ],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    node_control(
        &p,
        r#"
const assert = require('node:assert/strict'); const c = require('node:crypto');
const input = JSON.parse(require('node:fs').readFileSync(process.argv[1], 'utf8'));
const message = Buffer.from('flux assistant RS256 input');
const privateKey = c.createPrivateKey(input.privateKey); const publicKey = c.createPublicKey(input.publicKey);
assert.equal(publicKey.asymmetricKeyDetails.modulusLength, 2048);
assert.equal(c.verify('RSA-SHA256', message, publicKey, Buffer.from(input.signature, 'base64')), true);
assert.equal(c.verify('RSA-SHA256', message, input.publicInput, Buffer.from(input.importedSignature, 'base64')), true);
assert.equal(c.verify('RSA-SHA256', Buffer.from('changed'), publicKey, Buffer.from(input.signature, 'base64')), false);
assert.equal(c.verify('RSA-SHA256', message, publicKey, c.sign('RSA-SHA256', message, privateKey)), true);
"#,
        &output,
    );
}
