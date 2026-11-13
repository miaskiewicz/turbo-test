// Loopback HTTP/1 transport. Socket ownership and byte framing are native; callbacks stay in V8.
const __httpConnections = new Map();
const __httpServers = new Map();
let __httpPollTimer = null;
function __httpFailure(data) { return Object.assign(new Error(data.message), { code: data.code }); }
function __httpNative(operation, args = {}) {
  const result = JSON.parse(globalThis.__ttHttp(operation, JSON.stringify(args)));
  if (result.error) throw __httpFailure(result.error);
  return result.value;
}
function __httpBytes(value, encoding) {
  if (typeof value === 'string') return Buffer.from(value, encoding);
  if (value instanceof Uint8Array) return Buffer.from(value);
  throw new TypeError('HTTP chunks must be strings, Buffers or Uint8Arrays');
}
function __httpLater(fn) { queueMicrotask(fn); }
function __httpCapture(fn) { return typeof __captureAsyncContext === 'function' ? __captureAsyncContext(fn) : fn; }
class __HttpSocket extends __EventEmitter {
  constructor(connection, address) { super(); Object.assign(this, address); this._connection = connection; }
  get readable() { return !this._connection.closed; }
  get writable() { return !this._connection.closed; }
  get destroyed() { return Boolean(this._connection.closed); }
  destroy(error) { __httpClose(this._connection, error); return this; }
  setNoDelay(noDelay = true) {
    if (!this.destroyed) __httpNative('setNoDelay', { id: this._connection.id, enabled: !!noDelay });
    return this;
  }
  address() { return { address: this.localAddress, port: this.localPort, family: this.localAddress.includes(':') ? 'IPv6' : 'IPv4' }; }
}
function __httpSocketClosed(connection, error) {
  const socket = connection.socket;
  if (!socket) return;
  __httpLater(() => {
    if (error && socket.listenerCount('error')) socket.emit('error', error);
    socket.emit('close', Boolean(error));
  });
}
function __httpRegister(connection) {
  connection.socket = new __HttpSocket(connection, connection.socket);
  connection.dispatch = __httpCapture((event) => __httpConnectionEvent(connection, event));
  __httpConnections.set(connection.id, connection);
}
function __httpEnsurePoll() {
  if (__httpPollTimer !== null) return;
  __httpPollTimer = __realTimers.setTimeout(__httpPoll, 1);
}
function __httpPoll() {
  __httpPollTimer = null;
  const events = __httpNative('poll');
  for (const event of events) __httpEvent(event);
  if (__httpConnections.size || __httpServers.size) __httpEnsurePoll();
}
function __httpReset() {
  if (__httpPollTimer !== null) __realTimers.clearTimeout(__httpPollTimer);
  __httpPollTimer = null;
  __httpNative('reset'); __httpConnections.clear(); __httpServers.clear();
}
function __httpWrite(connection, bytes, finish, callback) {
  if (!bytes.length && !finish) { __httpLater(callback); return; }
  try {
    __httpNative('send', { id: connection.id, data: bytes.toString('base64'), finish });
    connection.callbacks.push(__httpCapture(callback)); __httpEnsurePoll();
  } catch (error) { __httpLater(() => callback(error)); }
}
function __httpForget(connection) {
  if (connection.closed) return false;
  connection.closed = true;
  __httpConnections.delete(connection.id);
  if (connection.server) { connection.server._connections.delete(connection); connection.server._checkClosed(); }
  return true;
}
function __httpClose(connection, reason, owner) {
  if (!connection || !__httpForget(connection)) return;
  __httpNative('close', { id: connection.id });
  const error = reason || __streamError('ECONNRESET', 'Socket was destroyed');
  connection.callbacks.splice(0).forEach((callback) => callback(error));
  if (connection.incoming !== owner) __httpIncomingClosed(connection.incoming, error);
  if (connection.outgoing !== owner) __httpOutgoingClosed(connection, error);
  __httpSocketClosed(connection, reason);
}
function __httpEvent(event) {
  if (event.kind === 'accept') { __httpAccept(event); return; }
  const connection = __httpConnections.get(event.id);
  if (!connection) return;
  connection.dispatch(event);
}
function __httpConnectionEvent(connection, event) {
  if (event.kind === 'headers') __httpHeaders(connection, event);
  else __httpBodyEvent(connection, event);
}
function __httpBodyEvent(connection, event) {
  if (event.kind === 'data') connection.incoming.push(Buffer.from(event.data, 'base64'));
  else if (event.kind === 'end') __httpEnd(connection);
  else if (event.kind === 'drain') connection.callbacks.splice(0).forEach((callback) => callback());
  else if (event.kind === 'close') __httpClosed(connection, event.error);
}
function __httpAccept(event) {
  const server = __httpServers.get(event.server);
  if (!server) { __httpNative('close', { id: event.id }); return; }
  server._accept(event);
}
function __httpAccepted(server, event) {
  const connection = { id: event.id, server, callbacks: [], incoming: null, outgoing: null, socket: event.socket };
  __httpRegister(connection); server._connections.add(connection);
  server.emit('connection', connection.socket);
}
function __httpHeaderObject(pairs) {
  const headers = Object.create(null);
  for (const [key, value] of pairs) __httpAddHeader(headers, key.toLowerCase(), value);
  return headers;
}
function __httpAddHeader(headers, key, value) {
  if (key === 'set-cookie') { (headers[key] || (headers[key] = [])).push(value); return; }
  headers[key] = headers[key] === undefined ? value : headers[key] + ', ' + value;
}
function __httpHeaders(connection, event) {
  const incoming = new __HttpIncoming(connection, event);
  connection.incoming = incoming;
  if (connection.server) {
    const response = new __HttpResponse(connection, incoming);
    connection.outgoing = response;
    connection.server.emit('request', incoming, response);
  } else connection.outgoing.emit('response', incoming);
}
function __httpEnd(connection) {
  if (!connection.incoming) return;
  connection.incoming.complete = true;
  connection.incoming.push(null);
}
function __httpClosed(connection, data) {
  if (!__httpForget(connection)) return;
  const error = data && __httpFailure(data);
  connection.callbacks.splice(0).forEach((callback) => callback(error || __streamError('ECONNRESET', 'Socket closed')));
  __httpIncomingClosed(connection.incoming, error);
  __httpOutgoingClosed(connection, error);
  __httpSocketClosed(connection, error);
  if (connection.server && !connection.incoming && error) connection.server.emit('clientError', error, { destroy() {} });
}
function __httpOutgoingClosed(connection, error) {
  const outgoing = connection.outgoing;
  if (!outgoing || outgoing.destroyed) return;
  if (!connection.server) { __httpRequestClosed(outgoing, error); return; }
  if (!outgoing._transportFinished) outgoing.destroy(error);
  else __httpLater(() => { outgoing.closed = true; outgoing.emit('close'); });
}
function __httpIncomingClosed(incoming, error) {
  if (!incoming) return;
  if (!incoming.complete) { incoming.aborted = true; incoming.emit('aborted'); incoming.destroy(error || __streamError('ECONNRESET', 'Socket closed')); }
  else __httpLater(() => { incoming.closed = true; incoming.emit('close'); });
}
function __httpRequestClosed(request, error) {
  if (request.destroyed) return;
  if (error && !request._connection.incoming) request.destroy(error);
  else if (!request._connection.incoming) request.destroy(__streamError('ECONNRESET', 'Socket closed before response'));
  else { request.closed = true; __httpLater(() => request.emit('close')); }
}
class __HttpIncoming extends __Readable {
  constructor(connection, event) {
    super({ autoDestroy: false });
    this._connection = connection; this.headers = __httpHeaderObject(event.headers);
    this.rawHeaders = event.headers.flat(); this.complete = false; this.aborted = false;
    const parts = event.line.split(' ');
    const version = connection.server ? parts[2] : parts[0];
    this.httpVersion = version.slice(5);
    this.httpVersionMajor = Number(this.httpVersion.split('.')[0]); this.httpVersionMinor = Number(this.httpVersion.split('.')[1]);
    if (connection.server) { this.method = parts[0]; this.url = parts[1]; this.on('error', () => {}); }
    else { this.statusCode = Number(parts[1]); this.statusMessage = parts.slice(2).join(' '); }
    this.socket = connection.socket; this.connection = this.socket;
  }
  _read() {}
  _destroy(error, callback) { __httpClose(this._connection, error, this); callback(error); }
}
function __httpHeaderName(name) {
  name = String(name);
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name)) throw __streamError('ERR_INVALID_HTTP_TOKEN', 'Invalid HTTP header name');
  return name.toLowerCase();
}
function __httpHeaderValue(value) {
  if (Array.isArray(value)) return value.map((part) => __httpHeaderValue(part));
  value = String(value);
  if (/[^\t\x20-\xff]/.test(value)) throw __streamError('ERR_INVALID_CHAR', 'Invalid character in HTTP header');
  return value;
}
function __httpHeaderLines(entry) {
  const values = Array.isArray(entry.value) ? entry.value : [entry.value];
  return values.map((value) => entry.name + ': ' + value);
}
function __httpLatinBytes(text) {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
  return Buffer.from(bytes);
}
class __HttpOutgoing extends __Writable {
  constructor(connection) {
    super({ autoDestroy: false }); this._connection = connection;
    this._headers = new Map(); this.headersSent = false; this._chunked = false; this._bodySize = 0;
  }
  setHeader(name, value) {
    if (this.headersSent) throw __streamError('ERR_HTTP_HEADERS_SENT', 'Headers already sent');
    const key = __httpHeaderName(name); this._headers.set(key, { name: String(name), value: __httpHeaderValue(value) }); return this;
  }
  getHeader(name) { return this._headers.get(String(name).toLowerCase())?.value; }
  hasHeader(name) { return this._headers.has(String(name).toLowerCase()); }
  removeHeader(name) {
    if (this.headersSent) throw __streamError('ERR_HTTP_HEADERS_SENT', 'Headers already sent');
    this._headers.delete(String(name).toLowerCase());
  }
  getHeaderNames() { return Array.from(this._headers.keys()); }
  getHeaders() { return Object.fromEntries(Array.from(this._headers, ([key, entry]) => [key, entry.value])); }
  _noBody() { return false; }
  _write(chunk, encoding, callback) {
    const bytes = __httpBytes(chunk, encoding); this._bodySize += bytes.length;
    const head = this._head();
    const body = this._noBody() ? Buffer.alloc(0) : __httpFrame(bytes, this._chunked);
    __httpWrite(this._connection, Buffer.concat([head, body]), false, callback);
  }
  _final(callback) {
    const head = this._head();
    const tail = this._chunked ? Buffer.from('0\r\n\r\n') : Buffer.alloc(0);
    __httpWrite(this._connection, Buffer.concat([head, tail]), this instanceof __HttpResponse, (error) => {
      if (!error) this._transportFinished = true;
      callback(error);
    });
  }
  _head() {
    if (this.headersSent) return Buffer.alloc(0);
    this.headersSent = true;
    this.setFraming();
    const lines = Array.from(this._headers.values(), (entry) => __httpHeaderLines(entry)).flat();
    return __httpLatinBytes(this._firstLine() + '\r\n' + lines.join('\r\n') + '\r\n\r\n');
  }
  setFraming() {
    __httpConnectionFraming(this);
    if (this._noBody()) return;
    if (!this._headers.has('content-length') && !this._headers.has('transfer-encoding')) this._headers.set('transfer-encoding', { name: 'Transfer-Encoding', value: 'chunked' });
    this._chunked = __httpTransferFraming(this);
  }
  _destroy(error, callback) { __httpClose(this._connection, error, this); callback(error); }
}
function __httpConnectionFraming(message) {
  const connection = String(message.getHeader('connection') || 'close').toLowerCase();
  if (connection !== 'close') throw __streamError('ERR_HTTP_UNSUPPORTED_KEEP_ALIVE', 'Persistent HTTP connections are not supported');
  message._headers.set('connection', { name: 'Connection', value: 'close' });
}
function __httpTransferFraming(message) {
  const transfer = message.getHeader('transfer-encoding');
  if (transfer === undefined) return false;
  if (String(transfer).toLowerCase() !== 'chunked') throw __streamError('ERR_HTTP_UNSUPPORTED_TRANSFER_ENCODING', 'Only chunked transfer encoding is supported');
  if (message.hasHeader('content-length')) throw __streamError('ERR_HTTP_CONTENT_LENGTH_MISMATCH', 'Conflicting HTTP body framing');
  return true;
}
function __httpFrame(bytes, chunked) {
  if (!chunked || !bytes.length) return bytes;
  return Buffer.concat([Buffer.from(bytes.length.toString(16) + '\r\n'), bytes, Buffer.from('\r\n')]);
}
class __HttpResponse extends __HttpOutgoing {
  constructor(connection, incoming) { super(connection); this.socket = connection.socket; this.connection = this.socket; this.req = incoming; this.statusCode = 200; this.statusMessage = undefined; }
  _noBody() { return this.req.method === 'HEAD' || this.statusCode === 204 || this.statusCode === 304; }
  _firstLine() {
    if (!Number.isInteger(this.statusCode) || this.statusCode < 100 || this.statusCode > 999) throw new RangeError('Invalid HTTP status');
    if (this.statusCode < 200) throw __streamError('ERR_HTTP_UNSUPPORTED_STATUS', 'Informational responses and upgrades are not supported');
    return 'HTTP/1.1 ' + this.statusCode + ' ' + __httpHeaderValue(this.statusMessage || globalThis.__STATUS_TEXT[this.statusCode] || 'unknown');
  }
  writeHead(status, reason, headers) {
    if (typeof reason === 'object') { headers = reason; reason = undefined; }
    this.statusCode = status; this.statusMessage = reason;
    if (headers) for (const [key, value] of Object.entries(headers)) this.setHeader(key, value);
    return this;
  }
}
function __httpInput(input) {
  if (typeof input !== 'string' && !(input instanceof URL)) return { ...input };
  const url = new URL(String(input));
  return { protocol: url.protocol, hostname: url.hostname, port: url.port || 80, path: url.pathname + url.search };
}
function __httpTarget(options) {
  options.hostname = options.hostname || options.host || '127.0.0.1';
  options.port = __httpPort(options.port ?? 80);
}
function __httpPort(value) {
  const port = Number(value);
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw __streamError('ERR_SOCKET_BAD_PORT', 'Invalid HTTP port');
  return port;
}
function __httpOptions(input, options) {
  const result = { ...__httpInput(input), ...options };
  if (result.protocol && result.protocol !== 'http:') throw __streamError('ERR_INVALID_PROTOCOL', 'Only http: is supported');
  __httpTarget(result);
  result.method = String(result.method || 'GET').toUpperCase(); result.path = String(result.path || '/');
  return result;
}
function __httpRequestOptions(options) {
  if (!/^[!#$%&'*+.^_`|~0-9A-Z-]+$/.test(options.method)) throw __streamError('ERR_INVALID_HTTP_TOKEN', 'Invalid HTTP method');
  if (/[^\x21-\xff]/.test(options.path)) throw __streamError('ERR_UNESCAPED_CHARACTERS', 'Request path contains unescaped characters');
  if (options.agent && options.agent !== __http.globalAgent) throw __streamError('ERR_HTTP_UNSUPPORTED_AGENT', 'Custom HTTP agents are not supported');
}
class __HttpRequest extends __HttpOutgoing {
  constructor(options, callback) {
    const connection = { id: 0, callbacks: [], incoming: null, outgoing: null };
    super(connection); connection.outgoing = this; this.method = options.method; this.path = options.path;
    if (callback) this.once('response', callback);
    this.setHeader('Host', options.hostname + ':' + options.port);
    for (const [name, value] of Object.entries(options.headers || {})) this.setHeader(name, value);
    try {
      const result = __httpNative('connect', { host: options.hostname, port: options.port, head: options.method === 'HEAD' });
      connection.id = result.id; connection.socket = result.socket; __httpRegister(connection);
      this.socket = connection.socket; this.connection = this.socket;
      __httpLater(() => this.emit('socket', this.socket)); __httpEnsurePoll();
    } catch (error) { this._connectionError = error; __httpLater(() => this.destroy(error)); }
  }
  _firstLine() { return this.method + ' ' + this.path + ' HTTP/1.1'; }
  _write(chunk, encoding, callback) {
    if (this._connectionError) { callback(this._connectionError); return; }
    super._write(chunk, encoding, callback);
  }
  _final(callback) {
    if (this._connectionError) { callback(this._connectionError); return; }
    super._final(callback);
  }
  setNoDelay(noDelay = true) {
    if (this._connectionError || this._connection.closed) return this;
    __httpNative('setNoDelay', { id: this._connection.id, enabled: !!noDelay });
    return this;
  }
  abort() { this.destroy(); return this; }
  setTimeout(delay, callback) {
    if (callback) this.once('timeout', callback);
    __realTimers.setTimeout(() => { if (!this.closed) this.emit('timeout'); }, delay); return this;
  }
}
function __httpRequest(input, options, callback) {
  if (typeof options === 'function') { callback = options; options = undefined; }
  const normalized = __httpOptions(input, options); __httpRequestOptions(normalized);
  return new __HttpRequest(normalized, callback);
}
function __httpGet(...args) { const request = __httpRequest(...args); request.end(); return request; }
function __httpListenOptions(args) {
  const callback = typeof args[args.length - 1] === 'function' ? args.pop() : undefined;
  const options = typeof args[0] === 'object' ? args[0] : { port: args[0], host: args[1] };
  const port = __httpPort(options.port ?? 0); const host = options.host || '127.0.0.1';
  return { callback, host, port };
}
class __HttpServer extends __EventEmitter {
  constructor(options, listener) {
    super(); this._id = 0; this._address = null; this._connections = new Set(); this._closing = false; this.listening = false;
    if (typeof options === 'function') listener = options;
    if (listener) this.on('request', listener);
  }
  listen(...args) {
    if (this.listening) throw __streamError('ERR_SERVER_ALREADY_LISTEN', 'Server is already listening');
    const options = __httpListenOptions(args);
    try {
      const address = __httpNative('listen', options); this._id = address.id;
      this._address = { port: address.port, address: address.address, family: address.family }; this.listening = true;
      this._accept = __httpCapture((event) => __httpAccepted(this, event));
      __httpServers.set(this._id, this); __httpEnsurePoll();
      __httpLater(() => { this.emit('listening'); if (options.callback) options.callback(); });
    } catch (error) { __httpLater(() => this.emit('error', error)); }
    return this;
  }
  address() { return this._address; }
  close(callback) {
    if (!this.listening) { if (callback) __httpLater(() => callback(__streamError('ERR_SERVER_NOT_RUNNING', 'Server is not running'))); return this; }
    if (callback) this.once('close', __httpCapture(callback));
    this.listening = false; this._closing = true; this._address = null;
    __httpNative('close', { id: this._id }); __httpServers.delete(this._id); this._checkClosed(); return this;
  }
  _checkClosed() {
    if (!this._closing || this._connections.size) return;
    this._closing = false; __httpLater(() => this.emit('close'));
  }
  ref() { return this; }
  unref() { return this; }
}
const __nodeHttp = { Server: __HttpServer, ClientRequest: __HttpRequest, IncomingMessage: __HttpIncoming,
  ServerResponse: __HttpResponse, createServer: (options, listener) => new __HttpServer(options, listener),
  request: __httpRequest, get: __httpGet };
