// Owned EC/RSA KeyObjects with real native signing and standard PKCS#8/SPKI encodings.
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
  if (options.type !== expected) __keyUnsupported('Key ' + type + ' export type ' + options.type);
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
  constructor(token, type, der, algorithm = 'ec') {
    if (token !== __keyToken) throw new TypeError('KeyObject cannot be constructed directly');
    __keyRecords.set(this, { type, der: Buffer.from(der), algorithm });
    Object.defineProperties(this, {
      type: { value: type, enumerable: true },
      asymmetricKeyType: { value: algorithm, enumerable: true },
    });
  }
  get asymmetricKeyDetails() {
    const record = __keyRecords.get(this);
    if (record.algorithm === 'ec') return { namedCurve: 'prime256v1' };
    const [modulusLength, exponent] = globalThis.__ttRsa('details-' + record.type, record.der);
    return { modulusLength, publicExponent: BigInt(exponent) };
  }
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
function __keyBackend(algorithm) { return algorithm === 'rsa' ? globalThis.__ttRsa : globalThis.__ttEc; }
function __keyImport(type, der, operation) {
  try { return new __KeyObject(__keyToken, type, globalThis.__ttEc(operation, der), 'ec'); }
  catch { return new __KeyObject(__keyToken, type, globalThis.__ttRsa(operation, der), 'rsa'); }
}
function __createPrivateKey(input) {
  if (__keyRecords.has(input)) throw new TypeError('createPrivateKey requires encoded key material');
  const decoded = __keyDecode(input);
  if (decoded.type !== 'private') throw new TypeError('A private key is required');
  return __keyImport('private', decoded.der, 'private');
}
function __keyObjectRecord(input) {
  return __keyRecords.get(input) || (input && __keyRecords.get(input.key));
}
function __publicFromRecord(record) {
  if (record.type === 'public') throw Object.assign(new TypeError('Invalid key object type public, expected private'), { code: 'ERR_CRYPTO_INVALID_KEY_OBJECT_TYPE' });
  return new __KeyObject(__keyToken, 'public', __keyBackend(record.algorithm)('derive', record.der), record.algorithm);
}
function __createPublicKey(input) {
  const own = __keyObjectRecord(input);
  if (own) return __publicFromRecord(own);
  const record = __keyDecode(input);
  return __keyImport('public', record.der, record.type === 'private' ? 'derive' : 'public');
}
function __keyEncoded(key, options) { return options === undefined ? key : key.export(options); }
function __keyGenerationEncodings(options) {
  if (options.publicKeyEncoding !== undefined) __keyExportOptions('public', options.publicKeyEncoding);
  if (options.privateKeyEncoding !== undefined) __keyExportOptions('private', options.privateKeyEncoding);
}
function __keyGenerated(pair, algorithm, options) {
  const [privateDer, publicDer] = pair;
  return {
    privateKey: __keyEncoded(new __KeyObject(__keyToken, 'private', privateDer, algorithm), options.privateKeyEncoding),
    publicKey: __keyEncoded(new __KeyObject(__keyToken, 'public', publicDer, algorithm), options.publicKeyEncoding),
  };
}
function __rsaInteger(value, name, minimum, maximum) {
  if (typeof value !== 'number') throw new TypeError(name + ' must be a number');
  if (!Number.isInteger(value) || value < minimum || value > maximum) throw new RangeError(name + ' must be an integer between ' + minimum + ' and ' + maximum);
  return value;
}
function __generateRsaKeyPair(options) {
  __keyOptions(options);
  const bits = __rsaInteger(options.modulusLength, 'modulusLength', 512, 4096);
  const exponent = __rsaInteger(options.publicExponent === undefined ? 65537 : options.publicExponent, 'publicExponent', 3, 4294967295);
  if (exponent % 2 === 0) throw new RangeError('publicExponent must be odd');
  __keyGenerationEncodings(options);
  return __keyGenerated(globalThis.__ttRsa('generate', undefined, undefined, undefined, bits, exponent), 'rsa', options);
}
function __generateKeyPairSync(type, options) {
  if (type === 'rsa') return __generateRsaKeyPair(options);
  if (type !== 'ec') __keyUnsupported('Key generation algorithm ' + type);
  __keyOptions(options);
  if (options.namedCurve !== 'prime256v1') __keyUnsupported('EC curve ' + options.namedCurve);
  if (options.paramEncoding !== undefined && options.paramEncoding !== 'named') __keyUnsupported('Explicit EC parameters');
  __keyGenerationEncodings(options);
  return __keyGenerated(globalThis.__ttEc('generate'), 'ec', options);
}
function __keyAlgorithm(algorithm) {
  const name = typeof algorithm === 'string' ? algorithm.toLowerCase().replace(/-/g, '') : '';
  if (name !== 'sha256' && name !== 'rsasha256') __keyUnsupported('Signing digest ' + String(algorithm));
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
  return Buffer.from(__keyBackend(record.algorithm)('sign', __keyBufferSource(data), record.der));
}
function __verify(algorithm, data, key, signature, callback) {
  if (callback !== undefined) __keyUnsupported('Asynchronous verification');
  __keyAlgorithm(algorithm);
  const record = __publicRecord(key);
  return __keyBackend(record.algorithm)('verify', __keyBufferSource(data), record.der, __keyBufferSource(signature));
}
function __keyEnd(accumulator, data, encoding) {
  if (typeof data === 'function' || typeof encoding === 'function') __keyUnsupported('Asynchronous signer completion');
  if (data !== undefined && data !== null) accumulator.update(data, encoding);
  return accumulator;
}
function __keyAccumulator(algorithm, verifying) {
  __keyAlgorithm(algorithm);
  const chunks = [];
  let finalized = false;
  const check = () => { if (finalized) throw new Error('Sign or verify already called'); };
  const accumulator = {
    update(data, encoding) { check(); chunks.push(__hashBytes(data, encoding)); return this; },
    end(data, encoding) { return __keyEnd(this, data, encoding); },
  };
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
