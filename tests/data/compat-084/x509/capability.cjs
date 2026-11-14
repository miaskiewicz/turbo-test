const assert = require('node:assert/strict');
const { X509Certificate } = require('node:crypto');
const input = require('./certificates.json');
it('parsing a certificate does not claim signature or trust verification support', () => {
  const certificate = new X509Certificate(Buffer.from(input.certificates.matching, 'base64'));
  assert.throws(() => certificate.verify(certificate.publicKey), { code: 'ERR_X509_UNSUPPORTED_OPERATION' });
});
