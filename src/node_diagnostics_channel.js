// ---- node:diagnostics_channel (Node 26; turbo-test#331, #407) ----
// Channels, bound stores (bindStore/runStores) and tracing channels (traceSync/tracePromise/
// traceCallback), as Node's lib/diagnostics_channel.js. A throwing subscriber or store transform
// never breaks the publisher: Node re-raises it as an uncaught exception on the next tick.
// Strict mode as Node's internals: a frozen traceCallback default context throws on write.
const __dc = (() => {
  'use strict';
  const channels = new Map();
  const traceEvents = ['start', 'end', 'asyncStart', 'asyncEnd', 'error'];
  const emptyContext = Object.freeze({ __proto__: null });
  function received(value) {
    if (value === undefined || value === null) return ' Received ' + value;
    return ' Received type ' + typeof value + ' (' + String(value) + ')';
  }
  function argType(message, value) {
    return Object.assign(new TypeError(message + received(value)), { code: 'ERR_INVALID_ARG_TYPE' });
  }
  function validateFunction(value, name) {
    if (typeof value !== 'function') throw argType('The "' + name + '" argument must be of type function.', value);
  }
  function uncaught(err) {
    process.nextTick(() => { throw err; });
  }
  function deliver(subscriber, data, name) {
    try { subscriber(data, name); } catch (err) { uncaught(err); }
  }
  function storeContext(transform, data) {
    if (!transform) return { value: data };
    try { return { value: transform(data) }; } catch (err) { uncaught(err); return undefined; }
  }
  // Stores are entered in bind order (the first bound is outermost), then the data is published.
  function enterStores(entries, index, data, last) {
    if (index === entries.length) return last();
    const next = () => enterStores(entries, index + 1, data, last);
    const context = storeContext(entries[index][1], data);
    return context ? entries[index][0].run(context.value, next) : next();
  }
  class Channel {
    constructor(name) { this._subscribers = []; this._stores = new Map(); this.name = name; }
    get hasSubscribers() { return this._subscribers.length > 0 || this._stores.size > 0; }
    subscribe(subscription) {
      validateFunction(subscription, 'subscription');
      this._subscribers = this._subscribers.concat([subscription]);
    }
    unsubscribe(subscription) {
      const index = this._subscribers.indexOf(subscription);
      if (index < 0) return false;
      this._subscribers = this._subscribers.filter((_, i) => i !== index);
      return true;
    }
    bindStore(store, transform) { this._stores.set(store, transform); }
    unbindStore(store) { return this._stores.delete(store); }
    publish(data) {
      for (const subscriber of this._subscribers) deliver(subscriber, data, this.name);
    }
    runStores(data, fn, thisArg, ...args) {
      return enterStores(Array.from(this._stores), 0, data, () => {
        this.publish(data);
        return Reflect.apply(fn, thisArg, args);
      });
    }
  }
  function channel(name) {
    if (typeof name !== 'string' && typeof name !== 'symbol') throw argType('The "channel" argument must be of type string or symbol.', name);
    if (!channels.has(name)) channels.set(name, new Channel(name));
    return channels.get(name);
  }
  function tracingSource(nameOrChannels) {
    if (typeof nameOrChannels === 'string' || (typeof nameOrChannels === 'object' && nameOrChannels !== null)) return nameOrChannels;
    throw argType('The "nameOrChannels" argument must be of type string or an instance of TracingChannel or Object.', nameOrChannels);
  }
  function tracingMember(source, event) {
    if (typeof source === 'string') return channel('tracing:' + source + ':' + event);
    const member = source[event];
    if (!(member instanceof Channel)) throw argType('The "nameOrChannels.' + event + '" property must be an instance of Channel.', member);
    return member;
  }
  // A start/end window: enter start's stores, publish start, run, publish end, leave the stores.
  function traceWindow(start, end, context, body) {
    if (!start.hasSubscribers && !end.hasSubscribers) return body();
    return start.runStores(context, () => {
      try { return body(); } finally { end.publish(context); }
    });
  }
  function traceCall(tc, context, call, settle) {
    try { return settle(call()); } catch (err) {
      context.error = err;
      tc.error.publish(context);
      throw err;
    }
  }
  function thenable(value) {
    return value !== null && value !== undefined && typeof value.then === 'function';
  }
  function nonThenable(fn) {
    process.emitWarning("tracePromise was called with the function '" + (fn.name || '<anonymous>') + "', which returned a non-thenable.");
  }
  function settleAsync(tc, context) {
    traceWindow(tc.asyncStart, tc.asyncEnd, context, () => undefined);
  }
  function promiseHandlers(tc, context) {
    const onResolve = (value) => { context.result = value; settleAsync(tc, context); return value; };
    const onReject = (err) => { context.error = err; tc.error.publish(context); settleAsync(tc, context); };
    return { onResolve, onReject };
  }
  function settlePromise(tc, context, fn, result) {
    if (!thenable(result)) {
      nonThenable(fn);
      context.result = result;
      return result;
    }
    const { onResolve, onReject } = promiseHandlers(tc, context);
    if (result instanceof Promise && Object.getPrototypeOf(result) === Promise.prototype) {
      return result.then(onResolve, (err) => { onReject(err); throw err; });
    }
    // A non-native thenable is observed but returned as-is (no unhandled-rejection tracking).
    result.then(onResolve, onReject);
    return result;
  }
  function wrapCallback(tc, context, callback) {
    return function wrappedCallback(err, res) {
      if (err) { context.error = err; tc.error.publish(context); } else context.result = res;
      const args = arguments;
      return traceWindow(tc.asyncStart, tc.asyncEnd, context, () => Reflect.apply(callback, this, args));
    };
  }
  function untracedPromise(fn, thisArg, args) {
    const result = Reflect.apply(fn, thisArg, args);
    if (!thenable(result)) nonThenable(fn);
    return result;
  }
  const same = (value) => value;
  class TracingChannel {
    #start; #end; #asyncStart; #asyncEnd;
    constructor(nameOrChannels) {
      const source = tracingSource(nameOrChannels);
      this.#start = tracingMember(source, 'start');
      this.#end = tracingMember(source, 'end');
      this.#asyncStart = tracingMember(source, 'asyncStart');
      this.#asyncEnd = tracingMember(source, 'asyncEnd');
      Object.defineProperty(this, 'error', { value: tracingMember(source, 'error') });
    }
    get start() { return this.#start; }
    get end() { return this.#end; }
    get asyncStart() { return this.#asyncStart; }
    get asyncEnd() { return this.#asyncEnd; }
    get hasSubscribers() { return traceEvents.some((event) => this[event].hasSubscribers); }
    subscribe(handlers) {
      for (const event of traceEvents) if (handlers[event]) this[event].subscribe(handlers[event]);
    }
    unsubscribe(handlers) {
      let done = true;
      for (const event of traceEvents) if (handlers[event] && !this[event].unsubscribe(handlers[event])) done = false;
      return done;
    }
    traceSync(fn, context = { __proto__: null }, thisArg, ...args) {
      if (!this.hasSubscribers) return Reflect.apply(fn, thisArg, args);
      const settle = (result) => { context.result = result; return result; };
      return traceWindow(this.#start, this.#end, context, () => traceCall(this, context, () => Reflect.apply(fn, thisArg, args), settle));
    }
    tracePromise(fn, context = { __proto__: null }, thisArg, ...args) {
      if (!this.hasSubscribers) return untracedPromise(fn, thisArg, args);
      const settle = (result) => settlePromise(this, context, fn, result);
      return traceWindow(this.#start, this.#end, context, () => traceCall(this, context, () => Reflect.apply(fn, thisArg, args), settle));
    }
    traceCallback(fn, position = -1, context = emptyContext, thisArg, ...args) {
      if (!this.hasSubscribers) return Reflect.apply(fn, thisArg, args);
      const callback = args.at(position);
      validateFunction(callback, 'callback');
      args.splice(position, 1, wrapCallback(this, context, callback));
      return traceWindow(this.#start, this.#end, context, () => traceCall(this, context, () => Reflect.apply(fn, thisArg, args), same));
    }
  }
  const dc = {
    Channel,
    channel,
    hasSubscribers: (name) => channel(name).hasSubscribers,
    subscribe: (name, fn) => channel(name).subscribe(fn),
    unsubscribe: (name, fn) => channel(name).unsubscribe(fn),
    tracingChannel: (nameOrChannels) => new TracingChannel(nameOrChannels),
  };
  dc.default = dc;
  return dc;
})();
