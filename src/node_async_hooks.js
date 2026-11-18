// Promise lifecycle events come from V8; task-source wrappers exist only while a hook is enabled.
const __enabledAsyncHooks = new Set();
let __hookSequence = 1;
let __hookGeneration = 0;
let __hookDispatching = false;
let __asyncHooksUsed = false;
let __hookPromises = new WeakMap();
let __hookResources = new WeakMap();
let __hookTimers = new Map();
let __hookPromiseFinalizer;
let __hookDrainingFinalizers = false;
let __hookCurrent = { id: 0, trigger: 0, resource: process };
let __hookStack = [];
const __hookOriginalSchedule = __schedule;
const __hookOriginalClear = __clearTimer;
const __hookOriginalNextTick = process.nextTick;
const __hookOriginalResource = __AsyncResource;
const __hookOriginalBind = __AsyncLocalStorage.bind;
const __hookOriginalSnapshot = __AsyncLocalStorage.snapshot;
function __dispatchAsyncHook(name, ...args) {
  if (__hookDispatching || !__enabledAsyncHooks.size) return;
  __hookDispatching = true;
  try {
    for (const hook of [...__enabledAsyncHooks]) {
      const callback = hook.callbacks[name];
      if (hook.enabled && callback) callback(...args);
    }
  } finally { __hookDispatching = false; }
}
function __newHookRecord(type, resource, trigger = __hookCurrent.id, notify = true) {
  const record = { id: __hookSequence++, trigger, resource, generation: __hookGeneration, lifecycle: { destroyed: false, registered: false }, destroyQueued: false };
  if (notify) __dispatchAsyncHook('init', record.id, type, trigger, resource);
  return record;
}
function __hookPromiseRecord(promise) {
  let record = __hookPromises.get(promise);
  if (!record) {
    record = __newHookRecord('PROMISE', promise, __hookCurrent.id, false);
    __hookPromises.set(promise, record);
  }
  return record;
}
function __promiseHookInit(promise, parent) {
  const previous = __hookPromises.get(parent);
  const record = __newHookRecord('PROMISE', promise, previous ? previous.id : __hookCurrent.id, false);
  __hookPromises.set(promise, record);
  __registerHookFinalizer(record, promise);
  __dispatchAsyncHook('init', record.id, 'PROMISE', record.trigger, promise);
}
function __hookBefore(record) {
  __drainAsyncHookFinalizers();
  __hookStack.push(__hookCurrent);
  __hookCurrent = record;
  __dispatchAsyncHook('before', record.id);
}
function __hookAfter(record) {
  try { __dispatchAsyncHook('after', record.id); }
  finally { __hookCurrent = __hookStack.pop() || { id: 0, trigger: 0, resource: process }; }
}
function __promiseHookBefore(promise) { __hookBefore(__hookPromiseRecord(promise)); }
function __promiseHookAfter(promise) {
  const record = __hookPromises.get(promise);
  if (record) __hookAfter(record);
}
function __promiseHookResolve(promise) {
  const record = __hookPromises.get(promise);
  if (record) __dispatchAsyncHook('promiseResolve', record.id);
}
function __registerHookFinalizer(record, resource, manual) {
  if (manual || record.lifecycle.registered) return;
  record.lifecycle.registered = true;
  __hookPromiseFinalizer.register(resource, { id: record.id, generation: record.generation, lifecycle: record.lifecycle });
}
function __finalizeAsyncHookResource(held) {
  if (held.generation !== __hookGeneration || held.lifecycle.destroyed) return;
  held.lifecycle.destroyed = true;
  __dispatchAsyncHook('destroy', held.id);
}
function __drainAsyncHookFinalizers() {
  if (__hookDrainingFinalizers || !__enabledAsyncHooks.size) return;
  __hookDrainingFinalizers = true;
  try { globalThis.__ttDrainAsyncHookTasks(); }
  finally { __hookDrainingFinalizers = false; }
}
function __maybeRestoreTimerClear() {
  if (!__enabledAsyncHooks.size && !__hookTimers.size) __clearTimer = __hookOriginalClear;
}
function __hookDestroy(record) {
  if (record.lifecycle.destroyed) return;
  record.lifecycle.destroyed = true;
  if (record.generation === __hookGeneration) __dispatchAsyncHook('destroy', record.id);
}
function __queueHookDestroy(record) {
  if (record.destroyQueued || record.lifecycle.destroyed) return;
  record.destroyQueued = true;
  __hookOriginalNextTick(() => __hookDestroy(record));
}
function __hookInvoke(record, callback, receiver, args) {
  if (!__enabledAsyncHooks.size || record.generation !== __hookGeneration) return callback.apply(receiver, args);
  __hookBefore(record);
  try { return callback.apply(receiver, args); }
  finally { __hookAfter(record); }
}
function __hookTimerCallback(record, callback, repeating) {
  return function(...args) {
    try { return __hookInvoke(record, callback, this, args); }
    finally {
      if (!repeating) { __hookTimers.delete(record.timer); __queueHookDestroy(record); __maybeRestoreTimerClear(); }
    }
  };
}
function __hookSchedule(callback, delay, interval, fake, immediate, clamped) {
  const resource = { _onTimeout: callback };
  const record = __newHookRecord(immediate ? 'Immediate' : 'Timeout', resource);
  const repeating = interval != null;
  const timer = __hookOriginalSchedule(__hookTimerCallback(record, callback, repeating), delay, interval, fake, immediate, clamped);
  record.timer = timer;
  __hookTimers.set(timer, record);
  return timer;
}
function __hookClear(timer) {
  const record = __hookTimers.get(timer);
  __hookTimers.delete(timer);
  __hookOriginalClear(timer);
  if (record) __queueHookDestroy(record);
  __maybeRestoreTimerClear();
}
function __hookNextTick(callback, ...args) {
  const resource = { callback };
  const record = __newHookRecord('TickObject', resource);
  __hookOriginalNextTick(() => {
    try { return __hookInvoke(record, callback, undefined, args); }
    finally { __queueHookDestroy(record); }
  });
}
function __hookResourceRecord(resource) {
  let record = __hookResources.get(resource);
  if (!record) {
    record = __newHookRecord('AsyncResource', resource, __hookCurrent.id, false);
    __hookResources.set(resource, record);
  }
  return record;
}
function __hookResourceRun(resource, callback, receiver, args) {
  __asyncCallback(callback);
  const record = __hookResourceRecord(resource);
  return __asyncCallFrame(resource._frame, function(...values) { return __hookInvoke(record, callback, receiver, values); }, receiver, args);
}
function __hookResourceDestroy(resource) {
  const record = __hookResources.get(resource);
  if (record) __queueHookDestroy(record);
}
function __hookResourceOptions(options) {
  const trigger = typeof options === 'number' ? options : options && options.triggerAsyncId;
  if (trigger === undefined) return __hookCurrent.id;
  if (!Number.isInteger(trigger) || trigger < -1) throw Object.assign(new RangeError('Invalid triggerAsyncId'), { code: 'ERR_INVALID_ASYNC_ID' });
  return trigger;
}
function __hookResourceInit(resource, type, options) {
  const trigger = __hookResourceOptions(options);
  const record = __newHookRecord(type, resource, trigger, false);
  __hookResources.set(resource, record);
  __registerHookFinalizer(record, resource, options && options.requireManualDestroy);
  __dispatchAsyncHook('init', record.id, type, trigger, resource);
}
function __hookBind(callback) {
  __asyncCallback(callback);
  const resource = new __hookOriginalResource(callback.name || 'bound-anonymous-fn');
  return function(...args) { return resource.runInAsyncScope(callback, this, ...args); };
}
function __hookSnapshot() { return __hookBind((callback, ...args) => callback(...args)); }
function __startAsyncHooks() {
  __asyncHooksUsed = true;
  if (!__hookPromiseFinalizer) __hookPromiseFinalizer = new FinalizationRegistry(__finalizeAsyncHookResource);
  globalThis.__ttSetPromiseHooks(__promiseHookInit, __promiseHookBefore, __promiseHookAfter, __promiseHookResolve);
  __schedule = __hookSchedule;
  __clearTimer = __hookClear;
  process.nextTick = __hookNextTick;
  __asyncResourceInitHook = __hookResourceInit;
  __asyncResourceRunHook = __hookResourceRun;
  __asyncResourceDestroyHook = __hookResourceDestroy;
  __AsyncLocalStorage.bind = __hookBind;
  __AsyncLocalStorage.snapshot = __hookSnapshot;
}
function __stopAsyncHooks() {
  globalThis.__ttSetPromiseHooks(undefined, undefined, undefined, undefined);
  __schedule = __hookOriginalSchedule;
  if (!__hookTimers.size) __clearTimer = __hookOriginalClear;
  process.nextTick = __hookOriginalNextTick;
  __asyncResourceInitHook = undefined;
  __asyncResourceRunHook = undefined;
  __AsyncLocalStorage.bind = __hookOriginalBind;
  __AsyncLocalStorage.snapshot = __hookOriginalSnapshot;
  __hookStack = [];
  __hookCurrent = { id: 0, trigger: 0, resource: process };
}
function __createAsyncHook(callbacks) {
  if (callbacks == null) throw new TypeError('callbacks must be an object');
  const record = {};
  for (const name of ['init', 'before', 'after', 'destroy', 'promiseResolve']) {
    const callback = callbacks[name];
    if (callback !== undefined && typeof callback !== 'function') throw Object.assign(new TypeError(name + ' must be a function'), { code: 'ERR_ASYNC_CALLBACK' });
    record[name] = callback;
  }
  return __asyncHookObject(record);
}
function __asyncHookObject(callbacks) {
  const hook = { callbacks, enabled: false,
    enable() {
      if (hook.enabled) return this;
      hook.enabled = true;
      __enabledAsyncHooks.add(hook);
      if (__enabledAsyncHooks.size === 1) __startAsyncHooks();
      return this;
    },
    disable() {
      if (!hook.enabled) return this;
      hook.enabled = false;
      __enabledAsyncHooks.delete(hook);
      if (!__enabledAsyncHooks.size) __stopAsyncHooks();
      return this;
    },
  };
  return hook;
}
function __resetAsyncHooks() {
  if (!__asyncHooksUsed) return;
  if (__enabledAsyncHooks.size) {
    for (const hook of __enabledAsyncHooks) hook.enabled = false;
    __enabledAsyncHooks.clear();
    __stopAsyncHooks();
  }
  __hookTimers.clear();
  __clearTimer = __hookOriginalClear;
  __asyncResourceDestroyHook = undefined;
  __hookGeneration++;
  __hookPromises = new WeakMap();
  __hookResources = new WeakMap();
  __hookPromiseFinalizer = undefined;
  __asyncHooksUsed = false;
}
