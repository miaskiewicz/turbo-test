const fs = require('node:fs');
const crypto = require('node:crypto');
const input = require('./certificates.json');
const expected = {};
for (const [name, base64] of Object.entries(input.certificates)) {
  const certificate = new crypto.X509Certificate(Buffer.from(base64, 'base64'));
  expected[name] = {
    serialNumber: certificate.serialNumber,
    validFrom: certificate.validFrom,
    validTo: certificate.validTo,
    subject: certificate.subject,
    issuer: certificate.issuer,
    pem: certificate.toString(),
    spki: certificate.publicKey.export({ type: 'spki', format: 'der' }).toString('base64'),
  };
}
require('node:assert/strict').equal(crypto.verify('RSA-SHA256', Buffer.from(input.message), new crypto.X509Certificate(Buffer.from(input.certificates.matching, 'base64')).publicKey, Buffer.from(input.signature, 'base64')), true);
fs.writeFileSync(__dirname + '/node-facts.json', JSON.stringify(expected));
