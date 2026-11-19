// ---- process.uptime / memoryUsage / cpuUsage / resourceUsage / umask (turbo-test#419) ----
// Real host figures from the natives in src/process_usage.rs, in Node's shapes and units, with
// Node's argument validation. uptime adds the time the drive loop jumped instead of sleeping, so
// it advances across a timer like process.hrtime does.
function __usageNative(name, api) {
  const native = globalThis[name];
  if (typeof native === 'function') return native;
  const error = new Error(`process.${api} is not supported by turbo-test on this platform`);
  throw Object.assign(error, { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
}
function __usageReceived(value) {
  if (value === null || value === undefined) return ` Received ${value}`;
  if (typeof value === 'object') return ' Received an instance of ' + ((value.constructor && value.constructor.name) || 'Object');
  const shown = typeof value === 'string' ? `'${value}'` : String(value);
  return ` Received type ${typeof value} (${shown})`;
}
function __usageTypeError(name, kind, type, value) {
  const message = `The "${name}" ${kind} must be of type ${type}.` + __usageReceived(value);
  return Object.assign(new TypeError(message), { code: 'ERR_INVALID_ARG_TYPE' });
}
function __processRss() {
  const rss = __usageNative('__ttResidentSetSize', 'memoryUsage')();
  if (rss === undefined) throw Object.assign(new Error('process.memoryUsage: the resident set size is unavailable'), { code: 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM' });
  return rss;
}
function __processMemoryUsage() {
  const [heapTotal, heapUsed, external] = __usageNative('__ttHeapUsage', 'memoryUsage')();
  return { rss: __processRss(), heapTotal, heapUsed, external, arrayBuffers: external };
}
__processMemoryUsage.rss = __processRss;
function __cpuPrevious(prev, key) {
  const value = prev[key];
  if (typeof value !== 'number') throw __usageTypeError('prevValue.' + key, 'property', 'number', value);
  if (!(value >= 0 && value <= Number.MAX_SAFE_INTEGER)) {
    throw Object.assign(new RangeError(`The property 'prevValue.${key}' is invalid. Received ${value}`), { code: 'ERR_INVALID_ARG_VALUE' });
  }
  return value;
}
function __processCpuUsage(prev) {
  const [user, system] = __usageNative('__ttCpuUsage', 'cpuUsage')();
  if (!prev) return { user, system };
  if (typeof prev !== 'object') throw __usageTypeError('prevValue', 'argument', 'object', prev);
  return { user: user - __cpuPrevious(prev, 'user'), system: system - __cpuPrevious(prev, 'system') };
}
const __RESOURCE_USAGE_KEYS = ['userCPUTime', 'systemCPUTime', 'maxRSS', 'sharedMemorySize', 'unsharedDataSize', 'unsharedStackSize', 'minorPageFault', 'majorPageFault', 'swappedOut', 'fsRead', 'fsWrite', 'ipcSent', 'ipcReceived', 'signalsCount', 'voluntaryContextSwitches', 'involuntaryContextSwitches'];
function __processResourceUsage() {
  const values = __usageNative('__ttResourceUsage', 'resourceUsage')();
  const usage = {};
  __RESOURCE_USAGE_KEYS.forEach((key, i) => { usage[key] = values[i]; });
  return usage;
}
// Node's parseFileMode: a uint32, or a string of octal digits
function __umaskMode(mask) {
  if (typeof mask === 'string') {
    if (!/^[0-7]+$/.test(mask)) throw Object.assign(new TypeError(`The argument 'mask' must be a 32-bit unsigned integer or an octal string. Received '${mask}'`), { code: 'ERR_INVALID_ARG_VALUE' });
    return parseInt(mask, 8);
  }
  if (typeof mask !== 'number') throw __usageTypeError('mask', 'argument', 'number', mask);
  if (!Number.isInteger(mask)) throw Object.assign(new RangeError(`The value of "mask" is out of range. It must be an integer. Received ${mask}`), { code: 'ERR_OUT_OF_RANGE' });
  if (mask < 0 || mask > 4294967295) throw Object.assign(new RangeError(`The value of "mask" is out of range. It must be >= 0 && <= 4294967295. Received ${mask}`), { code: 'ERR_OUT_OF_RANGE' });
  return mask;
}
function __processUmask(mask) {
  const umask = __usageNative('__ttUmask', 'umask');
  return mask === undefined ? umask() : umask(__umaskMode(mask));
}
const __processUptime = () => __usageNative('__ttUptime', 'uptime')() + __loop.waited / 1000;
for (const [name, fn] of [['uptime', __processUptime], ['memoryUsage', __processMemoryUsage], ['cpuUsage', __processCpuUsage], ['resourceUsage', __processResourceUsage], ['umask', __processUmask]]) {
  Object.defineProperty(fn, 'name', { value: name, configurable: true });
  process[name] = fn;
}
