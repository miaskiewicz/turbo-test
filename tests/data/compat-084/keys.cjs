const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const pair = (options = {}) => crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1', ...options });
const message = Buffer.from('disposable EC signing key');

testCase('EC generation returns real public and private KeyObjects', () => {
  const { publicKey, privateKey } = pair();
  assert.equal(publicKey.type, 'public'); assert.equal(privateKey.type, 'private');
  assert.equal(publicKey.asymmetricKeyType, 'ec'); assert.equal(privateKey.asymmetricKeyType, 'ec');
  assert.equal(publicKey.asymmetricKeyDetails.namedCurve, 'prime256v1');
  assert.equal(privateKey.asymmetricKeyDetails.namedCurve, 'prime256v1');
});
testCase('EC generation creates distinct pairs and matching usable keys', () => {
  const a = pair(); const b = pair();
  assert.notEqual(a.privateKey.export({ type: 'pkcs8', format: 'pem' }), b.privateKey.export({ type: 'pkcs8', format: 'pem' }));
  const signature = crypto.sign('sha256', message, a.privateKey);
  assert.equal(crypto.verify('sha256', message, a.publicKey, signature), true);
  assert.equal(crypto.verify('sha256', message, b.publicKey, signature), false);
});
testCase('EC keys export interoperable PEM and DER encodings', () => {
  const { publicKey, privateKey } = pair();
  const publicPem = publicKey.export({ type: 'spki', format: 'pem' });
  const privatePem = privateKey.export({ type: 'pkcs8', format: 'pem' });
  assert.equal(publicPem.startsWith('-----BEGIN PUBLIC KEY-----'), true);
  assert.equal(privatePem.startsWith('-----BEGIN PRIVATE KEY-----'), true);
  assert.equal(Buffer.isBuffer(publicKey.export({ type: 'spki', format: 'der' })), true);
  assert.equal(Buffer.isBuffer(privateKey.export({ type: 'pkcs8', format: 'der' })), true);
  const signature = crypto.sign('SHA256', message, privateKey);
  assert.equal(crypto.verify('SHA256', message, publicKey, signature), true);
  const output = process.env.TURBO_KEY_INTEROP_PATH;
  if (output) require('node:fs').writeFileSync(output, JSON.stringify({
    publicPem, privatePem,
    publicDer: publicKey.export({ type: 'spki', format: 'der' }).toString('base64'),
    privateDer: privateKey.export({ type: 'pkcs8', format: 'der' }).toString('base64'),
    signature: signature.toString('base64'), message: message.toString('base64'),
  }));
});
testCase('EC generation encoding options return usable encoded keys', () => {
  const encoded = pair({ publicKeyEncoding: { type: 'spki', format: 'pem' }, privateKeyEncoding: { type: 'pkcs8', format: 'pem' } });
  assert.equal(typeof encoded.publicKey, 'string'); assert.equal(typeof encoded.privateKey, 'string');
  const signature = crypto.sign('sha256', message, encoded.privateKey);
  assert.equal(crypto.verify('sha256', message, encoded.publicKey, signature), true);
  const der = pair({ publicKeyEncoding: { type: 'spki', format: 'der' }, privateKeyEncoding: { type: 'pkcs8', format: 'der' } });
  assert.equal(Buffer.isBuffer(der.publicKey), true); assert.equal(Buffer.isBuffer(der.privateKey), true);
  assert.equal(crypto.verify('sha256', message, { key: der.publicKey, type: 'spki', format: 'der' }, crypto.sign('sha256', message, { key: der.privateKey, type: 'pkcs8', format: 'der' })), true);
});
testCase('EC signing and verification preserve input byte views and reject changed messages', () => {
  const keys = pair(); const bytes = Buffer.from([0, 97, 98, 99, 0]);
  const signature = crypto.sign('sha256', bytes.subarray(1, 4), keys.privateKey);
  assert.equal(Buffer.isBuffer(signature), true);
  assert.equal(crypto.verify('sha256', Buffer.from('abc'), keys.publicKey, signature), true);
  assert.equal(crypto.verify('sha256', Buffer.from('abd'), keys.publicKey, signature), false);
  assert.equal(crypto.verify('sha256', Buffer.from('abc'), keys.publicKey, Buffer.from([1, 2, 3])), false);
});
testCase('EC streaming sign and verify support incremental bytes and signature encodings', () => {
  const keys = pair();
  const signer = crypto.createSign('SHA256');
  assert.equal(signer.update('ab'), signer); signer.update(Buffer.from('c'));
  const signature = signer.sign(keys.privateKey, 'base64');
  const verifier = crypto.createVerify('sha256');
  assert.equal(verifier.update('a'), verifier); verifier.update('bc');
  assert.equal(verifier.verify(keys.publicKey, signature, 'base64'), true);
  assert.throws(() => signer.sign(keys.privateKey));
  assert.throws(() => verifier.verify(keys.publicKey, signature, 'base64'));
});
testCase('EC exported keys import for further signing and public derivation', () => {
  const keys = pair();
  const importedPrivate = crypto.createPrivateKey(keys.privateKey.export({ type: 'pkcs8', format: 'pem' }));
  const importedPublic = crypto.createPublicKey({ key: keys.publicKey.export({ type: 'spki', format: 'der' }), type: 'spki', format: 'der' });
  const derivedPublic = crypto.createPublicKey(importedPrivate);
  const signature = crypto.sign('sha256', message, importedPrivate);
  assert.equal(crypto.verify('sha256', message, importedPublic, signature), true);
  assert.equal(crypto.verify('sha256', message, derivedPublic, signature), true);
});
testCase('EC options and key material errors never manufacture successful keys', () => {
  assert.throws(() => crypto.generateKeyPairSync('not-a-key-type', {}));
  assert.throws(() => crypto.generateKeyPairSync('ec', {}));
  assert.throws(() => pair({ namedCurve: 'not-a-curve' }));
  assert.throws(() => pair({ publicKeyEncoding: { type: 'pkcs8', format: 'pem' } }));
  assert.throws(() => pair({ privateKeyEncoding: { type: 'spki', format: 'pem' } }));
  assert.throws(() => crypto.createPrivateKey('not a key'));
  assert.throws(() => crypto.sign('not-a-digest', message, pair().privateKey));
  assert.throws(() => crypto.sign('sha256', message, pair().publicKey));
});
