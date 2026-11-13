// Node-mode pristine VM context wrappers; window snapshots remain DOM-owned.
const __vmContexts = new WeakMap();
function __vmArgument(message) { return Object.assign(new TypeError(message), { code: 'ERR_INVALID_ARG_TYPE' }); }
function __vmSandbox(sandbox) {
  if (sandbox === null || typeof sandbox !== 'object') throw __vmArgument('The contextObject argument must be an object');
  return sandbox;
}
function __vmCreateContext(sandbox = {}) {
  __vmSandbox(sandbox);
  if (!__vmContexts.has(sandbox)) __vmContexts.set(sandbox, globalThis.__ttVmCreate());
  return sandbox;
}
function __vmIsContext(sandbox) { return __vmContexts.has(__vmSandbox(sandbox)); }
function __vmTimeout(timeout) {
  if (timeout === undefined) return 0;
  if (typeof timeout !== 'number') throw __vmArgument('The timeout option must be a number');
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 4294967295) throw Object.assign(new RangeError('The timeout option must be a positive uint32'), { code: 'ERR_OUT_OF_RANGE' });
  return timeout;
}
function __vmOptions(options) {
  if (options == null) return {};
  if (typeof options === 'string') return { filename: options };
  if (typeof options !== 'object') return {};
  return options;
}
function __vmEvaluate(code, sandbox, options) {
  options = __vmOptions(options);
  const timeout = __vmTimeout(options.timeout);
  const filename = options.filename ?? 'evalmachine.<anonymous>';
  if (typeof filename !== 'string') throw __vmArgument('The filename option must be a string');
  if (typeof code === 'symbol') throw new TypeError('Cannot convert a Symbol value to a string');
  return globalThis.__ttVmRun(String(code), __vmContexts.get(sandbox), sandbox, filename, timeout);
}
function __vmRunInNewContext(code, sandbox = {}, options) {
  __vmCreateContext(sandbox);
  return __vmEvaluate(code, sandbox, options);
}
function __vmRunInContext(code, sandbox, options) {
  if (!__vmIsContext(sandbox)) throw Object.assign(new TypeError('The contextifiedObject must be a vm.Context'), { code: 'ERR_INVALID_ARG_TYPE' });
  return __vmEvaluate(code, sandbox, options);
}
