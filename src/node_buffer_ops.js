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
// BigInts get the n suffix, separated beyond 2n ** 32n.
const __bufferBigReceived = (value) => `${value > 2n ** 32n || value < -(2n ** 32n) ? __bufferSeparated(String(value)) : value}n`;
const __bufferRangeReceived = (value) => {
  if (typeof value === 'bigint') return __bufferBigReceived(value);
  return Number.isInteger(value) && Math.abs(value) > 2 ** 32 ? __bufferSeparated(String(value)) : String(value);
};

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
// Buffer.isEncoding (#434): the encoder table holds exactly Node's normalizeEncoding names.
const __bufferIsEncoding = (encoding) => typeof encoding === 'string' && encoding.toLowerCase() in __bufferWriteEncoders;
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

// Numeric reads and writes: Node's checkInt / checkBounds / boundsError (lib/internal/buffer.js).
function __bufferNumberArg(value, name) {
  if (typeof value !== 'number') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "${name}" argument must be of type number. Received ${__bufferReceived(value)}`);
}
// boundsError: `name` is "offset" (floor 0) or "byteLength" (floor 1).
function __bufferBoundsError(value, last, name = 'offset') {
  if (Math.floor(value) !== value) {
    __bufferNumberArg(value, name);
    throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be an integer. Received ${value}`);
  }
  if (last < 0) throw __bufferError(RangeError, 'ERR_BUFFER_OUT_OF_BOUNDS', 'Attempt to access memory outside buffer bounds');
  throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be >= ${name === 'offset' ? 0 : 1} and <= ${last}. Received ${__bufferRangeReceived(value)}`);
}
function __bufferIntBounds(buffer, offset, size) {
  __bufferNumberArg(offset, 'offset');
  if (buffer[offset] === undefined || buffer[offset + size - 1] === undefined) __bufferBoundsError(offset, buffer.length - size);
  return offset;
}
const __bufferIntSpec = (size, signed) => {
  const span = 2 ** (8 * size);
  return signed ? { size, min: -span / 2, max: span / 2 - 1, signed } : { size, min: 0, max: span - 1, signed };
};
const __bufferBigSpec = (signed) => (signed
  ? { size: 8, min: -(2n ** 63n), max: 2n ** 63n - 1n, signed, big: true }
  : { size: 8, min: 0n, max: 2n ** 64n - 1n, signed, big: true });
// checkInt's rule: the bounds up to 4 bytes, powers of two beyond (BigInts with an n suffix).
function __bufferRangeRule(spec) {
  if (spec.size <= 4) return `>= ${spec.min} and <= ${spec.max}`;
  const n = spec.big ? 'n' : '', bits = 8 * spec.size;
  if (!spec.signed) return `>= 0${n} and < 2${n} ** ${bits}${n}`;
  return `>= -(2${n} ** ${bits - 1}${n}) and < 2${n} ** ${bits - 1}${n}`;
}
function __bufferCheckInt(buffer, value, offset, spec) {
  if (value > spec.max || value < spec.min) throw __bufferError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "value" is out of range. It must be ${__bufferRangeRule(spec)}. Received ${__bufferRangeReceived(value)}`);
  __bufferIntBounds(buffer, offset, spec.size);
}
function __bufferWriteInt(buffer, value, offset, spec, little) {
  const number = +value;
  __bufferCheckInt(buffer, number, offset, spec);
  // Integer part in two's complement (NaN writes zeros, as a typed-array store does).
  let bits = Math.trunc(number) || 0;
  if (bits < 0) bits += 2 ** (8 * spec.size);
  for (let index = 0; index < spec.size; index++) buffer[offset + (little ? index : spec.size - 1 - index)] = Math.floor(bits / 2 ** (8 * index)) % 256;
  return offset + spec.size;
}
function __bufferReadInt(buffer, offset, spec, little) {
  const start = __bufferIntBounds(buffer, offset === undefined ? 0 : offset, spec.size);
  let value = 0;
  for (let index = 0; index < spec.size; index++) value = value * 256 + buffer[start + (little ? spec.size - 1 - index : index)];
  return spec.signed && value > spec.max ? value - 2 * (spec.max + 1) : value;
}
const __bufferDataView = (buffer) => new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
// BigInt writes mask the value, which throws V8's mixing TypeError for a Number, as Node's do.
function __bufferWriteBig(buffer, value, offset, spec, little) {
  __bufferCheckInt(buffer, value, offset, spec);
  __bufferDataView(buffer).setBigUint64(offset, value & 0xffffffffffffffffn, little);
  return offset + 8;
}
function __bufferReadBig(buffer, offset, spec, little) {
  const start = __bufferIntBounds(buffer, offset === undefined ? 0 : offset, 8);
  return spec.signed ? __bufferDataView(buffer).getBigInt64(start, little) : __bufferDataView(buffer).getBigUint64(start, little);
}
const __bufferFloat = { size: 4, get: 'getFloat32', set: 'setFloat32' };
const __bufferDouble = { size: 8, get: 'getFloat64', set: 'setFloat64' };
function __bufferWriteFloat(buffer, value, offset, spec, little) {
  const number = +value;
  __bufferIntBounds(buffer, offset, spec.size);
  __bufferDataView(buffer)[spec.set](offset, number, little);
  return offset + spec.size;
}
function __bufferReadFloat(buffer, offset, spec, little) {
  const start = __bufferIntBounds(buffer, offset === undefined ? 0 : offset, spec.size);
  return __bufferDataView(buffer)[spec.get](start, little);
}
// Node's fixed-width numeric methods, enumerable as Node assigns them, with the lower-case Uint
// aliases as the same functions. compress-commons writes zip headers with them (#404).
const __bufferNumericMethods = [];
for (const [bits, size] of [['8', 1], ['16', 2], ['32', 4]]) {
  for (const signed of [true, false]) {
    for (const end of size === 1 ? [''] : ['LE', 'BE']) __bufferNumericMethods.push([(signed ? 'Int' : 'UInt') + bits + end, __bufferIntSpec(size, signed), end === 'LE', __bufferReadInt, __bufferWriteInt]);
  }
}
for (const end of ['LE', 'BE']) {
  __bufferNumericMethods.push(['Float' + end, __bufferFloat, end === 'LE', __bufferReadFloat, __bufferWriteFloat],
    ['Double' + end, __bufferDouble, end === 'LE', __bufferReadFloat, __bufferWriteFloat],
    ['BigInt64' + end, __bufferBigSpec(true), end === 'LE', __bufferReadBig, __bufferWriteBig],
    ['BigUInt64' + end, __bufferBigSpec(false), end === 'LE', __bufferReadBig, __bufferWriteBig]);
}
// read/write{Int,UInt}{LE,BE}(…, byteLength): byteLength 1..6 picks the width.
const __bufferVarSpecs = { true: [], false: [] };
for (let size = 1; size <= 6; size++) for (const signed of [true, false]) __bufferVarSpecs[signed][size] = __bufferIntSpec(size, signed);
function __bufferVarSpec(byteLength, signed) {
  if (typeof byteLength === 'number' && __bufferVarSpecs[signed][byteLength]) return __bufferVarSpecs[signed][byteLength];
  return __bufferBoundsError(byteLength, 6, 'byteLength');
}
function __bufferVarMethods(name, signed, little) {
  return {
    ['read' + name](offset, byteLength) {
      if (offset === undefined) __bufferNumberArg(offset, 'offset');
      return __bufferReadInt(this, offset, __bufferVarSpec(byteLength, signed), little);
    },
    ['write' + name](value, offset, byteLength) { return __bufferWriteInt(this, value, offset, __bufferVarSpec(byteLength, signed), little); },
  };
}
function __bufferInstall(proto, methods) {
  for (const [key, method] of Object.entries(methods)) {
    proto[key] = method;
    if (key.includes('UInt')) proto[key.replace('UInt', 'Uint')] = method;
  }
}
function __bufferInstallIntOps(proto) {
  for (const [name, spec, little, read, write] of __bufferNumericMethods) {
    __bufferInstall(proto, {
      ['write' + name](value, offset = 0) { return write(this, value, offset, spec, little); },
      ['read' + name](offset) { return read(this, offset, spec, little); },
    });
  }
  for (const end of ['LE', 'BE']) {
    __bufferInstall(proto, __bufferVarMethods('Int' + end, true, end === 'LE'));
    __bufferInstall(proto, __bufferVarMethods('UInt' + end, false, end === 'LE'));
  }
}

