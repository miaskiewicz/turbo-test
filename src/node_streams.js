// In-memory Node stream lifecycle. Host sockets/processes supply their own I/O.
// Legacy consumers call Stream.call(this); an ES class cannot provide that API.
function __Stream() { this._ev = Object.create(null); }
Object.setPrototypeOf(__Stream, __EventEmitter);
Object.setPrototypeOf(__Stream.prototype, __EventEmitter.prototype);
__Stream.prototype.pipe = function(destination, options) { return __legacyPipe(this, destination, options); };
function __legacyPipeListen(state, target, name, listener, prepend = false) {
  state.listeners.push([target, name, listener]);
  if (prepend) target.prependListener(name, listener);
  else target.on(name, listener);
}
function __legacyPipeCleanup(state) {
  const listeners = state.listeners.splice(0);
  for (const [target, name, listener] of listeners) target.removeListener(name, listener);
}
function __legacyPipeFinish(state, closed) {
  if (state.completed) return;
  state.completed = true;
  try {
    if (!state.ending) return;
    if (!closed) state.destination.end();
    else if (typeof state.destination.destroy === 'function') state.destination.destroy();
  } finally { __legacyPipeCleanup(state); }
}
function __legacyPipe(source, destination, options = {}) {
  const state = { destination, listeners: [], completed: false, ending: !destination._isStdio && (!options || options.end !== false) };
  const data = (chunk) => { if (destination.writable && destination.write(chunk) === false && source.pause) source.pause(); };
  const drain = () => { if (source.readable && source.resume) source.resume(); };
  const cleanup = () => __legacyPipeCleanup(state);
  const error = function(failure) {
    cleanup();
    if (typeof this.listenerCount === 'function' && this.listenerCount('error') === 0) this.emit('error', failure);
  };
  __legacyPipeListen(state, source, 'data', data);
  __legacyPipeListen(state, destination, 'drain', drain);
  __legacyPipeListen(state, source, 'end', () => __legacyPipeFinish(state, false));
  __legacyPipeListen(state, source, 'close', () => __legacyPipeFinish(state, true));
  __legacyPipeListen(state, source, 'error', error, true);
  __legacyPipeListen(state, destination, 'error', error, true);
  __legacyPipeListen(state, destination, 'close', cleanup);
  destination.emit('pipe', source);
  return destination;
}
const __streamDefer = (fn) => queueMicrotask(fn);
// Node schedules the write/finish/destroy lifecycle with process.nextTick (ahead of promise jobs).
const __streamTick = (fn) => process.nextTick(fn);
const __streamNoop = () => {};
function __streamError(code, message) { const error = new Error(message); error.code = code; return error; }
function __streamInit(stream, options) {
  stream.destroyed = false;
  stream.closed = false;
  stream.errored = null;
  stream._autoDestroy = options.autoDestroy !== false;
  stream._emitClose = options.emitClose !== false;
  if (options.destroy) stream._destroy = options.destroy;
  if (options.construct) stream._construct = options.construct;
  stream._constructed = typeof stream._construct !== 'function';
  if (!stream._constructed) __streamTick(() => __streamConstruct(stream));
}
// As Node's construct(): _construct runs on the next tick. Until its callback (completed on a
// later tick), writes buffer, finish and _read wait, and destroy() defers _destroy.
function __streamConstruct(stream) {
  let called = false;
  const done = (error) => {
    if (called) { __streamErrorOrDestroy(stream, error || __streamError('ERR_MULTIPLE_CALLBACK', 'Callback called multiple times')); return; }
    called = true;
    __streamTick(() => __streamConstructed(stream, error));
  };
  try { stream._construct(done); } catch (error) { done(error); }
}
function __streamConstructed(stream, error) {
  stream._constructed = true;
  if (stream.destroyed) __streamRunDestroy(stream, stream._pendingDestroy || error);
  else if (error) __streamErrorOrDestroy(stream, error, true);
  else __streamResumeConstructed(stream);
}
function __streamResumeConstructed(stream) {
  if (stream._ws) __writableConstructed(stream, stream._ws);
  if (stream._rs) __readableSchedule(stream);
}
function __streamRecordError(stream, error) {
  if (error && !stream.errored) stream.errored = error;
}
function __streamEmitError(stream, error) {
  if (stream._errorEmitted) return;
  stream._errorEmitted = true;
  stream.emit('error', error);
}
function __streamEmitClose(stream, error) {
  if (error) __streamEmitError(stream, error);
  if (stream._emitClose) stream.emit('close');
}
// As Node's destroy(): _destroy runs now; 'error' and 'close' follow on the next tick.
function __streamDestroy(stream, error) {
  if (stream.destroyed) return stream;
  if (stream._ws) __writableDestroyBuffered(stream, stream._ws);
  __streamRecordError(stream, error);
  stream.destroyed = true;
  if (stream._rs) stream._rs.queue.length = 0;
  if (stream._constructed) __streamRunDestroy(stream, error);
  else stream._pendingDestroy = error;
  return stream;
}
function __streamRunDestroy(stream, error) {
  let called = false;
  const done = (failure) => {
    if (called) return;
    called = true;
    __streamRecordError(stream, failure);
    stream.closed = true;
    __streamTick(() => __streamEmitClose(stream, failure));
  };
  try { stream._destroy(error || null, done); } catch (failure) { done(failure); }
}
// Node's errorOrDestroy: autoDestroy streams are destroyed, others only emit the error.
function __streamErrorOrDestroy(stream, error, sync) {
  if (stream.destroyed) return;
  if (stream._autoDestroy) { stream.destroy(error); return; }
  if (!error) return;
  __streamRecordError(stream, error);
  if (sync) __streamTick(() => __streamEmitError(stream, error));
  else __streamEmitError(stream, error);
}
function __streamAutoDestroy(stream) {
  if (!stream._autoDestroy) return;
  const readableDone = !stream._rs || stream.readableEnded;
  const writableDone = !stream._ws || stream.writableFinished;
  if (readableDone && writableDone) stream.destroy();
}
function __streamChunk(chunk, state, encoding) {
  if (chunk === null) throw new TypeError('May not write null values to stream');
  if (state.objectMode) return chunk;
  if (typeof chunk === 'string') return Buffer.from(chunk, encoding);
  if (Buffer.isBuffer(chunk)) return chunk;
  if (chunk instanceof Uint8Array) return Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
  throw new TypeError('The chunk must be a string, Buffer or Uint8Array');
}
function __streamSize(chunk, state) { return state.objectMode ? 1 : chunk.length; }
const __streamEncodings = new Set(['utf8', 'utf-8', 'hex', 'base64', 'base64url', 'ascii', 'latin1', 'binary', 'ucs2', 'ucs-2', 'utf16le', 'utf-16le']);
function __streamIsEncoding(encoding) { return typeof encoding === 'string' && __streamEncodings.has(encoding.toLowerCase()); }
function __streamUnknownEncoding(encoding) {
  const error = new TypeError('Unknown encoding: ' + encoding);
  error.code = 'ERR_UNKNOWN_ENCODING';
  return error;
}
function __streamWatermark(options, mode, side) {
  const value = options[side + 'HighWaterMark'] ?? options.highWaterMark ?? (mode ? 16 : 16384);
  if (!Number.isInteger(value) || value < 0) throw new RangeError('Invalid highWaterMark');
  return value;
}
function __readableInit(stream, options) {
  const objectMode = Boolean(options.objectMode || options.readableObjectMode);
  stream._rs = { queue: [], length: 0, objectMode, highWaterMark: __streamWatermark(options, objectMode, 'readable'),
    flowing: false, reading: false, ended: false, scheduled: false, encoding: null };
  stream.readable = options.readable !== false;
  stream.readableEnded = !stream.readable;
  if (options.read) stream._read = options.read;
}
function __readableSchedule(stream) {
  if (stream._rs.scheduled || stream.destroyed) return;
  stream._rs.scheduled = true;
  __streamTick(() => { stream._rs.scheduled = false; __readableFlow(stream); });
}
function __readableEnd(stream) {
  if (stream.readableEnded || stream.destroyed) return;
  stream.readableEnded = true;
  stream.readable = false;
  if (stream._rs.decoder) { const tail = stream._rs.decoder.decode(); if (tail) stream.emit('data', tail); }
  stream.emit('end');
  __streamAutoDestroy(stream);
}
function __readablePull(stream) {
  const state = stream._rs;
  if (state.reading || state.ended || stream.destroyed || !stream._constructed) return;
  state.reading = true;
  state.sync = true;
  try { stream._read(state.highWaterMark); } catch (error) { stream.destroy(error); }
  state.sync = false;
}
function __readableDecode(stream, chunk) {
  const state = stream._rs;
  if (state.decoder) return state.decoder.decode(chunk, { stream: true });
  return state.encoding ? chunk.toString(state.encoding) : chunk;
}
function __readableTake(stream) {
  const state = stream._rs;
  const chunk = state.queue.shift();
  state.length -= __streamSize(chunk, state);
  return __readableDecode(stream, chunk);
}
function __readableFlow(stream) {
  const state = stream._rs;
  if (!state.flowing || stream.destroyed) return;
  __readableDrain(stream);
  if (!state.flowing || state.queue.length) return;
  if (state.ended) __readableScheduleEnd(stream);
  else __readablePull(stream);
}
// As Node's endReadable: 'end' follows on a nextTick, queued as the last chunk is taken (before
// its 'data' runs), and only if nothing was buffered again meanwhile.
function __readableScheduleEnd(stream) {
  const state = stream._rs;
  if (state.endScheduled) return;
  state.endScheduled = true;
  __streamTick(() => { state.endScheduled = false; if (!state.queue.length) __readableEnd(stream); });
}
function __readableDrain(stream) {
  const state = stream._rs;
  while (state.flowing && state.queue.length) {
    const chunk = __readableTake(stream);
    if (state.ended && !state.queue.length) __readableScheduleEnd(stream);
    if (chunk !== '') stream.emit('data', chunk);
  }
}
function __readablePush(stream, chunk, encoding) {
  const state = stream._rs;
  if (stream.destroyed) return false;
  state.reading = false;
  if (chunk === null) { state.ended = true; __readableSchedule(stream); return false; }
  if (state.ended) { stream.destroy(__streamError('ERR_STREAM_PUSH_AFTER_EOF', 'stream.push() after EOF')); return false; }
  __readableAddChunk(stream, state, __streamChunk(chunk, state, encoding));
  return state.length < state.highWaterMark;
}
// As Node's addChunk: a chunk pushed outside _read into a flowing, empty stream with 'data'
// listeners is emitted at once; otherwise it is buffered for the next flow (a nextTick).
function __readableEmitsNow(stream, state) {
  return state.flowing && !state.sync && !state.queue.length && stream.listenerCount('data') > 0;
}
function __readableAddChunk(stream, state, value) {
  if (__readableEmitsNow(stream, state)) {
    const chunk = __readableDecode(stream, value);
    if (chunk !== '') stream.emit('data', chunk);
  } else {
    state.queue.push(value);
    state.length += __streamSize(value, state);
    stream.emit('readable');
  }
  __readableSchedule(stream);
}
function __readableFrom(iterable, options) {
  const iterator = iterable[Symbol.asyncIterator]?.() || iterable[Symbol.iterator]?.();
  if (!iterator) throw new TypeError('iterable must be an Iterable');
  const stream = new __Readable({ objectMode: true, ...options });
  stream._read = () => {
    Promise.resolve().then(() => iterator.next()).then((item) => {
      if (stream.destroyed) return;
      stream.push(item.done ? null : item.value);
    }, (error) => stream.destroy(error));
  };
  const destroy = stream._destroy;
  stream._destroy = (error, callback) => {
    const close = iterator.return;
    if (!close) return destroy.call(stream, error, callback);
    Promise.resolve().then(() => close.call(iterator)).then(
      () => destroy.call(stream, error, callback),
      (failure) => destroy.call(stream, error || failure, callback));
  };
  return stream;
}
function __readablePipe(source, destination, options = {}) {
  const data = (chunk) => { if (destination.write(chunk) === false && source.pause) source.pause(); };
  const drain = () => { if (source.resume) source.resume(); };
  let detached = false;
  const cleanup = () => {
    if (detached) return;
    detached = true;
    source.off('data', data); source.off('end', end);
    destination.off('drain', drain); destination.off('close', cleanup);
    destination.off('finish', cleanup); destination.off('error', error);
  };
  const end = () => { cleanup(); if (options.end !== false) destination.end(); };
  const error = (failure) => {
    cleanup();
    if (destination.listenerCount('error') === 0) destination.emit('error', failure);
  };
  destination.on('drain', drain);
  destination.on('close', cleanup);
  destination.on('finish', cleanup);
  destination.prependListener('error', error);
  source.on('end', end);
  source.on('data', data);
  destination.emit('pipe', source);
  return destination;
}

