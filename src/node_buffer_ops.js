// Buffer#compare (turbo-test#408: csv-parse's BOM check), Buffer#write (turbo-test#409:
// pg-protocol's buffer-writer) and the byte operations those real packages run next to them
// (ranged toString, copy, 16/32-bit integer reads and writes), with Node 26's argument
// validation, error messages and encoders.
const __bufferMaxLength = 9007199254740991;
const __bufferByteTag = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get;
const __bufferError = (Type, code, message) => Object.assign(new Type(message), { code });

// Node's determineSpecificType: the "Received ..." tail of ERR_INVALID_ARG_TYPE.
function __bufferStringReceived(value) {
  const shown = value.length > 28 ? `${value.slice(0, 25)}...` : value;
  return shown.includes("'") ? `type string (${JSON.stringify(shown)})` : `type string ('${shown}')`;
}
function __bufferObjectReceived(value) {
  if (value === null) return 'null';
  const ctor = value.constructor;
  return ctor && 'name' in ctor ? `an instance of ${ctor.name}` : __util.inspect(value, { depth: -1 });
}
const __bufferReceivedByType = {
  string: __bufferStringReceived,
  object: __bufferObjectReceived,
  undefined: () => 'undefined',
  function: (value) => `function ${value.name}`,
  number: (value) => `type number (${Object.is(value, -0) ? '-0' : value})`,
  bigint: (value) => `type bigint (${value}n)`,
  boolean: (value) => `type boolean (${value})`,
  symbol: (value) => `type symbol (${String(value)})`,
};
const __bufferReceived = (value) => __bufferReceivedByType[typeof value](value);

// Node's addNumericalSeparator, used by ERR_OUT_OF_RANGE for integers beyond 2 ** 32.
function __bufferSeparated(text) {
  const start = text[0] === '-' ? 1 : 0;
  let end = text.length;
  let groups = '';
  for (; end >= start + 4; end -= 3) groups = `_${text.slice(end - 3, end)}${groups}`;
  return text.slice(0, end) + groups;
}
const __bufferRangeReceived = (value) => (Number.isInteger(value) && Math.abs(value) > 2 ** 32 ? __bufferSeparated(String(value)) : String(value));

// Node's validateOffset (validateInteger with a 0 floor).
function __bufferOffset(value, name, max) {
  if (typeof value !== 'number') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "${name}" argument must be of type number. Received ${__bufferReceived(value)}`);
  if (!Number.isInteger(value)) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be an integer. Received ${value}`);
  if (value < 0 || value > max) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be >= 0 && <= ${max}. Received ${__bufferRangeReceived(value)}`);
  return value;
}
const __bufferOptionalOffset = (value, name, fallback, max) => (value === undefined ? fallback : __bufferOffset(value, name, max));

function __bufferCompareBytes(source, sourceStart, sourceEnd, target, targetStart, targetEnd) {
  const sourceLength = sourceEnd - sourceStart;
  const targetLength = targetEnd - targetStart;
  for (let index = 0; index < Math.min(sourceLength, targetLength); index++) {
    const left = source[sourceStart + index];
    const right = target[targetStart + index];
    if (left !== right) return left < right ? -1 : 1;
  }
  return Math.sign(sourceLength - targetLength);
}
function __bufferCompare(source, target, targetStart, targetEnd, sourceStart, sourceEnd) {
  if (__bufferByteTag.call(target) !== 'Uint8Array') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "target" argument must be an instance of Buffer or Uint8Array. Received ${__bufferReceived(target)}`);
  const tStart = __bufferOptionalOffset(targetStart, 'targetStart', 0, __bufferMaxLength);
  const tEnd = __bufferOptionalOffset(targetEnd, 'targetEnd', target.length, target.length);
  const sStart = __bufferOptionalOffset(sourceStart, 'sourceStart', 0, __bufferMaxLength);
  const sEnd = __bufferOptionalOffset(sourceEnd, 'sourceEnd', source.length, source.length);
  if (sStart >= sEnd) return tStart >= tEnd ? 0 : -1;
  if (tStart >= tEnd) return 1;
  return __bufferCompareBytes(source, sStart, sEnd, target, tStart, tEnd);
}

