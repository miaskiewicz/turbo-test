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
