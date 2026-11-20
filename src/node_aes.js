// In-memory AES-256-GCM accumulators backed by RustCrypto; no cipher or tag placeholders.
function __aesError(message, code, Type = Error) { return Object.assign(new Type(message), { code }); }
function __aesCheckOpen(state) {
  if (state.closed) throw __aesError('Invalid state', 'ERR_CRYPTO_INVALID_STATE');
}
function __aesBytes(value, encoding, strings = true) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
  if (typeof value === 'string' && !strings) throw new TypeError('Expected a byte view');
  return __hashBytes(value, encoding);
}
// Node 26's argument checks, in its order (#450): the cipher, key and iv types, then the cipher
// name, key length, iv and auth tag. GCM tags of 4 / 8 bytes and IVs other than 12 bytes are
// valid in Node but beyond this AES-256-GCM implementation, which reports them explicitly.
const __aesKeyTypes = 'string or an instance of ArrayBuffer, Buffer, TypedArray, DataView, KeyObject, or CryptoKey';
const __aesIvTypes = 'string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView';
const __aesIsBytes = (value) => typeof value === 'string' || value instanceof ArrayBuffer || ArrayBuffer.isView(value);
function __aesArgType(name, types, value) {
  return __aesError(`The "${name}" argument must be of type ${types}. Received ${__bufferReceived(value)}`, 'ERR_INVALID_ARG_TYPE', TypeError);
}
function __aesCheckArgs(algorithm, key, iv) {
  if (typeof algorithm !== 'string') throw __aesArgType('cipher', 'string', algorithm);
  if (!__aesIsBytes(key)) throw __aesArgType('key', __aesKeyTypes, key);
  if (iv !== null && !__aesIsBytes(iv)) throw __aesArgType('iv', __aesIvTypes, iv);
}
function __aesTagLength(value) {
  if (value === undefined) return undefined;
  if (!Number.isInteger(value)) throw __aesError(`The property 'options.authTagLength' is invalid. Received ${__util.inspect(value)}`, 'ERR_INVALID_ARG_VALUE', TypeError);
  if (value === 4 || value === 8) throw __aesError('AES-GCM authentication tags of 4 or 8 bytes are not supported by turbo-test', 'ERR_CRYPTO_INVALID_AUTH_TAG', RangeError);
  if (value < 12 || value > 16) throw __aesError(`Invalid authentication tag length: ${value}`, 'ERR_CRYPTO_INVALID_AUTH_TAG', TypeError);
  return value;
}
function __aesMaterial(key, iv) {
  const material = __aesBytes(key);
  if (material.length !== 32) throw __aesError('Invalid key length', 'ERR_CRYPTO_INVALID_KEYLEN', RangeError);
  const nonce = iv === null ? new Uint8Array(0) : __aesBytes(iv);
  if (nonce.length === 0) throw __aesError('Invalid initialization vector', 'ERR_CRYPTO_INVALID_IV', TypeError);
  if (nonce.length !== 12) throw __aesError('AES-256-GCM currently requires a 12-byte IV', 'ERR_CRYPTO_INVALID_IV', TypeError);
  return { material, nonce };
}
function __aesCreateState(algorithm, key, iv, options, decrypt) {
  __aesCheckArgs(algorithm, key, iv);
  if (algorithm.toLowerCase() !== 'aes-256-gcm') throw __aesError('Unsupported cipher: ' + algorithm, 'ERR_CRYPTO_UNKNOWN_CIPHER');
  options = options || {};
  const { material, nonce } = __aesMaterial(key, iv);
  const tagLength = __aesTagLength(options.authTagLength);
  return { decrypt, key: material, iv: nonce, chunks: [], aad: [], offset: 0,
    tagLength: decrypt ? tagLength : tagLength ?? 16, tag: null, resultTag: null,
    closed: false, decoder: null, started: false };
}
function __aesNative(state, decrypt, data, tagLength) {
  return Buffer.from(globalThis.__ttAesGcm(decrypt, state.key, state.iv, data, Buffer.concat(state.aad), tagLength));
}
function __aesOutput(state, bytes, encoding, final) {
  if (!encoding || encoding === 'buffer') return Buffer.from(bytes);
  const decoder = new __StringDecoder(encoding);
  if (state.decoder && state.decoder.encoding !== decoder.encoding) throw __aesError('Cannot change encoding', 'ERR_CRYPTO_INVALID_STATE');
  state.decoder = state.decoder || decoder;
  return final ? state.decoder.end(bytes) : state.decoder.write(bytes);
}
function __aesUpdate(state, data, encoding, outputEncoding) {
  __aesCheckOpen(state);
  const bytes = __hashBytes(data, encoding);
  state.chunks.push(bytes);
  if (bytes.length) state.started = true;
  const input = Buffer.concat(state.chunks);
  // GCM encryption is CTR XOR followed by authentication. Encrypting ciphertext gives
  // the incremental plaintext bytes; decipher.final separately verifies the real tag.
  const output = __aesNative(state, false, input, 16).subarray(state.offset, input.length);
  state.offset = input.length;
  return __aesOutput(state, output, outputEncoding, false);
}
function __aesClear(state) {
  state.key.fill(0); state.iv.fill(0);
  state.chunks.forEach((bytes) => bytes.fill(0));
  state.aad.forEach((bytes) => bytes.fill(0));
  state.chunks = []; state.aad = [];
}
function __aesFinalBytes(state) {
  const input = Buffer.concat(state.chunks);
  if (state.decrypt) {
    if (!state.tag) throw new Error('Unsupported state or unable to authenticate data');
    return __aesNative(state, true, Buffer.concat([input, state.tag]), state.tag.length).subarray(state.offset);
  }
  const encrypted = __aesNative(state, false, input, state.tagLength);
  state.resultTag = Buffer.from(encrypted.subarray(input.length));
  return encrypted.subarray(state.offset, input.length);
}
function __aesFinal(state, encoding) {
  __aesCheckOpen(state);
  state.closed = true;
  try { return __aesOutput(state, __aesFinalBytes(state), encoding, true); }
  finally { __aesClear(state); }
}
function __aesAAD(state, value) {
  __aesCheckOpen(state);
  if (state.started) throw __aesError('Invalid state for operation setAAD', 'ERR_CRYPTO_INVALID_STATE');
  state.aad.push(__aesBytes(value, undefined, false));
}
function __aesSetTag(state, value) {
  __aesCheckOpen(state);
  if (state.tag) throw __aesError('Invalid state for operation setAuthTag', 'ERR_CRYPTO_INVALID_STATE');
  const tag = __aesBytes(value, undefined, false);
  __aesTagLength(tag.length);
  if (state.tagLength !== undefined && state.tagLength !== tag.length) throw __aesError('Invalid authentication tag length', 'ERR_CRYPTO_INVALID_AUTH_TAG');
  state.tag = tag;
}
function __aesGetTag(state) {
  if (!state.resultTag) throw __aesError('Invalid state for operation getAuthTag', 'ERR_CRYPTO_INVALID_STATE');
  return Buffer.from(state.resultTag);
}
function __aesCipher(algorithm, key, iv, options, decrypt) {
  const state = __aesCreateState(algorithm, key, iv, options, decrypt);
  const api = {
    update(data, encoding, outputEncoding) { return __aesUpdate(state, data, encoding, outputEncoding); },
    final(encoding) { return __aesFinal(state, encoding); },
    setAAD(value) { __aesAAD(state, value); return this; },
    setAutoPadding() { __aesCheckOpen(state); return this; },
  };
  if (decrypt) api.setAuthTag = function(value) { __aesSetTag(state, value); return this; };
  else api.getAuthTag = () => __aesGetTag(state);
  return api;
}
const __nodeAes = {
  createCipheriv: (algorithm, key, iv, options) => __aesCipher(algorithm, key, iv, options, false),
  createDecipheriv: (algorithm, key, iv, options) => __aesCipher(algorithm, key, iv, options, true),
};
