// Real keyed hashes and key derivation over the native digest implementation.
function __cryptoHashBytes(value, encoding) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
  return __hashBytes(value, encoding);
}
function __cryptoHashAlgorithm(algorithm) {
  if (typeof algorithm !== 'string') throw new TypeError('algorithm must be a string');
  const name = algorithm.toLowerCase();
  if (name !== 'sha1' && name !== 'sha256') throw new Error('Unsupported keyed hash: ' + algorithm);
  return name;
}
function __cryptoCreateHmac(algorithm, key, options) {
  const name = __cryptoHashAlgorithm(algorithm);
  const secret = __cryptoHashBytes(key, options && options.encoding);
  const chunks = [];
  let finalized = false;
  const api = {
    update(data, encoding) {
      if (finalized) throw Object.assign(new Error('Digest already called'), { code: 'ERR_CRYPTO_HASH_FINALIZED' });
      chunks.push(__cryptoHashBytes(data, encoding)); return api;
    },
    digest(encoding) {
      if (finalized) return __hashOutput(new Uint8Array(), encoding);
      const bytes = globalThis.__ttCryptoHashes('hmac-' + name, secret, Buffer.concat(chunks), 0, 0);
      finalized = true; chunks.length = 0; secret.fill(0);
      return __hashOutput(new Uint8Array(bytes), encoding);
    },
  };
  return api;
}
function __cryptoPositiveInteger(value, maximum, name) {
  if (!Number.isInteger(value) || value < 1 || value > maximum) throw new RangeError(name + ' must be 1 through ' + maximum);
}
function __cryptoPbkdf2(password, salt, iterations, keyLength, digest) {
  __cryptoPositiveInteger(iterations, 2147483647, 'iterations');
  __cryptoPositiveInteger(keyLength, 1048576, 'PBKDF2 keyLength');
  const algorithm = __cryptoHashAlgorithm(digest);
  if (algorithm !== 'sha1') throw new Error('PBKDF2 currently supports SHA1 only');
  return Buffer.from(globalThis.__ttCryptoHashes('pbkdf2-sha1', __cryptoHashBytes(password), __cryptoHashBytes(salt), iterations, keyLength));
}
const __nodeCryptoHashes = { createHmac: __cryptoCreateHmac, pbkdf2Sync: __cryptoPbkdf2 };
