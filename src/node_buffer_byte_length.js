// Count encoded bytes without decoding permissive hex/base64 into a different byte sequence.
const __bufferArrayByteLength = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength').get;
const __bufferSharedByteLength = typeof SharedArrayBuffer === 'function' ? Object.getOwnPropertyDescriptor(SharedArrayBuffer.prototype, 'byteLength').get : null;
function __bufferMemoryLength(value) {
  if (ArrayBuffer.isView(value)) return value.byteLength;
  try { return __bufferArrayByteLength.call(value); } catch {}
  if (__bufferSharedByteLength) { try { return __bufferSharedByteLength.call(value); } catch {} }
  throw Object.assign(new TypeError('The string argument must be a string, Buffer, ArrayBuffer or ArrayBufferView'), { code: 'ERR_INVALID_ARG_TYPE' });
}
function __bufferSurrogatePair(code, next) {
  return code >= 0xd800 && code <= 0xdbff && next >= 0xdc00 && next <= 0xdfff;
}
function __bufferUtf8Length(value) {
  let bytes = 0;
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    if (code < 0x80) bytes++;
    else if (code < 0x800) bytes += 2;
    else if (__bufferSurrogatePair(code, value.charCodeAt(index + 1))) { bytes += 4; index++; }
    else bytes += 3;
  }
  return bytes;
}
function __bufferBase64Length(value) {
  let length = value.length;
  if (value.charCodeAt(length - 1) === 61) length--;
  if (value.charCodeAt(length - 1) === 61) length--;
  return Math.floor(length * 3 / 4);
}
const __bufferSingleLength = (value) => value.length;
const __bufferDoubleLength = (value) => value.length * 2;
const __bufferHexLength = (value) => Math.floor(value.length / 2);
const __bufferLengthHandlers = Object.assign(Object.create(null), {
  utf8: __bufferUtf8Length, 'utf-8': __bufferUtf8Length,
  ascii: __bufferSingleLength, latin1: __bufferSingleLength, binary: __bufferSingleLength,
  ucs2: __bufferDoubleLength, 'ucs-2': __bufferDoubleLength, utf16le: __bufferDoubleLength, 'utf-16le': __bufferDoubleLength,
  hex: __bufferHexLength, base64: __bufferBase64Length, base64url: __bufferBase64Length,
});
function __bufferByteLength(value, encoding) {
  if (typeof value !== 'string') return __bufferMemoryLength(value);
  const handler = typeof encoding === 'string' ? __bufferLengthHandlers[encoding.toLowerCase()] : null;
  return (handler || __bufferUtf8Length)(value);
}
