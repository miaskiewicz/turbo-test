// Shared scheduling stays independent of native Node async contexts in DOM runtimes.
let __runtimeTaskCapture = (callback) => callback;
let __runtimeTaskResource = () => ({ runInAsyncScope: (callback, receiver, ...args) => callback.apply(receiver, args), emitDestroy() {} });
const __runtimeBackendResets = [];
let __runtimeBackendInstalled = false;
Object.defineProperty(globalThis, '__ttRuntimeBackendInstalled', { get: () => __runtimeBackendInstalled });
function __captureAsyncContext(callback) { return __runtimeTaskCapture(callback); }
function __runtimeResetBackend() { for (const reset of __runtimeBackendResets) reset(); }
function __runtimeBackendUnavailable(name) {
  return function() { throw Object.assign(new Error('turbo-test: ' + name + ' requires the Node backend runtime'), { code: 'ERR_TURBO_BACKEND_CAPABILITY' }); };
}
function __runtimeBackendSlot(target, name) {
  Object.defineProperty(target, name, {
    get: __runtimeBackendUnavailable(name),
    set(value) { Object.defineProperty(target, name, { value, writable: true, enumerable: true, configurable: true }); },
    enumerable: true, configurable: true,
  });
}
