// ---- fs access / chmod / rename / open + FileHandle (turbo-test#420) ----
// Real syscalls through the natives in src/fs_handles.rs, failing with Node's errors; the
// argument checks and FileHandle's closed state live here. The descriptor natives exist on unix
// hosts only: elsewhere these calls report ERR_FEATURE_UNAVAILABLE_ON_PLATFORM.
const __fsNative = (name, api) => {
  const native = globalThis[name];
  if (typeof native === 'function') return native;
  throw Object.assign(new Error(`fs.${api} is not supported by turbo-test on this platform`), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
};
const __fsAccessMode = (mode) => {
  if (mode === undefined) return 0;
  if (typeof mode !== 'number') throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'The "mode" argument must be of type number');
  if (!Number.isInteger(mode) || mode < 0 || mode > 7) throw __fsArgumentError('ERR_OUT_OF_RANGE', `The value of "mode" is out of range. It must be an integer >= 0 && <= 7. Received ${mode}`, RangeError);
  return mode;
};
// Node's parseFileMode: a uint32, or a string of octal digits
const __fsOctalMode = (mode, name) => {
  if (/^[0-7]+$/.test(mode)) return parseInt(mode, 8);
  throw __fsArgumentError('ERR_INVALID_ARG_VALUE', `The argument '${name}' must be a 32-bit unsigned integer or an octal string. Received '${mode}'`);
};
const __fsFileMode = (mode, name, fallback) => {
  if (mode === undefined || mode === null) return fallback;
  if (typeof mode === 'string') return __fsOctalMode(mode, name);
  if (!Number.isInteger(mode) || mode < 0 || mode > 4294967295) throw __fsArgumentError('ERR_OUT_OF_RANGE', `The value of "${name}" is out of range. It must be >= 0 && <= 4294967295. Received ${mode}`, RangeError);
  return mode;
};
const __accessSync = (p, mode) => { __fsNative('__fs_access', 'access')(__fsCheckedPath(p, 'path'), __fsAccessMode(mode)); };
const __chmodSync = (p, mode) => { __fsNative('__fs_chmod', 'chmod')(__fsCheckedPath(p, 'path'), __fsFileMode(mode, 'mode')); };
const __renameSync = (from, to) => { __fsNative('__fs_rename', 'rename')(__fsCheckedPath(from, 'oldPath'), __fsCheckedPath(to, 'newPath')); };
const __openFlags = (flags) => {
  if (flags === undefined || flags === null) return __fsNative('__fs_openFlags', 'open')('r');
  if (typeof flags === 'number') return flags;
  const value = __fsNative('__fs_openFlags', 'open')(String(flags));
  if (value === undefined) throw __fsArgumentError('ERR_INVALID_ARG_VALUE', `The argument 'flags' is invalid. Received '${flags}'`);
  return value;
};
// a call on a closed FileHandle: Node's EBADF 'file closed'
const __fileClosed = (syscall) => Object.assign(new Error('file closed'), { code: 'EBADF', syscall });
const __bytesOf = (data, encoding) => (typeof data === 'string' ? Buffer.from(data, encoding || 'utf8') : data);
// read(buffer, offset, length, position) / read(buffer, options) / read(options) / read()
const __isOptions = (value) => value !== null && typeof value === 'object' && !ArrayBuffer.isView(value);
// (offset, length, position) or an options object after the view; the length defaults to the rest
const __viewArgs = (view, offset, length, position) => {
  if (__isOptions(offset)) ({ offset, length, position } = offset);
  offset = offset || 0;
  return { offset, length: length === undefined ? view.byteLength - offset : length, position };
};
const __readArgs = (buffer, ...rest) => {
  if (__isOptions(buffer)) return __readArgs(buffer.buffer, buffer);
  if (buffer === undefined) buffer = Buffer.alloc(16384);
  return { buffer, ...__viewArgs(buffer, ...rest) };
};
const __fsPosition = (position) => (position === null || position === undefined ? -1 : Number(position));
const __checkRange = (view, offset, length) => {
  if (!(offset >= 0 && length >= 0 && offset + length <= view.byteLength)) throw __fsArgumentError('ERR_OUT_OF_RANGE', 'The value of "length" is out of range.', RangeError);
};
const __bytesView = (view) => new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
class FileHandle {
  #fd;
  constructor(fd) { this.#fd = fd; }
  get fd() { return this.#fd; }
  #open(syscall) { if (this.#fd < 0) throw __fileClosed(syscall); return this.#fd; }
  #op(op, value) { return globalThis.__fs_fdOp(op, this.#open(op), value); }
  #transfer(native, syscall, view, offset, length, position) {
    __checkRange(view, offset, length);
    return globalThis[native](this.#open(syscall), __bytesView(view), offset, length, __fsPosition(position));
  }
  async read(...args) {
    const { buffer, offset, length, position } = __readArgs(...args);
    const bytesRead = this.#transfer('__fs_read', 'read', buffer, offset, length, position);
    return { __proto__: null, bytesRead, buffer };
  }
  // write(buffer, offset, length, position) / write(buffer, options) / write(string, position, encoding)
  async write(data, offset, length, position) {
    if (typeof data === 'string') {
      const bytes = Buffer.from(data, length || 'utf8');
      return { __proto__: null, bytesWritten: this.#transfer('__fs_write', 'write', bytes, 0, bytes.length, offset), buffer: data };
    }
    const args = __viewArgs(data, offset, length, position);
    return { __proto__: null, bytesWritten: this.#transfer('__fs_write', 'write', data, args.offset, args.length, args.position), buffer: data };
  }
  // the rest of the file from the current position
  async readFile(options) {
    const chunks = [];
    for (let chunk = Buffer.alloc(65536), n; (n = this.#transfer('__fs_read', 'read', chunk, 0, chunk.length, null)) > 0; chunk = Buffer.alloc(65536)) chunks.push(chunk.subarray(0, n));
    const all = Buffer.concat(chunks), encoding = __encodingOf(options);
    return encoding ? all.toString(encoding) : all;
  }
  // every byte, at the current position (the end in append mode)
  async writeFile(data, options) {
    const bytes = __bytesOf(data, __encodingOf(options));
    for (let done = 0; done < bytes.byteLength;) done += this.#transfer('__fs_write', 'write', bytes, done, bytes.byteLength - done, null);
  }
  appendFile(data, options) { return this.writeFile(data, options); }
  async stat() { return new Stats(this.#op('fstat')); }
  async truncate(len) { this.#op('ftruncate', len || 0); }
  async sync() { this.#op('fsync'); }
  async datasync() { this.#op('fdatasync'); }
  async chmod(mode) { this.#op('fchmod', __fsFileMode(mode, 'mode')); }
  async close() {
    if (this.#fd < 0) return;
    const fd = this.#fd;
    this.#fd = -1;
    globalThis.__fs_fdOp('close', fd);
  }
  [Symbol.asyncDispose]() { return this.close(); }
}
const __openOperation = (p, flags, mode) => {
  const path = __fsCheckedPath(p, 'path');
  return new FileHandle(__fsNative('__fs_open', 'open')(path, __openFlags(flags), __fsFileMode(mode, 'mode', 0o666)));
};
Object.assign(__fs, { accessSync: __accessSync, chmodSync: __chmodSync, renameSync: __renameSync });
Object.assign(__fs.promises, {
  access: __promised(__accessSync),
  chmod: __promised(__chmodSync),
  rename: __promised(__renameSync),
  realpath: __promised(__realpathNative),
  open: __promised(__openOperation),
});