class __Readable extends __Stream {
  constructor(options = {}) { super(); __streamInit(this, options); __readableInit(this, options); }
  static from(iterable, options = {}) { return __readableFrom(iterable, options); }
  _read() { this.destroy(__streamError('ERR_METHOD_NOT_IMPLEMENTED', '_read() is not implemented')); }
  _destroy(error, callback) { callback(error); }
  push(chunk, encoding) { return __readablePush(this, chunk, encoding); }
  read() {
    const state = this._rs;
    if (state.queue.length) { const chunk = __readableTake(this); __readableSchedule(this); return chunk; }
    if (state.ended) __streamTick(() => __readableEnd(this));
    else __readablePull(this);
    return null;
  }
  on(event, listener) { super.on(event, listener); if (event === 'data') this.resume(); return this; }
  pipe(destination, options) { return __readablePipe(this, destination, options); }
  pause() { this._rs.flowing = false; return this; }
  resume() { this._rs.flowing = true; __readableSchedule(this); return this; }
  isPaused() { return !this._rs.flowing; }
  setEncoding(encoding) {
    this._rs.encoding = encoding;
    if (/^utf-?8$/i.test(encoding)) this._rs.decoder = new TextDecoder('utf-8', { ignoreBOM: true });
    return this;
  }
  destroy(error) { return __streamDestroy(this, error); }
  get readableLength() { return this._rs.length; }
  get readableHighWaterMark() { return this._rs.highWaterMark; }
}
// Writable side, ported from Node v26.11.0 lib/internal/streams/writable.js: _write runs inside
// write() when nothing is pending or corked; later chunks buffer until its callback (a batch
// goes to _writev); a callback made during _write is completed on the next tick, a later one
// at once. 'drain' only follows a buffer that was full and has emptied.
function __writableInit(stream, options) {
  const objectMode = Boolean(options.objectMode || options.writableObjectMode);
  stream._ws = { buffered: [], length: 0, objectMode, highWaterMark: __streamWatermark(options, objectMode, 'writable'),
    writing: false, corked: 0, sync: true, bufferProcessing: false, writelen: 0, writecb: null, expectCallback: false,
    pendingcb: 0, afterWritePending: false, tickInfo: null, needDrain: false, ending: false, finalCalled: false,
    prefinished: false, onFinished: [], onwrite: (error) => __writableOnwrite(stream, error),
    decodeStrings: options.decodeStrings !== false, defaultEncoding: __writableDefaultEncoding(options.defaultEncoding) };
  stream.writable = options.writable !== false;
  stream.writableFinished = !stream.writable;
  stream.writableEnded = !stream.writable;
  if (options.write) stream._write = options.write;
  if (options.writev) stream._writev = options.writev;
  if (options.final) stream._final = options.final;
}
function __writableDefaultEncoding(encoding) {
  if (encoding == null) return 'utf8';
  if (!__streamIsEncoding(encoding)) throw __streamUnknownEncoding(encoding);
  return encoding;
}
function __writableSetDefaultEncoding(stream, encoding) {
  if (typeof encoding === 'string') encoding = encoding.toLowerCase();
  if (!__streamIsEncoding(encoding)) throw __streamUnknownEncoding(encoding);
  stream._ws.defaultEncoding = encoding;
  return stream;
}
// As Node's _write(): a string is decoded with its (or the default) encoding unless
// decodeStrings is false, a Buffer passes as is, and an unknown encoding throws.
function __writableEncoding(state, encoding) {
  if (!encoding) return state.defaultEncoding;
  if (encoding !== 'buffer' && !__streamIsEncoding(encoding)) throw __streamUnknownEncoding(encoding);
  return encoding;
}
function __writableString(state, chunk, encoding) {
  if (encoding === 'buffer') throw __streamUnknownEncoding(encoding);
  if (!state.decodeStrings) return { chunk, encoding };
  return { chunk: Buffer.from(chunk, encoding), encoding: 'buffer' };
}
function __writableChunk(state, chunk, encoding) {
  if (chunk === null) throw Object.assign(new TypeError('May not write null values to stream'), { code: 'ERR_STREAM_NULL_VALUES' });
  if (state.objectMode) return { chunk, encoding };
  encoding = __writableEncoding(state, encoding);
  if (typeof chunk === 'string') return __writableString(state, chunk, encoding);
  if (Buffer.isBuffer(chunk)) return { chunk, encoding: 'buffer' };
  if (chunk instanceof Uint8Array) return { chunk: Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength), encoding: 'buffer' };
  throw Object.assign(new TypeError('The "chunk" argument must be of type string or an instance of Buffer, TypedArray, or DataView'), { code: 'ERR_INVALID_ARG_TYPE' });
}
function __writableIsWritable(stream) {
  const state = stream._ws;
  return Boolean(state) && state.writable !== false && !state.ending && !stream.destroyed && !stream.errored;
}
function __writableConstructed(stream, state) {
  if (!state.writing) __writableClearBuffer(stream, state);
  if (state.ending) __writableFinishMaybe(stream, state, false);
}
function __writableWrite(stream, chunk, encoding, callback) {
  if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
  return __writableAccept(stream, chunk, encoding, callback) === true;
}
function __writableAccept(stream, chunk, encoding, callback) {
  if (typeof callback !== 'function') callback = __streamNoop;
  const state = stream._ws;
  const value = __writableChunk(state, chunk, encoding);
  const refusal = __writableRefusal(stream, state);
  if (refusal) {
    __streamTick(() => callback(refusal));
    __streamErrorOrDestroy(stream, refusal, true);
    return refusal;
  }
  state.pendingcb++;
  return __writableWriteOrBuffer(stream, state, { size: __streamSize(value.chunk, state), chunk: value.chunk,
    encoding: value.encoding, callback });
}
function __writableRefusal(stream, state) {
  if (state.ending) return __streamError('ERR_STREAM_WRITE_AFTER_END', 'write after end');
  if (stream.destroyed) return __streamError('ERR_STREAM_DESTROYED', 'Cannot call write after a stream was destroyed');
  return null;
}
function __writableBusy(stream, state) { return state.writing || state.corked > 0 || Boolean(stream.errored) || !stream._constructed; }
function __writableWriteOrBuffer(stream, state, job) {
  state.length += job.size;
  if (__writableBusy(stream, state)) state.buffered.push(job);
  else __writableDo(stream, state, job);
  const ready = state.length < state.highWaterMark || state.length === 0;
  if (!ready) state.needDrain = true;
  return ready && !stream.destroyed && !stream.errored;
}
function __writableDo(stream, state, job) {
  state.writelen = job.size; state.writecb = job.callback;
  state.writing = true; state.sync = true; state.expectCallback = true;
  if (stream.destroyed) state.onwrite(__streamError('ERR_STREAM_DESTROYED', 'Cannot call write after a stream was destroyed'));
  else if (job.chunks) stream._writev(job.chunks, state.onwrite);
  else stream._write(job.chunk, job.encoding, state.onwrite);
  state.sync = false;
}
function __writableOnwrite(stream, error) {
  const state = stream._ws;
  if (!state.expectCallback) {
    __streamErrorOrDestroy(stream, __streamError('ERR_MULTIPLE_CALLBACK', 'Callback called multiple times'));
    return;
  }
  const { sync, writecb: callback } = state;
  state.writecb = null; state.writing = false; state.expectCallback = false;
  state.length -= state.writelen; state.writelen = 0;
  if (error) __writableOnwriteFailure(stream, state, error, sync, callback);
  else __writableOnwriteSuccess(stream, state, sync, callback);
}
function __writableOnwriteFailure(stream, state, error, sync, callback) {
  __streamRecordError(stream, error);
  const fail = () => {
    state.pendingcb--;
    callback(error);
    __writableErrorBuffer(stream, state);
    __streamErrorOrDestroy(stream, error);
  };
  if (sync) __streamTick(fail); else fail();
}
function __writableOnwriteSuccess(stream, state, sync, callback) {
  if (state.buffered.length) __writableClearBuffer(stream, state);
  if (!sync) __writableAfterWrite(stream, state, 1, callback);
  else if (state.tickInfo && state.tickInfo.callback === callback) state.tickInfo.count++;
  else if (__writableNeedsTick(stream, state, callback)) __writableScheduleAfterWrite(stream, state, callback);
  else __writableSettle(stream, state);
}
// Node's onwrite tests `state[kState] & kDestroyed !== 0`, which binds as `& 1`: its object-mode
// bit. So object mode (not a destroyed stream) is what makes a callback-less write tick.
function __writableNeedsTick(stream, state, callback) {
  if (callback !== __streamNoop) return true;
  if (state.afterWritePending) return false;
  return (state.needDrain && state.length === 0) || state.objectMode;
}
function __writableScheduleAfterWrite(stream, state, callback) {
  const info = { count: 1, callback };
  const counted = callback !== __streamNoop;
  if (counted) state.tickInfo = info;
  state.afterWritePending = true;
  __streamTick(() => {
    if (counted) state.tickInfo = null;
    __writableAfterWrite(stream, state, info.count, callback);
  });
}
function __writableSettle(stream, state) {
  state.pendingcb--;
  if (state.ending) __writableFinishMaybe(stream, state);
}
function __writableNeedsDrain(stream, state) {
  return state.needDrain && !state.ending && !stream.destroyed && state.length === 0;
}
function __writableAfterWrite(stream, state, count, callback) {
  state.afterWritePending = false;
  if (__writableNeedsDrain(stream, state)) { state.needDrain = false; stream.emit('drain'); }
  for (; count > 0; count--) { state.pendingcb--; callback(null); }
  if (stream.destroyed) __writableErrorBuffer(stream, state);
  if (state.ending) __writableFinishMaybe(stream, state);
}
function __writableErrorBuffer(stream, state) {
  if (state.writing) return;
  for (const job of state.buffered.splice(0)) {
    state.length -= job.size;
    job.callback(stream.errored || __streamError('ERR_STREAM_DESTROYED', 'Cannot call write after a stream was destroyed'));
  }
  __writableCallFinished(state, stream.errored || __streamError('ERR_STREAM_DESTROYED', 'Cannot call end after a stream was destroyed'));
}
function __writableDestroyBuffered(stream, state) {
  if (state.buffered.length || state.onFinished.length) __streamTick(() => __writableErrorBuffer(stream, state));
}
function __writableCallFinished(state, error) {
  for (const callback of state.onFinished.splice(0)) callback(error);
}
function __writableCanFlush(stream, state) {
  return !stream.destroyed && stream._constructed && !state.bufferProcessing && !state.corked && state.buffered.length > 0;
}
function __writableClearBuffer(stream, state) {
  if (!__writableCanFlush(stream, state)) return;
  state.bufferProcessing = true;
  if (state.buffered.length > 1 && stream._writev) __writableFlushBatch(stream, state);
  else __writableFlushEach(stream, state);
  state.bufferProcessing = false;
}
function __writableFlushBatch(stream, state) {
  const chunks = state.buffered.splice(0);
  state.pendingcb -= chunks.length - 1;
  const silent = chunks.every((job) => job.callback === __streamNoop);
  const callback = silent ? __streamNoop : (error) => { for (const job of chunks) job.callback(error); };
  chunks.allBuffers = chunks.every((job) => job.encoding === 'buffer');
  __writableDo(stream, state, { size: state.length, chunks, callback });
}
function __writableFlushEach(stream, state) {
  do __writableDo(stream, state, state.buffered.shift());
  while (state.buffered.length && !state.writing);
}
function __writableUncork(stream) {
  const state = stream._ws;
  if (!state.corked) return;
  state.corked--;
  if (!state.writing) __writableClearBuffer(stream, state);
}
function __writableEnd(stream, chunk, encoding, callback) {
  if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
  else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
  const state = stream._ws;
  const written = chunk != null ? __writableAccept(stream, chunk, encoding) : null;
  if (state.corked) { state.corked = 1; stream.uncork(); }
  const error = written instanceof Error ? written : __writableEnding(stream, state);
  __writableEndCallback(stream, state, error, callback);
  return stream;
}
function __writableEnding(stream, state) {
  if (!state.ending && !stream.errored) {
    state.ending = true;
    stream.writableEnded = true;
    __writableFinishMaybe(stream, state);
    return null;
  }
  if (stream.writableFinished) return __streamError('ERR_STREAM_ALREADY_FINISHED', 'Cannot call end after a stream was finished');
  if (stream.destroyed) return __streamError('ERR_STREAM_DESTROYED', 'Cannot call end after a stream was destroyed');
  return null;
}
function __writableEndCallback(stream, state, error, callback) {
  if (typeof callback !== 'function') return;
  const failure = error || stream.errored;
  if (failure) __streamTick(() => callback(failure));
  else if (stream.writableFinished) __streamTick(() => callback(null));
  else state.onFinished.push(callback);
}
function __writableIdle(state) { return !state.writing && !state.buffered.length && state.length === 0; }
function __writableNeedFinish(stream, state) {
  return state.ending && stream._constructed && !stream.destroyed && !stream.writableFinished && !stream.errored && __writableIdle(state);
}
function __writablePrefinish(stream, state) {
  if (state.prefinished || state.finalCalled) return;
  state.finalCalled = true;
  if (typeof stream._final !== 'function' || stream.destroyed) { state.prefinished = true; stream.emit('prefinish'); return; }
  state.sync = true;
  state.pendingcb++;
  try { stream._final((error) => __writableOnFinish(stream, state, error)); }
  catch (error) { __writableOnFinish(stream, state, error); }
  state.sync = false;
}
function __writableOnFinish(stream, state, error) {
  if (state.prefinished) {
    __streamErrorOrDestroy(stream, error || __streamError('ERR_MULTIPLE_CALLBACK', 'Callback called multiple times'));
    return;
  }
  state.pendingcb--;
  if (error) { __writableCallFinished(state, error); __streamErrorOrDestroy(stream, error, state.sync); return; }
  if (!__writableNeedFinish(stream, state)) return;
  state.prefinished = true;
  stream.emit('prefinish');
  state.pendingcb++;
  __streamTick(() => __writableFinish(stream, state));
}
function __writableFinishMaybe(stream, state, sync = true) {
  if (!__writableNeedFinish(stream, state)) return;
  __writablePrefinish(stream, state);
  if (state.pendingcb !== 0) return;
  if (sync) { state.pendingcb++; __streamTick(() => __writableFinishTick(stream, state)); }
  else if (__writableNeedFinish(stream, state)) { state.pendingcb++; __writableFinish(stream, state); }
}
function __writableFinishTick(stream, state) {
  if (__writableNeedFinish(stream, state)) __writableFinish(stream, state);
  else state.pendingcb--;
}
function __writableFinish(stream, state) {
  state.pendingcb--;
  stream.writableFinished = true;
  __writableCallFinished(state, null);
  stream.emit('finish');
  __streamAutoDestroy(stream);
}
function __writableDefaultWrite(stream, chunk, encoding, callback) {
  if (!stream._writev) throw __streamError('ERR_METHOD_NOT_IMPLEMENTED', 'The _write() method is not implemented');
  stream._writev([{ chunk, encoding }], callback);
}
class __Writable extends __Stream {
  constructor(options = {}) { super(); __streamInit(this, options); __writableInit(this, options); }
  _write(chunk, encoding, callback) { __writableDefaultWrite(this, chunk, encoding, callback); }
  _destroy(error, callback) { callback(error); }
  write(chunk, encoding, callback) { return __writableWrite(this, chunk, encoding, callback); }
  end(chunk, encoding, callback) { return __writableEnd(this, chunk, encoding, callback); }
  cork() { this._ws.corked++; }
  uncork() { __writableUncork(this); }
  setDefaultEncoding(encoding) { return __writableSetDefaultEncoding(this, encoding); }
  destroy(error) { return __streamDestroy(this, error); }
  get writable() { return __writableIsWritable(this); }
  set writable(value) { if (this._ws) this._ws.writable = value; }
  get writableLength() { return this._ws.length; }
  get writableHighWaterMark() { return this._ws.highWaterMark; }
  get writableNeedDrain() { return this._ws.needDrain; }
  get writableCorked() { return this._ws.corked; }
}
__Writable.prototype._writev = null;
class __Duplex extends __Readable {
  constructor(options = {}) { super(options); __writableInit(this, options); }
  _write(chunk, encoding, callback) { __writableDefaultWrite(this, chunk, encoding, callback); }
  write(chunk, encoding, callback) { return __writableWrite(this, chunk, encoding, callback); }
  end(chunk, encoding, callback) { return __writableEnd(this, chunk, encoding, callback); }
  cork() { this._ws.corked++; }
  uncork() { __writableUncork(this); }
  setDefaultEncoding(encoding) { return __writableSetDefaultEncoding(this, encoding); }
  get writable() { return __writableIsWritable(this); }
  set writable(value) { if (this._ws) this._ws.writable = value; }
  get writableLength() { return this._ws.length; }
  get writableHighWaterMark() { return this._ws.highWaterMark; }
  get writableNeedDrain() { return this._ws.needDrain; }
  get writableCorked() { return this._ws.corked; }
}
__Duplex.prototype._writev = null;
function __transformComplete(stream, callback, error, value) {
  if (error) return callback(error);
  if (value !== undefined && value !== null) stream.push(value);
  if (stream._rs.length < stream._rs.highWaterMark) callback();
  else stream._transformCallback = callback;
}
function __Transform(options = {}) {
  if (!(this instanceof __Transform)) return new __Transform(options);
  __Stream.call(this);
  __streamInit(this, options);
  __readableInit(this, options);
  __writableInit(this, options);
  if (options.transform) this._transform = options.transform;
  if (options.flush) this._flush = options.flush;
}
Object.setPrototypeOf(__Transform, __Duplex);
Object.setPrototypeOf(__Transform.prototype, __Duplex.prototype);
Object.assign(__Transform.prototype, {
  _read() {
    const callback = this._transformCallback;
    this._transformCallback = null;
    if (callback) callback();
  },
  _transform(chunk, encoding, callback) { callback(__streamError('ERR_METHOD_NOT_IMPLEMENTED', '_transform() is not implemented')); },
  _write(chunk, encoding, callback) {
    this._transform(chunk, encoding, (error, value) => __transformComplete(this, callback, error, value));
  },
  _final(callback) {
    if (!this._flush) { this.push(null); return callback(); }
    this._flush((error, value) => {
      if (error) return callback(error);
      if (value !== undefined && value !== null) this.push(value);
      this.push(null); callback();
    });
  },
});
class __PassThrough extends __Transform {
  _transform(chunk, encoding, callback) { callback(null, chunk); }
}
function __finishedState(stream, options) {
  return {
    readable: options.readable !== false && Boolean(stream._rs),
    writable: options.writable !== false && Boolean(stream._ws),
  };
}
function __finishedDone(stream, state) {
  return (!state.readable || stream.readableEnded) && (!state.writable || stream.writableFinished);
}
function __finished(stream, options, callback) {
  if (typeof options === 'function') { callback = options; options = {}; }
  options = options || {};
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  return __finishedWatch(stream, options, callback);
}
function __finishedWatch(stream, options, callback) {
  const state = __finishedState(stream, options);
  let active = true;
  const complete = (error) => { if (!active) return; active = false; callback(error); };
  const check = () => { if (__finishedDone(stream, state)) complete(); };
  const error = (failure) => complete(failure);
  const close = () => {
    if (__finishedDone(stream, state)) complete();
    else complete(__streamError('ERR_STREAM_PREMATURE_CLOSE', 'Premature close'));
  };
  stream.on('end', check); stream.on('finish', check);
  stream.on('error', error); stream.on('close', close);
  const cleanup = () => {
    active = false;
    stream.off('end', check); stream.off('finish', check);
    stream.off('error', error); stream.off('close', close);
  };
  __streamDefer(() => __finishedInitial(stream, state, check, error, close));
  return cleanup;
}
function __finishedInitial(stream, state, check, error, close) {
  if (stream.errored) error(stream.errored);
  else if (__finishedDone(stream, state)) check();
  else if (stream.closed) close();
}
function __finishedPromise(stream, options = {}) {
  return new Promise((resolve, reject) => {
    const cleanup = __finished(stream, options, (error) => {
      if (options.cleanup) cleanup();
      if (error) reject(error);
      else resolve();
    });
  });
}
function __pipelineStreams(args) {
  const streams = Array.isArray(args[0]) ? args[0] : args;
  if (streams.length < 2) throw new TypeError('pipeline requires at least two streams');
  return streams;
}
function __pipelineNeedsClose(stream) {
  if (!stream._autoDestroy) return false;
  const readableDone = !stream._rs || stream.readableEnded;
  const writableDone = !stream._ws || stream.writableFinished;
  return readableDone && writableDone;
}
function __pipelineWatch(stream, options, complete) {
  let awaitingClose = false;
  const failure = (error) => complete(error);
  const close = () => complete(stream.errored || undefined);
  stream.on('error', failure);
  const cleanup = __finished(stream, options, (error) => {
    if (error) return complete(error);
    if (!__pipelineNeedsClose(stream) || stream.closed) return close();
    awaitingClose = true;
    stream.once('close', close);
  });
  return () => {
    cleanup(); stream.off('error', failure);
    if (awaitingClose) stream.off('close', close);
  };
}
function __pipelineRun(streams, callback) {
  let settled = false;
  let remaining = streams.length;
  const cleanups = [];
  const complete = (error) => {
    if (settled) return;
    if (error) { settled = true; __pipelineDestroy(streams, error, callback); return; }
    remaining--;
    if (remaining) return;
    settled = true;
    cleanups.forEach((cleanup) => cleanup());
    __streamDefer(() => callback());
  };
  streams.forEach((stream, index) => {
    cleanups.push(__pipelineWatch(stream, { readable: index !== streams.length - 1, writable: index !== 0 }, complete));
  });
  __streamDefer(() => {
    try { for (let index = 0; index < streams.length - 1; index++) streams[index].pipe(streams[index + 1]); }
    catch (error) { complete(error); }
  });
  return streams[streams.length - 1];
}
function __pipelineDestroy(streams, error, callback) {
  let remaining = streams.length;
  const closed = () => { if (--remaining === 0) __streamDefer(() => callback(error)); };
  streams.forEach((stream) => {
    if (stream.closed) closed();
    else { stream.once('close', closed); stream.destroy(error); }
  });
}
function __pipeline(...args) {
  const callback = args.pop();
  if (typeof callback !== 'function') throw new TypeError('pipeline callback must be a function');
  return __pipelineRun(__pipelineStreams(args), callback);
}
function __pipelinePromise(...args) {
  return new Promise((resolve, reject) => {
    __pipelineRun(__pipelineStreams(args), (error) => error ? reject(error) : resolve());
  });
}
const __streamPromises = { pipeline: __pipelinePromise, finished: __finishedPromise };
__streamPromises.default = __streamPromises;
const __stream = Object.assign(__Stream, { Readable: __Readable, Writable: __Writable, Duplex: __Duplex, Transform: __Transform,
  PassThrough: __PassThrough, Stream: __Stream, pipeline: __pipeline, finished: __finished, promises: __streamPromises });
__stream.default = __stream;