// indexOf / lastIndexOf / includes: Node's bidirectionalIndexOf over byte needles (#404).
// Where a search starts, as Node's IndexOfOffset: negative offsets count from the end; an
// empty needle points at the clamped offset.
function __bufferSearchStartBeforeEnd(length, offset, needle, forward) {
  if (offset + length >= 0) return length + offset;
  return forward || needle === 0 ? 0 : -1;
}
function __bufferSearchStart(length, offset, needle, forward) {
  if (offset < 0) return __bufferSearchStartBeforeEnd(length, offset, needle, forward);
  if (offset + needle <= length) return offset;
  if (needle === 0) return length;
  return forward ? -1 : length - 1;
}
function __bufferMatchAt(buffer, needle, at) {
  for (let index = 0; index < needle.length; index++) if (buffer[at + index] !== needle[index]) return false;
  return true;
}
function __bufferFindForward(buffer, needle, start) {
  for (let at = start; at + needle.length <= buffer.length; at++) if (__bufferMatchAt(buffer, needle, at)) return at;
  return -1;
}
function __bufferFindBackward(buffer, needle, start) {
  for (let at = Math.min(start, buffer.length - needle.length); at >= 0; at--) if (__bufferMatchAt(buffer, needle, at)) return at;
  return -1;
}
function __bufferNeedle(value, encoding) {
  if (typeof value === 'number') return [(value >>> 0) & 255];
  if (typeof value === 'string') return __bufferFromString(value, encoding === undefined ? 'utf8' : `${encoding}`);
  if (value instanceof Uint8Array) return value;
  throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', `The "value" argument must be one of type number or string or an instance of Buffer or Uint8Array. Received ${__bufferReceived(value)}`);
}
function __bufferIndexOf(buffer, value, byteOffset, encoding, forward) {
  if (typeof byteOffset === 'string') { encoding = byteOffset; byteOffset = undefined; }
  const offset = +byteOffset;
  const needle = __bufferNeedle(value, encoding);
  const start = __bufferSearchStart(buffer.length, Number.isNaN(offset) ? (forward ? 0 : buffer.length) : Math.trunc(offset), needle.length, forward);
  if (needle.length === 0 || start < 0) return start;
  return forward ? __bufferFindForward(buffer, needle, start) : __bufferFindBackward(buffer, needle, start);
}

