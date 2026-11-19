// Streaming zlib (turbo-test#404) over the native stateful handles of src/zlib_stream.rs.
// Node's shape: function constructors (callable without new) whose prototypes chain through
// ZlibBase to Transform, so subclasses such as crc32-stream's `class extends zlib.DeflateRaw`
// (archiver / zip-stream) override push and _transform and call super._transform. Every chunk
// is compressed as it is written; flush() and end() flush the compressor like Node's.
const __zlibModes = { Deflate: 1, Inflate: 2, Gzip: 3, Gunzip: 4, DeflateRaw: 5, InflateRaw: 6, Unzip: 7 };
const __zlibEmpty = Buffer.alloc(0);
// flush() writes one of these distinct empty buffers (Node's kFlushBuffers), so the flush runs
// in order with the writes before it whether the write runs at once or is buffered.
const __zlibFlushBuffers = new Map([2, 3, 4].map((kind) => [kind, Buffer.alloc(0)]));
const __zlibFlushKindOf = (chunk) => { for (const [kind, buf] of __zlibFlushBuffers) if (buf === chunk) return kind; return 0; };
function __zlibCloseHandle(stream) {
  if (stream._handle) globalThis.__ttZlibClose(stream._handle);
  stream._handle = null;
}
function __zlibOpen(stream) {
  stream._handle = globalThis.__ttZlibOpen(stream._zlibMode, stream._level);
}
function __zlibProcess(stream, chunk, flush, callback) {
  if (!stream._handle) return callback(Object.assign(new Error('zlib binding closed'), { code: 'ERR_ZLIB_BINDING_CLOSED' }));
  let out;
  try { out = globalThis.__ttZlibWrite(stream._handle, chunk, flush); } catch (error) { stream._handle = null; return callback(error); }
  stream.bytesWritten += chunk.length;
  if (flush === 4) stream._handle = null;
  if (out.length) stream.push(Buffer.from(out.buffer, out.byteOffset, out.length));
  callback();
}
// Z_FULL_FLUSH and Z_FINISH as given; Z_PARTIAL_FLUSH, Z_BLOCK and the rest as Z_SYNC_FLUSH.
const __zlibFlushKind = (kind) => (kind === 3 || kind === 4 ? kind : 2);
// Node: after end() a flush callback waits for 'end', or runs at once when already finished.
function __zlibFlushEnded(stream, callback) {
  if (!callback) return;
  if (stream.writableFinished) __streamDefer(callback);
  else stream.once('end', callback);
}
function __ZlibBase(options, mode) {
  options = options || {};
  const level = __gzipLevel(__deflateOptions(options));
  __Transform.call(this, options);
  this._zlibMode = mode;
  this._level = level;
  this.bytesWritten = 0;
  __zlibOpen(this);
}
Object.setPrototypeOf(__ZlibBase, __Transform);
Object.setPrototypeOf(__ZlibBase.prototype, __Transform.prototype);
Object.assign(__ZlibBase.prototype, {
  _transform(chunk, encoding, callback) { __zlibProcess(this, chunk, __zlibFlushKindOf(chunk), callback); },
  _flush(callback) { __zlibProcess(this, __zlibEmpty, 4, callback); },
  _destroy(error, callback) { __zlibCloseHandle(this); callback(error); },
  flush(kind, callback) {
    if (typeof kind === 'function' || kind === undefined) { callback = kind; kind = 3; }
    if (this.writableEnded) return __zlibFlushEnded(this, callback);
    this.write(__zlibFlushBuffers.get(__zlibFlushKind(kind)), callback);
  },
  close(callback) { if (callback) __streamDefer(callback); this.destroy(); },
  reset() { __zlibCloseHandle(this); __zlibOpen(this); },
});
function __zlibClass(name) {
  const mode = __zlibModes[name];
  const ctor = { [name]: function(options) {
    if (!(this instanceof ctor)) return new ctor(options);
    __ZlibBase.call(this, options, mode);
  } }[name];
  Object.setPrototypeOf(ctor, __ZlibBase);
  Object.setPrototypeOf(ctor.prototype, __ZlibBase.prototype);
  return ctor;
}
// One-shot helpers run the buffer through a stream, as Node's zlibBuffer / zlibBufferSync.
function __zlibAsync(ctor) {
  return function(input, options, callback) {
    if (typeof options === 'function') { callback = options; options = {}; }
    if (typeof callback !== 'function') throw new TypeError('The "callback" argument must be of type function');
    const bytes = __gzipBytes(input);
    const stream = new ctor(options);
    const chunks = [];
    stream.on('data', (chunk) => chunks.push(chunk));
    stream.on('error', (error) => callback(error));
    stream.on('end', () => callback(null, Buffer.concat(chunks)));
    stream.end(bytes);
  };
}
function __zlibSync(ctor) {
  return function(input, options) {
    const bytes = __gzipBytes(input);
    const stream = new ctor(options);
    const handle = stream._handle;
    stream._handle = null;
    return Buffer.from(globalThis.__ttZlibWrite(handle, bytes, 4));
  };
}
const __nodeZlib = {};
for (const name of Object.keys(__zlibModes)) {
  const ctor = __zlibClass(name);
  const call = name[0].toLowerCase() + name.slice(1);
  Object.assign(__nodeZlib, { [name]: ctor, ['create' + name]: (options) => new ctor(options), [call]: __zlibAsync(ctor) });
  __nodeZlib[call + 'Sync'] = __zlibSync(ctor);
}
