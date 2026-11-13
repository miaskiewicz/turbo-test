// Synchronous child processes execute on the host; unsupported async entry points fail clearly.
function __processType(message) { return Object.assign(new TypeError(message), { code: 'ERR_INVALID_ARG_TYPE' }); }
function __processString(value, label) {
  if (typeof value !== 'string') throw __processType(label + ' must be a string');
  if (value.includes('\0')) throw Object.assign(new TypeError(label + ' must not contain null bytes'), { code: 'ERR_INVALID_ARG_VALUE' });
  return value;
}
function __processLimit(value, fallback, label) {
  if (value === undefined) return fallback;
  if (typeof value !== 'number') throw __processType(label + ' must be a number');
  if (!Number.isFinite(value) || value < 0 || !Number.isInteger(value)) throw Object.assign(new RangeError(label + ' must be a non-negative integer'), { code: 'ERR_OUT_OF_RANGE' });
  return value;
}
function __processEncoding(value) {
  if (value == null || value === 'buffer') return undefined;
  const encoding = __hashEncoding(value);
  if (!encoding) throw Object.assign(new TypeError('Unknown encoding: ' + value), { code: 'ERR_UNKNOWN_ENCODING' });
  return encoding;
}
function __processHex(value) {
  const bytes = [];
  for (let index = 0; index + 1 < value.length; index += 2) {
    const pair = value.slice(index, index + 2);
    if (!/^[0-9a-f]{2}$/i.test(pair)) break;
    bytes.push(parseInt(pair, 16));
  }
  return new Uint8Array(bytes);
}
function __processInputString(value, encoding) {
  if (encoding === 'hex') return __processHex(value);
  if (encoding === 'base64' || encoding === 'base64url') return new Uint8Array(globalThis.__b64decodeToBytes(value.replace(/-/g, '+').replace(/_/g, '/')));
  return __hashStringBytes(value, encoding);
}
function __processInput(value, encoding) {
  if (value == null) return new Uint8Array();
  if (typeof value === 'string') return __processInputString(value, encoding);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw __processType('input must be a string, Buffer, TypedArray or DataView');
}
function __processEnvironment(value) {
  if (value === undefined) return undefined;
  if (value === null || typeof value !== 'object') throw __processType('env must be an object');
  const result = Object.create(null);
  for (const key of Object.keys(value)) {
    __processString(key, 'environment key');
    if (value[key] !== undefined) result[key] = __processString(String(value[key]), 'environment value');
  }
  return result;
}
function __processSignal(signal) {
  if (signal === undefined) return 15;
  const names = { SIGTERM: 15, SIGKILL: 9, SIGINT: 2 };
  if (names[signal]) return names[signal];
  if (signal === 15 || signal === 9 || signal === 2) return signal;
  throw Object.assign(new TypeError('Unsupported killSignal: ' + signal), { code: 'ERR_UNKNOWN_SIGNAL' });
}
function __processUnsupported(operation) {
  throw new Error('turbo-test: child_process.' + operation + ' is not supported; use a synchronous process API');
}
function __processShellOption(shell) {
  if (shell !== undefined && typeof shell !== 'boolean' && typeof shell !== 'string') throw __processType('shell must be a boolean or string');
}
function __processExecOptions(options) {
  if (options.stdio !== undefined && options.stdio !== 'pipe') __processUnsupported('stdio ' + options.stdio);
  if (options.detached) __processUnsupported('detached');
  if (options.uid !== undefined || options.gid !== undefined) __processUnsupported('uid/gid');
  __processShellOption(options.shell);
}
function __processOptions(options) {
  if (options == null) return {};
  if (typeof options !== 'object' || Array.isArray(options)) throw __processType('options must be an object');
  __processExecOptions(options);
  return options;
}
function __processArguments(args) {
  if (!Array.isArray(args)) throw __processType('args must be an array');
  return args.map((value) => {
    if (typeof value === 'symbol') throw __processType('args must not contain symbols');
    return __processString(String(value), 'argument');
  });
}
function __processShell(file, args, shell) {
  if (!shell) return { file, args };
  const windows = process.platform === 'win32';
  const executable = typeof shell === 'string' ? __processString(shell, 'shell') : windows ? process.env.ComSpec || 'cmd.exe' : '/bin/sh';
  return { file: executable, args: windows ? ['/d', '/s', '/c', [file, ...args].join(' ')] : ['-c', [file, ...args].join(' ')] };
}
function __processRequest(file, args, options) {
  const command = __processShell(file, args, options.shell);
  const maximum = options.maxBuffer === Infinity ? Number.MAX_SAFE_INTEGER : options.maxBuffer;
  return { ...command, cwd: options.cwd === undefined ? process.cwd() : __processString(options.cwd, 'cwd'),
    env: __processEnvironment(options.env === undefined ? process.env : options.env), timeout: __processLimit(options.timeout, 0, 'timeout'),
    maxBuffer: __processLimit(maximum, 1048576, 'maxBuffer'), killSignal: __processSignal(options.killSignal) };
}
function __processDecode(bytes, encoding) {
  if (bytes === null) return null;
  const buffer = Buffer.from(bytes, 'base64');
  return encoding ? __hashOutput(buffer, encoding) : buffer;
}
function __processResult(result, file, args, encoding) {
  result.stdout = __processDecode(result.stdout, encoding);
  result.stderr = __processDecode(result.stderr, encoding);
  result.output = result.stdout === null ? null : [null, result.stdout, result.stderr];
  if (result.error) {
    const details = result.error;
    result.error = Object.assign(new Error('spawnSync ' + file + ' ' + details.code), details, { syscall: 'spawnSync ' + file, path: file, spawnargs: args });
  } else delete result.error;
  return result;
}
function __spawnSync(file, args = [], options = {}) {
  file = __processString(file, 'file');
  if (!file) throw Object.assign(new TypeError('file must not be empty'), { code: 'ERR_INVALID_ARG_VALUE' });
  if (!Array.isArray(args) && args && typeof args === 'object') { options = args; args = []; }
  args = __processArguments(args);
  options = __processOptions(options);
  const encoding = __processEncoding(options.encoding);
  const request = __processRequest(file, args, options);
  const input = __processInput(options.input, encoding);
  const result = JSON.parse(globalThis.__ttSpawnSync(JSON.stringify(request), input));
  return __processResult(result, file, args, encoding);
}
function __processChecked(result, command) {
  if (!result.error && result.status === 0) return result.stdout;
  const error = result.error || new Error('Command failed: ' + command);
  for (const key of ['pid', 'status', 'signal', 'stdout', 'stderr', 'output']) error[key] = result[key];
  throw error;
}
function __execFileSync(file, args = [], options = {}) {
  if (!Array.isArray(args) && args && typeof args === 'object') { options = args; args = []; }
  return __processChecked(__spawnSync(file, args, options), file);
}
function __execSync(command, options = {}) {
  __processString(command, 'command');
  options = __processOptions(options);
  return __processChecked(__spawnSync(command, [], { ...options, shell: options.shell || true }), command);
}
const __childProcess = {
  spawnSync: __spawnSync, execFileSync: __execFileSync, execSync: __execSync,
  spawn: () => __processUnsupported('spawn'), exec: () => __processUnsupported('exec'),
  execFile: () => __processUnsupported('execFile'), fork: () => __processUnsupported('fork'),
};
__childProcess.default = __childProcess;
