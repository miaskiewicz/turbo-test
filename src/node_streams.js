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
function __streamError(code, message) { const error = new Error(message); error.code = code; return error; }
function __streamInit(stream, options) {
  stream.destroyed = false;
  stream.closed = false;
  stream.errored = null;
  stream._autoDestroy = options.autoDestroy !== false;
  if (options.destroy) stream._destroy = options.destroy;
}
function __streamClose(stream, error) {
  if (stream.closed) return;
  stream.closed = true;
  stream.errored = error || stream.errored;
  if (error) stream.emit('error', error);
  stream.emit('close');
}
function __streamDestroy(stream, error) {
  if (stream.destroyed) return stream;
  stream.destroyed = true;
  stream.errored = error || null;
  if (stream._rs) stream._rs.queue.length = 0;
  if (stream._ws) __streamDefer(() => __writableAbort(stream, error));
  __streamDefer(() => stream._destroy(error || null, (failure) => __streamClose(stream, failure)));
  return stream;
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
  if (chunk instanceof Uint8Array) return Buffer.from(chunk);
  throw new TypeError('The chunk must be a string, Buffer or Uint8Array');
}
function __streamSize(chunk, state) { return state.objectMode ? 1 : chunk.byteLength; }
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
  __streamDefer(() => { stream._rs.scheduled = false; __readableFlow(stream); });
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
  if (state.reading || state.ended || stream.destroyed) return;
  state.reading = true;
  try { stream._read(state.highWaterMark); } catch (error) { stream.destroy(error); }
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
  if (state.ended) __readableEnd(stream);
  else __readablePull(stream);
}
function __readableDrain(stream) {
  const state = stream._rs;
  while (state.flowing && state.queue.length) {
    const chunk = __readableTake(stream);
    if (chunk !== '') stream.emit('data', chunk);
  }
}
function __readablePush(stream, chunk, encoding) {
  const state = stream._rs;
  if (stream.destroyed) return false;
  state.reading = false;
  if (chunk === null) { state.ended = true; __readableSchedule(stream); return false; }
  if (state.ended) { stream.destroy(__streamError('ERR_STREAM_PUSH_AFTER_EOF', 'stream.push() after EOF')); return false; }
  const value = __streamChunk(chunk, state, encoding);
  state.queue.push(value);
  state.length += __streamSize(value, state);
  stream.emit('readable');
  __readableSchedule(stream);
  return state.length < state.highWaterMark;
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
  const end = () => { if (options.end !== false) destination.end(); };
  const cleanup = () => {
    source.off('data', data); source.off('end', end);
    destination.off('drain', drain); destination.off('close', cleanup);
  };
  destination.on('drain', drain);
  destination.on('close', cleanup);
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
    if (state.ended) __streamDefer(() => __readableEnd(this));
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
function __writableInit(stream, options) {
  const objectMode = Boolean(options.objectMode || options.writableObjectMode);
  stream._ws = { queue: [], length: 0, objectMode, highWaterMark: __streamWatermark(options, objectMode, 'writable'),
    active: false, ending: false, finalizing: false, needDrain: false };
  stream.writable = options.writable !== false;
  stream.writableFinished = !stream.writable;
  stream.writableEnded = !stream.writable;
  if (options.write) stream._write = options.write;
  if (options.final) stream._final = options.final;
}
function __writableFailure(stream, error, callback) {
  if (callback) __streamDefer(() => callback(error));
  stream.destroy(error);
  return false;
}
function __writableWrite(stream, chunk, encoding, callback) {
  if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
  const state = stream._ws;
  if (state.ending) return __writableFailure(stream, __streamError('ERR_STREAM_WRITE_AFTER_END', 'write after end'), callback);
  if (stream.destroyed) return __writableFailure(stream, __streamError('ERR_STREAM_DESTROYED', 'Cannot call write after a stream was destroyed'), callback);
  const value = __streamChunk(chunk, state, encoding);
  state.queue.push({ chunk: value, encoding: state.objectMode ? encoding : 'buffer', callback });
  state.length += __streamSize(value, state);
  const ready = state.length < state.highWaterMark;
  if (!ready) state.needDrain = true;
  __streamDefer(() => __writableAdvance(stream));
  return ready;
}
function __writableFinish(stream) {
  if (stream.destroyed || stream.writableFinished) return;
  stream.writableFinished = true;
  stream.writable = false;
  stream.emit('finish');
  __streamAutoDestroy(stream);
}
function __writableFinal(stream) {
  const state = stream._ws;
  if (state.finalizing) return;
  state.finalizing = true;
  let called = false;
  const complete = (error) => {
    if (called) return;
    called = true;
    __streamDefer(() => {
      if (error) stream.destroy(error);
      else __writableFinish(stream);
    });
  };
  try { stream._final(complete); } catch (error) { complete(error); }
}
function __writableIdle(stream) {
  const state = stream._ws;
  if (state.needDrain && !state.ending) { state.needDrain = false; stream.emit('drain'); }
  if (state.ending) __writableFinal(stream);
}
function __writableComplete(stream, item, error) {
  const state = stream._ws;
  state.active = false;
  state.complete = null;
  state.length -= __streamSize(item.chunk, state);
  if (item.callback) item.callback(error);
  if (error) { stream.destroy(error); return; }
  __streamDefer(() => __writableAdvance(stream));
}
function __writableAdvance(stream) {
  const state = stream._ws;
  if (state.active || stream.destroyed) return;
  const item = state.queue.shift();
  if (!item) { __writableIdle(stream); return; }
  state.active = true;
  let called = false;
  const complete = (error) => {
    if (called) return;
    called = true;
    __streamDefer(() => __writableComplete(stream, item, error));
  };
  state.complete = complete;
  try { stream._write(item.chunk, item.encoding, complete); } catch (error) { complete(error); }
}
function __writableEnd(stream, chunk, encoding, callback) {
  if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
  if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
  if (chunk !== undefined && chunk !== null) stream.write(chunk, encoding);
  if (callback) stream.once('finish', callback);
  stream._ws.ending = true;
  stream.writableEnded = true;
  __streamDefer(() => __writableAdvance(stream));
  return stream;
}
class __Writable extends __Stream {
  constructor(options = {}) { super(); __streamInit(this, options); __writableInit(this, options); }
  _write(chunk, encoding, callback) { callback(__streamError('ERR_METHOD_NOT_IMPLEMENTED', '_write() is not implemented')); }
  _final(callback) { callback(); }
  _destroy(error, callback) { callback(error); }
  write(chunk, encoding, callback) { return __writableWrite(this, chunk, encoding, callback); }
  end(chunk, encoding, callback) { return __writableEnd(this, chunk, encoding, callback); }
  destroy(error) { return __streamDestroy(this, error); }
  get writableLength() { return this._ws.length; }
  get writableHighWaterMark() { return this._ws.highWaterMark; }
  get writableNeedDrain() { return this._ws.needDrain; }
}
class __Duplex extends __Readable {
  constructor(options = {}) { super(options); __writableInit(this, options); }
  _write(chunk, encoding, callback) { return __Writable.prototype._write.call(this, chunk, encoding, callback); }
  _final(callback) { callback(); }
  write(chunk, encoding, callback) { return __writableWrite(this, chunk, encoding, callback); }
  end(chunk, encoding, callback) { return __writableEnd(this, chunk, encoding, callback); }
  get writableLength() { return this._ws.length; }
  get writableHighWaterMark() { return this._ws.highWaterMark; }
  get writableNeedDrain() { return this._ws.needDrain; }
}
function __transformComplete(stream, callback, error, value) {
  if (error) return callback(error);
  if (value !== undefined && value !== null) stream.push(value);
  if (stream._rs.length < stream._rs.highWaterMark) callback();
  else stream._transformCallback = callback;
}
function __writableAbort(stream, error) {
  const state = stream._ws;
  const failure = error || __streamError('ERR_STREAM_DESTROYED', 'Stream destroyed before write completed');
  if (state.complete) state.complete(failure);
  const queue = state.queue.splice(0);
  for (const item of queue) {
    state.length -= __streamSize(item.chunk, state);
    if (item.callback) item.callback(failure);
  }
}
class __Transform extends __Duplex {
  constructor(options = {}) {
    super(options);
    if (options.transform) this._transform = options.transform;
    if (options.flush) this._flush = options.flush;
  }
  _read() {
    const callback = this._transformCallback;
    this._transformCallback = null;
    if (callback) callback();
  }
  _transform(chunk, encoding, callback) { callback(__streamError('ERR_METHOD_NOT_IMPLEMENTED', '_transform() is not implemented')); }
  _write(chunk, encoding, callback) {
    this._transform(chunk, encoding, (error, value) => __transformComplete(this, callback, error, value));
  }
  _final(callback) {
    if (!this._flush) { this.push(null); return callback(); }
    this._flush((error, value) => {
      if (error) return callback(error);
      if (value !== undefined && value !== null) this.push(value);
      this.push(null); callback();
    });
  }
}
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
