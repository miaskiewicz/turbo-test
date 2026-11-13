const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const message = Buffer.from('flux assistant RS256 input');
let cached;
let encoded;
let unrelated;
function pair() { return cached || (cached = crypto.generateKeyPairSync('rsa', { modulusLength: 2048 })); }
function pemPair() {
  return encoded || (encoded = crypto.generateKeyPairSync('rsa', {
    modulusLength: 2048,
    publicKeyEncoding: { type: 'spki', format: 'pem' },
    privateKeyEncoding: { type: 'pkcs8', format: 'pem' },
  }));
}
testCase('RSA generation returns usable 2048-bit KeyObjects with the real exponent', () => {
  const keys = pair();
  assert.equal(keys.privateKey.type, 'private'); assert.equal(keys.publicKey.type, 'public');
  assert.equal(keys.privateKey instanceof crypto.KeyObject, true);
  assert.equal(keys.publicKey.asymmetricKeyType, 'rsa');
  assert.equal(keys.privateKey.asymmetricKeyType, 'rsa');
  assert.deepEqual(keys.publicKey.asymmetricKeyDetails, { modulusLength: 2048, publicExponent: 65537n });
  assert.deepEqual(keys.privateKey.asymmetricKeyDetails, keys.publicKey.asymmetricKeyDetails);
});
testCase('Flux RSA generation encoding options return real SPKI and PKCS8 PEM keys', () => {
  const keys = pemPair();
  assert.equal(typeof keys.publicKey, 'string'); assert.equal(typeof keys.privateKey, 'string');
  assert.equal(keys.publicKey.startsWith('-----BEGIN PUBLIC KEY-----'), true);
  assert.equal(keys.privateKey.startsWith('-----BEGIN PRIVATE KEY-----'), true);
  assert.equal(crypto.createPublicKey(keys.publicKey).asymmetricKeyType, 'rsa');
  assert.equal(crypto.createPrivateKey(keys.privateKey).asymmetricKeyDetails.modulusLength, 2048);
});
testCase('RSA PEM and DER exports import and derive public material', () => {
  const keys = pair();
  const publicDer = keys.publicKey.export({ format: 'der', type: 'spki' });
  const privateDer = keys.privateKey.export({ format: 'der', type: 'pkcs8' });
  assert.equal(Buffer.isBuffer(publicDer), true); assert.equal(Buffer.isBuffer(privateDer), true);
  const privateKey = crypto.createPrivateKey({ key: privateDer, format: 'der', type: 'pkcs8' });
  const publicKey = crypto.createPublicKey({ key: publicDer, format: 'der', type: 'spki' });
  const derived = crypto.createPublicKey({ key: privateKey });
  assert.deepEqual([...derived.export({ format: 'der', type: 'spki' })], [...publicDer]);
  assert.equal(crypto.verify('sha256', message, publicKey, crypto.sign('sha256', message, privateKey)), true);
});
testCase('Flux RSA-SHA256 update end sign and verify produce exact RS256 compact signatures', () => {
  const keys = pemPair();
  const input = Buffer.from(JSON.stringify({ alg: 'RS256', typ: 'at+jwt', kid: 'k1' })).toString('base64url') + '.' + Buffer.from(JSON.stringify({ sub: 'actor' })).toString('base64url');
  const signer = crypto.createSign('RSA-SHA256');
  assert.equal(signer.update(input), signer); assert.equal(signer.end(), signer);
  const signature = signer.sign(keys.privateKey);
  assert.equal(Buffer.isBuffer(signature), true); assert.equal(signature.length, 256);
  const compact = input + '.' + signature.toString('base64url');
  const parts = compact.split('.');
  const verifier = crypto.createVerify('RSA-SHA256');
  verifier.update(parts[0] + '.' + parts[1]); assert.equal(verifier.end(), verifier);
  assert.equal(verifier.verify(keys.publicKey, Buffer.from(parts[2], 'base64url')), true);
  assert.equal(crypto.verify('RSA-SHA256', Buffer.from(input + 'x'), keys.publicKey, signature), false);
});
testCase('RSA direct signatures reject changed messages wrong keys and malformed signatures', () => {
  const keys = pair();
  unrelated = unrelated || crypto.generateKeyPairSync('rsa', { modulusLength: 2048 });
  const signature = crypto.sign('sha256', message, keys.privateKey);
  assert.deepEqual([...crypto.sign('SHA256', message, keys.privateKey)], [...signature]);
  assert.equal(crypto.verify('sha256', message, keys.publicKey, signature), true);
  assert.equal(crypto.verify('sha256', Buffer.from('changed'), keys.publicKey, signature), false);
  assert.equal(crypto.verify('sha256', message, unrelated.publicKey, signature), false);
  assert.equal(crypto.verify('sha256', message, keys.publicKey, Buffer.from([1, 2, 3])), false);
  assert.notDeepEqual([...unrelated.publicKey.export({ format: 'der', type: 'spki' })], [...keys.publicKey.export({ format: 'der', type: 'spki' })]);
});
testCase('RSA signing preserves sliced input and end data with encoded signatures', () => {
  const keys = pair(); const bytes = Buffer.from([0, 97, 98, 99, 0]);
  const signer = crypto.createSign('RSA-SHA256'); signer.update(bytes.subarray(1, 3)); signer.end('c');
  const signature = signer.sign(keys.privateKey, 'base64');
  const verifier = crypto.createVerify('RSA-SHA256'); verifier.update('a'); verifier.end('bc');
  assert.equal(verifier.verify(keys.publicKey, signature, 'base64'), true);
  assert.equal(crypto.verify('sha256', new Uint8Array([97, 98, 99]).buffer, keys.publicKey, Buffer.from(signature, 'base64')), true);
  assert.throws(() => signer.sign(keys.privateKey));
});
testCase('RSA keys and signatures interoperate with independently generated Node material', () => {
  const inputPath = process.env.TURBO_RSA_NODE_INPUT;
  const input = inputPath ? JSON.parse(require('node:fs').readFileSync(inputPath, 'utf8')) : {
    ...pemPair(), signature: crypto.sign('RSA-SHA256', message, pemPair().privateKey).toString('base64'),
  };
  const imported = crypto.createPrivateKey(input.privateKey);
  const publicKey = crypto.createPublicKey(input.publicKey);
  assert.equal(crypto.verify('RSA-SHA256', message, publicKey, Buffer.from(input.signature, 'base64')), true);
  const signature = crypto.sign('RSA-SHA256', message, imported);
  const outputPath = process.env.TURBO_RSA_NODE_OUTPUT;
  if (outputPath) require('node:fs').writeFileSync(outputPath, JSON.stringify({
    privateKey: pair().privateKey.export({ type: 'pkcs8', format: 'pem' }),
    publicKey: pair().publicKey.export({ type: 'spki', format: 'pem' }),
    signature: crypto.sign('RSA-SHA256', message, pair().privateKey).toString('base64'),
    importedSignature: signature.toString('base64'), publicInput: input.publicKey,
  }));
});
testCase('RSA rejects unsupported digest invalid sizes and malformed material explicitly', () => {
  for (const modulusLength of [undefined, null, '2048', 0, 256, NaN, Infinity]) assert.throws(() => crypto.generateKeyPairSync('rsa', { modulusLength }));
  for (const publicExponent of [0, 2, -1, 1.5]) assert.throws(() => crypto.generateKeyPairSync('rsa', { modulusLength: 2048, publicExponent }));
  assert.throws(() => crypto.createPrivateKey({ key: Buffer.from([1, 2, 3]), type: 'pkcs8', format: 'der' }));
  assert.throws(() => crypto.createPublicKey({ key: Buffer.from([1, 2, 3]), type: 'spki', format: 'der' }));
  assert.throws(() => crypto.sign('not-a-digest', message, pair().privateKey));
  assert.throws(() => crypto.sign('sha256', message, pair().publicKey));
});
