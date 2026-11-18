const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const abc = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad';
testCase('SHA256 known answers and default Buffer', () => {
  assert.equal(createHash('sha256').update('abc').digest('hex'), abc);
  const empty = createHash('SHA256').digest();
  assert.equal(Buffer.isBuffer(empty), true);
  assert.equal(empty.toString('hex'), 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
});
testCase('SHA256 incremental byte views preserve offsets and copy update input', () => {
  const view = new Uint8Array([0, 97, 98, 0]);
  const hash = createHash('sha256').update(view.subarray(1, 3));
  view.fill(0);
  assert.equal(hash.update(new DataView(new Uint8Array([0, 99, 0]).buffer, 1, 1)).digest('hex'), abc);
});
testCase('SHA256 input and output encodings hash real bytes', () => {
  for (const [input, encoding] of [['616263', 'hex'], ['YWJj', 'base64'], ['YWJj', 'base64url']]) {
    assert.equal(createHash('sha256').update(input, encoding).digest('hex'), abc);
  }
  assert.equal(createHash('sha256').update('abc').digest('base64'), 'ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=');
  assert.equal(createHash('sha256').update('abc').digest('base64url'), 'ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0');
  assert.equal(createHash('sha256').update('61zz', 'hex').digest('hex'), 'ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb');
  assert.throws(() => createHash('sha256').update('abc', 'hex'), { code: 'ERR_INVALID_ARG_VALUE' });
  assert.equal(createHash('sha256').update('0').digest('utf16le').charCodeAt(8), 0xdbc2);
  assert.equal(createHash('sha256').update('é').digest('hex'), '4a99557e4033c3539de2eb65472017cad5f9557f7a0625a09f1c3f6e2ba69c4c');
});
testCase('SHA256 rejects unsupported algorithms and invalid inputs', () => {
  assert.throws(() => createHash('not-a-hash'));
  assert.throws(() => createHash(null), TypeError);
  assert.throws(() => createHash('sha256').update({}), TypeError);
  assert.equal(createHash('sha256').update('abc', 'unknown').digest('hex'), abc);
  assert.equal(Buffer.isBuffer(createHash('sha256').digest('unknown')), true);
});
testCase('SHA256 finalized state rejects digest and update', () => {
  const hash = createHash('sha256'); hash.digest();
  assert.throws(() => hash.update('abc'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
  assert.throws(() => hash.digest('hex'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
});
testCase('Web Crypto SHA256 returns an ArrayBuffer for view and raw buffer', async () => {
  const subtle = (globalThis.crypto || require('node:crypto').webcrypto).subtle;
  const bytes = new Uint8Array([0, 97, 98, 99, 0]);
  const result = await subtle.digest({ name: 'sha-256' }, bytes.subarray(1, 4));
  assert.equal(result instanceof ArrayBuffer, true);
  assert.equal(Buffer.from(result).toString('hex'), abc);
  assert.equal(Buffer.from(await subtle.digest('SHA-256', new Uint8Array([97, 98, 99]).buffer)).toString('hex'), abc);
  assert.equal(Buffer.from(await subtle.digest('SHA-256', new DataView(bytes.buffer, 1, 3))).toString('hex'), abc);
});
testCase('Web Crypto errors reject promises', async () => {
  const subtle = (globalThis.crypto || require('node:crypto').webcrypto).subtle;
  await assert.rejects(subtle.digest('not-a-hash', new Uint8Array()), { name: 'NotSupportedError' });
  await assert.rejects(subtle.digest('SHA-256', 'abc'), TypeError);
  await assert.rejects(subtle.digest({}, new Uint8Array()), TypeError);
});
