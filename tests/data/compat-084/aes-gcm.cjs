const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const key = Buffer.alloc(32);
const iv = Buffer.alloc(12);
function encrypt(clear, material = key, nonce = iv, aad) {
  const cipher = crypto.createCipheriv('aes-256-gcm', material, nonce);
  if (aad) cipher.setAAD(aad);
  const ciphertext = Buffer.concat([cipher.update(clear), cipher.final()]);
  return { ciphertext, tag: cipher.getAuthTag() };
}
function decrypt(ciphertext, tag, material = key, nonce = iv, aad) {
  const decipher = crypto.createDecipheriv('aes-256-gcm', material, nonce);
  if (aad) decipher.setAAD(aad);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}
testCase('AES256 GCM matches independent known ciphertext and authentication tag', () => {
  const output = encrypt(Buffer.alloc(16));
  assert.equal(output.ciphertext.toString('hex'), 'cea7403d4d606b6e074ec5d3baf39d18');
  assert.equal(output.tag.toString('hex'), 'd0d1c8a799996bf0265b98b5d48ab919');
  assert.deepEqual([...decrypt(output.ciphertext, output.tag)], [...Buffer.alloc(16)]);
});
testCase('Flux secret blob flow uses SHA256 key and distinct random IVs', () => {
  const material = crypto.createHash('sha256').update('unit-test-secret-key').digest();
  const pack = (value) => {
    const nonce = crypto.randomBytes(12);
    const cipher = crypto.createCipheriv('aes-256-gcm', material, nonce);
    const bytes = Buffer.concat([cipher.update(value, 'utf8'), cipher.final()]);
    return [nonce, cipher.getAuthTag(), bytes].map((value) => value.toString('base64')).join('.');
  };
  const unpack = (blob) => {
    const [nonce, tag, ciphertext] = blob.split('.').map((part) => Buffer.from(part, 'base64'));
    return decrypt(ciphertext, tag, material, nonce).toString('utf8');
  };
  const first = pack('sat-certificate-password-🌍'); const second = pack('sat-certificate-password-🌍');
  assert.notEqual(first, second);
  assert.notEqual(first.split('.')[0], second.split('.')[0]);
  assert.equal(unpack(first), 'sat-certificate-password-🌍');
  assert.equal(unpack(second), 'sat-certificate-password-🌍');
});
testCase('GCM rejects tampered ciphertext tag nonce and wrong key', () => {
  const output = encrypt(Buffer.from('secret'));
  const changed = Buffer.from(output.ciphertext); changed[0] ^= 1;
  const tag = Buffer.from(output.tag); tag[0] ^= 1;
  const nonce = Buffer.from(iv); nonce[0] ^= 1;
  for (const call of [
    () => decrypt(changed, output.tag), () => decrypt(output.ciphertext, tag),
    () => decrypt(output.ciphertext, output.tag, Buffer.alloc(32, 1)),
    () => decrypt(output.ciphertext, output.tag, key, nonce),
  ]) assert.throws(call, /unable to authenticate data/i);
});
testCase('Flux cursor additional authenticated data is cryptographically bound', () => {
  const aad = Buffer.from('flux-assistant-cursor-v1');
  const output = encrypt(Buffer.from('{"cursor":42}'), key, iv, aad);
  assert.equal(decrypt(output.ciphertext, output.tag, key, iv, aad).toString(), '{"cursor":42}');
  assert.throws(() => decrypt(output.ciphertext, output.tag), /unable to authenticate data/i);
  assert.throws(() => decrypt(output.ciphertext, output.tag, key, iv, Buffer.from('wrong')), /unable to authenticate data/i);
});
testCase('GCM updates preserve byte view offsets and copy constructor material', () => {
  const material = Buffer.alloc(32); const nonce = Buffer.alloc(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', material, nonce);
  material.fill(1); nonce.fill(1);
  const padded = new Uint8Array([9, 97, 98, 99, 9]);
  const first = cipher.update(padded.subarray(1, 3));
  const second = cipher.update(new DataView(padded.buffer, 3, 1));
  const ciphertext = Buffer.concat([first, second, cipher.final()]);
  assert.equal(first.length, 2); assert.equal(second.length, 1);
  assert.equal(decrypt(ciphertext, cipher.getAuthTag()).toString(), 'abc');
  const rawKey = new Uint8Array(32).buffer; const rawIv = new Uint8Array(12).buffer;
  const rawAAD = new Uint8Array([97, 97, 100]).buffer;
  const rawCipher = crypto.createCipheriv('aes-256-gcm', rawKey, rawIv);
  rawCipher.setAAD(rawAAD);
  const rawBytes = Buffer.concat([rawCipher.update('raw buffers'), rawCipher.final()]);
  const rawDecipher = crypto.createDecipheriv('aes-256-gcm', rawKey, rawIv);
  rawDecipher.setAAD(rawAAD);
  rawDecipher.setAuthTag(Uint8Array.from(rawCipher.getAuthTag()).buffer);
  assert.equal(Buffer.concat([rawDecipher.update(rawBytes), rawDecipher.final()]).toString(), 'raw buffers');
});
testCase('GCM output encodings stream base64 and UTF8 without chunk corruption', () => {
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  let encoded = cipher.update('f', 'utf8', 'base64');
  encoded += cipher.update('oo🌍', 'utf8', 'base64');
  encoded += cipher.final('base64');
  const ciphertext = Buffer.from(encoded, 'base64');
  const decipher = crypto.createDecipheriv('aes-256-gcm', key, iv);
  decipher.setAuthTag(cipher.getAuthTag());
  let clear = '';
  for (let index = 0; index < ciphertext.length; index++) clear += decipher.update(ciphertext.subarray(index, index + 1), undefined, 'utf8');
  clear += decipher.final('utf8');
  assert.equal(clear, 'foo🌍');
});
testCase('GCM supports empty messages and configured 12 through 16 byte tags', () => {
  for (let length = 12; length <= 16; length++) {
    const cipher = crypto.createCipheriv('aes-256-gcm', key, iv, { authTagLength: length });
    assert.equal(cipher.final().length, 0);
    assert.equal(cipher.getAuthTag().length, length);
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, iv, { authTagLength: length });
    decipher.setAuthTag(cipher.getAuthTag());
    assert.equal(decipher.final().length, 0);
  }
});
testCase('GCM checks state transitions rather than returning false successful output', () => {
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  assert.throws(() => cipher.getAuthTag(), { code: 'ERR_CRYPTO_INVALID_STATE' });
  cipher.update('a');
  assert.throws(() => cipher.setAAD(Buffer.from('late')), { code: 'ERR_CRYPTO_INVALID_STATE' });
  cipher.final();
  assert.throws(() => cipher.final()); assert.throws(() => cipher.update('b'));
  const decipher = crypto.createDecipheriv('aes-256-gcm', key, iv);
  decipher.setAuthTag(Buffer.alloc(16));
  assert.throws(() => decipher.setAuthTag(Buffer.alloc(16)), { code: 'ERR_CRYPTO_INVALID_STATE' });
  assert.throws(() => decipher.final(), /unable to authenticate data/i);
  assert.throws(() => decipher.update('b'));
  assert.throws(() => crypto.createDecipheriv('aes-256-gcm', key, iv).final(), /unable to authenticate data/i);
});
testCase('GCM validates algorithms key lengths IVs input types and tag lengths', () => {
  assert.throws(() => crypto.createCipheriv('not-a-cipher', key, iv));
  assert.throws(() => crypto.createCipheriv('aes-256-gcm', Buffer.alloc(31), iv), { code: 'ERR_CRYPTO_INVALID_KEYLEN' });
  assert.throws(() => crypto.createCipheriv('aes-256-gcm', key, Buffer.alloc(0)), { code: 'ERR_CRYPTO_INVALID_IV' });
  assert.throws(() => crypto.createCipheriv('aes-256-gcm', key, iv, {authTagLength: 17}));
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  assert.throws(() => cipher.update({}), TypeError);
  const decipher = crypto.createDecipheriv('aes-256-gcm', key, iv);
  assert.throws(() => decipher.setAuthTag(Buffer.alloc(11)), { code: 'ERR_CRYPTO_INVALID_AUTH_TAG' });
});
testCase('OS-backed randomBytes supports buffers callback delivery and argument errors', async () => {
  assert.equal(Buffer.isBuffer(crypto.randomBytes(0)), true);
  assert.equal(crypto.randomBytes(0).length, 0);
  assert.notEqual(crypto.randomBytes(32).toString('hex'), crypto.randomBytes(32).toString('hex'));
  let synchronous = true;
  await new Promise((resolve, reject) => {
    crypto.randomBytes(12, (error, bytes) => {
      try { assert.ifError(error); assert.equal(synchronous, false); assert.equal(bytes.length, 12); resolve(); }
      catch (failure) { reject(failure); }
    });
    synchronous = false;
  });
  assert.throws(() => crypto.randomBytes('12'), TypeError);
  assert.throws(() => crypto.randomBytes(-1), RangeError);
});
testCase('independent Node verification can decrypt turbo ciphertext and re-encrypt it', () => {
  const material = crypto.createHash('sha256').update('interop material').digest();
  const nonce = Buffer.from('00112233445566778899aabb', 'hex');
  const aad = Buffer.from('interop-aad');
  const clear = Buffer.from('real authenticated encryption 🌍');
  const output = encrypt(clear, material, nonce, aad);
  if (process.env.TURBO_AES_INTEROP_PATH) require('node:fs').writeFileSync(process.env.TURBO_AES_INTEROP_PATH, JSON.stringify({
    key: material.toString('base64'), iv: nonce.toString('base64'), aad: aad.toString('base64'),
    clear: clear.toString('base64'), ciphertext: output.ciphertext.toString('base64'), tag: output.tag.toString('base64'),
  }));
  assert.equal(decrypt(output.ciphertext, output.tag, material, nonce, aad).toString(), clear.toString());
});
