// V8 owns Promise/await propagation; the runner captures this frame for scheduled resources.
let __asyncContextGeneration = 0;
let __asyncFrameActivated = false;
function __asyncFrame() {
  if (!__asyncFrameActivated || typeof globalThis.__ttAsyncFrameGet !== 'function') return undefined;
  const frame = globalThis.__ttAsyncFrameGet();
  return frame && frame.generation === __asyncContextGeneration ? frame : undefined;
}
function __asyncSetFrame(frame) {
  if (frame !== undefined) __asyncFrameActivated = true;
  globalThis.__ttAsyncFrameSet(frame);
}
function __asyncCopyFrame() {
  const previous = __asyncFrame();
  return { generation: __asyncContextGeneration, stores: new Map(previous && previous.stores) };
}
function __asyncCallback(fn) {
  if (typeof fn !== 'function') throw Object.assign(new TypeError('callback must be a function'), { code: 'ERR_INVALID_ARG_TYPE' });
}
function __asyncCallFrame(frame, fn, receiver, args) {
  __asyncCallback(fn);
  if (!__asyncFrameActivated && frame === undefined) {
    try { return fn.apply(receiver, args); }
    finally {
      if (__asyncFrameActivated) globalThis.__ttAsyncFrameSet(undefined);
    }
  }
  const previous = globalThis.__ttAsyncFrameGet();
  __asyncSetFrame(frame);
  try { return fn.apply(receiver, args); }
  finally { __asyncSetFrame(previous); }
}
function __asyncCallStorage(storage, frame, fn, args) {
  __asyncCallback(fn);
  const previous = __asyncFrame();
  const record = previous && previous.stores.get(storage);
  __asyncSetFrame(frame);
  try { return fn.apply(null, args); }
  finally {
    const restored = __asyncCopyFrame();
    if (record) restored.stores.set(storage, record);
    else restored.stores.delete(storage);
    __asyncSetFrame(restored);
  }
}
function __captureAsyncContext(fn) {
  const frame = __asyncFrame();
  if (!frame) return fn;
  return function(...args) { return __asyncCallFrame(frame, fn, this, args); };
}
function __resetAsyncContext() {
  __asyncContextGeneration++;
  __asyncSetFrame(undefined);
  __asyncFrameActivated = false;
}
class __AsyncLocalStorage {
  constructor() { this._enabled = true; this._version = 0; }
  getStore() {
    if (!this._enabled) return undefined;
    const frame = __asyncFrame();
    const record = frame && frame.stores.get(this);
    return record && record.version === this._version ? record.value : undefined;
  }
  _frame(store) {
    const frame = __asyncCopyFrame();
    frame.stores.set(this, { version: this._version, value: store });
    return frame;
  }
  run(store, fn, ...args) {
    __asyncCallback(fn);
    this._enabled = true;
    return __asyncCallStorage(this, this._frame(store), fn, args);
  }
  enterWith(store) {
    this._enabled = true;
    __asyncSetFrame(this._frame(store));
  }
  exit(fn, ...args) {
    const frame = __asyncCopyFrame();
    frame.stores.delete(this);
    return __asyncCallStorage(this, frame, fn, args);
  }
  disable() {
    this._enabled = false;
    this._version++;
    const frame = __asyncCopyFrame();
    frame.stores.delete(this);
    __asyncSetFrame(frame);
  }
  static bind(fn) {
    __asyncCallback(fn);
    const frame = __asyncFrame();
    return function(...args) { return __asyncCallFrame(frame, fn, this, args); };
  }
  static snapshot() {
    const frame = __asyncFrame();
    return (fn, ...args) => __asyncCallFrame(frame, fn, undefined, args);
  }
}
class __AsyncResource {
  constructor(type) {
    if (typeof type !== 'string') throw new TypeError('type must be a string');
    this._frame = __asyncFrame();
  }
  runInAsyncScope(fn, receiver, ...args) { return __asyncCallFrame(this._frame, fn, receiver, args); }
  emitDestroy() { return this; }
  bind(fn, receiver = this) {
    __asyncCallback(fn);
    const resource = this;
    const bound = function(...args) { return resource.runInAsyncScope(fn, receiver, ...args); };
    bound.asyncResource = this;
    return bound;
  }
  static bind(fn, type = fn.name || 'bound-anonymous-fn', receiver) {
    return new __AsyncResource(type).bind(fn, receiver);
  }
}
