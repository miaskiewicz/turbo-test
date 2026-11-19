// turbo-test#403 follow-up: KeyObject.export of an encrypted PKCS#8 key (expected values: Node 26.11).
const crypto = require('node:crypto');
const fs = require('node:fs');

const ec = crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1' }).privateKey;
const rsa = crypto.generateKeyPairSync('rsa', { modulusLength: 1024 }).privateKey;
const plain = (key) => key.export({ format: 'der', type: 'pkcs8' });
const encrypted = (key, cipher, format = 'der', passphrase = 'pw') => key.export({ format, type: 'pkcs8', cipher, passphrase });
const thrown = (f) => { try { f(); } catch (e) { return [e.name, e.code, e.message]; } return null; };

test('an encrypted DER export is PBES2: PBKDF2-HMAC-SHA256, 2048 rounds, a 16-byte salt and the cipher IV', () => {
  const hex = encrypted(ec, 'aes-256-cbc').toString('hex');
  // Node's own layout for a P-256 key: only the salt, the IV and the ciphertext vary
  expect(hex).toMatch(/^3081f4305f06092a864886f70d01050d3052303106092a864886f70d01050c30240410[0-9a-f]{32}02020800300c06082a864886f70d02090500301d060960864801650304012a0410[0-9a-f]{32}048190[0-9a-f]{288}$/);
  expect(encrypted(ec, 'aes-128-cbc').toString('hex')).toContain('060960864801650304010204');
  expect(encrypted(ec, 'aes-192-cbc').toString('hex')).toContain('060960864801650304011604');
  expect(encrypted(ec, 'des-ede3-cbc').toString('hex')).toContain('06082a864886f70d03070408');
  expect(encrypted(ec, 'aes-256-cbc').equals(encrypted(ec, 'aes-256-cbc'))).toBe(false);
});

test('encrypted exports decrypt back to the key, in DER and PEM, with any cipher spelling', () => {
  for (const key of [ec, rsa]) {
    for (const cipher of ['aes-256-cbc', 'AES-128-CBC', 'aes192', 'des-ede3-cbc']) {
      const der = encrypted(key, cipher);
      expect(crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8', passphrase: 'pw' }).export({ format: 'der', type: 'pkcs8' }).equals(plain(key))).toBe(true);
    }
    const pem = encrypted(key, 'aes-256-cbc', 'pem', Buffer.from('pw'));
    expect(typeof pem).toBe('string');
    expect(pem.split('\n')[0]).toBe('-----BEGIN ENCRYPTED PRIVATE KEY-----');
    expect(crypto.createPrivateKey({ key: pem, passphrase: 'pw' }).export({ format: 'der', type: 'pkcs8' }).equals(plain(key))).toBe(true);
  }
  const empty = encrypted(ec, 'aes-256-cbc', 'der', '');
  expect(crypto.createPrivateKey({ key: empty, format: 'der', type: 'pkcs8', passphrase: '' }).export({ format: 'der', type: 'pkcs8' }).equals(plain(ec))).toBe(true);
});

test('generateKeyPairSync encodes the private key encrypted', () => {
  const { privateKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1', privateKeyEncoding: { format: 'pem', type: 'pkcs8', cipher: 'aes-256-cbc', passphrase: 'pw' } });
  expect(privateKey.startsWith('-----BEGIN ENCRYPTED PRIVATE KEY-----\n')).toBe(true);
  expect(crypto.createPrivateKey({ key: privateKey, passphrase: 'pw' }).asymmetricKeyType).toBe('ec');
});

test('invalid encryption options throw as in Node', () => {
  const base = { format: 'der', type: 'pkcs8' };
  expect(thrown(() => ec.export({ ...base, cipher: 'aes-256-cbc' }))).toEqual(['TypeError', 'ERR_INVALID_ARG_VALUE', "The property 'options.passphrase' is invalid. Received undefined"]);
  expect(thrown(() => ec.export({ ...base, passphrase: 'pw' }))).toEqual(['TypeError', 'ERR_INVALID_ARG_VALUE', "The property 'options.cipher' is required when a passphrase is specified. Received undefined"]);
  expect(thrown(() => ec.export({ ...base, cipher: 'aes-256-cbc', passphrase: 5 }))).toEqual(['TypeError', 'ERR_INVALID_ARG_VALUE', "The property 'options.passphrase' is invalid. Received 5"]);
  expect(thrown(() => ec.export({ ...base, cipher: 'nope', passphrase: 'pw' }))).toEqual(['Error', 'ERR_CRYPTO_UNKNOWN_CIPHER', 'Unknown cipher']);
});

test('hands the exports to Node', () => {
  fs.writeFileSync(process.env.TURBO_KEY_EXPORT_PATH, JSON.stringify({
    plain: plain(rsa).toString('base64'),
    der: encrypted(rsa, 'aes-256-cbc').toString('base64'),
    pem: encrypted(rsa, 'des-ede3-cbc', 'pem'),
  }));
});
