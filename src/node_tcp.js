// Raw loopback TCP owns native handles independently of the HTTP framing transport.
const __tcpSockets = new Map();
const __tcpServers = new Map();
const __tcpOwnedSockets = new Set();
let __tcpPollTimer = null;
let __tcpUsed = false;
function __tcpFailure(data) { return Object.assign(new Error(data.message), { code: data.code }); }
function __tcpNative(operation, args = {}) {
  __tcpUsed = true;
  const result = JSON.parse(globalThis.__ttTcp(operation, JSON.stringify(args)));
  if (result.error) throw __tcpFailure(result.error);
  return result.value;
}
function __tcpLater(fn) { queueMicrotask(fn); }
function __tcpOtherWork() {
  if (__loop.nextTicks.length) return true;
  if (__loop.internal.some(timer => timer.gen === __loop.gen)) return true;
  return __loop.macro.some(timer => timer.gen === __loop.gen && timer.id !== __tcpPollTimer);
}
function __tcpReferenced() {
  for (const socket of __tcpSockets.values()) if (socket._tcpRef) return true;
  for (const server of __tcpServers.values()) if (server._tcpRef) return true;
  return false;
}
function __tcpStopPoll() {
  if (__tcpPollTimer !== null) __realTimers.clearTimeout(__tcpPollTimer);
  __tcpPollTimer = null;
}
function __tcpPreparePoll() {
  if (!__tcpSockets.size && !__tcpServers.size) { __tcpStopPoll(); return false; }
  if (!__tcpReferenced() && !__tcpOtherWork()) { __tcpStopPoll(); return false; }
  if (__tcpPollTimer === null) __tcpPollTimer = __realTimers.setTimeout(__tcpPoll, 1);
  return true;
}
globalThis.__ttTcpPreparePoll = __tcpPreparePoll;
// Readiness is a poll phase, so recursively queued immediates cannot starve socket events.
globalThis.__ttTcpPump = () => { __tcpStopPoll(); return __tcpPoll(); };
function __tcpPoll() {
  __tcpPollTimer = null;
  const events = __tcpNative('poll');
  for (const event of events) __tcpEvent(event);
  let progressed = events.length > 0;
  for (const socket of __tcpSockets.values()) if (socket._tcpCheckTimeout()) progressed = true;
  __tcpPreparePoll();
  return progressed;
}
function __tcpReference(handle, enabled) {
  handle._tcpRef = enabled;
  if (handle instanceof __TcpSocket) handle._tcpReferenceTimeout();
  if (handle._tcpId !== null) __tcpNative('ref', { id: handle._tcpId, enabled });
  __tcpPreparePoll(); return handle;
}
function __tcpResource(owner, type) {
  owner._tcpResource = new __async_hooks.AsyncResource(type);
  owner._tcpDispatch = event => owner._tcpResource.runInAsyncScope(() => owner._tcpEvent(event), owner);
}
function __tcpDestroyResource(owner, resource = owner._tcpResource) {
  if (owner._tcpResource === resource) owner._tcpResource = null;
  if (resource) resource.emitDestroy();
}
function __tcpBytes(chunk, encoding) {
  if (typeof chunk === 'string') return Buffer.from(chunk, encoding);
  if (chunk instanceof Uint8Array) return Buffer.from(chunk);
  throw new TypeError('TCP chunks must be strings, Buffers or Uint8Arrays');
}
function __tcpPort(value) {
  const port = Number(value);
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw __streamError('ERR_SOCKET_BAD_PORT', 'TCP port must be between 0 and 65535');
  return port;
}
function __tcpArguments(args) {
  const values = Array.from(args);
  const callback = typeof values[values.length - 1] === 'function' ? values.pop() : null;
  if (typeof values[0] === 'string' && !/^\d+$/.test(values[0])) throw __streamError('ERR_TCP_UNSUPPORTED_OPTION', 'Unix-domain socket paths are unsupported');
  const options = typeof values[0] === 'object' ? { ...values[0] } : { port: values[0], host: values[1] };
  __tcpValidateOptions(options);
  options.port = __tcpPort(options.port === undefined ? 0 : options.port);
  options.host = options.host || '127.0.0.1';
  return { options, callback };
}
function __tcpValidateOptions(options) {
  const unsupported = ['path', 'fd', 'lookup', 'localAddress', 'localPort', 'onread', 'signal', 'reusePort', 'exclusive', 'backlog', 'pauseOnConnect', 'blockList'];
  for (const key of unsupported) if (options[key] !== undefined) throw __streamError('ERR_TCP_UNSUPPORTED_OPTION', `TCP option ${key} is unsupported`);
}
function __tcpKeepAliveDelay(value) {
  const delay = Number(value);
  if (!Number.isInteger(delay) || delay < 0 || delay > 4294967295) throw __streamError('ERR_OUT_OF_RANGE', 'TCP keepalive delay must be an unsigned 32-bit integer');
  return delay;
}
function __tcpSocketOptions(options) {
  __tcpValidateOptions(options);
  if (options.objectMode || options.readableObjectMode || options.writableObjectMode) throw __streamError('ERR_INVALID_ARG_VALUE', 'TCP sockets carry bytes');
  return { ...options, autoDestroy: true };
}
class __TcpSocket extends __Duplex {
  constructor(options = {}) {
    super(__tcpSocketOptions(options));
    this._tcpId = null; this._tcpRef = true; this._tcpWrite = null; this._tcpFinal = null;
    this._tcpNoDelay = options.noDelay === undefined ? null : Boolean(options.noDelay); this._tcpKeepAlive = null; this._tcpNativeClosed = false; this._tcpTimeout = 0; this._tcpTimeoutId = null; this._tcpTimeoutDue = null;
    this.connecting = false; this.pending = true; this.allowHalfOpen = Boolean(options.allowHalfOpen);
    this.bytesRead = 0; this.bytesWritten = 0;
    this.on('end', () => __tcpLater(() => { if (!this.allowHalfOpen && !this.writableEnded) this.end(); }));
    this.on('close', () => { this._tcpClearTimeout(); __tcpOwnedSockets.delete(this); __tcpDestroyResource(this); });
    if (options.keepAlive !== undefined) this.setKeepAlive(options.keepAlive, options.keepAliveInitialDelay);
  }
  emit(event, ...args) {
    if (event === 'close') args = [Boolean(this.errored)];
    return super.emit(event, ...args);
  }
  connect(...args) {
    const { options, callback } = __tcpArguments(args);
    if (this.destroyed) throw __streamError('ERR_SOCKET_CLOSED', 'TCP socket is destroyed');
    if (this._tcpId !== null) throw __streamError('ERR_SOCKET_CONNECTING', 'TCP socket already has a connection');
    if (callback) this.once('connect', callback);
    this.connecting = true; __tcpResource(this, 'TCPWRAP');
    try { this._tcpAttach(__tcpNative('connect', options).id); }
    catch (error) { __tcpLater(() => this.destroy(error)); }
    return this;
  }
  _tcpAttach(id) {
    this._tcpId = id; __tcpSockets.set(id, this); __tcpOwnedSockets.add(this);
    if (!this._tcpRef) __tcpNative('ref', { id, enabled: false });
    if (this._tcpNoDelay !== null) __tcpNative('setNoDelay', { id, enabled: this._tcpNoDelay });
    if (this._tcpKeepAlive) __tcpNative('setKeepAlive', { id, ...this._tcpKeepAlive });
    __tcpPreparePoll();
  }
  _tcpEvent(event) {
    const handlers = { connect: __tcpConnected, data: __tcpData, end: __tcpEnd, write: __tcpWritten, finish: __tcpFinished, closed: __tcpClosed, error: __tcpErrored };
    handlers[event.kind](this, event);
  }
  _read() { this._tcpPause(false); }
  _tcpPause(enabled) {
    if (this._tcpId !== null && !this._tcpNativeClosed) __tcpNative('pause', { id: this._tcpId, enabled });
  }
  pause() { this._tcpPause(true); return super.pause(); }
  resume() { this._tcpPause(false); return super.resume(); }
  _write(chunk, encoding, callback) {
    this._tcpWrite = { bytes: __tcpBytes(chunk, encoding), callback, sent: false };
    if (!this.connecting) this._tcpSend();
  }
  _tcpSend() {
    const pending = this._tcpWrite;
    if (!pending || pending.sent || this.destroyed) return;
    if (this._tcpId === null) { pending.callback(__streamError('ERR_SOCKET_CLOSED', 'TCP socket is not connected')); return; }
    try { pending.sent = true; __tcpNative('send', { id: this._tcpId, data: pending.bytes.toString('base64') }); this._tcpActivity(); __tcpPreparePoll(); }
    catch (error) { this._tcpWrite = null; pending.callback(error); }
  }
  _final(callback) {
    this._tcpFinal = callback;
    if (!this.connecting) this._tcpFinish();
  }
  _tcpFinish() {
    if (this.destroyed) return;
    if (this._tcpId === null) { this._tcpFinal(__streamError('ERR_SOCKET_CLOSED', 'TCP socket is not connected')); return; }
    __tcpNative('end', { id: this._tcpId }); __tcpPreparePoll();
  }
  _destroy(error, callback) {
    this.connecting = false; this.pending = false;
    if (this._tcpId !== null) { __tcpNative('close', { id: this._tcpId }); __tcpSockets.delete(this._tcpId); }
    this._tcpId = null; this._tcpNativeClosed = true;
    this._tcpClearTimeout(); __tcpPreparePoll(); callback(error);
  }
  write(chunk, encoding, callback) {
    const size = __tcpBytes(chunk, typeof encoding === 'string' ? encoding : undefined).length;
    const result = super.write(chunk, encoding, callback); this.bytesWritten += size; return result;
  }
  address() {
    if (this._tcpId === null || this.localPort === undefined) return {};
    return { address: this.localAddress, port: this.localPort, family: this.localFamily };
  }
  setNoDelay(enabled = true) {
    this._tcpNoDelay = Boolean(enabled);
    if (this._tcpId !== null && !this._tcpNativeClosed) __tcpNative('setNoDelay', { id: this._tcpId, enabled: this._tcpNoDelay });
    return this;
  }
  setKeepAlive(enabled = false, initialDelay = 0) {
    const delay = __tcpKeepAliveDelay(initialDelay);
    this._tcpKeepAlive = { enabled: Boolean(enabled), delay };
    if (this._tcpId !== null && !this._tcpNativeClosed) __tcpNative('setKeepAlive', { id: this._tcpId, ...this._tcpKeepAlive });
    return this;
  }
  setTimeout(timeout, callback) {
    const value = Number(timeout);
    if (!Number.isFinite(value) || value < 0) throw __streamError('ERR_OUT_OF_RANGE', 'TCP timeout must be a nonnegative number');
    this._tcpTimeout = value; if (callback) this.once('timeout', callback); this._tcpActivity(); return this;
  }
  _tcpClearTimeout() {
    if (this._tcpTimeoutId !== null) __realTimers.clearTimeout(this._tcpTimeoutId);
    this._tcpTimeoutId = null;
  }
  _tcpActivity() {
    this._tcpTimeoutDue = this._tcpTimeout > 0 ? __loop.now + this._tcpTimeout : null;
    this._tcpReferenceTimeout();
  }
  _tcpReferenceTimeout() {
    this._tcpClearTimeout();
    if (this._tcpRef && this._tcpTimeoutDue !== null && !this.destroyed) this._tcpTimeoutId = __realTimers.setTimeout(() => this._tcpCheckTimeout(), Math.max(0, this._tcpTimeoutDue - __loop.now));
  }
  _tcpCheckTimeout() {
    if (this._tcpTimeoutDue === null || __loop.now < this._tcpTimeoutDue) return;
    this._tcpClearTimeout(); this._tcpTimeoutDue = null; this.emit('timeout'); return true;
  }
  ref() { return __tcpReference(this, true); }
  unref() { return __tcpReference(this, false); }
}
function __tcpConnected(socket, event) {
  Object.assign(socket, event.address); socket.connecting = false; socket.pending = false;
  socket._tcpActivity(); socket.emit('connect'); socket.emit('ready');
  socket._tcpSend(); if (socket._tcpFinal) socket._tcpFinish();
}
function __tcpData(socket, event) {
  const bytes = Buffer.from(event.data, 'base64'); socket.bytesRead += bytes.length; socket._tcpActivity();
  if (!socket.push(bytes)) socket._tcpPause(true);
}
function __tcpEnd(socket) {
  socket.push(null);
  // Native EOF must complete an empty readable side even when it never flowed.
  // Keep paused buffered bytes intact until the application consumes them.
  if (!socket.readableLength) socket.read(0);
}
function __tcpWritten(socket) {
  const pending = socket._tcpWrite; socket._tcpWrite = null;
  if (pending) pending.callback();
}
function __tcpFinished(socket) {
  const callback = socket._tcpFinal; socket._tcpFinal = null;
  if (callback) callback();
}
function __tcpClosed(socket) { socket._tcpNativeClosed = true; __tcpSockets.delete(socket._tcpId); socket._tcpId = null; }
function __tcpErrored(socket, event) { socket._tcpNativeClosed = true; socket.destroy(__tcpFailure(event.error)); }
class __TcpServer extends __EventEmitter {
  constructor(options = {}, listener) {
    super();
    if (typeof options === 'function') { listener = options; options = {}; }
    __tcpValidateOptions(options); this._tcpOptions = options;
    this._tcpId = null; this._tcpRef = true; this._tcpAddress = null; this._tcpConnections = new Set(); this._tcpClosing = false;
    this.listening = false;
    if (listener) this.on('connection', listener);
  }
  listen(...args) {
    const { options, callback } = __tcpArguments(args);
    if (this.listening) throw __streamError('ERR_SERVER_ALREADY_LISTEN', 'TCP server is already listening');
    if (callback) this.once('listening', callback);
    __tcpResource(this, 'TCPSERVERWRAP');
    try { this._tcpListen(options); } catch (error) { const resource = this._tcpResource; __tcpLater(() => { __tcpDestroyResource(this, resource); this.emit('error', error); }); }
    return this;
  }
  _tcpListen(options) {
    const result = __tcpNative('listen', options);
    this._tcpId = result.id; this._tcpAddress = result.address; this.listening = true; this._tcpClosing = false;
    __tcpServers.set(result.id, this); __tcpReference(this, this._tcpRef);
    __tcpLater(() => { if (this.listening) this.emit('listening'); });
  }
  _tcpEvent(event) {
    if (event.kind === 'serverError') { this.emit('error', __tcpFailure(event.error)); return; }
    const socket = new __TcpSocket(this._tcpOptions); __tcpResource(socket, 'TCPWRAP'); socket._tcpAttach(event.id);
    Object.assign(socket, event.address); socket.pending = false; this._tcpConnections.add(socket);
    socket.once('close', () => { this._tcpConnections.delete(socket); this._tcpCheckClosed(); });
    this.emit('connection', socket);
  }
  address() { return this._tcpAddress; }
  getConnections(callback) { __tcpLater(() => callback(null, this._tcpConnections.size)); }
  close(callback) {
    if (!this.listening) { if (callback) __tcpLater(() => callback(__streamError('ERR_SERVER_NOT_RUNNING', 'TCP server is not running'))); return this; }
    if (callback) this.once('close', () => callback());
    __tcpNative('close', { id: this._tcpId }); __tcpServers.delete(this._tcpId);
    this._tcpId = null; this._tcpAddress = null; this.listening = false; this._tcpClosing = true;
    __tcpPreparePoll(); this._tcpCheckClosed(); return this;
  }
  _tcpCheckClosed() {
    if (!this._tcpClosing || this._tcpConnections.size) return;
    this._tcpClosing = false;
    const resource = this._tcpResource;
    __tcpLater(() => { __tcpDestroyResource(this, resource); this.emit('close'); });
  }
  ref() { return __tcpReference(this, true); }
  unref() { return __tcpReference(this, false); }
}
function __tcpEvent(event) {
  if (event.server !== undefined) {
    const server = __tcpServers.get(event.server);
    if (server) server._tcpDispatch(event);
    else if (event.kind === 'accept') __tcpNative('close', { id: event.id });
    return;
  }
  const socket = __tcpSockets.get(event.id);
  if (socket && !socket.destroyed) socket._tcpDispatch(event);
}
function __tcpReset() {
  __tcpStopPoll();
  for (const socket of __tcpOwnedSockets) { socket._tcpClearTimeout(); socket.destroyed = true; socket.closed = true; socket._tcpId = null; socket._tcpNativeClosed = true; }
  if (__tcpUsed) __tcpNative('reset');
  __tcpSockets.clear(); __tcpOwnedSockets.clear(); __tcpServers.clear(); __tcpUsed = false;
}
function __tcpConnect(...args) { const { options } = __tcpArguments(args); const socket = new __TcpSocket(options); return socket.connect(...args); }
const __nodeTcp = { Socket: __TcpSocket, Server: __TcpServer, createServer: (options, listener) => new __TcpServer(options, listener), createConnection: __tcpConnect, connect: __tcpConnect };