// Encoders return at most `limit` bytes, never a partial character.
function __bufferUtf8Bytes(string, limit) {
  // Every UTF-16 unit takes at least one byte, so a longer prefix can never fit.
  const bytes = new globalThis.TextEncoder().encode(string.slice(0, limit));
  let end = Math.min(bytes.length, limit);
  if (end < bytes.length) while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
  return bytes.subarray(0, end);
}
function __bufferLatin1Bytes(string, limit) {
  const bytes = new Uint8Array(Math.min(string.length, limit));
  for (let index = 0; index < bytes.length; index++) bytes[index] = string.charCodeAt(index);
  return bytes;
}
function __bufferUtf16Bytes(string, limit) {
  const units = Math.min(string.length, Math.floor(limit / 2));
  const bytes = new Uint8Array(units * 2);
  for (let index = 0; index < units; index++) {
    const code = string.charCodeAt(index);
    bytes[index * 2] = code;
    bytes[index * 2 + 1] = code >> 8;
  }
  return bytes;
}
function __bufferHexBytes(string, limit) {
  const bytes = [];
  const pairs = Math.min(Math.floor(string.length / 2), limit);
  for (let index = 0; index < pairs; index++) {
    const value = parseInt(string[index * 2], 16) * 16 + parseInt(string[index * 2 + 1], 16);
    if (Number.isNaN(value)) break;
    bytes.push(value);
  }
  return bytes;
}
// Node decodes both alphabets in either mode, skips other characters and stops at padding.
const __bufferBase64Values = new Int8Array(128).fill(-1);
for (let index = 0; index < 64; index++) __bufferBase64Values['ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'.charCodeAt(index)] = index;
__bufferBase64Values[45] = 62;
__bufferBase64Values[95] = 63;
function __bufferBase64Sextets(string) {
  const sextets = [];
  for (let index = 0; index < string.length; index++) {
    const code = string.charCodeAt(index);
    if (code === 61) break;
    const value = code < 128 ? __bufferBase64Values[code] : -1;
    if (value >= 0) sextets.push(value);
  }
  return sextets;
}
function __bufferBase64Bytes(string, limit) {
  const bytes = [];
  let bits = 0;
  let pending = 0;
  for (const sextet of __bufferBase64Sextets(string)) {
    pending = ((pending << 6) | sextet) & 0xffff;
    bits += 6;
    if (bits >= 8) { bits -= 8; bytes.push((pending >> bits) & 0xff); }
  }
  return bytes.slice(0, limit);
}
const __bufferWriteEncoders = Object.assign(Object.create(null), {
  utf8: __bufferUtf8Bytes, 'utf-8': __bufferUtf8Bytes,
  ascii: __bufferLatin1Bytes, latin1: __bufferLatin1Bytes, binary: __bufferLatin1Bytes,
  ucs2: __bufferUtf16Bytes, 'ucs-2': __bufferUtf16Bytes, utf16le: __bufferUtf16Bytes, 'utf-16le': __bufferUtf16Bytes,
  hex: __bufferHexBytes, base64: __bufferBase64Bytes, base64url: __bufferBase64Bytes,
});
function __bufferWriteEncoder(encoding) {
  if (!encoding) return __bufferUtf8Bytes;
  const encoder = __bufferWriteEncoders[`${encoding}`.toLowerCase()];
  if (!encoder) throw __bufferError(TypeError, 'ERR_UNKNOWN_ENCODING', `Unknown encoding: ${encoding}`);
  return encoder;
}
// Buffer.from(string, encoding): a non-string or empty encoding means utf8 (unlike write, where
// only a falsy one does); csv-parse encodes its utf16le delimiters and BOM this way.
function __bufferFromString(string, encoding) {
  const encode = typeof encoding === 'string' && encoding ? __bufferWriteEncoder(encoding) : __bufferUtf8Bytes;
  return encode(string, Infinity);
}
// Node's overloads: (string), (string, encoding), (string, offset[, length][, encoding]).
function __bufferWriteArgs(buffer, offset, length, encoding) {
  if (offset === undefined) return [0, buffer.length, 'utf8'];
  if (length === undefined && typeof offset === 'string') return [0, buffer.length, offset];
  const remaining = buffer.length - __bufferOffset(offset, 'offset', buffer.length);
  if (length === undefined) return [offset, remaining, encoding];
  if (typeof length === 'string') return [offset, remaining, length];
  return [offset, Math.min(__bufferOffset(length, 'length', buffer.length), remaining), encoding];
}
function __bufferWrite(buffer, string, offset, length, encoding) {
  const [start, limit, name] = __bufferWriteArgs(buffer, offset, length, encoding);
  const encode = __bufferWriteEncoder(name);
  if (typeof string !== 'string') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', 'argument must be a string');
  const bytes = encode(string, limit);
  buffer.set(bytes, start);
  return bytes.length;
}

