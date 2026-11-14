// Real certificate facts are parsed natively; public keys use the owned crypto registry.
const __x509Records = new WeakMap();
function __x509Input(input) {
  if (typeof input === 'string') return Buffer.from(input, 'utf8');
  if (!ArrayBuffer.isView(input)) throw Object.assign(new TypeError('X509Certificate input must be a string, Buffer, TypedArray or DataView'), { code: 'ERR_INVALID_ARG_TYPE' });
  return Buffer.from(new Uint8Array(input.buffer, input.byteOffset, input.byteLength));
}
function __x509Der(input) {
  const bytes = __x509Input(input);
  if (bytes[0] === 0x30) return bytes;
  const text = new TextDecoder().decode(bytes);
  const match = /^\s*-----BEGIN CERTIFICATE-----\s+([A-Za-z0-9+/=\s]+)-----END CERTIFICATE-----/.exec(text);
  return match ? Buffer.from(match[1].replace(/\s/g, ''), 'base64') : bytes;
}
function __x509Pem(der) {
  const body = Buffer.from(der).toString('base64').match(/.{1,64}/g).join('\n');
  return '-----BEGIN CERTIFICATE-----\n' + body + '\n-----END CERTIFICATE-----\n';
}
class __X509Certificate {
  constructor(input) {
    const facts = globalThis.__ttX509(__x509Der(input));
    __x509Records.set(this, { facts, raw: Buffer.from(facts.raw), pem: __x509Pem(facts.raw), key: undefined });
  }
  get raw() { return __x509Records.get(this).raw; }
  get serialNumber() { return __x509Records.get(this).facts.serialNumber; }
  get validFrom() { return __x509Records.get(this).facts.validFrom; }
  get validTo() { return __x509Records.get(this).facts.validTo; }
  get subject() { return __x509Records.get(this).facts.subject; }
  get issuer() { return __x509Records.get(this).facts.issuer; }
  get publicKey() {
    const record = __x509Records.get(this);
    if (!record.key) record.key = __createPublicKey({ key: record.facts.spki, format: 'der', type: 'spki' });
    return record.key;
  }
  toString() { return __x509Records.get(this).pem; }
  toJSON() { return this.toString(); }
  verify() { throw Object.assign(new Error('turbo-test: X.509 signature and trust verification is not supported'), { code: 'ERR_X509_UNSUPPORTED_OPERATION' }); }
}
const __nodeX509 = { X509Certificate: __X509Certificate };
