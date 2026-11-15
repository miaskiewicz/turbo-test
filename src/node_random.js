function __secureRandomSize(size) {
  if (typeof size !== 'number') throw new TypeError('size must be a number');
  if (!Number.isInteger(size) || size < 0 || size > 2147483647) throw new RangeError('size must be an integer between 0 and 2147483647');
}
function __secureRandomCallback(size, callback) {
  let bytes;
  try { bytes = Buffer.from(globalThis.__ttRandomBytes(size)); }
  catch (error) { callback(error); return; }
  callback(null, bytes);
}
function __secureRandomBytes(size, callback) {
  __secureRandomSize(size);
  if (callback === undefined) return Buffer.from(globalThis.__ttRandomBytes(size));
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  __realTimers.setImmediate(() => __secureRandomCallback(size, callback));
}
