// Real owned Unix children. Only active children install the native-poll timer.
const __asyncChildren = new Map();
let __asyncChildTimer = null;
let __asyncChildSerial = 0;
function __asyncChildError(data) { return Object.assign(new Error(data.message), data); }
function __asyncChildNative(operation, args = {}) {
  const result = JSON.parse(globalThis.__ttAsyncProcess(operation, JSON.stringify(args)));
  if (result.error) throw __asyncChildError(result.error);
  return result.value;
}
function __asyncChildCapture(callback) { return typeof __captureAsyncContext === 'function' ? __captureAsyncContext(callback) : callback; }
function __asyncChildLater(callback) { queueMicrotask(__asyncChildCapture(callback)); }
function __asyncChildEnsurePoll() {
  if (__asyncChildTimer === null) __asyncChildTimer = __realTimers.setTimeout(__asyncChildPoll, 1);
}
function __asyncChildDrainEvents() {
  const events = __asyncChildNative('poll');
  for (const event of events) {
    const child = __asyncChildren.get(event.id);
    if (child) child._dispatch(event);
  }
  return events.length > 0;
}
function __asyncChildPoll() {
  __asyncChildTimer = null;
  __asyncChildDrainEvents();
  if (__asyncChildren.size) __asyncChildEnsurePoll();
}
globalThis.__ttAsyncProcessPump = () => {
  if (!__asyncChildren.size) return false;
  const progressed = __asyncChildDrainEvents();
  if (!__asyncChildren.size && __asyncChildTimer !== null) {
    __realTimers.clearTimeout(__asyncChildTimer); __asyncChildTimer = null;
  }
  return progressed;
};
function __asyncProcessReset() {
  if (__asyncChildTimer !== null) __realTimers.clearTimeout(__asyncChildTimer);
  __asyncChildTimer = null;
  __asyncChildren.clear();
  __asyncChildNative('reset');
}
function __asyncChildControl(child, name, args = {}) {
  try { return __asyncChildNative(name, { id: child._id, ...args }); }
  catch (error) { if (error.code !== 'ERR_CHILD_PROCESS_CLOSED') throw error; return undefined; }
}
function __asyncChildWrite(child, name, args, callback) {
  const serial = ++__asyncChildSerial;
  child._callbacks.set(serial, { kind: name, callback: __asyncChildCapture(callback) });
  try { return Boolean(__asyncChildNative(name, { id: child._id, serial, ...args })); }
  catch (error) { __asyncChildLater(() => __asyncChildComplete(child, serial, error)); return false; }
}
function __asyncChildComplete(child, serial, error) {
  const entry = child._callbacks.get(serial);
  if (!entry) return;
  child._callbacks.delete(serial); entry.callback(error);
}
function __asyncChildAbortInput(child, error) {
  for (const [serial, entry] of child._callbacks) {
    if (entry.kind === 'write' || entry.kind === 'end') __asyncChildComplete(child, serial, error || __streamError('ERR_STREAM_DESTROYED', 'Child stdin destroyed'));
  }
}
function __asyncChildReadable(child, fd) {
  return new __Readable({
    read() { __asyncChildControl(child, 'resume', { fd }); },
    destroy(error, callback) { __asyncChildControl(child, 'closeOutput', { fd }); callback(error); },
  });
}
function __asyncChildWritable(child) {
  return new __Writable({
    write(chunk, encoding, callback) { __asyncChildWrite(child, 'write', { data: Buffer.from(chunk).toString('base64') }, callback); },
    final(callback) { __asyncChildWrite(child, 'end', {}, callback); },
    destroy(error, callback) { __asyncChildControl(child, 'closeInput'); __asyncChildAbortInput(child, error); callback(error); },
  });
}
function __asyncChildStreams(child, modes) {
  child.stdin = modes[0] === 'pipe' ? __asyncChildWritable(child) : null;
  child.stdout = modes[1] === 'pipe' ? __asyncChildReadable(child, 1) : null;
  child.stderr = modes[2] === 'pipe' ? __asyncChildReadable(child, 2) : null;
  child.stdio = [child.stdin, child.stdout, child.stderr];
}
function __asyncChildSpawnError(child, data) {
  const error = __asyncChildError(data);
  Object.assign(error, { path: child.spawnfile, spawnargs: child.spawnargs.slice(1), syscall: 'spawn ' + child.spawnfile });
  child.emit('error', error);
}
function __asyncChildData(child, event) {
  const stream = child.stdio[event.fd];
  if (!stream || stream.destroyed) return;
  if (!stream.push(Buffer.from(event.data, 'base64'))) __asyncChildControl(child, 'pause', { fd: event.fd });
}
function __asyncChildExit(child, event) {
  child.exitCode = event.code; child.signalCode = event.signal;
  child.emit('exit', event.code, event.signal);
  for (const stream of [child.stdout, child.stderr]) if (stream && !stream.destroyed) stream.resume();
}
function __asyncChildClose(child, event) {
  __asyncChildren.delete(child._id);
  const streams = child.stdio.filter((stream) => stream && !stream.closed);
  let remaining = streams.length;
  const complete = () => { if (--remaining === 0) child.emit('close', event.code, event.signal); };
  if (!remaining) { __asyncChildLater(() => child.emit('close', event.code, event.signal)); return; }
  for (const stream of streams) {
    stream.once('close', complete);
    if (stream === child.stdin || child.pid === undefined) stream.destroy();
    else stream.resume();
  }
}
function __asyncChildStreamEvent(child, event) {
  const stream = child.stdio[event.fd];
  if (!stream) return;
  if (event.kind === 'data') __asyncChildData(child, event);
  else if (event.kind === 'end') stream.push(null);
  else stream.destroy(__asyncChildError(event.error));
}
function __asyncChildMessageEvent(child, event) {
  if (event.kind === 'disconnect') { child.connected = false; child.emit('disconnect'); }
  else child.emit(event.kind, event.message);
}
const __asyncChildEvents = {
  spawn: (child) => child.emit('spawn'),
  error: (child, event) => __asyncChildSpawnError(child, event.error),
  exit: __asyncChildExit, close: __asyncChildClose,
  data: __asyncChildStreamEvent, end: __asyncChildStreamEvent, streamError: __asyncChildStreamEvent,
  message: __asyncChildMessageEvent, internalMessage: __asyncChildMessageEvent, disconnect: __asyncChildMessageEvent,
  timeout: (child) => { child.killed = true; },
  written: (child, event) => __asyncChildComplete(child, event.serial),
  inputEnd: (child, event) => __asyncChildComplete(child, event.serial),
  sent: (child, event) => __asyncChildComplete(child, event.serial),
  writeError: (child, event) => __asyncChildComplete(child, event.serial, __asyncChildError(event.error)),
  sendError: (child, event) => __asyncChildComplete(child, event.serial, __asyncChildError(event.error)),
};
class __AsyncChildProcess extends __EventEmitter {
  constructor(record, request) {
    super(); this._id = record.id; this.pid = record.pid || undefined;
    this.spawnfile = request.file; this.spawnargs = [request.file, ...request.args];
    this.exitCode = null; this.signalCode = null; this.killed = false; this.connected = Boolean(request.ipc && record.pid);
    this._callbacks = new Map();
    this._dispatch = __asyncChildCapture((event) => __asyncChildEvents[event.kind](this, event));
    __asyncChildStreams(this, request.stdio);
    if (request.ipc) { __asyncChildIpc(this); this.stdio.push(null); }
    __asyncChildren.set(this._id, this); __asyncChildEnsurePoll();
  }
  kill(signal) {
    if (this.exitCode !== null || this.signalCode !== null) return false;
    const sent = Boolean(__asyncChildControl(this, 'kill', { signal: __processSignal(signal) }));
    if (sent) this.killed = true;
    return sent;
  }
  ref() { throw __asyncChildUnsupported('ref/unref'); }
  unref() { throw __asyncChildUnsupported('ref/unref'); }
}
function __asyncChildUnsupported(name) {
  return Object.assign(new Error('turbo-test: asynchronous child_process ' + name + ' is not supported'), { code: 'ERR_ASYNC_PROCESS_UNSUPPORTED' });
}
function __asyncChildIpc(child) {
  child.send = (...args) => __asyncChildSend(child, args);
  child.disconnect = () => {
    if (!child.connected) throw Object.assign(new Error('IPC channel is already disconnected'), { code: 'ERR_IPC_DISCONNECTED' });
    __asyncChildNative('disconnect', { id: child._id }); child.connected = false;
  };
}
function __asyncChildSend(child, args) {
  const message = args[0]; const tail = args[args.length - 1];
  const callback = typeof tail === 'function' ? tail : (error) => { if (error) child.emit('error', error); };
  __asyncChildValidateMessage(message); __asyncChildValidateHandle(args);
  if (!child.connected) {
    __asyncChildLater(() => callback(Object.assign(new Error('Channel closed'), { code: 'ERR_IPC_CHANNEL_CLOSED' })));
    return false;
  }
  // Reject cycles synchronously and retain a snapshot of the sent JSON value.
  const snapshot = JSON.parse(JSON.stringify(message));
  return __asyncChildWrite(child, 'send', { message: snapshot }, callback);
}
function __asyncChildValidateMessage(message) {
  if (message === undefined || typeof message === 'function' || typeof message === 'symbol') throw __processType('message must be a JSON-serializable value');
}
function __asyncChildValidateHandle(args) {
  if (args.length > 2 && args[1] != null && typeof args[1] !== 'function') throw __asyncChildUnsupported('IPC handle transfer');
}
function __asyncChildMode(mode) {
  if (mode === undefined || mode === null) return 'pipe';
  if (!['pipe', 'ignore', 'inherit'].includes(mode)) throw __asyncChildUnsupported('stdio ' + mode);
  return mode;
}
function __asyncChildDefaultMode(options, ipc) { return ipc && !options.silent ? 'inherit' : 'pipe'; }
function __asyncChildStdio(options, ipc) {
  const modes = options.stdio === undefined ? __asyncChildDefaultMode(options, ipc) : options.stdio;
  if (typeof modes === 'string') return [0, 1, 2].map(() => __asyncChildMode(modes));
  return __asyncChildArrayStdio(modes);
}
function __asyncChildArrayStdio(modes) {
  if (!Array.isArray(modes) || modes.length > 4) throw __asyncChildUnsupported('stdio configuration');
  if (modes[3] !== undefined && modes[3] !== 'ipc') throw __asyncChildUnsupported('extra stdio descriptors');
  return [0, 1, 2].map((index) => __asyncChildMode(modes[index]));
}
function __asyncChildOptions(options) {
  if (options == null) options = {};
  if (typeof options !== 'object' || Array.isArray(options)) throw __processType('options must be an object');
  const result = __processOptions({ ...options, stdio: 'pipe' });
  if (result.signal !== undefined) throw __asyncChildUnsupported('AbortSignal');
  if (result.serialization !== undefined && result.serialization !== 'json') throw __asyncChildUnsupported('advanced IPC serialization');
  return result;
}
function __asyncChildArguments(args) {
  if (!Array.isArray(args)) throw __processType('args must be an array');
  return args.map((value) => __processString(String(value), 'argument'));
}
function __asyncChildNormalize(file, args, options) {
  file = __processString(file, 'file');
  if (!file) throw Object.assign(new TypeError('file must not be empty'), { code: 'ERR_INVALID_ARG_VALUE' });
  if (args && !Array.isArray(args) && typeof args === 'object') { options = args; args = []; }
  const original = options || {};
  options = __asyncChildOptions(options);
  return { file, args: __asyncChildArguments(args || []), options, original };
}
function __spawnAsync(file, args, options) {
  const normalized = __asyncChildNormalize(file, args, options);
  const ipc = Array.isArray(normalized.original.stdio) && normalized.original.stdio[3] === 'ipc';
  const request = __processRequest(normalized.file, normalized.args, normalized.options);
  request.ipc = ipc; request.stdio = __asyncChildStdio(normalized.original, ipc);
  return new __AsyncChildProcess(__asyncChildNative('spawn', request), request);
}
function __forkAsync(module, args, options) {
  const normalized = __asyncChildNormalize(module, args, options);
  const executable = normalized.options.execPath || process.execPath;
  const execArgs = __asyncChildArguments(normalized.options.execArgv || []);
  const request = __processRequest(__processString(executable, 'execPath'), [...execArgs, normalized.file, ...normalized.args], { ...normalized.options, shell: false });
  request.ipc = true; request.stdio = __asyncChildStdio(normalized.original, true);
  return new __AsyncChildProcess(__asyncChildNative('spawn', request), request);
}
__childProcess.spawn = __spawnAsync;
__childProcess.fork = __forkAsync;