// Buffer.from, as Node 26's lib/buffer.js: an (Shared)ArrayBuffer becomes a view over the same
// memory honouring byteOffset/length, any other object is copied element-wise (array-likes,
// typed arrays, {type:'Buffer',data}), valueOf / Symbol.toPrimitive strings convert, and
// anything else throws ERR_INVALID_ARG_TYPE.
const __bufferIsAnyArrayBuffer = (value) =>
  value instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer);
const __bufferOutOfBounds = (name) =>
  __bufferError(RangeError, 'ERR_BUFFER_OUT_OF_BOUNDS', `"${name}" is outside of buffer bounds`);
function __bufferViewLength(length, maxLength) {
  if (length === undefined) return maxLength;
  const n = +length;
  if (!(n > 0)) return 0;
  if (n > maxLength) throw __bufferOutOfBounds('length');
  return n;
}
function __bufferFromArrayBuffer(Ctor, buffer, byteOffset, length) {
  const offset = byteOffset === undefined || Number.isNaN(+byteOffset) ? 0 : +byteOffset;
  const maxLength = buffer.byteLength - offset;
  if (maxLength < 0) throw __bufferOutOfBounds('offset');
  return new Ctor(buffer, offset, __bufferViewLength(length, maxLength));
}
function __bufferFromArrayLike(Ctor, list) {
  const out = new Ctor(list.length > 0 ? list.length : 0);
  for (let i = 0; i < out.length; i++) out[i] = list[i];
  return out;
}
function __bufferFromObject(Ctor, value) {
  if (value.length !== undefined || __bufferIsAnyArrayBuffer(value.buffer)) {
    return typeof value.length === 'number' ? __bufferFromArrayLike(Ctor, value) : new Ctor(0);
  }
  if (value.type === 'Buffer' && Array.isArray(value.data)) return __bufferFromArrayLike(Ctor, value.data);
  return null;
}
const __bufferFromInvalid = (value) => __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE',
  'The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received ' + __bufferReceived(value));
