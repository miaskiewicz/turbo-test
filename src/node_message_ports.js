// Owned Node MessagePorts: native handles carry identity; endpoints keep queues per context.
const __portMapGet = Function.prototype.call.bind(Map.prototype.get);
const __portMapSet = Function.prototype.call.bind(Map.prototype.set);
const __portMapDelete = Function.prototype.call.bind(Map.prototype.delete);
const __portWeakGet = Function.prototype.call.bind(WeakMap.prototype.get);
const __portWeakSet = Function.prototype.call.bind(WeakMap.prototype.set);
const __portWeakRef = WeakRef;
const __portFinalizer = FinalizationRegistry;
const __portEndpoints = new Map();
const __portHandles = new WeakMap();
let __portSequence = 1;
let __portRegistry = null;
let __portResourceClass = null;
function __portHandle(port) {
  const handle = __portWeakGet(__portHandles, port);
  if (!handle) throw new TypeError('Invalid MessagePort receiver');
  return handle;
}
function __portNewHandle(port) {
  __portWeakSet(__portHandles, port, { listeners: new Map(), onmessage: null, onmessageerror: null, refed: false });
}
function __portEndpoint(port) { return __portMapGet(__portEndpoints, globalThis.__ttPortId(port)); }
function __portOwner(record) { return record.owner.deref(); }
function __portResource() {
  if (!__portResourceClass) __portResourceClass = __async_hooks.AsyncResource;
  return new __portResourceClass('MESSAGEPORT');
}
function __portDestroyResource(resource) { if (typeof resource.emitDestroy === 'function') resource.emitDestroy(); }
function __portCloseEvent(port, resource, cleanup = false) {
  __realTimers.setImmediate(() => {
    try { resource.runInAsyncScope(__portDispatch, port, port, 'close'); }
    finally { if (cleanup) __portDestroyResource(resource); }
  });
}
function __portForget(id, generation, keepResource = false) {
  const record = __portMapGet(__portEndpoints, id);
  if (!record || record.generation !== generation) return;
  __portMapDelete(__portEndpoints, id);
  record.queue.length = 0;
  if (!keepResource) __portDestroyResource(record.resource);
  const peer = __portMapGet(__portEndpoints, record.peer);
  if (!peer) return;
  peer.peer = null;
  const owner = __portOwner(peer);
  if (owner) __portCloseEvent(owner, peer.resource);
}
function __portRegister(port, record) {
  if (!__portRegistry) __portRegistry = new __portFinalizer(({ id, generation }) => __portForget(id, generation));
  record.owner = new __portWeakRef(port);
  __portRegistry.register(port, { id: record.id, generation: record.generation }, port);
}
function __portCreate(id, peer) {
  const port = globalThis.__ttPortCreate(id, __NodeMessagePort.prototype, __portCommit);
  __portNewHandle(port);
  const record = { id, peer, queue: [], started: false, scheduled: false, generation: 0, owner: null, resource: __portResource() };
  __portMapSet(__portEndpoints, id, record);
  __portRegister(port, record);
  return port;
}
function __portCommit(before, after) {
  for (let index = 0; index < before.length; index++) {
    const old = before[index], port = after[index];
    const record = __portEndpoint(old);
    __portNewHandle(port);
    __portRegistry.unregister(old);
    const previousResource = record.resource;
    record.resource = __portResource();
    record.generation++;
    record.started = false;
    __portRegister(port, record);
    __portCloseEvent(old, previousResource, true);
  }
}
function __portSchedule(record) {
  if (record.scheduled || !record.started || !record.queue.length) return;
  record.scheduled = true;
  __realTimers.setImmediate(() => __portDeliver(record.id));
}
function __portDeliver(id) {
  const record = __portMapGet(__portEndpoints, id);
  if (!record) return;
  record.scheduled = false;
  if (!record.started || !record.queue.length) return;
  const owner = __portOwner(record);
  if (!owner) { __portForget(id, record.generation); return; }
  const message = record.queue.shift();
  try { record.resource.runInAsyncScope(__portDispatch, owner, owner, 'message', message.data, message.ports); }
  finally { __portSchedule(record); }
}
function __portEvent(port, type, data, ports) {
  return { type, target: port, currentTarget: port, data, ports: ports || [], defaultPrevented: false,
    preventDefault() { this.defaultPrevented = true; } };
}
function __portCall(port, entry, type, data, event) {
  if (entry.once) __portRemove(port, type, entry.callback, entry.event);
  const value = entry.event ? event : data;
  if (typeof entry.callback === 'function') entry.callback.call(port, value);
  else entry.callback.handleEvent(event);
}
function __portDispatch(port, type, data, ports) {
  const handle = __portHandle(port);
  const event = __portEvent(port, type, data, ports);
  const listeners = __portMapGet(handle.listeners, type) || [];
  for (const entry of listeners.slice()) __portCall(port, entry, type, data, event);
  const handler = handle['on' + type];
  if (typeof handler === 'function') handler.call(port, event);
  return listeners.length > 0;
}
function __portStart(port) {
  __portHandle(port);
  if (globalThis.__ttPortDetached(port)) return;
  const record = __portEndpoint(port);
  record.started = true;
  __portSchedule(record);
}
function __portValidListener(callback, event) {
  if (callback == null && event) return false;
  if (typeof callback === 'function') return true;
  if (event && typeof callback.handleEvent === 'function') return true;
  throw new TypeError('Listener must be a function');
}
function __portActivate(port, type, event) {
  if (type !== 'message') return;
  __portHandle(port).refed = true;
  if (!event) __portStart(port);
}
function __portListen(port, type, callback, event, once) {
  const handle = __portHandle(port);
  if (!__portValidListener(callback, event)) return port;
  let listeners = __portMapGet(handle.listeners, type);
  if (!listeners) { listeners = []; __portMapSet(handle.listeners, type, listeners); }
  if (event && listeners.some((entry) => entry.callback === callback && entry.event)) return port;
  listeners.push({ callback, event, once });
  __portActivate(port, type, event);
  return port;
}
function __portRemove(port, type, callback, event) {
  const handle = __portHandle(port);
  const listeners = __portMapGet(handle.listeners, type) || [];
  const index = listeners.findIndex((entry) => entry.callback === callback && entry.event === event);
  if (index >= 0) listeners.splice(index, 1);
  return port;
}
function __portTransferOptions(options) {
  if (options === undefined) return [];
  if (Array.isArray(options)) return options;
  if (options && typeof options === 'object') return options.transfer || [];
  throw new TypeError('Transfer options must be an array or object');
}
function __portPost(port, value, options) {
  __portHandle(port);
  const transfer = Array.from(__portTransferOptions(options));
  if (transfer.includes(port)) throw new DOMException('Transfer list contains source port', 'DataCloneError');
  const ports = [];
  const data = globalThis.__ttStructuredClone(value, transfer, ports);
  if (globalThis.__ttPortDetached(port)) return;
  const record = __portEndpoint(port);
  const peer = __portMapGet(__portEndpoints, record.peer);
  if (!peer) return;
  peer.queue.push({ data, ports });
  __portSchedule(peer);
}
function __portClose(port) {
  __portHandle(port);
  if (globalThis.__ttPortDetached(port)) return;
  const record = __portEndpoint(port);
  globalThis.__ttPortDetach(port);
  __portRegistry.unregister(port);
  __portForget(record.id, record.generation, true);
  __portCloseEvent(port, record.resource, true);
}
function __portOnmessage(port, type, callback) {
  __portHandle(port)['on' + type] = callback;
  if (typeof callback === 'function' && type === 'message') { __portHandle(port).refed = true; __portStart(port); }
}
class __NodeMessagePort {
  constructor() { throw new TypeError('Illegal constructor'); }
  postMessage(value, options) { __portPost(this, value, options); }
  start() { __portStart(this); }
  close(callback) { if (callback) this.once('close', callback); __portClose(this); }
  ref() { __portHandle(this).refed = true; return this; }
  unref() { __portHandle(this).refed = false; return this; }
  hasRef() { return !globalThis.__ttPortDetached(this) && __portHandle(this).refed; }
  on(type, callback) { return __portListen(this, type, callback, false, false); }
  addListener(type, callback) { return this.on(type, callback); }
  once(type, callback) { return __portListen(this, type, callback, false, true); }
  off(type, callback) { return __portRemove(this, type, callback, false); }
  removeListener(type, callback) { return this.off(type, callback); }
  removeAllListeners(type) {
    const handle = __portHandle(this);
    if (type === undefined) handle.listeners = new Map();
    else __portMapDelete(handle.listeners, type);
    return this;
  }
  listeners(type) { return (__portMapGet(__portHandle(this).listeners, type) || []).filter((entry) => !entry.event).map((entry) => entry.callback); }
  listenerCount(type) { return this.listeners(type).length; }
  addEventListener(type, callback, options) { __portListen(this, type, callback, true, Boolean(options && options.once)); }
  removeEventListener(type, callback) { __portRemove(this, type, callback, true); }
  get onmessage() { return __portHandle(this).onmessage; }
  set onmessage(callback) { __portOnmessage(this, 'message', callback); }
  get onmessageerror() { return __portHandle(this).onmessageerror; }
  set onmessageerror(callback) { __portOnmessage(this, 'messageerror', callback); }
}
class __NodeMessageChannel {
  constructor() {
    const one = __portSequence++, two = __portSequence++;
    this.port1 = __portCreate(one, two);
    this.port2 = __portCreate(two, one);
  }
}
if (typeof globalThis.MessageChannel === 'undefined') {
  globalThis.MessageChannel = __NodeMessageChannel;
  globalThis.MessagePort = __NodeMessagePort;
}