// toString(encoding, start, end): Node clamps the range before it resolves the encoding.
const __bufferStringStart = (start, length) => (start <= 0 ? 0 : Math.min(Math.trunc(start) || 0, length));
const __bufferStringEnd = (end, length) => (end === undefined || end > length ? length : Math.trunc(end) || 0);
function __bufferCharString(codes) {
  let out = '';
  for (let index = 0; index < codes.length; index += 8192) out += String.fromCharCode.apply(null, codes.subarray(index, index + 8192));
  return out;
}
function __bufferUtf16String(bytes) {
  const units = new Uint16Array(bytes.length >> 1);
  for (let index = 0; index < units.length; index++) units[index] = bytes[index * 2] | (bytes[index * 2 + 1] << 8);
  return __bufferCharString(units);
}
const __bufferUtf8String = (bytes) => new globalThis.TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes); // Node keeps a BOM
const __bufferBase64String = (bytes) => globalThis.__b64encodeBytes(bytes);
const __bufferStringDecoders = Object.assign(Object.create(null), {
  utf8: __bufferUtf8String, 'utf-8': __bufferUtf8String,
  latin1: __bufferCharString, binary: __bufferCharString, ascii: (bytes) => __bufferCharString(bytes.map((byte) => byte & 0x7f)),
  ucs2: __bufferUtf16String, 'ucs-2': __bufferUtf16String, utf16le: __bufferUtf16String, 'utf-16le': __bufferUtf16String,
  hex: (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join(''),
  base64: __bufferBase64String,
  // base64url: RFC 4648 §5 — `+` → `-`, `/` → `_`, no `=` padding (Node WHATWG, turbo-test#260).
  base64url: (bytes) => __bufferBase64String(bytes).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''),
});
function __bufferStringDecoder(encoding) {
  if (encoding === undefined) return __bufferUtf8String;
  const decoder = __bufferStringDecoders[`${encoding}`.toLowerCase()];
  if (!decoder) throw __bufferError(TypeError, 'ERR_UNKNOWN_ENCODING', `Unknown encoding: ${encoding}`);
  return decoder;
}
function __bufferToString(buffer, encoding, start, end) {
  const from = __bufferStringStart(start, buffer.length);
  const to = __bufferStringEnd(end, buffer.length);
  if (to <= from) return '';
  return __bufferStringDecoder(encoding)(new Uint8Array(buffer.buffer, buffer.byteOffset + from, to - from));
}

// copy(target, targetStart, sourceStart, sourceEnd): any ArrayBufferView, byte-wise, memmove-safe.
const __bufferSafeInteger = (number) => (number >= Number.MIN_SAFE_INTEGER && number <= Number.MAX_SAFE_INTEGER ? Math.floor(number) : 0);
const __bufferCopyIndex = (value) => (Number.isInteger(value) ? value : __bufferSafeInteger(+value));
function __bufferCopyIndexArg(value, name, fallback, max) {
  if (value === undefined) return fallback;
  const index = __bufferCopyIndex(value);
  const rule = max === Infinity ? '>= 0' : `>= 0 && <= ${max}`;
  if (index < 0 || index > max) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be ${rule}. Received ${__bufferRangeReceived(index)}`);
  return index;
}
function __bufferView(value, name) {
  if (!ArrayBuffer.isView(value)) throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "${name}" argument must be an instance of Buffer or Uint8Array. Received ${__bufferReceived(value)}`);
  return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
}
function __bufferCopy(sourceValue, targetValue, targetStart, sourceStart, sourceEnd) {
  const source = __bufferView(sourceValue, 'source');
  const target = __bufferView(targetValue, 'target');
  const at = __bufferCopyIndexArg(targetStart, 'targetStart', 0, Infinity);
  const from = __bufferCopyIndexArg(sourceStart, 'sourceStart', 0, source.length);
  const to = Math.min(__bufferCopyIndexArg(sourceEnd, 'sourceEnd', source.length, Infinity), source.length, from + target.length - at);
  if (to <= from) return 0;
  target.set(source.subarray(from, to), at);
  return to - from;
}

// Integer reads and writes: Node's checkInt / checkBounds / boundsError.
function __bufferBoundsError(offset, last) {
  if (Math.floor(offset) !== offset) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "offset" is out of range. It must be an integer. Received ${offset}`);
  if (last < 0) throw __bufferError(RangeError, 'ERR_BUFFER_OUT_OF_BOUNDS', 'Attempt to access memory outside buffer bounds');
  throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "offset" is out of range. It must be >= 0 and <= ${last}. Received ${__bufferRangeReceived(offset)}`);
}
function __bufferIntBounds(buffer, offset, size) {
  if (typeof offset !== 'number') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "offset" argument must be of type number. Received ${__bufferReceived(offset)}`);
  if (buffer[offset] === undefined || buffer[offset + size - 1] === undefined) __bufferBoundsError(offset, buffer.length - size);
  return offset;
}
const __bufferInt16 = { size: 2, min: -32768, max: 32767, signed: true };
const __bufferInt32 = { size: 4, min: -2147483648, max: 2147483647, signed: true };
const __bufferUInt32 = { size: 4, min: 0, max: 4294967295, signed: false };
function __bufferWriteInt(buffer, value, offset, spec, little) {
  const number = +value;
  if (number > spec.max || number < spec.min) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "value" is out of range. It must be >= ${spec.min} and <= ${spec.max}. Received ${__bufferRangeReceived(number)}`);
  __bufferIntBounds(buffer, offset, spec.size);
  const bits = number >>> 0;
  for (let index = 0; index < spec.size; index++) buffer[offset + (little ? index : spec.size - 1 - index)] = bits >>> (8 * index);
  return offset + spec.size;
}
function __bufferReadIntBE(buffer, offset, spec) {
  const start = __bufferIntBounds(buffer, offset === undefined ? 0 : offset, spec.size);
  let value = 0;
  for (let index = 0; index < spec.size; index++) value = value * 256 + buffer[start + index];
  return spec.signed && value > spec.max ? value - 2 * (spec.max + 1) : value;
}
