const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const input = require('./certificates.json');
const expected = require('./node-facts.json');
const matching = () => Buffer.from(input.certificates.matching, 'base64');

test('DER certificate facts come from the actual certificate, including expired and alternate serials', () => {
  assert.equal(typeof crypto.X509Certificate, 'function');
  for (const [name, base64] of Object.entries(input.certificates)) {
    const der = Buffer.from(base64, 'base64');
    const certificate = new crypto.X509Certificate(der);
    for (const field of ['serialNumber', 'validFrom', 'validTo', 'subject', 'issuer']) assert.equal(certificate[field], expected[name][field]);
    assert.deepEqual(certificate.raw, der);
  }
  const certificate = new crypto.X509Certificate(matching());
  assert.equal(Buffer.from(certificate.serialNumber, 'hex').toString('latin1'), '30001000000500002031');
  assert.equal(new Date(certificate.validFrom).toISOString().slice(0, 10), '2026-01-01');
  assert.equal(new Date(certificate.validTo).toISOString().slice(0, 10), '2046-01-01');
});

test('PEM string and Buffer inputs serialize to real PEM', () => {
  for (const input of [expected.matching.pem, Buffer.from(expected.matching.pem)]) {
    const certificate = new crypto.X509Certificate(input);
    assert.deepEqual(certificate.raw, matching());
    assert.equal(certificate.toString(), expected.matching.pem);
    assert.equal(certificate.toJSON(), expected.matching.pem);
  }
});

test('typed input offsets are precise and unsupported argument types are rejected', () => {
  const bytes = Buffer.concat([Buffer.from([0, 0]), matching(), Buffer.from([0])]);
  for (const view of [new Uint8Array(bytes.buffer, bytes.byteOffset + 2, bytes.length - 3), new DataView(bytes.buffer, bytes.byteOffset + 2, bytes.length - 3)]) {
    assert.deepEqual(new crypto.X509Certificate(view).raw, matching());
  }
  for (const invalid of [undefined, null, {}, [], 1, new ArrayBuffer(8)]) assert.throws(() => new crypto.X509Certificate(invalid), { code: 'ERR_INVALID_ARG_TYPE' });
});

test('publicKey is a genuine RSA KeyObject and verifies an independent Node signature', () => {
  const certificate = new crypto.X509Certificate(matching());
  const key = certificate.publicKey;
  assert.equal(key instanceof crypto.KeyObject, true);
  assert.equal(key.type, 'public');
  assert.equal(key.asymmetricKeyType, 'rsa');
  assert.equal(key.asymmetricKeyDetails.modulusLength, 2048);
  assert.equal(certificate.publicKey, key);
  assert.equal(key.export({ type: 'spki', format: 'der' }).toString('base64'), expected.matching.spki);
  assert.equal(crypto.verify('RSA-SHA256', Buffer.from(input.message), key, Buffer.from(input.signature, 'base64')), true);
  assert.equal(crypto.verify('RSA-SHA256', Buffer.from('wrong message'), key, Buffer.from(input.signature, 'base64')), false);
});

test('certificate ownership survives input mutation and its raw Buffer does not alter the parsed certificate', () => {
  const source = matching();
  const certificate = new crypto.X509Certificate(source);
  source.fill(0);
  assert.deepEqual(certificate.raw, matching());
  const raw = certificate.raw;
  assert.equal(certificate.raw, raw);
  raw.fill(0);
  assert.equal(certificate.toString(), expected.matching.pem);
  assert.equal(certificate.publicKey.export({ type: 'spki', format: 'der' }).toString('base64'), expected.matching.spki);
});

test('invalid DER and PEM are rejected while trailing DER bytes remain visible to the caller raw-length check', () => {
  for (const invalid of [Buffer.alloc(0), Buffer.from([0x30, 0x01, 0x00]), matching().subarray(0, 30), matching().subarray(0, matching().length - 1), '-----BEGIN CERTIFICATE-----\nbroken\n-----END CERTIFICATE-----']) assert.throws(() => new crypto.X509Certificate(invalid));
  const certificate = new crypto.X509Certificate(Buffer.concat([matching(), Buffer.from('trailing')]));
  assert.deepEqual(certificate.raw, matching());
});
