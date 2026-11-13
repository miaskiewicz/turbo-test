// Owned P-256 KeyObjects. Native __ttEc uses OS entropy and RustCrypto ECDSA/PKCS#8.
const __keyRecords = new WeakMap();
const __keyToken = {};
function __keyUnsupported(message) { throw new Error('turbo-test: ' + message + ' is not supported'); }
function __keyOptions(options) {
  if (!options || typeof options !== 'object') throw new TypeError('Key options must be an object');
  return options;
}
function __keyExportOptions(type, options) {
  __keyOptions(options);
  const expected = type === 'private' ? 'pkcs8' : 'spki';
  if (options.type !== expected) __keyUnsupported('EC ' + type + ' export type ' + options.type);
  if (options.format !== 'pem' && options.format !== 'der') __keyUnsupported('Key format ' + options.format);
  if (options.cipher !== undefined || options.passphrase !== undefined) __keyUnsupported('Encrypted key export');
  return options;
}
function __keyPem(bytes, type) {
  const label = type === 'private' ? 'PRIVATE KEY' : 'PUBLIC KEY';
  const body = Buffer.from(bytes).toString('base64').match(/.{1,64}/g).join('\n');
  return '-----BEGIN ' + label + '-----\n' + body + '\n-----END ' + label + '-----\n';
}
class __KeyObject {
  constructor(token, type, der) {
    if (token !== __keyToken) throw new TypeError('KeyObject cannot be constructed directly');
    __keyRecords.set(this, { type, der: Buffer.from(der) });
    Object.defineProperties(this, {
      type: { value: type, enumerable: true },
      asymmetricKeyType: { value: 'ec', enumerable: true },
    });
  }
  get asymmetricKeyDetails() { return { namedCurve: 'prime256v1' }; }
  export(options) {
    const record = __keyRecords.get(this);
    __keyExportOptions(record.type, options);
    return options.format === 'der' ? Buffer.from(record.der) : __keyPem(record.der, record.type);
  }
}
function __keyDecodePem(input) {
  const text = typeof input === 'string' ? input : new TextDecoder().decode(input);
  const match = /^\s*-----BEGIN (PRIVATE KEY|PUBLIC KEY)-----\s+([A-Za-z0-9+/=\s]+)-----END \1-----\s*$/.exec(text);
  if (!match) throw new Error('Invalid or unsupported PEM key');
  return { type: match[1] === 'PRIVATE KEY' ? 'private' : 'public', der: Buffer.from(match[2].replace(/\s/g, ''), 'base64') };
}
function __keyDecodeDer(descriptor) {
  if (descriptor.format !== 'der') __keyUnsupported('Key format ' + descriptor.format);
  if (descriptor.type !== 'spki' && descriptor.type !== 'pkcs8') __keyUnsupported('Key encoding type ' + descriptor.type);
  if (!ArrayBuffer.isView(descriptor.key)) throw new TypeError('DER key must be a Buffer or TypedArray');
  const der = new Uint8Array(descriptor.key.buffer, descriptor.key.byteOffset, descriptor.key.byteLength);
  return { type: descriptor.type === 'pkcs8' ? 'private' : 'public', der };
}
function __keyDecode(input) {
  const descriptor = __keyDescriptor(input);
  if (descriptor.passphrase !== undefined) __keyUnsupported('Encrypted key import');
  if (descriptor.format === undefined || descriptor.format === 'pem') return __keyDecodePem(descriptor.key);
  return __keyDecodeDer(descriptor);
}
function __keyDescriptor(input) {
  return input && typeof input === 'object' && !ArrayBuffer.isView(input) ? input : { key: input };
}
function __createPrivateKey(input) {
  if (__keyRecords.has(input)) throw new TypeError('createPrivateKey requires encoded key material');
  const decoded = __keyDecode(input);
  if (decoded.type !== 'private') throw new TypeError('A private key is required');
  return new __KeyObject(__keyToken, 'private', globalThis.__ttEc('private', decoded.der));
}
function __keyObjectRecord(input) {
  return __keyRecords.get(input) || (input && __keyRecords.get(input.key));
}
function __createPublicKey(input) {
  const own = __keyObjectRecord(input);
  if (own && own.type === 'public') throw Object.assign(new TypeError('Invalid key object type public, expected private'), { code: 'ERR_CRYPTO_INVALID_KEY_OBJECT_TYPE' });
  const record = own || __keyDecode(input);
  const operation = record.type === 'private' ? 'derive' : 'public';
  return new __KeyObject(__keyToken, 'public', globalThis.__ttEc(operation, record.der));
}
function __keyEncoded(key, options) { return options === undefined ? key : key.export(options); }
function __generateKeyPairSync(type, options) {
  if (type !== 'ec') __keyUnsupported('Key generation algorithm ' + type);
  __keyOptions(options);
  if (options.namedCurve !== 'prime256v1') __keyUnsupported('EC curve ' + options.namedCurve);
  if (options.paramEncoding !== undefined && options.paramEncoding !== 'named') __keyUnsupported('Explicit EC parameters');
  if (options.publicKeyEncoding !== undefined) __keyExportOptions('public', options.publicKeyEncoding);
  if (options.privateKeyEncoding !== undefined) __keyExportOptions('private', options.privateKeyEncoding);
  const [privateDer, publicDer] = globalThis.__ttEc('generate');
  return {
    privateKey: __keyEncoded(new __KeyObject(__keyToken, 'private', privateDer), options.privateKeyEncoding),
    publicKey: __keyEncoded(new __KeyObject(__keyToken, 'public', publicDer), options.publicKeyEncoding),
  };
}
function __keyAlgorithm(algorithm) {
  if (typeof algorithm !== 'string' || algorithm.toLowerCase().replace(/-/g, '') !== 'sha256') __keyUnsupported('ECDSA digest ' + String(algorithm));
}
function __keySignOptions(input) {
  if (input && input.dsaEncoding !== undefined && input.dsaEncoding !== 'der') __keyUnsupported('ECDSA signature encoding ' + input.dsaEncoding);
  if (input && input.padding !== undefined) __keyUnsupported('ECDSA padding options');
}
function __privateRecord(input) {
  __keySignOptions(input);
  if (__keyRecords.has(input)) {
    const record = __keyRecords.get(input);
    if (record.type !== 'private') throw new TypeError('A private key is required');
    return record;
  }
  if (input && __keyRecords.has(input.key)) return __privateRecord(input.key);
  return __keyRecords.get(__createPrivateKey(input));
}
function __publicRecord(input) {
  __keySignOptions(input);
  const record = __keyObjectRecord(input);
  if (record && record.type === 'public') return record;
  return __keyRecords.get(__createPublicKey(input));
}
function __keyBufferSource(data) { return data instanceof ArrayBuffer ? new Uint8Array(data) : __hashBytes(data); }
function __sign(algorithm, data, key, callback) {
  if (callback !== undefined) __keyUnsupported('Asynchronous signing');
  __keyAlgorithm(algorithm);
  const record = __privateRecord(key);
  return Buffer.from(globalThis.__ttEc('sign', __keyBufferSource(data), record.der));
}
function __verify(algorithm, data, key, signature, callback) {
  if (callback !== undefined) __keyUnsupported('Asynchronous verification');
  __keyAlgorithm(algorithm);
  const record = __publicRecord(key);
  return globalThis.__ttEc('verify', __keyBufferSource(data), record.der, __keyBufferSource(signature));
}
function __keyAccumulator(algorithm, verifying) {
  __keyAlgorithm(algorithm);
  const chunks = [];
  let finalized = false;
  const check = () => { if (finalized) throw new Error('Sign or verify already called'); };
  const accumulator = { update(data, encoding) { check(); chunks.push(__hashBytes(data, encoding)); return this; } };
  const consume = () => { check(); finalized = true; const data = Buffer.concat(chunks); chunks.length = 0; return data; };
  if (verifying) accumulator.verify = (key, signature, encoding) => __verify(algorithm, consume(), key, __hashBytes(signature, encoding));
  else accumulator.sign = (key, encoding) => __hashOutput(__sign(algorithm, consume(), key), encoding);
  return accumulator;
}
const __nodeKeys = {
  KeyObject: __KeyObject, generateKeyPairSync: __generateKeyPairSync,
  createPrivateKey: __createPrivateKey, createPublicKey: __createPublicKey,
  sign: __sign, verify: __verify,
  createSign: (algorithm) => __keyAccumulator(algorithm, false),
  createVerify: (algorithm) => __keyAccumulator(algorithm, true),
};
