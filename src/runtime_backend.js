// Backend-only source. This text is never compiled into a DOM runtime.
/* __TT_NODE_KEYS__ */
/* __TT_NODE_AES__ */
/* __TT_NODE_X509__ */
/* __TT_NODE_CRYPTO_KDF__ */
__crypto.constants = {
    RSA_PKCS1_PADDING: 1, RSA_NO_PADDING: 3, RSA_PKCS1_OAEP_PADDING: 4, RSA_X931_PADDING: 5,
    RSA_PKCS1_PSS_PADDING: 6, RSA_PSS_SALTLEN_DIGEST: -1, RSA_PSS_SALTLEN_MAX_SIGN: -2,
    RSA_PSS_SALTLEN_AUTO: -2, RSA_SSLV23_PADDING: 2,
    SSL_OP_ALL: 0x80000bff, ENGINE_METHOD_ALL: 0xffff,
    DH_CHECK_P_NOT_SAFE_PRIME: 2, DH_NOT_SUITABLE_GENERATOR: 8,
    POINT_CONVERSION_COMPRESSED: 2, POINT_CONVERSION_UNCOMPRESSED: 4, POINT_CONVERSION_HYBRID: 6,
    defaultCoreCipherList: 'TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256',
  };
Object.assign(__constants, __crypto.constants);
Object.assign(__crypto, __nodeKeys, __nodeAes, __nodeX509, { pbkdf2Sync: __cryptoPbkdf2 });
// http/https/net/tls: enough surface that connection-agent libs (agent-base, https-proxy-agent —
// pulled in transitively by e.g. @playwright/test) load. agent-base does `class X extends
// http.Agent`, so http.Agent MUST be a real (extendable) class. No real networking happens in
// unit tests (request contexts are mocked); these just need to construct + be subclassed.
class __Agent extends __EventEmitter { constructor(opts) { super(); this.options = opts || {}; this.maxSockets = Infinity; this.sockets = {}; this.requests = {}; } destroy() {} getName() { return 'localhost::'; } addRequest() {} createConnection() { return new __EventEmitter(); } }
class __ClientRequest extends __EventEmitter { setHeader() {} getHeader() {} end() { return this; } write() { return true; } abort() {} destroy() { return this; } setTimeout() { return this; } }
class __IncomingMessage extends __Readable { constructor() { super(); this.headers = {}; this.statusCode = 200; } }
const __mkServer = () => { const s = new __EventEmitter(); s.listen = (..._a) => { const cb = _a[_a.length - 1]; if (typeof cb === 'function') cb(); return s; }; s.close = (cb) => { if (typeof cb === 'function') cb(); return s; }; s.address = () => ({ port: 0, address: '127.0.0.1' }); return s; };
const __http = { Agent: __Agent, globalAgent: new __Agent(), ClientRequest: __ClientRequest, IncomingMessage: __IncomingMessage, ServerResponse: class extends __EventEmitter { setHeader() {} end() { return this; } write() { return true; } writeHead() { return this; } }, Server: class extends __EventEmitter {}, METHODS: ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS'], STATUS_CODES: {}, createServer: __mkServer, request: () => new __ClientRequest(), get: () => new __ClientRequest() };
__http.default = __http;
const __https = Object.assign({}, __http, { Agent: __Agent, globalAgent: new __Agent() }); __https.default = __https;
/* __TT_NODE_HTTP__ */
Object.assign(__http, __nodeHttp);
function __httpUnsupportedTls() { throw Object.assign(new Error('HTTPS is not supported by the loopback HTTP transport'), { code: 'ERR_HTTP_UNSUPPORTED_TLS' }); }
Object.assign(__https, { request: __httpUnsupportedTls, get: __httpUnsupportedTls, createServer: __httpUnsupportedTls });
/* __TT_NODE_BLOCKLIST__ */
const __net = { isIP: (input) => globalThis.__ttIsIP(input), isIPv4: (input) => globalThis.__ttIsIP(input) === 4, isIPv6: (input) => globalThis.__ttIsIP(input) === 6 }; __net.BlockList = __NodeBlockList; __net.default = __net;
/* __TT_NODE_TCP__ */
Object.assign(__net, __nodeTcp);
// HTTP clients inspect this constructor even on plain HTTP paths. Its identity must not
// imply that a fake listener supplies TLS: construction still reports the capability limit.
function __TlsServer() { throw Object.assign(new Error('TLS server transport is not supported'), { code: 'ERR_TLS_UNSUPPORTED_SERVER' }); }
Object.setPrototypeOf(__TlsServer.prototype, __net.Server.prototype);
Object.setPrototypeOf(__TlsServer, __net.Server);
const __tls = { Server: __TlsServer, TLSSocket: class extends __EventEmitter { connect() { return this; } }, connect: () => new __EventEmitter(), createSecureContext: () => ({}), createServer: __TlsServer, checkServerIdentity: () => undefined, rootCertificates: [] }; __tls.default = __tls;
function __gzipBytes(input) {
  if (typeof input === 'string') return Buffer.from(input);
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  throw new TypeError('Expected a string, Buffer, TypedArray, DataView or ArrayBuffer');
}
function __gzipLevel(options) {
  const level = options.level ?? -1;
  if (typeof level !== 'number') throw new TypeError('level must be a number');
  if (!Number.isInteger(level) || level < -1 || level > 9) throw new RangeError('level must be between -1 and 9');
  return level === -1 ? 6 : level;
}
function __gzipSync(input, options = {}, decompress = false) {
  return Buffer.from(globalThis.__ttGzip(__gzipBytes(input), decompress, __gzipLevel(options || {})));
}
function __deflateOptions(options) {
  const value = options || {};
  for (const name of ['dictionary', 'windowBits', 'memLevel', 'strategy', 'flush', 'finishFlush', 'maxOutputLength']) {
    if (value[name] !== undefined) throw Object.assign(new Error(`turbo-test: zlib option ${name} is unsupported`), {code: 'ERR_TURBO_UNSUPPORTED_ZLIB_OPTION'});
  }
  return value;
}
function __deflateSync(input, options, decompress = false) {
  const level = __gzipLevel(__deflateOptions(options));
  return Buffer.from(globalThis.__ttGzip(__gzipBytes(input), decompress, level, true));
}
function __unsupportedDeflateStream() {
  throw Object.assign(new Error('turbo-test: asynchronous and streaming deflate/inflate are unsupported; use the synchronous APIs'), {code: 'ERR_TURBO_UNSUPPORTED_ZLIB_STREAM'});
}
const __zlib = { createGzip: () => new __Duplex(), createGunzip: () => new __Duplex(), createDeflate: __unsupportedDeflateStream, createInflate: __unsupportedDeflateStream, createBrotliCompress: () => new __Duplex(), createBrotliDecompress: () => new __Duplex(), gzip: (_b, cb) => cb && cb(null, _b), gunzip: (_b, cb) => cb && cb(null, _b), gzipSync: (b, opts) => __gzipSync(b, opts), gunzipSync: (b, opts) => __gzipSync(b, opts, true), deflateSync: (b, opts) => __deflateSync(b, opts), inflateSync: (b, opts) => __deflateSync(b, opts, true), deflate: __unsupportedDeflateStream, inflate: __unsupportedDeflateStream }; __zlib.default = __zlib;
/* __TT_NODE_ZLIB_CONSTANTS__ */
const __tty = { isatty: () => false, ReadStream: __EventEmitter, WriteStream: __EventEmitter }; __tty.default = __tty;
const __dns = { lookup: (_h, _o, cb) => { const c = typeof _o === 'function' ? _o : cb; if (c) c(null, '127.0.0.1', 4); }, resolve: (_h, cb) => cb && cb(null, []), promises: { lookup: () => Promise.resolve({ address: '127.0.0.1', family: 4 }), resolve: () => Promise.resolve([]) } }; __dns.default = __dns;
const __http2 = { connect: () => new __EventEmitter(), createServer: __mkServer, createSecureServer: __mkServer, constants: {}, getDefaultSettings: () => ({}), Http2ServerRequest: __EventEmitter, Http2ServerResponse: __EventEmitter }; __http2.default = __http2;
const __dgram = { createSocket: () => Object.assign(new __EventEmitter(), { bind() { return this; }, send() {}, close() {}, address: () => ({ port: 0 }) }) }; __dgram.default = __dgram;
const __readline = { createInterface: () => Object.assign(new __EventEmitter(), { question: (_q, cb) => cb && cb(''), close() {}, prompt() {}, write() {} }), clearLine: () => {}, cursorTo: () => {}, moveCursor: () => {} }; __readline.default = __readline;
const __v8 = { serialize: (value) => Buffer.from(globalThis.__ttV8Serialize(value)), deserialize: (bytes) => { if (!ArrayBuffer.isView(bytes)) throw new TypeError('buffer must be a TypedArray or DataView'); return globalThis.__ttV8Deserialize(bytes); }, getHeapStatistics: () => ({ total_heap_size: 0, used_heap_size: 0 }), setFlagsFromString: () => {} }; __v8.default = __v8;
/* __TT_NODE_ASYNC_CONTEXT__ */
/* __TT_NODE_ASYNC_HOOKS__ */
const __async_hooks = { AsyncLocalStorage: __AsyncLocalStorage, AsyncResource: __AsyncResource,
  createHook: __createAsyncHook, executionAsyncId: () => __hookCurrent.id,
  triggerAsyncId: () => __hookCurrent.trigger, executionAsyncResource: () => __hookCurrent.resource };
__async_hooks.default = __async_hooks;
/* __TT_NODE_VM__ */
const __vm = {
  Script: __VmScript,
  runInThisContext: (code) => (0, eval)(String(code)),
  compileFunction: (code, params) => new Function(...(params || []), String(code)),
  isContext: __vmIsContext,
  createContext: __vmCreateContext,
  runInContext: __vmRunInContext,
  runInNewContext: __vmRunInNewContext,
};
const __inspectorError = (code, message) => { const e = new Error(message); e.code = code; return e; };
const __inspector = {
  url: () => undefined, // no inspector is active
  open: __unavailable('inspector.open', 'the test isolate exposes no inspector'),
  close: () => {},
  waitForDebugger: () => { throw __inspectorError('ERR_INSPECTOR_NOT_ACTIVE', 'Inspector is not active'); },
  get console() { return globalThis.console; },
  Session: class Session extends __EventEmitter {
    connect() { __unavailable('inspector.Session.connect', 'the test isolate exposes no inspector')(); }
    connectToMainThread() { __unavailable('inspector.Session.connectToMainThread', 'the test isolate exposes no inspector')(); }
    post() { throw __inspectorError('ERR_INSPECTOR_NOT_CONNECTED', 'Session is not connected'); }
    disconnect() {}
  },
};
/* __TT_NODE_PROCESSES__ */
/* __TT_NODE_ASYNC_PROCESSES__ */
Object.assign(globalThis.__nodeBuiltins, {
  http2: __http2,
  v8: __v8,
  child_process: __childProcess,
  'node:child_process': __childProcess,
  http: __http,
  'node:http': __http,
  https: __https,
  'node:https': __https,
  net: __net,
  'node:net': __net,
  tls: __tls,
  'node:tls': __tls,
  zlib: __zlib,
  'node:zlib': __zlib,
  tty: __tty,
  'node:tty': __tty,
  dns: __dns,
  'node:dns': __dns,
  'dns/promises': __dns.promises,
  'node:dns/promises': __dns.promises,
  'node:http2': __http2,
  dgram: __dgram,
  'node:dgram': __dgram,
  readline: __readline,
  'node:readline': __readline,
  'node:v8': __v8,
  async_hooks: __async_hooks,
  'node:async_hooks': __async_hooks,
  vm: __vm,
  inspector: __inspector,
});
// Prefix aliases must replace their DOM capability slots as well as the bare exports.
for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(globalThis.__nodeBuiltins))) {
  if (!name.startsWith('node:') && Object.hasOwn(descriptor, 'value')) {
    const alias = Object.getOwnPropertyDescriptor(globalThis.__nodeBuiltins, 'node:' + name);
    if (alias && alias.set) globalThis.__nodeBuiltins['node:' + name] = descriptor.value;
  }
}
__runtimeTaskCapture = __captureBackendAsyncContext;
__runtimeTaskResource = () => new __async_hooks.AsyncResource('MESSAGEPORT');
__runtimeBackendResets.push(__resetAsyncHooks, __resetAsyncContext, __httpReset, __asyncProcessReset, __tcpReset);
__runtimeBackendInstalled = true;
