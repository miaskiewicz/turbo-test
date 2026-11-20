// Shared scheduling stays independent of native Node async contexts in DOM runtimes.
let __runtimeTaskCapture = (callback) => callback;
let __runtimeTaskResource = () => ({ runInAsyncScope: (callback, receiver, ...args) => callback.apply(receiver, args), emitDestroy() {} });
const __runtimeBackendResets = [];
let __runtimeBackendInstalled = false;
Object.defineProperty(globalThis, '__ttRuntimeBackendInstalled', { get: () => __runtimeBackendInstalled });
function __captureAsyncContext(callback) { return __runtimeTaskCapture(callback); }
function __runtimeResetBackend() { for (const reset of __runtimeBackendResets) reset(); }
// A DOM context activates the backend payload on first use of a backend capability, as vitest's
// jsdom environment has all of Node (a regression since #390: @playwright/test requires tty).
// The runner binds __ttActivateBackend only in DOM contexts; without it the capability throws.
function __runtimeBackendError(name) {
  return Object.assign(new Error('turbo-test: ' + name + ' requires the Node backend runtime'), { code: 'ERR_TURBO_BACKEND_CAPABILITY' });
}
function __runtimeEnsureBackend() {
  if (!__runtimeBackendInstalled && typeof globalThis.__ttActivateBackend === 'function') globalThis.__ttActivateBackend();
  return __runtimeBackendInstalled;
}
// `name` is 'module.function': the placeholder re-dispatches to the backend's function.
function __runtimeBackendUnavailable(name) {
  const [module, method] = name.split('.');
  const placeholder = function(...args) {
    const real = __runtimeEnsureBackend() ? globalThis.__nodeBuiltins[module][method] : placeholder;
    if (real === placeholder) throw __runtimeBackendError(name);
    return new.target ? Reflect.construct(real, args) : real.apply(this, args);
  };
  return placeholder;
}
function __runtimeBackendSlot(target, name) {
  const get = () => {
    __runtimeEnsureBackend();
    if (Object.getOwnPropertyDescriptor(target, name).get === get) throw __runtimeBackendError(name);
    return target[name];
  };
  Object.defineProperty(target, name, {
    get,
    set(value) { Object.defineProperty(target, name, { value, writable: true, enumerable: true, configurable: true }); },
    enumerable: true, configurable: true,
  });
}
