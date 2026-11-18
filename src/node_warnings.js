// Warnings are real process events, delivered after the current call stack on nextTick.
function __warningString(value, name) {
  if (value !== undefined && typeof value !== 'string') throw Object.assign(new TypeError(name + ' must be a string'), { code: 'ERR_INVALID_ARG_TYPE' });
}
function __warningPositional(type, code, ctor, detail) {
  __warningString(type, 'type');
  if (typeof code === 'function') { ctor = code; code = undefined; }
  else __warningString(code, 'code');
  return { type: type || 'Warning', code, ctor, detail };
}
function __warningFromOptions(options) {
  const detail = typeof options.detail === 'string' ? options.detail : undefined;
  return __warningPositional(options.type || 'Warning', options.code, options.ctor, detail);
}
function __warningOptions(type, code, ctor) {
  if (type !== null && typeof type === 'object' && !Array.isArray(type)) return __warningFromOptions(type);
  if (typeof type === 'function') return __warningPositional('Warning', undefined, type);
  return __warningPositional(type, code, ctor);
}
function __warningError(warning, options) {
  if (typeof warning !== 'string') {
    if (warning instanceof Error) return warning;
    throw Object.assign(new TypeError('warning must be a string or Error'), { code: 'ERR_INVALID_ARG_TYPE' });
  }
  const error = new Error(warning);
  error.name = options.type;
  if (options.code !== undefined) error.code = options.code;
  if (options.detail !== undefined) error.detail = options.detail;
  __warningStack(error, options.ctor);
  return error;
}
function __warningStack(error, ctor) {
  if (typeof Error.captureStackTrace === 'function') Error.captureStackTrace(error, typeof ctor === 'function' ? ctor : process.emitWarning);
}
function __printProcessWarning(warning) {
  const prefix = warning.code ? '[' + warning.code + '] ' : '';
  let text = prefix + warning.name + ': ' + warning.message + '\n';
  if (typeof warning.detail === 'string') text += warning.detail + '\n';
  process.stderr.write(text);
}
function __deliverProcessWarning(warning, generation) {
  // A file's pending warning remains visible, but must not call the next file's listeners.
  if (generation !== __loop.gen) __printProcessWarning(warning);
  else process.emit('warning', warning);
}
function __emitProcessWarning(warning, type, code, ctor) {
  const error = __warningError(warning, __warningOptions(type, code, ctor));
  const generation = __loop.gen;
  process.nextTick(() => __deliverProcessWarning(error, generation));
}
function __resetProcessWarnings() {
  process.removeAllListeners('warning');
  process.on('warning', __printProcessWarning);
}
function __installProcessWarnings() {
  Object.defineProperty(process, '_ev', { value: Object.create(null), writable: true, configurable: true });
  const methods = ['on', 'off', 'once', 'emit', 'addListener', 'removeListener', 'removeAllListeners', 'listeners', 'listenerCount', 'prependListener', 'setMaxListeners'];
  for (const name of methods) process[name] = __EventEmitter.prototype[name];
  process.emitWarning = __emitProcessWarning;
  __resetProcessWarnings();
}
__installProcessWarnings();