function __bufferValueOf(value) {
  const primitive = value.valueOf && value.valueOf();
  const usable = primitive != null && primitive !== value;
  return usable && (typeof primitive === 'string' || typeof primitive === 'object') ? primitive : undefined;
}
function __bufferFromPrimitive(Ctor, value, encoding) {
  if (typeof value[Symbol.toPrimitive] !== 'function') return null;
  const primitive = value[Symbol.toPrimitive]('string');
  return typeof primitive === 'string' ? new Ctor(__bufferFromString(primitive, encoding)) : null;
}
function __bufferFromValue(Ctor, value, encodingOrOffset, length) {
  if (__bufferIsAnyArrayBuffer(value)) return __bufferFromArrayBuffer(Ctor, value, encodingOrOffset, length);
  const inner = __bufferValueOf(value);
  if (inner !== undefined) return __bufferFrom(Ctor, inner, encodingOrOffset, length);
  return __bufferFromObject(Ctor, value) || __bufferFromPrimitive(Ctor, value, encodingOrOffset);
}
function __bufferFrom(Ctor, value, encodingOrOffset, length) {
  if (typeof value === 'string') return new Ctor(__bufferFromString(value, encodingOrOffset));
  const out = typeof value === 'object' && value !== null ? __bufferFromValue(Ctor, value, encodingOrOffset, length) : null;
  if (out) return out;
  throw __bufferFromInvalid(value);
}

// Node's internal per-encoding methods (latin1Slice, utf8Write, ...; #418), which busboy calls.
// They are bindings in Node: the receiver may be any ArrayBufferView, indices go through
// ParseArrayIndex (ToInteger, negative = "Index out of range") except the latin1 / ascii / utf8
// writes, which compare Number(offset) / Number(length) with the buffer first.
const __bufferIndexError = () => __bufferError(RangeError, 'ERR_OUT_OF_RANGE', 'Index out of range');
function __bufferArrayIndex(value, fallback) {
  if (value === undefined) return fallback;
  const index = Math.trunc(+value) || 0;
  if (index < 0) throw __bufferIndexError();
  return index;
}
function __bufferReceiverBytes(view) {
  if (!ArrayBuffer.isView(view)) throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', 'argument must be a buffer');
  return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
}
// StringSlice: an empty buffer is '' whatever the indices; the end bound is checked before
// an end below the start empties the range.
function __bufferInternalSlice(view, start, end, encoding) {
  const bytes = __bufferReceiverBytes(view);
  if (bytes.length === 0) return '';
  const from = __bufferArrayIndex(start, 0), to = __bufferArrayIndex(end, bytes.length);
  if (to > bytes.length) throw __bufferIndexError();
  return to <= from ? '' : __bufferStringDecoder(encoding)(bytes.subarray(from, to));
}
function __bufferWriteReceiver(view, string) {
  const bytes = __bufferReceiverBytes(view);
  if (typeof string !== 'string') throw __bufferError(TypeError, 'ERR_INVALID_ARG_TYPE', 'argument must be a string');
  return bytes;
}
function __bufferEncodeInto(bytes, string, at, limit, encoding) {
  const encoded = __bufferWriteEncoder(encoding)(string, limit);
  bytes.set(encoded, at);
  return encoded.length;
}
// StringWrite (base64, base64url, hex, ucs2): the length is clamped to the room left.
function __bufferInternalWrite(view, string, offset, length, encoding) {
  const bytes = __bufferWriteReceiver(view, string);
  const at = __bufferArrayIndex(offset, 0);
  if (at > bytes.length) throw __bufferOutOfBounds('offset');
  const limit = Math.min(bytes.length - at, __bufferArrayIndex(length, bytes.length - at));
  return __bufferEncodeInto(bytes, string, at, limit, encoding);
}
// The latin1 / ascii / utf8 writes: offset and length outside the buffer throw.
function __bufferChecked(value, fallback, room, name) {
  const number = value === undefined ? fallback : Number(value);
  if (number < 0 || number > room) throw __bufferOutOfBounds(name);
  return number;
}
function __bufferCheckedWrite(view, string, offset, length, encoding) {
  const bytes = __bufferWriteReceiver(view, string);
  const at = __bufferChecked(offset, 0, bytes.length, 'offset');
  const limit = __bufferChecked(length, bytes.length - at, bytes.length - at, 'length');
  return __bufferEncodeInto(bytes, string, Math.trunc(at) || 0, Math.trunc(limit) || 0, encoding);
}
function __bufferInstallInternalOps(proto) {
  for (const encoding of ['latin1', 'ascii', 'utf8', 'base64', 'base64url', 'hex', 'ucs2']) {
    const write = ['latin1', 'ascii', 'utf8'].includes(encoding) ? __bufferCheckedWrite : __bufferInternalWrite;
    proto[encoding + 'Slice'] = function(start, end) { return __bufferInternalSlice(this, start, end, encoding); };
    proto[encoding + 'Write'] = function(string, offset, length) { return write(this, string, offset, length, encoding); };
  }
}
