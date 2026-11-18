#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn native_crypto_boundaries_reject_invalid_guest_arguments() {
    let p = Project::new("native-crypto-boundaries-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"native-crypto-boundaries-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("boundaries.test.cjs",r#"
const assert=require('node:assert/strict');
test('native byte view contracts fail before cryptographic work',()=>{
 assert.throws(()=>globalThis.__ttSha1('bad'),/ArrayBufferView/);
 assert.throws(()=>globalThis.__ttSha256('bad'),/ArrayBufferView/);
 assert.throws(()=>globalThis.__ttX509('bad'),/byte view/);
 assert.throws(()=>globalThis.__ttAesGcm(false,new Uint8Array(31),new Uint8Array(12),new Uint8Array(),new Uint8Array(),16),/32-byte key/);
 assert.throws(()=>globalThis.__ttRandomBytes(-1),/Invalid random byte size/);
 assert.throws(()=>globalThis.__ttCryptoHashes('unknown',new Uint8Array(),new Uint8Array(),0,0),/Unsupported keyed hash/);
});
test('plaintext private key imports ignore an irrelevant passphrase',()=>{
 const crypto=require('node:crypto');const pair=crypto.generateKeyPairSync('ec',{namedCurve:'prime256v1'});
 const der=pair.privateKey.export({format:'der',type:'pkcs8'});
 assert.equal(crypto.createPrivateKey({key:der,format:'der',type:'pkcs8',passphrase:'unused'}).asymmetricKeyType,'ec');
});
"#);
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 2, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
