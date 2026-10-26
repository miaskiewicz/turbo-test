// Minimal test runtime injected before a test module loads.
// PLACEHOLDER for the real @vitest/* framework, which gets baked into the V8 snapshot
// at M3 (compatible-by-construction). This M1 version exists only to run the
// non-timer/non-DOM logic subset of the gauntlet end-to-end and report pass/fail.

// ---- console shim (V8 contexts have none; Node/browser provide it) ----
// `--silent` (host → globalThis.__TT_SILENT): suppress test console output entirely. Checked at
// call time, not eval time — the host injects __TT_SILENT AFTER this module is snapshot-evaluated.
const __noop = () => {};
// The arguments are formatted as Node's util.format (turbo-test#93).
const __emit = (tag) => (...a) => { if (!globalThis.__TT_SILENT) globalThis.log(tag, __nodeFormat(...a)); };
globalThis.console = {
  log: __emit('[log]'),
  info: __emit('[info]'),
  warn: __emit('[warn]'),
  error: __emit('[error]'),
  debug: __noop,
  trace: __noop,
  group: __noop,
  groupEnd: __noop,
  table: __noop,
  dir: __noop,
  assert: __noop,
  count: __noop,
  time: __noop,
  timeEnd: __noop,
};

// ---- process shim (no Node in our embedding) ----
// platform/arch come from the host binary (Rust injects __ttPlatform/__ttArch/__ttGlibc
// from its compile-time target). They MUST be the real host values: turbo-dom's index.js
// loader switches on process.platform/arch to require the matching prebuilt .node, and
// its isMusl() reads process.report().header.glibcVersionRuntime to pick gnu vs musl.
// Hardcoding 'darwin'/'arm64' made every non-mac host load the macOS .node -> dlopen fails.
globalThis.process = {
  env: { NODE_ENV: 'test', VITEST: 'true', TZ: 'UTC', TURBO_DOM_PARSER: 'native' },
  platform: globalThis.__ttPlatform || 'darwin',
  arch: globalThis.__ttArch || 'arm64',
  // glibcVersionRuntime truthy => glibc (gnu); falsy => musl. __ttGlibc is the glibc
  // version for a gnu build, '' for a musl build.
  report: { getReport: () => ({ header: { glibcVersionRuntime: globalThis.__ttGlibc || undefined } }) },
  version: 'v24.0.0',
  // napi: the Node-API version the native host implements (src/napi_host.rs NAPI_VERSION)
  versions: { node: '24.0.0', v8: '12.0.0', modules: '127', napi: '10' },
  argv: ['node', 'turbo-test'],
  cwd: () => globalThis.__ttProcessCwd || globalThis.__cwd || '/',
  nextTick: (fn, ...a) => __loop.nextTicks.push(() => fn(...a)),
  hrtime: Object.assign(() => [0, 0], { bigint: () => 0n }),
  exit: () => {},
  on: () => globalThis.process,
  off: () => globalThis.process,
  once: () => globalThis.process,
  emit: () => false,
  stdout: { write: (...a) => __streamWrite(1, a), isTTY: false },
  stderr: { write: (...a) => __streamWrite(2, a), isTTY: false },
};
// process.stdout/stderr.write (turbo-test#70): the text (a string, or bytes decoded as UTF-8) goes
// to the process's stream as is (native `__ttWrite`); `--silent` drops it like console output. A
// trailing callback runs once written, asynchronously, as Node's does. Always returns true.
function __streamWrite(fd, args) {
  const chunk = args[0];
  const cb = typeof args[args.length - 1] === 'function' ? args[args.length - 1] : null;
  const text = typeof chunk === 'string' ? chunk : (chunk instanceof Uint8Array ? new TextDecoder().decode(chunk) : String(chunk));
  if (!globalThis.__TT_SILENT) globalThis.__ttWrite(fd, text);
  if (cb) Promise.resolve().then(() => cb());
  return true;
}

// ===========================================================================
// Event loop + timers (M2). Native macro/micro/nextTick ordering.
//   - microtasks: native V8 Promise jobs, drained by Rust (perform_microtask_checkpoint)
//   - process.nextTick: own queue, drained BEFORE microtasks each turn (Node priority)
//   - macrotasks (setTimeout/Interval/Immediate): virtual-clock queue
// Real mode: Rust drive loop interleaves the three (no actual sleeping; the clock jumps
// to the next due timer — deterministic, order-preserving). Fake mode: vi.* drives the
// clock synchronously from test code. Foundation for binding real @sinonjs/fake-timers.
// ===========================================================================
function __reportUncaught(e) {
  try { globalThis.console.error('uncaught', (e && e.message) || e); } catch {}
}
// the native host (napi_fatal_exception, an exception left by an async native callback)
// reports through the same path
globalThis.__reportUncaught = __reportUncaught;
const __loop = {
  macro: [],       // {id, due, cb, interval, gen}
  // Internal one-shot timers (test-timeout enforcement). Kept SEPARATE from `macro` so they are
  // invisible to the fake-timer API (getTimerCount / runAllTimers / advance / clearAllTimers) —
  // otherwise a test's `vi.runAllTimers()` would fire the timeout reject. Driven only by the
  // real-mode drive loop (stepMacro) so a genuinely hung async test still times out.
  internal: [],    // {id, due, cb, gen}
  nextTicks: [],
  seq: 1,
  now: 0,          // logical clock (ms)
  gen: 0,          // file generation (isolate-reuse): timers from a prior file are dropped, unrun
  fake: false,
  systemTime: 0,   // wall-clock base used by Date when faking
  RealDate: Date,
  realNow: Date.now.bind(Date),
};
function __schedule(cb, delay, interval) {
  const id = __loop.seq++;
  __loop.macro.push({ id, due: __loop.now + Math.max(0, delay | 0), cb, interval, gen: __loop.gen });
  return id;
}
// Isolate-reuse: drop timers left over from a PRIOR file (gen < current) WITHOUT running them.
// A finished file's leaked setInterval / self-rescheduling setTimeout would otherwise fire in the
// next file and re-arm itself → the loop never settles → the 2M guard spins (run crawls/hangs).
// Dropping at fire-time (not at reset) lets the CURRENT file keep its own pending one-shots
// (clearing those broke ~40 async tests).
function __dropStaleTimers() {
  if (__loop.macro.length && __loop.macro.some((t) => t.gen < __loop.gen)) {
    __loop.macro = __loop.macro.filter((t) => t.gen >= __loop.gen);
  }
  if (__loop.internal.length && __loop.internal.some((t) => t.gen < __loop.gen)) {
    __loop.internal = __loop.internal.filter((t) => t.gen >= __loop.gen);
  }
}
function __clearTimer(id) { __loop.macro = __loop.macro.filter((t) => t.id !== id); }
// Internal one-shot timer (test-timeout). Separate queue → invisible to the fake-timer API.
function __scheduleInternal(cb, delay) {
  const id = __loop.seq++;
  __loop.internal.push({ id, due: __loop.now + Math.max(0, delay | 0), cb, gen: __loop.gen });
  return id;
}
function __clearInternal(id) { __loop.internal = __loop.internal.filter((t) => t.id !== id); }
function __dueSorted(upto) {
  return __loop.macro.filter((t) => t.due <= upto).sort((a, b) => a.due - b.due || a.id - b.id);
}
__loop.drainNextTicks = function () {
  let ran = false;
  while (__loop.nextTicks.length) {
    const q = __loop.nextTicks; __loop.nextTicks = [];
    for (const f of q) { ran = true; try { f(); } catch (e) { __reportUncaught(e); } }
  }
  return ran;
};
// Real mode: run the single earliest-due macrotask (Rust drains micro/nextTick around it).
// Considers BOTH the user macro queue and the internal test-timeout queue, firing whichever is
// due first — so a hung test (no user timers) still advances to its timeout and rejects.
__loop.stepMacro = function () {
  __dropStaleTimers();
  __loop.macro.sort((a, b) => a.due - b.due || a.id - b.id);
  __loop.internal.sort((a, b) => a.due - b.due || a.id - b.id);
  const m = __loop.macro[0];
  const it = __loop.internal[0];
  if (!m && !it) return false;
  // earliest-due wins; ties favor the user timer (matches scheduling order intent).
  const useInternal = it && (!m || it.due < m.due);
  if (useInternal) {
    __loop.now = Math.max(__loop.now, it.due);
    __clearInternal(it.id);
    try { it.cb(); } catch (e) { __reportUncaught(e); }
    return true;
  }
  __loop.now = Math.max(__loop.now, m.due);
  if (m.interval == null) __clearTimer(m.id);
  else m.due = __loop.now + m.interval;
  try { m.cb(); } catch (e) { __reportUncaught(e); }
  return true;
};
// Fake mode: advance the virtual clock, firing timers in order (synchronous).
__loop.advance = function (ms) {
  __dropStaleTimers();
  const target = __loop.now + Math.max(0, ms | 0);
  let guard = 0;
  while (guard++ < 1e6) {
    const next = __dueSorted(target)[0];
    if (!next) break;
    __loop.now = next.due;
    if (next.interval == null) __clearTimer(next.id);
    else next.due = __loop.now + next.interval;
    try { next.cb(); } catch (e) { __reportUncaught(e); }
    __loop.drainNextTicks();
  }
  __loop.now = target;
};
__loop.runAll = function () {
  let guard = 0;
  while (__loop.macro.length && guard++ < 1e6) {
    __loop.macro.sort((a, b) => a.due - b.due || a.id - b.id);
    const t = __loop.macro[0];
    __loop.now = Math.max(__loop.now, t.due);
    if (t.interval == null) __clearTimer(t.id);
    else t.due = __loop.now + t.interval; // intervals would loop forever; guard caps it
    try { t.cb(); } catch (e) { __reportUncaught(e); }
    __loop.drainNextTicks();
    if (t.interval != null) break; // don't spin a periodic timer in runAll
  }
};
__loop.runOnlyPending = function () {
  const snapshot = __loop.macro.slice().sort((a, b) => a.due - b.due || a.id - b.id);
  for (const t of snapshot) {
    if (!__loop.macro.includes(t)) continue;
    __loop.now = Math.max(__loop.now, t.due);
    if (t.interval == null) __clearTimer(t.id);
    else t.due = __loop.now + t.interval;
    try { t.cb(); } catch (e) { __reportUncaught(e); }
    __loop.drainNextTicks();
  }
};

// timer globals (bare V8 has none)
globalThis.setTimeout = (cb, d, ...a) => __schedule(() => cb(...a), d, null);
globalThis.setInterval = (cb, d, ...a) => __schedule(() => cb(...a), d, Math.max(1, d | 0));
globalThis.setImmediate = (cb, ...a) => __schedule(() => cb(...a), 0, null);
globalThis.clearTimeout = (id) => __clearTimer(id);
globalThis.clearInterval = (id) => __clearTimer(id);
globalThis.clearImmediate = (id) => __clearTimer(id);
if (typeof globalThis.queueMicrotask !== 'function') {
  globalThis.queueMicrotask = (cb) => Promise.resolve().then(cb);
}

// Date faking
function __installFakeDate() {
  const Real = __loop.RealDate;
  function FakeDate(...args) {
    if (!(this instanceof FakeDate)) return new Real(__loop.systemTime + __loop.now).toString();
    if (args.length === 0) return new Real(__loop.systemTime + __loop.now);
    return new Real(...args);
  }
  FakeDate.prototype = Real.prototype;
  FakeDate.now = () => __loop.systemTime + __loop.now;
  FakeDate.UTC = Real.UTC;
  FakeDate.parse = Real.parse;
  globalThis.Date = FakeDate;
}
// Default clock: a Date whose now() is a REAL wall-clock base + the VIRTUAL elapsed (__loop.now,
// which advances as the loop fires timers). So `const t=Date.now(); await sleep(100); Date.now()-t`
// reports ~100 even though no real time passed — elapsed-across-setTimeout assertions work, while
// timestamps stay ~real (base captured at install). Only `now()` and arg-less `new Date()` are
// virtualized; `new Date(args)` and the prototype stay real (instanceof works).
let __virtualBase = 0;
function __installVirtualDate() {
  const Real = __loop.RealDate;
  if (!__virtualBase) __virtualBase = __loop.realNow();
  function VDate(...args) {
    if (!(this instanceof VDate)) return new Real(__virtualBase + __loop.now).toString();
    if (args.length === 0) return new Real(__virtualBase + __loop.now);
    return new Real(...args);
  }
  VDate.prototype = Real.prototype;
  VDate.now = () => __virtualBase + __loop.now;
  VDate.UTC = Real.UTC;
  VDate.parse = Real.parse;
  globalThis.Date = VDate;
}
function __restoreDate() { __installVirtualDate(); }
__installVirtualDate();

// A vi.mock/doMock factory that threw (or rejected): the module is registered as this stand-in,
// and importing it throws the factory's error (vitest wraps it, with the cause; jest rethrows it
// as is) instead of silently falling back to the real module. The native loader asks
// __ttMockFactoryError(exports) on each import of a mocked path; any other access to the
// stand-in throws the same error.
const __ttMockErrors = new WeakMap();
function __ttFactoryErrorMock(e, vitest) {
  const err = vitest
    ? new Error('[vitest] There was an error when mocking a module. If you are using "vi.mock" factory, '
      + 'make sure there are no top level variables inside, since this call is hoisted to top of the file. '
      + 'Read more: https://vitest.dev/api/vi.html#vi-mock Caused by: ' + String(e), { cause: e })
    : e;
  const stand = new Proxy({}, { get: (t, k) => { if (typeof k === 'symbol' || k === 'then') return undefined; throw err; } });
  __ttMockErrors.set(stand, err);
  return stand;
}
globalThis.__ttFactoryErrorMock = __ttFactoryErrorMock;
globalThis.__ttMockFactoryError = (m) => __ttMockErrors.get(m);

// await any async mock-factory results before the native loader drains them; a rejected factory
// registers the error stand-in
globalThis.__resolvePendingMocks = async () => {
  const list = globalThis.__pendingMocks || [];
  for (const m of list) {
    if (m.exports && typeof m.exports.then === 'function') {
      try { m.exports = __ttStrictMock(await m.exports, m.key, m.spec); }
      catch (e) { m.exports = __ttFactoryErrorMock(e, m.vitest !== false); }
    }
  }
};

// hooks for the Rust drive loop
globalThis.__drainNextTicks = () => __loop.drainNextTicks();
// Run one due macrotask (used while pumping the loop to resolve async vi.mock factories that
// `await import(...)` — dynamic imports can resolve across macrotask turns, not just microtasks).
globalThis.__stepMacro = () => __loop.stepMacro();
globalThis.__hasNextTicks = () => __loop.nextTicks.length > 0;
globalThis.__stepMacro = () => __loop.stepMacro();
// ms until the earliest macrotask (user or internal) is due, -1 when none is queued: how long
// the Rust drive loop waits for native addon work before firing it
globalThis.__ttNextMacroDelay = () => {
  let due = Infinity;
  for (const t of __loop.macro) due = Math.min(due, t.due);
  for (const t of __loop.internal) due = Math.min(due, t.due);
  return due === Infinity ? -1 : Math.max(0, due - __loop.now);
};

// ---- minimal Web Storage. A real `Storage` class (not an object literal) so the methods live on
// `Storage.prototype` — tests do `vi.spyOn(Storage.prototype, 'setItem')`, which only intercepts
// localStorage/sessionStorage calls when those are genuine Storage instances. ----
class __Storage {
  constructor() { this._m = new Map(); }
  getItem(k) { return this._m.has(String(k)) ? this._m.get(String(k)) : null; }
  setItem(k, v) { this._m.set(String(k), String(v)); }
  removeItem(k) { this._m.delete(String(k)); }
  clear() { this._m.clear(); }
  key(i) { return Array.from(this._m.keys())[i] ?? null; }
  get length() { return this._m.size; }
}
globalThis.Storage = __Storage;
globalThis.localStorage = new __Storage();
globalThis.sessionStorage = new __Storage();

// ---- TextEncoder/TextDecoder (host globals; bare V8 has none) ----
if (typeof globalThis.TextEncoder === 'undefined') {
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(s) {
      s = String(s); const b = [];
      for (let i = 0; i < s.length; i++) {
        let c = s.charCodeAt(i);
        if (c < 0x80) b.push(c);
        else if (c < 0x800) b.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
        else if (c >= 0xd800 && c < 0xdc00) {
          const c2 = s.charCodeAt(++i);
          c = 0x10000 + ((c & 0x3ff) << 10) + (c2 & 0x3ff);
          b.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 0x3f), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
        } else b.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
      }
      return new Uint8Array(b);
    }
  };
}
if (typeof globalThis.TextDecoder === 'undefined') {
  globalThis.TextDecoder = class TextDecoder {
    constructor(enc) { this.encoding = enc || 'utf-8'; }
    decode(buf) {
      if (!buf) return '';
      const b = buf instanceof Uint8Array ? buf : new Uint8Array(buf.buffer || buf);
      let s = '';
      for (let i = 0; i < b.length;) {
        let c = b[i++];
        if (c < 0x80) s += String.fromCharCode(c);
        else if (c < 0xe0) s += String.fromCharCode(((c & 0x1f) << 6) | (b[i++] & 0x3f));
        else if (c < 0xf0) s += String.fromCharCode(((c & 0xf) << 12) | ((b[i++] & 0x3f) << 6) | (b[i++] & 0x3f));
        else {
          let cp = ((c & 0x7) << 18) | ((b[i++] & 0x3f) << 12) | ((b[i++] & 0x3f) << 6) | (b[i++] & 0x3f);
          cp -= 0x10000;
          s += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
        }
      }
      return s;
    }
  };
}

// ---- Web platform globals (bare V8 lacks these) ----
if (typeof globalThis.URLSearchParams === 'undefined') {
  globalThis.URLSearchParams = class URLSearchParams {
    constructor(init) {
      this._ = []; // list of [key, value] pairs — supports multiple values per key (append)
      if (typeof init === 'string') {
        // application/x-www-form-urlencoded: '+' decodes to a space (decodeURIComponent alone leaves
        // it). Matches Node/jsdom so `new URLSearchParams('a=John+Doe').get('a') === 'John Doe'`.
        const dec = (s) => { try { return decodeURIComponent(s.replace(/\+/g, ' ')); } catch { return s; } };
        init.replace(/^\?/, '').split('&').filter(Boolean).forEach((p) => {
          const i = p.indexOf('='); const k = i < 0 ? p : p.slice(0, i); const v = i < 0 ? '' : p.slice(i + 1);
          this._.push([dec(k), dec(v)]);
        });
      } else if (Array.isArray(init)) {
        init.forEach(([k, v]) => this._.push([String(k), String(v)]));
      } else if (init && typeof init.forEach === 'function') {
        init.forEach((v, k) => this._.push([String(k), String(v)]));
      } else if (init && typeof init === 'object') {
        for (const k of Object.keys(init)) this._.push([k, String(init[k])]);
      }
    }
    get(k) { const e = this._.find((p) => p[0] === k); return e ? e[1] : null; }
    getAll(k) { return this._.filter((p) => p[0] === k).map((p) => p[1]); }
    set(k, v) { this._ = this._.filter((p) => p[0] !== k); this._.push([k, String(v)]); }
    append(k, v) { this._.push([k, String(v)]); }
    has(k) { return this._.some((p) => p[0] === k); }
    delete(k) { this._ = this._.filter((p) => p[0] !== k); }
    forEach(f) { this._.slice().forEach(([k, v]) => f(v, k, this)); }
    keys() { return this._.map((p) => p[0])[Symbol.iterator](); }
    values() { return this._.map((p) => p[1])[Symbol.iterator](); }
    entries() { return this._.map((p) => [p[0], p[1]])[Symbol.iterator](); }
    sort() { this._.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); }
    // form-urlencoded serialization: encode, then render space as '+' (Node/jsdom do; turbo-test
    // previously emitted %20, diverging on `name=John+Doe`).
    toString() { const enc = (s) => encodeURIComponent(s).replace(/%20/g, '+'); return this._.map(([k, v]) => `${enc(k)}=${enc(v)}`).join('&'); }
    [Symbol.iterator]() { return this.entries(); }
  };
}
if (typeof globalThis.URL === 'undefined') {
  globalThis.URL = class URL {
    constructor(url, base) {
      // leading/trailing C0 controls and spaces go, tabs and newlines anywhere (WHATWG)
      url = String(url).replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, '').replace(/[\t\n\r]/g, '');
      if (base !== undefined && !/^[a-z][a-z0-9+.-]*:/i.test(url)) {
        url = URL._resolve(url, new globalThis.URL(String(base)));
      }
      // Real URL throws on input without a valid scheme (e.g. "not a url"). Matching this is
      // required for validators built on `new URL()` (e.g. zod's .url()).
      if (!/^[a-z][a-z0-9+.-]*:/i.test(url)) {
        throw new TypeError('Invalid URL: ' + url);
      }
      // protocol is the scheme up to the first ':' (works for opaque schemes like
      // javascript:/data:/mailto: that have no '//' authority).
      const scheme = /^([a-z][a-z0-9+.-]*:)/i.exec(url);
      this.protocol = scheme ? scheme[1] : 'http:';
      const m = /^([a-z][a-z0-9+.-]*:)\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(url);
      let searchStr;
      if (m) {
        this._hierarchical = true;
        // a file URL's `localhost` host is the empty host (WHATWG)
        this.host = this.protocol.toLowerCase() === 'file:' && /^localhost$/i.test(m[2]) ? '' : m[2] || '';
        this.hostname = this.host.split(':')[0];
        this.port = (m[2] || '').split(':')[1] || '';
        this.pathname = m[3] || '/';
        searchStr = m[4] || '';
        this.hash = m[5] || '';
        this.origin = this.protocol.toLowerCase() === 'file:' ? 'null' : this.protocol + '//' + this.host;
      } else {
        // opaque URL (no authority): everything after the scheme is the path (sans query/hash).
        this._hierarchical = false;
        this.host = ''; this.hostname = ''; this.port = '';
        const rest = url.slice(this.protocol.length);
        const qi = rest.indexOf('?'); const hi = rest.indexOf('#');
        let cut = rest.length;
        if (qi >= 0) cut = Math.min(cut, qi);
        if (hi >= 0) cut = Math.min(cut, hi);
        this._path = URL._encode(rest.slice(0, cut), URL._C0);
        const after = rest.slice(cut);
        const am = /^(\?[^#]*)?(#.*)?$/.exec(after) || [];
        searchStr = am[1] || ''; this.hash = am[2] || ''; this.origin = 'null';
      }
      // searchParams is the SOURCE OF TRUTH for the query; `search`/`href`/`toString` derive from it
      // so post-construction `searchParams.set()/append()` mutations are reflected (real URL semantics
      // — previously href was frozen at construction and dropped them).
      this._sp = new globalThis.URLSearchParams(searchStr);
    }
    // The path and fragment keep the WHATWG percent-encode sets (turbo-test#85): a hierarchical
    // path is dot-normalised and encoded on every set, as Node's URL does.
    get pathname() { return this._path; }
    set pathname(v) { this._path = this._hierarchical ? URL._encode(URL._removeDots(String(v).replace(/^(?!\/)/, '/')), URL._PATH) : this._path; }
    get hash() { return this._hash === '#' ? '' : this._hash; }
    set hash(v) { v = String(v); this._hash = v === '' ? '' : '#' + URL._encode(v.replace(/^#/, ''), URL._FRAGMENT); }
    // Percent-encode the code points of `s` in `set` (and every non-ASCII one, as UTF-8); an
    // existing `%XX` stays as written.
    static _encode(s, set) {
      let out = '';
      for (const ch of s) {
        const c = ch.codePointAt(0);
        if (c > 0x7e) { let e; try { e = encodeURIComponent(ch); } catch { e = '%EF%BF%BD'; } out += e; }
        else if (c < 0x20 || set.includes(ch)) out += '%' + c.toString(16).toUpperCase().padStart(2, '0');
        else out += ch;
      }
      return out;
    }
    get searchParams() { return this._sp; }
    get search() { const s = this._sp.toString(); return s ? '?' + s : ''; }
    set search(v) { this._sp = new globalThis.URLSearchParams(String(v).replace(/^\?/, '')); }
    get href() {
      const auth = this._hierarchical ? this.protocol + '//' + this.host : this.protocol;
      return auth + this._path + this.search + this._hash;
    }
    set href(v) { const u = new globalThis.URL(v); this.protocol = u.protocol; this.host = u.host; this.hostname = u.hostname; this.port = u.port; this._path = u._path; this._hash = u._hash; this.origin = u.origin; this._hierarchical = u._hierarchical; this._sp = u._sp; }
    toString() { return this.href; }
    // WHATWG relative-reference resolution against a parsed base (Node's `new URL(rel, base)`):
    // scheme-relative `//h/p`, absolute path `/p`, query-only `?q`, fragment-only `#f`, empty,
    // and a relative path merged with the base path's directory. Dot segments are removed by
    // the caller's pathname normalization. An opaque base (`mailto:x`) only takes a fragment.
    static _resolve(rel, b) {
      const auth = b.protocol + '//' + b.host;
      const noHash = b.href.replace(/#.*$/, '');
      if (!b._hierarchical) {
        if (rel.startsWith('#')) return noHash + rel;
        throw new TypeError('Invalid URL: ' + rel);
      }
      if (rel.startsWith('//')) return b.protocol + rel;
      if (rel.startsWith('/')) return auth + rel;
      if (rel === '') return noHash;
      if (rel.startsWith('#')) return noHash + rel;
      if (rel.startsWith('?')) return auth + b.pathname + rel;
      return auth + b.pathname.replace(/[^/]*$/, '') + rel;
    }
    // RFC 3986 / WHATWG dot-segment removal on a path (`/a/./b/../c` -> `/a/c`).
    static _removeDots(path) {
      const out = [];
      const segs = path.split('/');
      for (let i = 1; i < segs.length; i++) {
        const seg = segs[i];
        const last = i === segs.length - 1;
        if (seg === '..' || seg.toLowerCase() === '%2e%2e') {
          if (out.length) out.pop();
          if (last) out.push('');
        } else if (seg === '.' || seg.toLowerCase() === '%2e') {
          if (last) out.push('');
        } else {
          out.push(seg);
        }
      }
      return '/' + out.join('/');
    }
    static createObjectURL() { return 'blob:mock'; }
    static get _C0() { return '\x7f'; }
    static get _PATH() { return '\x7f "#<>?^`{}'; }
    static get _FRAGMENT() { return '\x7f "<>`'; }
    static revokeObjectURL() {}
    static parse(url, base) { try { return new globalThis.URL(url, base); } catch { return null; } }
    static canParse(url, base) { try { new globalThis.URL(url, base); return true; } catch { return false; } }
  };
}
if (globalThis.URL && typeof globalThis.URL.parse !== 'function') {
  globalThis.URL.parse = (url, base) => { try { return new globalThis.URL(url, base); } catch { return null; } };
  globalThis.URL.canParse = (url, base) => { try { new globalThis.URL(url, base); return true; } catch { return false; } };
}
if (typeof globalThis.DOMException === 'undefined') {
  globalThis.DOMException = class DOMException extends Error {
    constructor(message, name) { super(message); this.name = name || 'Error'; this.code = 0; }
  };
}
globalThis.__STATUS_TEXT = globalThis.__STATUS_TEXT || {
  200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 301: 'Moved Permanently',
  302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden',
  404: 'Not Found', 405: 'Method Not Allowed', 409: 'Conflict', 422: 'Unprocessable Entity',
  429: 'Too Many Requests', 500: 'Internal Server Error', 501: 'Not Implemented',
  502: 'Bad Gateway', 503: 'Service Unavailable', 504: 'Gateway Timeout',
};
if (typeof globalThis.AbortController === 'undefined') {
  globalThis.AbortSignal = class AbortSignal {
    constructor() { this.aborted = false; this.reason = undefined; this.onabort = null; this._listeners = []; }
    addEventListener(type, fn) { if (type === 'abort') this._listeners.push(fn); }
    removeEventListener(type, fn) { this._listeners = this._listeners.filter((f) => f !== fn); }
    dispatchEvent(ev) { if (ev && ev.type === 'abort') this._fire(); return true; }
    _fire() { const ev = { type: 'abort', target: this }; if (typeof this.onabort === 'function') this.onabort(ev); this._listeners.slice().forEach((f) => { try { f(ev); } catch (e) {} }); }
    throwIfAborted() { if (this.aborted) throw this.reason; }
    static abort(reason) { const s = new globalThis.AbortSignal(); s.aborted = true; s.reason = reason || new DOMException('Aborted', 'AbortError'); return s; }
    static timeout() { return new globalThis.AbortSignal(); }
  };
  globalThis.AbortController = class AbortController {
    constructor() { this.signal = new globalThis.AbortSignal(); }
    abort(reason) { if (!this.signal || this.signal.aborted) return; this.signal.aborted = true; this.signal.reason = reason || new DOMException('Aborted', 'AbortError'); this.signal._fire(); }
  };
}
if (typeof globalThis.Blob === 'undefined') {
  globalThis.Blob = class Blob {
    constructor(parts = [], opts = {}) {
      this._parts = parts || [];
      this.type = (opts && opts.type) || '';
      this.size = this._parts.reduce((n, p) => {
        if (p == null) return n;
        if (typeof p === 'string') return n + p.length;
        if (typeof p.byteLength === 'number') return n + p.byteLength; // ArrayBuffer / TypedArray
        if (typeof p.size === 'number') return n + p.size;             // Blob / File
        if (typeof p.length === 'number') return n + p.length;
        return n;
      }, 0);
    }
    text() {
      return Promise.resolve(this._parts.map((p) => {
        if (typeof p === 'string') return p;
        if (p instanceof ArrayBuffer) return new globalThis.TextDecoder().decode(new Uint8Array(p));
        if (p && p.buffer instanceof ArrayBuffer) return new globalThis.TextDecoder().decode(p); // TypedArray
        if (p && typeof p._parts !== 'undefined') return p._parts.join(''); // nested Blob
        return String(p);
      }).join(''));
    }
    arrayBuffer() { return Promise.resolve(new ArrayBuffer(this.size)); }
    slice() { return new globalThis.Blob(); }
    stream() { return null; }
  };
}
if (typeof globalThis.File === 'undefined') {
  globalThis.File = class File extends globalThis.Blob {
    constructor(parts, name, opts) { super(parts, opts); this.name = String(name); this.lastModified = (opts && opts.lastModified) || 0; }
  };
}
// Pure base64 (no btoa<->Buffer interdependency — turbo-dom's btoa calls Buffer.from, so
// Buffer.toString('base64') must NOT call back into btoa or it infinitely recurses).
const __B64CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
globalThis.__b64encodeBytes = (bytes) => {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i], b1 = i + 1 < bytes.length ? bytes[i + 1] : 0, b2 = i + 2 < bytes.length ? bytes[i + 2] : 0;
    out += __B64CHARS[b0 >> 2] + __B64CHARS[((b0 & 3) << 4) | (b1 >> 4)];
    out += i + 1 < bytes.length ? __B64CHARS[((b1 & 15) << 2) | (b2 >> 6)] : '=';
    out += i + 2 < bytes.length ? __B64CHARS[b2 & 63] : '=';
  }
  return out;
};
globalThis.__b64decodeToBytes = (str) => {
  str = String(str).replace(/[^A-Za-z0-9+/]/g, '');
  const bytes = [];
  for (let i = 0; i < str.length; i += 4) {
    const n = (__B64CHARS.indexOf(str[i]) << 18) | (__B64CHARS.indexOf(str[i + 1]) << 12) | ((str[i + 2] ? __B64CHARS.indexOf(str[i + 2]) : 0) << 6) | (str[i + 3] ? __B64CHARS.indexOf(str[i + 3]) : 0);
    bytes.push((n >> 16) & 255);
    if (i + 2 < str.length) bytes.push((n >> 8) & 255);
    if (i + 3 < str.length) bytes.push(n & 255);
  }
  return bytes;
};
if (typeof globalThis.btoa === 'undefined') {
  globalThis.btoa = (s) => { const str = String(s); const b = new Array(str.length); for (let i = 0; i < str.length; i++) b[i] = str.charCodeAt(i) & 255; return globalThis.__b64encodeBytes(b); };
  globalThis.atob = (s) => String.fromCharCode.apply(null, globalThis.__b64decodeToBytes(s));
}
if (typeof globalThis.FileReader === 'undefined') {
  globalThis.FileReader = class FileReader {
    constructor() { this.result = null; this.error = null; this.readyState = 0; this.onload = null; this.onerror = null; this.onloadend = null; this.onabort = null; }
    _emit(name, ev) { if (typeof this['on' + name] === 'function') this['on' + name](ev); }
    _read(blob, kind) {
      Promise.resolve().then(async () => {
        try {
          const text = blob && blob.text ? await blob.text() : '';
          if (kind === 'data') { const bytes = []; for (let i = 0; i < text.length; i++) bytes.push(text.charCodeAt(i) & 255); this.result = 'data:' + ((blob && blob.type) || 'application/octet-stream') + ';base64,' + globalThis.__b64encodeBytes(bytes); }
          else if (kind === 'buffer') { const u = new Uint8Array((text || '').length); for (let i = 0; i < u.length; i++) u[i] = text.charCodeAt(i) & 255; this.result = u.buffer; }
          else this.result = text;
          this.readyState = 2;
          const ev = { target: this };
          this._emit('load', ev); this._emit('loadend', ev);
        } catch (e) {
          this.error = e; this.readyState = 2;
          const ev = { target: this };
          this._emit('error', ev); this._emit('loadend', ev);
        }
      });
    }
    readAsDataURL(b) { this._read(b, 'data'); }
    readAsText(b) { this._read(b, 'text'); }
    readAsArrayBuffer(b) { this._read(b, 'buffer'); }
    addEventListener(n, f) { this['on' + n] = f; }
    removeEventListener(n) { this['on' + n] = null; }
    abort() { this._emit('abort', { target: this }); }
  };
}
// turbo-dom's installGlobals replaces Blob/File with SoA-backed versions whose .text()/byte
// reads return zero-filled content. Capture our working implementations so the DOM bootstrap
// can restore them (FileReader/fileToBase64/CSV parsing need real bytes).
globalThis.__ttRestoreFileApis = (() => {
  const B = globalThis.Blob, F = globalThis.File, FR = globalThis.FileReader;
  return () => { globalThis.Blob = B; globalThis.File = F; globalThis.FileReader = FR; };
})();
if (typeof globalThis.fetch === 'undefined') {
  globalThis.fetch = () => Promise.reject(new Error('fetch is not supported in turbo-test env'));
  globalThis.Headers = class Headers {
    constructor(init) {
      this._ = new Map();
      if (init) {
        if (typeof init.forEach === 'function' && !Array.isArray(init)) init.forEach((v, k) => this.set(k, v));
        else if (Array.isArray(init)) init.forEach(([k, v]) => this.set(k, v));
        else for (const k of Object.keys(init)) this.set(k, init[k]);
      }
    }
    get(k) { return this._.get(String(k).toLowerCase()) ?? null; }
    set(k, v) { this._.set(String(k).toLowerCase(), String(v)); }
    append(k, v) { const e = this._.get(String(k).toLowerCase()); this._.set(String(k).toLowerCase(), e ? e + ', ' + v : String(v)); }
    has(k) { return this._.has(String(k).toLowerCase()); }
    delete(k) { this._.delete(String(k).toLowerCase()); }
    forEach(f) { this._.forEach((v, k) => f(v, k, this)); }
    entries() { return this._.entries(); }
    keys() { return this._.keys(); }
    values() { return this._.values(); }
    getSetCookie() { const v = this._.get('set-cookie'); return v ? [v] : []; }
    [Symbol.iterator]() { return this._.entries(); }
  };
  globalThis.Request = class Request {
    constructor(url, init) {
      init = init || {};
      // define url as an own, writable, configurable prop — a plain `this.url = x` would invoke
      // a getter-only `url` on a subclass prototype (e.g. Next's NextRequest) and throw.
      const u = typeof url === 'object' && url ? String(url.url || url.href || url) : String(url);
      try { Object.defineProperty(this, 'url', { value: u, writable: true, configurable: true, enumerable: true }); }
      catch (e) {}
      this.method = init.method || 'GET';
      this.headers = init.headers instanceof globalThis.Headers ? init.headers : (() => { const h = new globalThis.Headers(); if (init.headers) for (const k of Object.keys(init.headers)) h.set(k, init.headers[k]); return h; })();
      this._body = init.body;
    }
    json() { return Promise.resolve(typeof this._body === 'string' ? JSON.parse(this._body) : this._body); }
    text() { return Promise.resolve(this._body == null ? '' : String(this._body)); }
    clone() { return new globalThis.Request(this.url, { method: this.method, headers: this.headers, body: this._body }); }
  };
  globalThis.Response = class Response {
    constructor(body, init) { this.body = body; this._body = body; this.status = (init && init.status) || 200; this.statusText = (init && init.statusText) || globalThis.__STATUS_TEXT[this.status] || ''; this.ok = this.status >= 200 && this.status < 300; this.headers = (init && init.headers instanceof globalThis.Headers) ? init.headers : new globalThis.Headers(init && init.headers); }
    json() { return Promise.resolve(typeof this._body === 'string' ? JSON.parse(this._body) : this._body); }
    text() { return Promise.resolve(this._body == null ? '' : String(this._body)); }
    clone() { return new globalThis.Response(this._body, { status: this.status }); }
    static json(data, init) { const r = new globalThis.Response(JSON.stringify(data), init); r._body = data; r.headers.set('content-type', 'application/json'); return r; }
    static error() { return new globalThis.Response(null, { status: 500 }); }
    static redirect(url, status) { const r = new globalThis.Response(null, { status: status || 302 }); r.headers.set('location', String(url)); return r; }
  };
}
if (typeof globalThis.FormData === 'undefined') {
  globalThis.FormData = class FormData { constructor() { this._ = []; } append(k, v) { this._.push([k, v]); } get(k) { const e = this._.find((x) => x[0] === k); return e ? e[1] : null; } getAll(k) { return this._.filter((x) => x[0] === k).map((x) => x[1]); } has(k) { return this._.some((x) => x[0] === k); } forEach(f) { this._.forEach(([k, v]) => f(v, k)); } };
}
if (typeof globalThis.structuredClone === 'undefined') {
  // V8's structured clone (src/structured_clone.rs), with Node's argument checks: the ArrayBuffers
  // of `transfer` move to the clone
  globalThis.structuredClone = function structuredClone(value, options) {
    if (arguments.length === 0) throw new TypeError('The "The value argument must be specified" argument must be specified');
    let transfer = [];
    const t = options == null ? undefined : options.transfer;
    if (t !== undefined) {
      if (t === null || (typeof t !== 'object' && typeof t !== 'function') || typeof t[Symbol.iterator] !== 'function') {
        throw new TypeError("Failed to execute 'structuredClone': transfer in Options cannot be converted to sequence.");
      }
      transfer = [...t];
    }
    return globalThis.__ttStructuredClone(value, transfer);
  };
}
if (typeof globalThis.global === 'undefined') globalThis.global = globalThis;
// Tell React it's in a test act() environment: concurrent work defers to the act queue instead
// of the real (MessageChannel) scheduler, so act() flushing doesn't re-enter React's work loop
// ("Should not already be working"). @testing-library/react sets this; our env must too.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
// Minimal DOMMatrix/DOMPoint (SVG transform math used by charting libs at module load).
if (typeof globalThis.DOMMatrix === 'undefined') {
  class DOMMatrix {
    constructor(init) {
      this.a = 1; this.b = 0; this.c = 0; this.d = 1; this.e = 0; this.f = 0;
      this.m11 = 1; this.m12 = 0; this.m13 = 0; this.m14 = 0;
      this.m21 = 0; this.m22 = 1; this.m23 = 0; this.m24 = 0;
      this.m31 = 0; this.m32 = 0; this.m33 = 1; this.m34 = 0;
      this.m41 = 0; this.m42 = 0; this.m43 = 0; this.m44 = 1;
      this.is2D = true; this.isIdentity = true;
      if (Array.isArray(init) && init.length >= 6) { [this.a, this.b, this.c, this.d, this.e, this.f] = init; this.m11 = this.a; this.m12 = this.b; this.m21 = this.c; this.m22 = this.d; this.m41 = this.e; this.m42 = this.f; }
    }
    multiply() { return new DOMMatrix(); }
    multiplySelf() { return this; }
    preMultiplySelf() { return this; }
    translate() { return new DOMMatrix(); }
    translateSelf() { return this; }
    scale() { return new DOMMatrix(); }
    scaleSelf() { return this; }
    scale3d() { return new DOMMatrix(); }
    rotate() { return new DOMMatrix(); }
    rotateSelf() { return this; }
    rotateFromVector() { return new DOMMatrix(); }
    skewX() { return new DOMMatrix(); }
    skewY() { return new DOMMatrix(); }
    inverse() { return new DOMMatrix(); }
    invertSelf() { return this; }
    flipX() { return new DOMMatrix(); }
    flipY() { return new DOMMatrix(); }
    transformPoint(p) { return new globalThis.DOMPoint(p && p.x, p && p.y, p && p.z, p && p.w); }
    toFloat32Array() { return new Float32Array(16); }
    toFloat64Array() { return new Float64Array(16); }
    toString() { return 'matrix(1, 0, 0, 1, 0, 0)'; }
    static fromMatrix() { return new DOMMatrix(); }
    static fromFloat32Array() { return new DOMMatrix(); }
    static fromFloat64Array() { return new DOMMatrix(); }
  }
  globalThis.DOMMatrix = DOMMatrix;
  globalThis.DOMMatrixReadOnly = DOMMatrix;
  globalThis.WebKitCSSMatrix = DOMMatrix;
  globalThis.SVGMatrix = DOMMatrix;
}
// Web Streams. This used to be a stub whose reader always resolved `{ done: true }` — the globals
// existed (so `class X extends TransformStream` loaded) but any code that actually CONSUMED a
// stream (fetch body / SSE parsing) silently saw an empty stream. This is a working implementation:
// chunk queue + pull backpressure, reader/writer locking, cancel/abort propagation, tee, async
// iteration, pipeTo/pipeThrough. Not spec-exhaustive (no BYOB readers, no queuing strategy `size`).
// A host that already provides native streams (napi/Node) still wins via the guard.
if (typeof globalThis.TransformStream === 'undefined') {
  // `silent` pre-attaches a no-op catch: `reader.closed` / `writer.closed` reject when the stream
  // errors, and a test that never touches them must not trip an unhandled-rejection report.
  const __sDefer = (silent) => {
    let resolve;
    let reject;
    const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
    if (silent) promise.catch(() => {});
    return { promise, resolve, reject };
  };
  // Return a promise the caller may ignore without tripping unhandled-rejection reporting, while
  // still rejecting for a caller that does await it.
  const __sQuiet = (p) => { p.catch(() => {}); return p; };

  // ---- readable ----
  // Ask the source for more data. Re-pulls only when the last pull actually enqueued something,
  // so a source whose pull() never enqueues can't spin forever.
  const __rsPull = (s) => {
    if (s._err || s._closed || s._pulling || typeof s._src.pull !== 'function') return;
    if (s._queue.length >= s._hwm && !s._pending.length) return;
    s._pulling = true;
    const before = s._enq;
    Promise.resolve(s._startP)
      .then(() => s._src.pull(s._ctrl))
      .then(
        () => { s._pulling = false; if (s._enq !== before) __rsPull(s); },
        (e) => { s._pulling = false; try { s._ctrl.error(e); } catch (_) {} },
      );
  };
  const __rsRead = (s) => {
    if (s._queue.length) {
      const value = s._queue.shift();
      // A close() with chunks still queued stays readable until they drain (spec behavior).
      if (s._closed && !s._queue.length) s._closedD.resolve(undefined);
      __rsPull(s);
      return Promise.resolve({ value, done: false });
    }
    if (s._err) return Promise.reject(s._err.e);
    if (s._closed) return Promise.resolve({ value: undefined, done: true });
    const d = __sDefer();
    s._pending.push(d);
    __rsPull(s);
    return d.promise;
  };
  const __rsCancel = (s, reason) => {
    if (s._err) return Promise.reject(s._err.e);
    s._queue.length = 0;
    if (!s._closed) {
      s._closed = true;
      for (const p of s._pending.splice(0)) p.resolve({ value: undefined, done: true });
      s._closedD.resolve(undefined);
    }
    let r;
    try { r = typeof s._src.cancel === 'function' ? s._src.cancel(reason) : undefined; } catch (e) { return Promise.reject(e); }
    return Promise.resolve(r).then(() => undefined);
  };

  class ReadableStreamDefaultController {
    constructor(stream) { this._s = stream; }
    get desiredSize() {
      const s = this._s;
      if (s._err) return null;
      if (s._closed) return 0;
      return s._hwm - s._queue.length;
    }
    enqueue(chunk) {
      const s = this._s;
      if (s._closed || s._err) throw new TypeError('Cannot enqueue a chunk into a closed or errored readable stream');
      s._enq++;
      const p = s._pending.shift();
      if (p) p.resolve({ value: chunk, done: false });
      else s._queue.push(chunk);
    }
    close() {
      const s = this._s;
      if (s._closed || s._err) throw new TypeError('Cannot close a closed or errored readable stream');
      s._closed = true;
      for (const p of s._pending.splice(0)) p.resolve({ value: undefined, done: true });
      if (!s._queue.length) s._closedD.resolve(undefined);
    }
    error(e) {
      const s = this._s;
      if (s._err) return;
      s._err = { e };
      s._queue.length = 0;
      for (const p of s._pending.splice(0)) p.reject(e);
      s._closedD.reject(e);
    }
  }

  class ReadableStreamDefaultReader {
    constructor(stream) {
      if (stream._reader) throw new TypeError('This stream has already been locked for exclusive reading by another reader');
      this._s = stream;
      stream._reader = this;
    }
    get closed() { return this._s ? this._s._closedD.promise : Promise.resolve(undefined); }
    read() { return this._s ? __rsRead(this._s) : Promise.reject(new TypeError('Cannot read from a released reader')); }
    cancel(reason) { return this._s ? __rsCancel(this._s, reason) : Promise.reject(new TypeError('Cannot cancel a released reader')); }
    releaseLock() { if (this._s) { this._s._reader = null; this._s = null; } }
  }

  class ReadableStream {
    constructor(underlyingSource, strategy) {
      this._src = underlyingSource || {};
      this._hwm = strategy && typeof strategy.highWaterMark === 'number' ? strategy.highWaterMark : 1;
      this._queue = [];
      this._pending = [];
      this._closed = false;
      this._err = null;
      this._enq = 0;          // enqueue counter — drives the re-pull decision in __rsPull
      this._pulling = false;
      this._reader = null;
      this._closedD = __sDefer(true);
      this._ctrl = new ReadableStreamDefaultController(this);
      this._startP = undefined;
      if (typeof this._src.start === 'function') {
        try { this._startP = this._src.start(this._ctrl); } catch (e) { this._ctrl.error(e); }
      }
      if (this._startP && typeof this._startP.then === 'function') {
        this._startP.then(() => __rsPull(this), (e) => { try { this._ctrl.error(e); } catch (_) {} });
      } else {
        __rsPull(this);
      }
    }
    get locked() { return this._reader != null; }
    getReader(opts) {
      if (opts && opts.mode === 'byob') throw new TypeError('BYOB readers are not supported in the turbo-test env');
      return new ReadableStreamDefaultReader(this);
    }
    cancel(reason) {
      if (this._reader) return Promise.reject(new TypeError('Cannot cancel a stream locked by a reader'));
      return __rsCancel(this, reason);
    }
    tee() {
      const reader = this.getReader();
      const branches = [];
      let inflight = null;
      // One source read feeds both branches; whichever branch pulls first starts it.
      const pump = () => {
        if (!inflight) {
          inflight = reader.read().then(
            ({ value, done }) => {
              inflight = null;
              for (const b of branches) { try { if (done) b._ctrl.close(); else b._ctrl.enqueue(value); } catch (_) {} }
            },
            (e) => { inflight = null; for (const b of branches) { try { b._ctrl.error(e); } catch (_) {} } },
          );
        }
        return inflight;
      };
      let cancelled = 0;
      const src = { pull: pump, cancel: () => (++cancelled === 2 ? reader.cancel() : undefined) };
      branches.push(new ReadableStream(src), new ReadableStream(src));
      return branches;
    }
    pipeTo(dest, opts) {
      const o = opts || {};
      const reader = this.getReader();
      const writer = dest.getWriter();
      const pump = () => reader.read().then(({ value, done }) => {
        if (done) return o.preventClose ? undefined : writer.close();
        return Promise.resolve(writer.write(value)).then(pump);
      });
      return __sQuiet(pump().then(
        () => { reader.releaseLock(); writer.releaseLock(); },
        (e) => {
          reader.releaseLock();
          const p = o.preventAbort ? Promise.resolve() : Promise.resolve(writer.abort(e)).catch(() => {});
          return p.then(() => { writer.releaseLock(); throw e; });
        },
      ));
    }
    pipeThrough(pair, opts) {
      if (!pair || !pair.writable || !pair.readable) throw new TypeError('pipeThrough requires a { readable, writable } pair');
      this.pipeTo(pair.writable, opts);
      return pair.readable;
    }
    values(opts) { return this[Symbol.asyncIterator](opts); }
    [Symbol.asyncIterator]() {
      const reader = this.getReader();
      return {
        next: () => reader.read().then((r) => { if (r.done) reader.releaseLock(); return r; }),
        return: (v) => reader.cancel().then(() => { reader.releaseLock(); return { value: v, done: true }; }),
        [Symbol.asyncIterator]() { return this; },
      };
    }
  }

  // ---- writable ----
  const __wsError = (s, e) => {
    if (s._err || s._closedFlag) return;
    s._err = { e };
    s._closedD.reject(e);
    s._readyD.reject(e);
  };
  // Writes are serialized through a single promise chain so the sink sees them in order.
  const __wsWrite = (s, chunk) => {
    if (s._err) return Promise.reject(s._err.e);
    if (s._closedFlag) return Promise.reject(new TypeError('Cannot write to a closed stream'));
    s._chain = s._chain.then(() => (typeof s._sink.write === 'function' ? s._sink.write(chunk, s._ctrl) : undefined));
    const p = s._chain;
    p.catch((e) => __wsError(s, e));
    return __sQuiet(p.then(() => undefined));
  };
  const __wsClose = (s) => {
    if (s._err) return Promise.reject(s._err.e);
    if (s._closedFlag) return Promise.reject(new TypeError('Cannot close an already-closed stream'));
    s._closedFlag = true;
    const p = s._chain.then(() => (typeof s._sink.close === 'function' ? s._sink.close(s._ctrl) : undefined));
    s._chain = p;
    p.then(() => s._closedD.resolve(undefined), (e) => { s._err = { e }; s._closedD.reject(e); });
    return __sQuiet(p.then(() => undefined));
  };
  const __wsAbort = (s, reason) => {
    if (s._err) return Promise.resolve(undefined);
    s._ctrl.signal.aborted = true;
    s._ctrl.signal.reason = reason;
    __wsError(s, reason instanceof Error ? reason : new Error(reason === undefined ? 'The stream was aborted' : String(reason)));
    let r;
    try { r = typeof s._sink.abort === 'function' ? s._sink.abort(reason) : undefined; } catch (e) { return Promise.reject(e); }
    return Promise.resolve(r).then(() => undefined);
  };

  class WritableStreamDefaultController {
    constructor(stream) {
      this._s = stream;
      this.signal = { aborted: false, reason: undefined, addEventListener() {}, removeEventListener() {} };
    }
    error(e) { __wsError(this._s, e); }
  }

  class WritableStreamDefaultWriter {
    constructor(stream) {
      if (stream._writer) throw new TypeError('This stream has already been locked for exclusive writing by another writer');
      this._s = stream;
      stream._writer = this;
    }
    get closed() { return this._s ? this._s._closedD.promise : Promise.resolve(undefined); }
    get ready() { return this._s ? this._s._readyD.promise : Promise.resolve(undefined); }
    get desiredSize() { const s = this._s; if (!s || s._err) return null; return s._closedFlag ? 0 : s._hwm; }
    write(chunk) { return this._s ? __wsWrite(this._s, chunk) : Promise.reject(new TypeError('Cannot write with a released writer')); }
    close() { return this._s ? __wsClose(this._s) : Promise.reject(new TypeError('Cannot close with a released writer')); }
    abort(reason) { return this._s ? __wsAbort(this._s, reason) : Promise.reject(new TypeError('Cannot abort with a released writer')); }
    releaseLock() { if (this._s) { this._s._writer = null; this._s = null; } }
  }

  class WritableStream {
    constructor(underlyingSink, strategy) {
      this._sink = underlyingSink || {};
      this._hwm = strategy && typeof strategy.highWaterMark === 'number' ? strategy.highWaterMark : 1;
      this._writer = null;
      this._err = null;
      this._closedFlag = false;
      this._closedD = __sDefer(true);
      this._readyD = __sDefer(true);
      this._readyD.resolve(undefined);
      this._ctrl = new WritableStreamDefaultController(this);
      this._chain = Promise.resolve();
      if (typeof this._sink.start === 'function') {
        this._chain = __sQuiet(Promise.resolve().then(() => this._sink.start(this._ctrl)).catch((e) => { __wsError(this, e); throw e; }));
      }
    }
    get locked() { return this._writer != null; }
    getWriter() { return new WritableStreamDefaultWriter(this); }
    close() { return this._writer ? Promise.reject(new TypeError('Cannot close a stream locked by a writer')) : __wsClose(this); }
    abort(reason) { return this._writer ? Promise.reject(new TypeError('Cannot abort a stream locked by a writer')) : __wsAbort(this, reason); }
  }

  // ---- transform ----
  class TransformStreamDefaultController {
    constructor(ts) { this._ts = ts; }
    get desiredSize() { return this._ts.readable._ctrl.desiredSize; }
    enqueue(chunk) { this._ts.readable._ctrl.enqueue(chunk); }
    error(e) { try { this._ts.readable._ctrl.error(e); } catch (_) {} __wsError(this._ts.writable, e); }
    terminate() { try { this._ts.readable._ctrl.close(); } catch (_) {} }
  }

  class TransformStream {
    constructor(transformer, writableStrategy, readableStrategy) {
      const t = transformer || {};
      // Push-driven: transform() enqueues straight into the readable side (no readable-side
      // backpressure onto the writable side, which is the one spec behavior we don't model).
      this.readable = new ReadableStream({}, readableStrategy);
      const ctrl = new TransformStreamDefaultController(this);
      if (typeof t.start === 'function') t.start(ctrl);
      this.writable = new WritableStream({
        write: (chunk) => (typeof t.transform === 'function' ? t.transform(chunk, ctrl) : ctrl.enqueue(chunk)),
        close: () => Promise.resolve(typeof t.flush === 'function' ? t.flush(ctrl) : undefined)
          .then(() => { try { this.readable._ctrl.close(); } catch (_) {} }),
        abort: (reason) => { try { this.readable._ctrl.error(reason instanceof Error ? reason : new Error(String(reason))); } catch (_) {} },
      }, writableStrategy);
    }
  }

  globalThis.ReadableStream = ReadableStream;
  globalThis.ReadableStreamDefaultReader = ReadableStreamDefaultReader;
  globalThis.ReadableStreamDefaultController = ReadableStreamDefaultController;
  globalThis.WritableStream = WritableStream;
  globalThis.WritableStreamDefaultWriter = WritableStreamDefaultWriter;
  globalThis.WritableStreamDefaultController = WritableStreamDefaultController;
  globalThis.TransformStream = TransformStream;
  globalThis.TransformStreamDefaultController = TransformStreamDefaultController;
}
if (typeof globalThis.DOMPoint === 'undefined') {
  class DOMPoint {
    constructor(x, y, z, w) { this.x = x || 0; this.y = y || 0; this.z = z || 0; this.w = w == null ? 1 : w; }
    matrixTransform() { return this; }
    static fromPoint(p) { return new DOMPoint(p && p.x, p && p.y, p && p.z, p && p.w); }
  }
  globalThis.DOMPoint = DOMPoint;
  globalThis.DOMPointReadOnly = DOMPoint;
}
if (typeof globalThis.Buffer === 'undefined') {
  class Buffer extends Uint8Array {
    static from(d, enc) {
      if (typeof d === 'string') {
        if (enc === 'base64') { return new Buffer(globalThis.__b64decodeToBytes(d)); }
        if (enc === 'hex') { const b = new Buffer(d.length / 2); for (let i = 0; i < b.length; i++) b[i] = parseInt(d.substr(i * 2, 2), 16); return b; }
        return new Buffer(new globalThis.TextEncoder().encode(d));
      }
      if (Array.isArray(d) || d instanceof Uint8Array) return new Buffer(d);
      if (d instanceof ArrayBuffer) return new Buffer(new Uint8Array(d));
      return new Buffer(0);
    }
    static alloc(n, fill) { const b = new Buffer(n); if (fill != null) b.fill(typeof fill === 'string' ? fill.charCodeAt(0) : fill); return b; }
    static allocUnsafe(n) { return new Buffer(n); }
    static isBuffer(x) { return x instanceof Buffer; }
    static concat(list) { const len = list.reduce((n, b) => n + b.length, 0); const out = new Buffer(len); let o = 0; for (const b of list) { out.set(b, o); o += b.length; } return out; }
    toString(enc) {
      if (enc === 'hex') return Array.from(this).map((b) => b.toString(16).padStart(2, '0')).join('');
      if (enc === 'base64') return globalThis.__b64encodeBytes(Array.from(this));
      return new globalThis.TextDecoder().decode(this);
    }
    toJSON() { return { type: 'Buffer', data: Array.from(this) }; }
  }
  globalThis.Buffer = Buffer;
}
if (typeof globalThis.MessageChannel === 'undefined') {
  // React's scheduler posts work via MessageChannel; postMessage MUST deliver to the other
  // port's onmessage asynchronously (as a macrotask) or async React updates never flush.
  globalThis.MessageChannel = class MessageChannel {
    constructor() {
      const mk = () => ({
        onmessage: null,
        onmessageerror: null,
        _other: null,
        postMessage(data) {
          const o = this._other;
          if (o) globalThis.setTimeout(() => {
            const ev = { data };
            if (typeof o.onmessage === 'function') o.onmessage(ev);
            (o._listeners || []).forEach((f) => f(ev));
          }, 0);
        },
        close() {},
        start() {},
        addEventListener(t, f) { if (t === 'message') (this._listeners || (this._listeners = [])).push(f); },
        removeEventListener(t, f) { if (this._listeners) this._listeners = this._listeners.filter((x) => x !== f); },
      });
      this.port1 = mk(); this.port2 = mk();
      this.port1._other = this.port2; this.port2._other = this.port1;
    }
  };
  globalThis.MessagePort = class MessagePort { postMessage() {} close() {} start() {} addEventListener() {} removeEventListener() {} };
}
if (typeof globalThis.performance === 'undefined') {
  globalThis.performance = { now: () => 0, mark() {}, measure() {}, getEntriesByName: () => [], clearMarks() {}, clearMeasures() {} };
}
if (typeof globalThis.btoa === 'undefined') {
  globalThis.btoa = (s) => { const c = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'; let o = '', i = 0; s = String(s); while (i < s.length) { const a = s.charCodeAt(i++), b = s.charCodeAt(i++), d = s.charCodeAt(i++); o += c[a >> 2] + c[((a & 3) << 4) | (b >> 4)] + (isNaN(b) ? '=' : c[((b & 15) << 2) | (d >> 6)]) + (isNaN(d) ? '=' : c[d & 63]); } return o; };
  globalThis.atob = (s) => { const c = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'; s = String(s).replace(/=+$/, ''); let o = ''; for (let bc = 0, bs = 0, bu, i = 0; (bu = s.charAt(i++)); ) { const ci = c.indexOf(bu); if (ci < 0) continue; bs = bc % 4 ? bs * 64 + ci : ci; if (bc++ % 4) o += String.fromCharCode(255 & (bs >> ((-2 * bc) & 6))); } return o; };
}

// ---- EventTarget/Event (turbo-dom's Node extends EventTarget) ----
if (typeof globalThis.EventTarget === 'undefined') {
  globalThis.EventTarget = class EventTarget {
    constructor() { this.__listeners = Object.create(null); }
    addEventListener(type, fn) { if (!fn) return; (this.__listeners[type] || (this.__listeners[type] = [])).push(fn); }
    removeEventListener(type, fn) { const a = this.__listeners[type]; if (a) this.__listeners[type] = a.filter((x) => x !== fn); }
    dispatchEvent(ev) {
      ev.target = ev.target || this; ev.currentTarget = this;
      (this.__listeners[ev.type] || []).slice().forEach((fn) => { try { (fn.handleEvent || fn).call(this, ev); } catch {} });
      return !ev.defaultPrevented;
    }
  };
}
if (typeof globalThis.Event === 'undefined') {
  globalThis.Event = class Event {
    constructor(type, init) { init = init || {}; this.type = type; this.bubbles = !!init.bubbles; this.cancelable = !!init.cancelable; this.composed = !!init.composed; this.defaultPrevented = false; this.target = null; this.currentTarget = null; this.timeStamp = 0; }
    preventDefault() { this.defaultPrevented = true; }
    stopPropagation() {}
    stopImmediatePropagation() {}
  };
}
// MessageEvent — postMessage-style events (Calendly/Stripe embeds, BroadcastChannel, web workers).
// jsdom provides it; turbo-test didn't, so `new MessageEvent('message', { data })` / a component's
// window 'message' listener threw "MessageEvent is not defined".
if (typeof globalThis.MessageEvent === 'undefined') {
  globalThis.MessageEvent = class MessageEvent extends globalThis.Event {
    constructor(type, init) { super(type, init); init = init || {}; this.data = init.data == null ? null : init.data; this.origin = init.origin || ''; this.lastEventId = init.lastEventId || ''; this.source = init.source || null; this.ports = init.ports || []; }
  };
}
if (typeof globalThis.MutationObserver === 'undefined') {
  globalThis.MutationObserver = class MutationObserver { constructor(cb) { this._cb = cb; } observe() {} disconnect() {} takeRecords() { return []; } };
}
if (typeof globalThis.requestAnimationFrame === 'undefined') {
  globalThis.requestAnimationFrame = (cb) => globalThis.setTimeout(() => cb(0), 0);
  globalThis.cancelAnimationFrame = (id) => globalThis.clearTimeout(id);
}

// ---- node builtin shims (so turbo-dom's loader reaches its native .node) ----
// node:url's fileURLToPath / pathToFileURL (Node 26, posix; turbo-test#84)
const __urlError = (code, message) => { const e = new TypeError(message); e.code = code; return e; };
globalThis.__fileURLToPath = (u) => {
  const url = typeof u === 'string' ? new globalThis.URL(u) : u;
  if (url.protocol !== 'file:') throw __urlError('ERR_INVALID_URL_SCHEME', 'The URL must be of scheme file');
  if (url.hostname !== '') throw __urlError('ERR_INVALID_FILE_URL_HOST', `File URL host must be "localhost" or empty on ${globalThis.__ttPlatform || 'darwin'}`);
  if (/%2f/i.test(url.pathname)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded / characters');
  return decodeURIComponent(url.pathname);
};
globalThis.__pathToFileURL = (p) => {
  let path = String(p);
  if (!path.startsWith('/')) path = globalThis.process.cwd() + '/' + path;
  const trailing = path.endsWith('/') ? '/' : '';
  const out = [];
  for (const seg of path.split('/')) { if (seg === '..') out.pop(); else if (seg && seg !== '.') out.push(seg); }
  const resolved = '/' + out.join('/') + (out.length ? trailing : '');
  const url = new globalThis.URL('file:///');
  // Node's set: controls, space, "#%<>?[\]^`{|}~ and DEL (the setter encodes non-ASCII)
  url.pathname = resolved.replace(/[\x00-\x20"#%<>?[\\\]^`{|}~\x7f]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));
  return url;
};
globalThis.__dirnameOf = (p) => { const s = String(p).replace(/\/+$/, ''); const i = s.lastIndexOf('/'); return i <= 0 ? '/' : s.slice(0, i); };
const __os = {
  platform: () => globalThis.__ttPlatform || 'darwin', arch: () => globalThis.__ttArch || 'arm64', type: () => globalThis.__ttOsType || 'Darwin', release: () => '23.0.0',
  version: () => 'Darwin Kernel', EOL: '\n', tmpdir: () => globalThis.__ttTmpdir || '/tmp', homedir: () => globalThis.__ttHomedir || '/', hostname: () => 'localhost',
  cpus: () => [{ model: 'cpu', speed: 1, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } }],
  totalmem: () => 16 * 1024 * 1024 * 1024, freemem: () => 8 * 1024 * 1024 * 1024, uptime: () => 0,
  loadavg: () => [0, 0, 0], endianness: () => 'LE', networkInterfaces: () => ({}),
  userInfo: () => ({ username: 'user', uid: -1, gid: -1, shell: null, homedir: '/' }), availableParallelism: () => 4,
  constants: { signals: {}, errno: {} },
};
const __path = {
  sep: '/',
  join: (...a) => a.filter((x) => x != null && x !== '').join('/').replace(/\/+/g, '/'),
  resolve: (...a) => { let p = a.filter(Boolean).join('/').replace(/\/+/g, '/'); return p.startsWith('/') ? p : '/' + p; },
  dirname: (p) => globalThis.__dirnameOf(p),
  basename: (p, ext) => { let b = String(p).split('/').pop(); if (ext && b.endsWith(ext)) b = b.slice(0, -ext.length); return b; },
  extname: (p) => { const m = /\.[^./]+$/.exec(String(p)); return m ? m[0] : ''; },
  isAbsolute: (p) => String(p).startsWith('/'),
  relative: (_f, t) => String(t),
  normalize: (p) => String(p).replace(/\/+/g, '/'),
  parse: (p) => { const s = String(p); const dir = globalThis.__dirnameOf(s); const base = s.split('/').pop() || ''; const m = /\.[^./]+$/.exec(base); const ext = m ? m[0] : ''; return { root: s.startsWith('/') ? '/' : '', dir, base, ext, name: ext ? base.slice(0, -ext.length) : base }; },
  format: (o) => { o = o || {}; if (o.dir) return (o.dir + '/' + (o.base || ((o.name || '') + (o.ext || '')))).replace(/\/+/g, '/'); return o.base || ((o.name || '') + (o.ext || '')); },
  delimiter: ':',
};
// glob-parent and others reach for path.posix / path.win32; expose both (posix is our default).
__path.posix = __path;
__path.win32 = Object.assign({}, __path, { sep: '\\', delimiter: ';' });
// A path argument as Node takes it: a `file:` URL (fileURLToPath), a Buffer, or a string.
const __fsPath = (p) => {
  if (p !== null && typeof p === 'object' && typeof p.href === 'string' && typeof p.protocol === 'string') return globalThis.__fileURLToPath(p);
  return String(p);
};
// Node's Dirent / Stats: the kind is the S_IFMT bits (a Dirent keeps them under a symbol, so
// its own keys are `name` and `parentPath`; Stats reads them from `mode`).
const __S_IFMT = 0o170000;
const __kType = Symbol('type');
const __kinds = { isFile: 0o100000, isDirectory: 0o040000, isBlockDevice: 0o060000, isCharacterDevice: 0o020000, isSymbolicLink: 0o120000, isFIFO: 0o010000, isSocket: 0o140000 };
class Dirent {
  constructor(name, type, parentPath) { this.name = name; this.parentPath = parentPath; this[__kType] = type; }
}
class StatsBase {}
for (const [name, bits] of Object.entries(__kinds)) {
  Object.defineProperty(Dirent.prototype, name, { value: function () { return this[__kType] === bits; }, writable: true, configurable: true });
  Object.defineProperty(StatsBase.prototype, name, { value: function () { return (this.mode & __S_IFMT) === bits; }, writable: true, configurable: true });
}
const __statFields = ['dev', 'mode', 'nlink', 'uid', 'gid', 'rdev', 'blksize', 'ino', 'size', 'blocks', 'atimeMs', 'mtimeMs', 'ctimeMs', 'birthtimeMs'];
class Stats extends StatsBase {
  constructor(values) { super(); __statFields.forEach((k, i) => { this[k] = values[i]; }); }
}
for (const t of ['atime', 'mtime', 'ctime', 'birthtime']) {
  Object.defineProperty(Stats.prototype, t, { get() { return new Date(Math.round(this[t + 'Ms'])); }, configurable: true, enumerable: false });
}
// statSync / lstatSync: `throwIfNoEntry: false` gives undefined for a missing entry.
const __stat = (p, options, lstat) => {
  try { return new Stats(globalThis.__fs_stat(__fsPath(p), lstat)); }
  catch (e) { if (options && options.throwIfNoEntry === false && (e.code === 'ENOENT' || e.code === 'ENOTDIR')) return undefined; throw e; }
};
// readdirSync: sorted names, or Dirents with `withFileTypes`; `recursive` walks breadth-first
// as Node does (names relative to the base; a symlink to a directory is descended too).
const __readdir = (p, options) => {
  const base = __fsPath(p);
  const opts = typeof options === 'object' && options !== null ? options : {};
  const read = (dir) => globalThis.__fs_readdir(dir);
  const join = (a, b) => (a.endsWith('/') ? a + b : a + '/' + b);
  if (!opts.recursive) return read(base).map(([name, type]) => (opts.withFileTypes ? new Dirent(name, type, base) : name));
  const out = [], queue = [base];
  for (let i = 0; i < queue.length; i++) {
    for (const [name, type] of read(queue[i])) {
      const full = join(queue[i], name);
      out.push(opts.withFileTypes ? new Dirent(name, type, queue[i]) : full.slice(base.length + (base.endsWith('/') ? 0 : 1)));
      let dir = type === 0o040000;
      if (type === 0o120000) { try { dir = (globalThis.__fs_stat(full, false)[1] & __S_IFMT) === 0o040000; } catch (e) {} }
      if (dir) queue.push(full);
    }
  }
  return out;
};
// readFileSync / writeFileSync / appendFileSync / mkdirSync / rmSync / rmdirSync / unlinkSync
// with Node's errors and options (turbo-test#86): a string with an encoding, else a Buffer; no
// implicit parent directories; mkdir `recursive`, rm `recursive` / `force`.
const __encodingOf = (o) => (typeof o === 'string' ? o : o && o.encoding) || null;
const __readFile = (p, options) => {
  const enc = __encodingOf(options);
  const utf8 = enc === 'utf8' || enc === 'utf-8';
  const data = globalThis.__fs_readFile(__fsPath(p), utf8);
  if (utf8) return data;
  const buf = globalThis.Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  return enc ? buf.toString(enc) : buf;
};
const __writeFile = (p, data, append) => globalThis.__fs_writeFile(__fsPath(p), ArrayBuffer.isView(data) ? data : String(data), append);
const __mkdir = (p, options) => globalThis.__fs_mkdir(__fsPath(p), !!(options && typeof options === 'object' && options.recursive));
const __rm = (p, options) => {
  const path = __fsPath(p), o = options || {};
  let st;
  try { st = globalThis.__fs_stat(path, true); } catch (e) { if (o.force && e.code === 'ENOENT') return; throw e; }
  if ((st[1] & __S_IFMT) !== 0o040000) return globalThis.__fs_remove(path, 'unlink');
  if (!o.recursive) {
    const e = new Error(`Path is a directory: rm returned EISDIR (is a directory) ${path}`);
    Object.defineProperty(e, 'name', { value: 'SystemError', configurable: true, writable: true });
    Object.assign(e, { code: 'ERR_FS_EISDIR', errno: 21, syscall: 'rm', path });
    throw e;
  }
  globalThis.__fs_remove(path, 'rm');
};
const __promised = (f) => (...a) => { try { return Promise.resolve(f(...a)); } catch (e) { return Promise.reject(e); } };
const __fs = {
  Dirent,
  Stats,
  existsSync: (p) => globalThis.__fs_existsSync(__fsPath(p)),
  readFileSync: __readFile,
  writeFileSync: (p, data) => __writeFile(p, data, false),
  // a real append (O_APPEND on the Rust side), not read + rewrite: parallel workers appending to
  // one file would otherwise overwrite each other's lines.
  appendFileSync: (p, data) => __writeFile(p, data, true),
  mkdirSync: __mkdir,
  rmSync: __rm,
  rmdirSync: (p) => globalThis.__fs_remove(__fsPath(p), 'rmdir'),
  unlinkSync: (p) => globalThis.__fs_remove(__fsPath(p), 'unlink'),
  statSync: (p, options) => __stat(p, options, false),
  lstatSync: (p, options) => __stat(p, options, true),
  readdirSync: __readdir,
  symlinkSync: (target, p) => globalThis.__fs_symlinkSync(__fsPath(target), __fsPath(p)),
  promises: {
    symlink: (target, p) => { try { __fs.symlinkSync(target, p); return Promise.resolve(); } catch (e) { return Promise.reject(e); } },
    readFile: __promised(__readFile),
    writeFile: __promised((p, data) => __writeFile(p, data, false)),
    appendFile: __promised((p, data) => __writeFile(p, data, true)),
    mkdir: __promised(__mkdir),
    rm: __promised(__rm),
    rmdir: __promised((p) => globalThis.__fs_remove(__fsPath(p), 'rmdir')),
    unlink: __promised((p) => globalThis.__fs_remove(__fsPath(p), 'unlink')),
    readdir: __promised(__readdir),
    stat: __promised((p, options) => __stat(p, options, false)),
    lstat: __promised((p, options) => __stat(p, options, true)),
  },
};
const __module = {
  createRequire: (url) => globalThis.__mkRequire(globalThis.__dirnameOf(String(url).startsWith('file:') ? globalThis.__fileURLToPath(url) : String(url))),
  Module: function () {},
  builtinModules: [],
};
let __seedCtr = 1;
function __pseudoRandomBytes(n) {
  const b = new Uint8Array(n);
  for (let i = 0; i < n; i++) { __seedCtr = (__seedCtr * 1103515245 + 12345) & 0x7fffffff; b[i] = (__seedCtr >> 16) & 255; }
  return b;
}
const __crypto = {
  randomUUID: () => {
    const b = __pseudoRandomBytes(16);
    b[6] = (b[6] & 0x0f) | 0x40; b[8] = (b[8] & 0x3f) | 0x80;
    const h = Array.from(b, (x) => x.toString(16).padStart(2, '0'));
    return `${h[0]}${h[1]}${h[2]}${h[3]}-${h[4]}${h[5]}-${h[6]}${h[7]}-${h[8]}${h[9]}-${h[10]}${h[11]}${h[12]}${h[13]}${h[14]}${h[15]}`;
  },
  randomBytes: (n) => globalThis.Buffer ? globalThis.Buffer.from(__pseudoRandomBytes(n)) : __pseudoRandomBytes(n),
  getRandomValues: (arr) => { const b = __pseudoRandomBytes(arr.length); for (let i = 0; i < arr.length; i++) arr[i] = b[i]; return arr; },
  randomInt: (min, max) => { if (max === undefined) { max = min; min = 0; } return min + (__pseudoRandomBytes(4).reduce((a, x) => a * 256 + x, 0) % (max - min)); },
  createHash: () => { let data = ''; const h = { update: (d) => { data += String(d); return h; }, digest: (enc) => { let n = 0; for (let i = 0; i < data.length; i++) { n = (n * 31 + data.charCodeAt(i)) & 0x7fffffff; } const hex = n.toString(16).padStart(8, '0'); return enc === 'hex' || !enc ? hex.repeat(8).slice(0, 64) : hex; } }; return h; },
  createHmac: () => { let data = ''; const h = { update: (d) => { data += String(d); return h; }, digest: () => 'hmac' }; return h; },
  // Node crypto constants — many packages (e.g. @propelauth/node) read these at module load to
  // pick a padding/algorithm. turbo-test runs in bare V8 (no real OpenSSL), so the values just
  // need to EXIST so modules load; tests that exercise actual crypto mock it or are out of scope.
  constants: {
    RSA_PKCS1_PADDING: 1, RSA_NO_PADDING: 3, RSA_PKCS1_OAEP_PADDING: 4, RSA_X931_PADDING: 5,
    RSA_PKCS1_PSS_PADDING: 6, RSA_PSS_SALTLEN_DIGEST: -1, RSA_PSS_SALTLEN_MAX_SIGN: -2,
    RSA_PSS_SALTLEN_AUTO: -2, RSA_SSLV23_PADDING: 2,
    SSL_OP_ALL: 0x80000bff, ENGINE_METHOD_ALL: 0xffff,
    DH_CHECK_P_NOT_SAFE_PRIME: 2, DH_NOT_SUITABLE_GENERATOR: 8,
    POINT_CONVERSION_COMPRESSED: 2, POINT_CONVERSION_UNCOMPRESSED: 4, POINT_CONVERSION_HYBRID: 6,
    defaultCoreCipherList: 'TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256',
  },
  // Cipher/sign stubs: bare V8 has no OpenSSL, so these can't do real AES/RSA. They exist so a
  // module that merely references them at load doesn't crash; actually invoking them throws a
  // clear, catchable error rather than producing wrong ciphertext.
  createCipheriv: () => { throw new Error('turbo-test: crypto.createCipheriv is not available in the V8 runtime (no OpenSSL) — mock it in the test'); },
  createDecipheriv: () => { throw new Error('turbo-test: crypto.createDecipheriv is not available in the V8 runtime (no OpenSSL) — mock it in the test'); },
  createSign: () => { throw new Error('turbo-test: crypto.createSign is not available in the V8 runtime'); },
  createVerify: () => { throw new Error('turbo-test: crypto.createVerify is not available in the V8 runtime'); },
  pbkdf2Sync: () => { throw new Error('turbo-test: crypto.pbkdf2Sync is not available in the V8 runtime'); },
  scryptSync: () => { throw new Error('turbo-test: crypto.scryptSync is not available in the V8 runtime'); },
  timingSafeEqual: (a, b) => { if (a.length !== b.length) return false; let r = 0; for (let i = 0; i < a.length; i++) r |= a[i] ^ b[i]; return r === 0; },
};
if (typeof globalThis.crypto === 'undefined' || !globalThis.crypto.randomUUID) {
  globalThis.crypto = Object.assign(globalThis.crypto || {}, { randomUUID: __crypto.randomUUID, getRandomValues: __crypto.getRandomValues });
}
class __EventEmitter {
  constructor() { this._ev = {}; }
  on(e, f) { (this._ev[e] || (this._ev[e] = [])).push(f); return this; }
  addListener(e, f) { return this.on(e, f); }
  once(e, f) { const g = (...a) => { this.off(e, g); f(...a); }; return this.on(e, g); }
  off(e, f) { if (this._ev[e]) this._ev[e] = this._ev[e].filter((x) => x !== f); return this; }
  removeListener(e, f) { return this.off(e, f); }
  removeAllListeners(e) { if (e) delete this._ev[e]; else this._ev = {}; return this; }
  emit(e, ...a) { (this._ev[e] || []).slice().forEach((f) => f(...a)); return (this._ev[e] || []).length > 0; }
  listeners(e) { return (this._ev[e] || []).slice(); }
  listenerCount(e) { return (this._ev[e] || []).length; }
  setMaxListeners() { return this; }
  prependListener(e, f) { (this._ev[e] || (this._ev[e] = [])).unshift(f); return this; }
}
const __events = Object.assign(__EventEmitter, { EventEmitter: __EventEmitter, default: __EventEmitter, once: (em, e) => new Promise((res) => em.once(e, (...a) => res(a))) });
class __Readable extends __EventEmitter { pipe(d) { return d; } read() { return null; } push() { return true; } destroy() { return this; } setEncoding() { return this; } resume() { return this; } pause() { return this; } }
class __Writable extends __EventEmitter { write() { return true; } end() { return this; } destroy() { return this; } }
class __Duplex extends __Readable { write() { return true; } end() { return this; } }
const __stream = { Readable: __Readable, Writable: __Writable, Duplex: __Duplex, Transform: __Duplex, PassThrough: __Readable, Stream: __EventEmitter, pipeline: (...a) => { const cb = a[a.length - 1]; if (typeof cb === 'function') cb(); }, finished: (_s, cb) => { if (typeof cb === 'function') cb(); } };
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
const __net = { Socket: class extends __EventEmitter { connect() { return this; } write() { return true; } end() { return this; } destroy() { return this; } setTimeout() { return this; } setNoDelay() { return this; } setKeepAlive() { return this; } ref() { return this; } unref() { return this; } }, Server: class extends __EventEmitter { listen(..._a) { const cb = _a[_a.length - 1]; if (typeof cb === 'function') cb(); return this; } close() { return this; } address() { return { port: 0 }; } }, createConnection: () => new __EventEmitter(), connect: () => new __EventEmitter(), createServer: __mkServer, isIP: () => 0, isIPv4: () => false, isIPv6: () => false }; __net.default = __net;
const __tls = { TLSSocket: class extends __EventEmitter { connect() { return this; } }, connect: () => new __EventEmitter(), createSecureContext: () => ({}), createServer: __mkServer, checkServerIdentity: () => undefined, rootCertificates: [] }; __tls.default = __tls;
const __zlib = { createGzip: () => new __Duplex(), createGunzip: () => new __Duplex(), createDeflate: () => new __Duplex(), createInflate: () => new __Duplex(), createBrotliCompress: () => new __Duplex(), createBrotliDecompress: () => new __Duplex(), gzip: (_b, cb) => cb && cb(null, _b), gunzip: (_b, cb) => cb && cb(null, _b), gzipSync: (b) => b, gunzipSync: (b) => b, deflateSync: (b) => b, inflateSync: (b) => b }; __zlib.default = __zlib;
const __tty = { isatty: () => false, ReadStream: __EventEmitter, WriteStream: __EventEmitter }; __tty.default = __tty;
const __dns = { lookup: (_h, _o, cb) => { const c = typeof _o === 'function' ? _o : cb; if (c) c(null, '127.0.0.1', 4); }, resolve: (_h, cb) => cb && cb(null, []), promises: { lookup: () => Promise.resolve({ address: '127.0.0.1', family: 4 }), resolve: () => Promise.resolve([]) } }; __dns.default = __dns;
const __http2 = { connect: () => new __EventEmitter(), createServer: __mkServer, createSecureServer: __mkServer, constants: {}, getDefaultSettings: () => ({}), Http2ServerRequest: __EventEmitter, Http2ServerResponse: __EventEmitter }; __http2.default = __http2;
const __dgram = { createSocket: () => Object.assign(new __EventEmitter(), { bind() { return this; }, send() {}, close() {}, address: () => ({ port: 0 }) }) }; __dgram.default = __dgram;
const __readline = { createInterface: () => Object.assign(new __EventEmitter(), { question: (_q, cb) => cb && cb(''), close() {}, prompt() {}, write() {} }), clearLine: () => {}, cursorTo: () => {}, moveCursor: () => {} }; __readline.default = __readline;
const __v8 = { serialize: (x) => { try { return JSON.stringify(x); } catch (e) { return '{}'; } }, deserialize: () => ({}), getHeapStatistics: () => ({ total_heap_size: 0, used_heap_size: 0 }), setFlagsFromString: () => {} }; __v8.default = __v8;
const __dc = { channel: () => ({ publish: () => {}, subscribe: () => {}, unsubscribe: () => {}, hasSubscribers: false }), hasSubscribers: () => false, subscribe: () => {}, unsubscribe: () => {} }; __dc.default = __dc;
class __AsyncLocalStorage { constructor() { this._store = undefined; } run(store, cb, ...a) { const p = this._store; this._store = store; try { return cb(...a); } finally { this._store = p; } } getStore() { return this._store; } enterWith(s) { this._store = s; } disable() {} exit(cb, ...a) { return cb(...a); } }
const __async_hooks = { AsyncLocalStorage: __AsyncLocalStorage, AsyncResource: class { runInAsyncScope(fn, thisArg, ...a) { return fn.apply(thisArg, a); } emitDestroy() { return this; } bind(fn) { return fn; } }, createHook: () => ({ enable() { return this; }, disable() { return this; } }), executionAsyncId: () => 0, triggerAsyncId: () => 0 }; __async_hooks.default = __async_hooks;
// ---- Node's util.inspect / util.format (Node 26, colors off; turbo-test#93) ----
// The shape of Node's formatValue / formatRaw / reduceToSingleString / groupArrayElements:
// depth 2, breakLength 80, compact 3, maxArrayLength 100; console.log formats through it.
const __inspectMeta = ['\\x00', '\\x01', '\\x02', '\\x03', '\\x04', '\\x05', '\\x06', '\\x07', '\\b', '\\t', '\\n', '\\x0B', '\\f', '\\r', '\\x0E', '\\x0F', '\\x10', '\\x11', '\\x12', '\\x13', '\\x14', '\\x15', '\\x16', '\\x17', '\\x18', '\\x19', '\\x1A', '\\x1B', '\\x1C', '\\x1D', '\\x1E', '\\x1F'];
function __strEscape(str) {
  let quote = "'";
  if (str.includes("'")) {
    if (!str.includes('"')) quote = '"';
    else if (!str.includes('`') && !str.includes('${')) quote = '`';
  }
  let out = '';
  for (const ch of str) {
    const c = ch.charCodeAt(0);
    if (ch === quote || ch === '\\') out += '\\' + ch;
    else if (c < 0x20) out += __inspectMeta[c];
    else if (c === 0x7f) out += '\\x7F';
    else out += ch;
  }
  return quote + out + quote;
}
function __inspectKey(key) {
  if (typeof key === 'symbol') return key.toString();
  return /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : __strEscape(key);
}
function __ctorName(obj) {
  let o = obj;
  while (o !== null && o !== undefined) {
    const d = Object.getOwnPropertyDescriptor(o, 'constructor');
    if (d && typeof d.value === 'function' && d.value.name !== '') return d.value.name;
    o = Object.getPrototypeOf(o);
  }
  return null;
}
function __fnBase(fn, ctor) {
  const src = Function.prototype.toString.call(fn);
  if (src.startsWith('class') && /^class\s*[\s{A-Za-z_$]/.test(src)) {
    let base = `[class ${fn.name || '(anonymous)'}`;
    const sup = Object.getPrototypeOf(fn);
    if (sup && sup.name) base += ` extends ${sup.name}`;
    return base + ']';
  }
  let type = 'Function';
  const tag = fn[Symbol.toStringTag];
  if (tag === 'AsyncFunction' || tag === 'GeneratorFunction' || tag === 'AsyncGeneratorFunction') type = tag;
  let base = `[${type}`;
  base += fn.name ? `: ${fn.name}]` : ' (anonymous)]';
  if (ctor === null) base += ' (null prototype)';
  return base;
}
function __inspectPrimitive(v, ctx) {
  switch (typeof v) {
    case 'string': {
      const trailer = '';
      if (v.length > 16 && v.length > 80 - ctx.indentationLvl - 4) {
        const parts = v.split(/(?<=\n)/);
        if (parts.length > 1) return parts.map((l) => __strEscape(l)).join(` +\n${' '.repeat(ctx.indentationLvl + 2)}`) + trailer;
      }
      return __strEscape(v) + trailer;
    }
    case 'number': return Object.is(v, -0) ? '-0' : String(v);
    case 'bigint': return `${v}n`;
    case 'boolean': return String(v);
    case 'undefined': return 'undefined';
    case 'symbol': return v.toString();
  }
  return String(v);
}
function __formatValue(ctx, value, recurseTimes, typedArray) {
  if (typeof value !== 'object' && typeof value !== 'function') return __inspectPrimitive(value, ctx);
  if (value === null) return 'null';
  // a [util.inspect.custom](depth, options, inspect) method formats its own value
  const custom = value[Symbol.for('nodejs.util.inspect.custom')];
  if (typeof custom === 'function' && custom !== __util.inspect) {
    const r = custom.call(value, ctx.depth - recurseTimes, { depth: ctx.depth - recurseTimes }, __util.inspect);
    if (r !== value) return typeof r === 'string' ? r : __formatValue(ctx, r, recurseTimes);
  }
  if (ctx.seen.includes(value)) {
    let index = 1;
    if (ctx.circular === undefined) { ctx.circular = new Map(); ctx.circular.set(value, index); }
    else {
      index = ctx.circular.get(value);
      if (index === undefined) { index = ctx.circular.size + 1; ctx.circular.set(value, index); }
    }
    return `[Circular *${index}]`;
  }
  return __formatRaw(ctx, value, recurseTimes, typedArray);
}
function __formatProperty(ctx, value, recurseTimes, key, type, desc) {
  let name, str;
  desc = desc || Object.getOwnPropertyDescriptor(value, key) || { value: value[key], enumerable: true };
  if (desc.value !== undefined) {
    const diff = ctx.compact !== true || type !== 0 ? 2 : 3;
    ctx.indentationLvl += diff;
    str = __formatValue(ctx, desc.value, recurseTimes);
    ctx.indentationLvl -= diff;
  } else if (desc.get !== undefined) {
    str = desc.set !== undefined ? '[Getter/Setter]' : '[Getter]';
  } else if (desc.set !== undefined) {
    str = '[Setter]';
  } else {
    str = 'undefined';
  }
  if (type === 1) return str; // an array element (2: an array's extra key)
  name = typeof key === 'symbol' ? key.toString() : desc.enumerable === false ? `[${key}]` : __inspectKey(key);
  return `${name}: ${str}`;
}
function __groupArrayElements(ctx, output, value) {
  let totalLength = 0;
  let maxLength = 0;
  let outputLength = output.length;
  if (ctx.maxArrayLength < output.length) outputLength--;
  const separatorSpace = 2;
  const dataLen = new Array(outputLength);
  for (let i = 0; i < outputLength; i++) {
    const len = output[i].length;
    dataLen[i] = len;
    totalLength += len + separatorSpace;
    if (maxLength < len) maxLength = len;
  }
  const actualMax = maxLength + separatorSpace;
  if (actualMax * 3 + ctx.indentationLvl < ctx.breakLength && (totalLength / actualMax > 5 || maxLength <= 6)) {
    const averageBias = Math.sqrt(actualMax - totalLength / output.length);
    const biasedMax = Math.max(actualMax - 3 - averageBias, 1);
    const columns = Math.min(
      Math.round(Math.sqrt(2.5 * biasedMax * outputLength) / biasedMax),
      Math.floor((ctx.breakLength - ctx.indentationLvl) / actualMax),
      ctx.compact * 4,
      15,
    );
    if (columns <= 1) return output;
    const tmp = [];
    const maxLineLength = [];
    for (let i = 0; i < columns; i++) {
      let lineLength = 0;
      for (let j = i; j < output.length; j += columns) if (dataLen[j] > lineLength) lineLength = dataLen[j];
      maxLineLength.push(lineLength + separatorSpace);
    }
    let padStart = true;
    if (value !== undefined) {
      for (let i = 0; i < output.length; i++) {
        if (typeof value[i] !== 'number' && typeof value[i] !== 'bigint') { padStart = false; break; }
      }
    }
    for (let i = 0; i < outputLength; i += columns) {
      const max = Math.min(i + columns, outputLength);
      let str = '';
      let j = i;
      for (; j < max - 1; j++) {
        const padding = maxLineLength[j - i] + output[j].length - dataLen[j];
        str += padStart ? `${output[j]}, `.padStart(padding, ' ') : `${output[j]}, `.padEnd(padding, ' ');
      }
      if (padStart) {
        const padding = maxLineLength[j - i] + output[j].length - dataLen[j] - separatorSpace;
        str += output[j].padStart(padding, ' ');
      } else {
        str += output[j];
      }
      tmp.push(str);
    }
    if (ctx.maxArrayLength < output.length) tmp.push(output[outputLength]);
    output = tmp;
  }
  return output;
}
function __isBelowBreakLength(ctx, output, start, base) {
  let totalLength = output.length + start;
  if (totalLength + output.length > ctx.breakLength) return false;
  for (let i = 0; i < output.length; i++) {
    totalLength += output[i].length;
    if (totalLength > ctx.breakLength) return false;
  }
  return base === '' || !base.includes('\n');
}
function __reduceToSingleString(ctx, output, base, braces, extrasType, recurseTimes, value) {
  if (ctx.compact !== true) {
    if (typeof ctx.compact === 'number' && ctx.compact >= 1) {
      const entries = output.length;
      if (extrasType === 2 && entries > 6) output = __groupArrayElements(ctx, output, value);
      if (ctx.currentDepth - recurseTimes < ctx.compact && entries === output.length) {
        const start = output.length + ctx.indentationLvl + braces[0].length + base.length + 10;
        if (__isBelowBreakLength(ctx, output, start, base)) {
          const joined = output.join(', ');
          if (!joined.includes('\n')) return `${base ? `${base} ` : ''}${braces[0]} ${joined} ${braces[1]}`;
        }
      }
    }
    const indentation = `\n${' '.repeat(ctx.indentationLvl)}`;
    return `${base ? `${base} ` : ''}${braces[0]}${indentation}  ${output.join(`,${indentation}  `)}${indentation}${braces[1]}`;
  }
  if (__isBelowBreakLength(ctx, output, 0, base)) return `${braces[0]}${base ? ` ${base}` : ''} ${output.join(', ')} ${braces[1]}`;
  const indentation = ' '.repeat(ctx.indentationLvl);
  const ln = base === '' && braces[0].length === 1 ? ' ' : `${base ? ` ${base}` : ''}\n${indentation}  `;
  return `${braces[0]}${ln}${output.join(`,\n${indentation}  `)} ${braces[1]}`;
}
function __formatRaw(ctx, value, recurseTimes, typedArray) {
  const ctor = __ctorName(value);
  const tag = value[Symbol.toStringTag];
  const prefix = (fallback, size) => {
    let p = ctor === null ? `[${fallback}: null prototype] ` : `${ctor}${size} `;
    if (typeof tag === 'string' && tag !== '' && tag !== (ctor || fallback)) p = `${p.slice(0, -1)} [${tag}] `;
    return p;
  };
  let keys = Object.keys(value).concat(Object.getOwnPropertySymbols(value).filter((s) => Object.prototype.propertyIsEnumerable.call(value, s)));
  let base = '';
  let braces;
  let extrasType = 0;
  let formatter = () => [];
  let noIterator = true;
  if (Array.isArray(value)) {
    const p = ctor !== 'Array' || (typeof tag === 'string' && tag !== '') ? prefix('Array', `(${value.length})`) : '';
    keys = keys.filter((k) => typeof k === 'symbol' || !(/^(0|[1-9][0-9]*)$/.test(k) && Number(k) < value.length));
    braces = [`${p}[`, ']'];
    if (value.length === 0 && keys.length === 0) return `${braces[0]}]`;
    extrasType = 2;
    formatter = () => {
      const out = [];
      const len = Math.min(ctx.maxArrayLength, value.length);
      let holes = 0;
      for (let i = 0; i < len; i++) {
        if (!Object.prototype.hasOwnProperty.call(value, i)) { holes++; continue; }
        if (holes) { out.push(`<${holes} empty item${holes > 1 ? 's' : ''}>`); holes = 0; }
        out.push(__formatProperty(ctx, value, recurseTimes, i, 1));
      }
      if (holes) out.push(`<${holes} empty item${holes > 1 ? 's' : ''}>`);
      if (value.length > len) { const n = value.length - len; out.push(`... ${n} more item${n > 1 ? 's' : ''}`); }
      return out;
    };
  } else if (value instanceof Map) {
    noIterator = false;
    braces = [`${prefix('Map', `(${value.size})`)}{`, '}'];
    if (value.size === 0 && keys.length === 0) return `${braces[0]}}`;
    formatter = () => {
      const out = [];
      ctx.indentationLvl += 2;
      for (const [k, v] of value) out.push(`${__formatValue(ctx, k, recurseTimes)} => ${__formatValue(ctx, v, recurseTimes)}`);
      ctx.indentationLvl -= 2;
      return out;
    };
  } else if (value instanceof Set) {
    noIterator = false;
    braces = [`${prefix('Set', `(${value.size})`)}{`, '}'];
    if (value.size === 0 && keys.length === 0) return `${braces[0]}}`;
    formatter = () => {
      const out = [];
      ctx.indentationLvl += 2;
      for (const v of value) out.push(__formatValue(ctx, v, recurseTimes));
      ctx.indentationLvl -= 2;
      return out;
    };
  } else if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    if (globalThis.Buffer && globalThis.Buffer.isBuffer && globalThis.Buffer.isBuffer(value)) {
      const bytes = Array.from(value.subarray(0, 50), (b) => b.toString(16).padStart(2, '0')).join(' ');
      const more = value.length > 50 ? ` ... ${value.length - 50} more byte${value.length - 50 > 1 ? 's' : ''}` : '';
      return `<${ctor || 'Buffer'}${bytes ? ` ${bytes}` : ''}${more}>`;
    }
    keys = keys.filter((k) => !/^(0|[1-9][0-9]*)$/.test(k));
    braces = [`${prefix(tag || 'TypedArray', `(${value.length})`)}[`, ']'];
    if (value.length === 0 && keys.length === 0) return `${braces[0]}]`;
    extrasType = 2;
    formatter = () => {
      const out = [];
      const len = Math.min(ctx.maxArrayLength, value.length);
      for (let i = 0; i < len; i++) out.push(__inspectPrimitive(value[i], ctx));
      if (value.length > len) { const n = value.length - len; out.push(`... ${n} more item${n > 1 ? 's' : ''}`); }
      return out;
    };
  } else {
    if (typeof value === 'function') {
      base = __fnBase(value, ctor);
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof RegExp) {
      base = RegExp.prototype.toString.call(value);
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof Date) {
      base = Number.isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof Error) {
      base = value.stack ? String(value.stack) : `[${Error.prototype.toString.call(value)}]`;
      keys = keys.filter((k) => k !== 'stack' && k !== 'message');
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof Number || value instanceof String || value instanceof Boolean) {
      const type = value instanceof Number ? 'Number' : value instanceof String ? 'String' : 'Boolean';
      base = `[${type}: ${__inspectPrimitive(value.valueOf(), ctx)}]`;
      if (value instanceof String) keys = keys.filter((k) => !/^(0|[1-9][0-9]*)$/.test(k));
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof Promise) {
      braces = [`${prefix('Promise', '')}{`, '}'];
      formatter = () => ['<pending>'];
    } else if (value instanceof WeakMap || value instanceof WeakSet) {
      braces = [`${prefix(value instanceof WeakMap ? 'WeakMap' : 'WeakSet', '')}{`, '}'];
      formatter = () => ['<items unknown>'];
    } else {
      const p = ctor === 'Object' && !(typeof tag === 'string' && tag !== '') ? '' : prefix('Object', '');
      braces = [`${p}{`, '}'];
      if (keys.length === 0) return `${p}{}`;
    }
  }
  if (recurseTimes > ctx.depth) {
    const name = (ctor || tag || 'Object');
    return Array.isArray(value) ? '[Array]' : `[${name}]`;
  }
  recurseTimes++;
  ctx.seen.push(value);
  ctx.currentDepth = recurseTimes;
  let output = formatter();
  for (const k of keys) output.push(__formatProperty(ctx, value, recurseTimes, k, extrasType));
  ctx.seen.pop();
  let res = __reduceToSingleString(ctx, output, base, braces, extrasType, recurseTimes, value);
  if (ctx.circular !== undefined) {
    const index = ctx.circular.get(value);
    if (index !== undefined) res = `<ref *${index}> ${res}`;
  }
  void noIterator;
  return res;
}
function __nodeInspect(value, opts) {
  const ctx = { seen: [], indentationLvl: 0, currentDepth: 0, depth: 2, breakLength: 80, compact: 3, maxArrayLength: 100, circular: undefined };
  if (opts) {
    if (typeof opts.depth === 'number' || opts.depth === null) ctx.depth = opts.depth === null ? Infinity : opts.depth;
    if (typeof opts.breakLength === 'number') ctx.breakLength = opts.breakLength;
    if (opts.compact !== undefined) ctx.compact = opts.compact;
  }
  return __formatValue(ctx, value, 0);
}
// util.format: %s %d %i %f %j %o %O %c %%, the rest inspected (strings as they are)
function __nodeFormat(...args) {
  const first = args[0];
  const insp = (v) => (typeof v === 'string' ? v : __nodeInspect(v));
  if (typeof first !== 'string') return args.map(insp).join(' ');
  let i = 1;
  let out = '';
  let last = 0;
  for (let p = 0; p < first.length - 1; p++) {
    if (first[p] !== '%') continue;
    const c = first[p + 1];
    if (c === '%') { out += first.slice(last, p) + '%'; last = p + 2; p++; continue; }
    if (i >= args.length || !'sdifjoOc'.includes(c)) continue;
    const a = args[i++];
    let s;
    switch (c) {
      case 's':
        s = typeof a === 'string' ? a : typeof a === 'bigint' ? `${a}n` : typeof a === 'symbol' ? String(a)
          : typeof a === 'object' && a !== null ? __nodeInspect(a, { depth: 0 }) : Object.is(a, -0) ? '-0' : String(a);
        break;
      case 'd': s = typeof a === 'bigint' ? `${a}n` : typeof a === 'symbol' ? 'NaN' : (Object.is(Number(a), -0) ? '-0' : String(Number(a))); break;
      case 'i': s = typeof a === 'bigint' ? `${a}n` : typeof a === 'symbol' ? 'NaN' : String(parseInt(a)); break;
      case 'f': s = typeof a === 'symbol' ? 'NaN' : String(parseFloat(a)); break;
      case 'j': try { s = JSON.stringify(a); } catch (e) { s = '[Circular]'; } break;
      case 'o': s = __nodeInspect(a, { depth: 4 }); break;
      case 'O': s = __nodeInspect(a); break;
      case 'c': s = ''; break;
    }
    out += first.slice(last, p) + s;
    last = p + 2;
    p++;
  }
  out += first.slice(last);
  for (; i < args.length; i++) out += ' ' + insp(args[i]);
  return out;
}
const __util = {
  inherits(ctor, superCtor) { if (ctor && ctor.prototype && superCtor && superCtor.prototype) { ctor.super_ = superCtor; Object.setPrototypeOf(ctor.prototype, superCtor.prototype); } },
  promisify: (f) => (...args) => new Promise((res, rej) => { try { f(...args, (e, v) => e ? rej(e) : res(v)); } catch (e) { rej(e); } }),
  callbackify: (f) => (...args) => { const cb = args.pop(); Promise.resolve(f(...args)).then((v) => cb(null, v), (e) => cb(e)); },
  inspect: (x, opts) => __nodeInspect(x, opts),
  format: (...a) => __nodeFormat(...a),
  formatWithOptions: (_o, ...a) => __nodeFormat(...a),
  deprecate: (fn, _msg) => fn,
  debuglog: () => (() => {}),
  isDeepStrictEqual: (a, b) => { try { return JSON.stringify(a) === JSON.stringify(b); } catch (e) { return a === b; } },
  types: { isPromise: (x) => x && typeof x.then === 'function', isDate: (x) => x instanceof Date, isRegExp: (x) => x instanceof RegExp, isNativeError: (x) => x instanceof Error, isArrayBuffer: (x) => x instanceof ArrayBuffer, isTypedArray: (x) => ArrayBuffer.isView(x) },
  TextEncoder: globalThis.TextEncoder,
  TextDecoder: globalThis.TextDecoder,
};
__util.inspect.custom = Symbol.for('nodejs.util.inspect.custom');
// ---- querystring (Node's legacy API, per its docs; verified against Node's results) ----
const __qsUnescape = (s) => {
  try { return decodeURIComponent(s); } catch (e) {
    // malformed %-sequences: decode the valid %XX bytes, keep the rest, then UTF-8 decode
    const bytes = [];
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      if (c === 37 && /^[0-9a-fA-F]{2}$/.test(s.substr(i + 1, 2))) { bytes.push(parseInt(s.substr(i + 1, 2), 16)); i += 2; }
      else if (c < 0x80) bytes.push(c);
      else for (const b of new globalThis.TextEncoder().encode(s[i])) bytes.push(b);
    }
    return __utf8Decode(bytes);
  }
};
const __qsPrim = (v) => {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' && isFinite(v)) return '' + v;
  if (typeof v === 'bigint') return '' + v;
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  return '';
};
const __querystring = {
  unescape: __qsUnescape,
  escape: (s) => encodeURIComponent(typeof s === 'string' ? s : String(s)),
  stringify(obj, sep, eq, options) {
    sep = sep || '&'; eq = eq || '=';
    const enc = (options && typeof options.encodeURIComponent === 'function') ? options.encodeURIComponent : __querystring.escape;
    if (obj === null || typeof obj !== 'object') return '';
    const out = [];
    for (const k of Object.keys(obj)) {
      const v = obj[k];
      const ks = enc(__qsPrim(k)) + eq;
      if (Array.isArray(v)) { for (const x of v) out.push(ks + enc(__qsPrim(x))); }
      else out.push(ks + enc(__qsPrim(v)));
    }
    return out.join(sep);
  },
  parse(qs, sep, eq, options) {
    const obj = Object.create(null);
    if (typeof qs !== 'string' || qs.length === 0) return obj;
    sep = sep || '&'; eq = eq || '=';
    const maxKeys = options && typeof options.maxKeys === 'number' ? options.maxKeys : 1000;
    const dec = (options && typeof options.decodeURIComponent === 'function') ? options.decodeURIComponent : __querystring.unescape;
    let parts = qs.split(sep);
    if (maxKeys > 0 && parts.length > maxKeys) parts = parts.slice(0, maxKeys);
    for (const part of parts) {
      if (part.length === 0) continue;
      const i = part.indexOf(eq);
      const k = dec((i >= 0 ? part.slice(0, i) : part).replace(/\+/g, ' '));
      const v = i >= 0 ? dec(part.slice(i + eq.length).replace(/\+/g, ' ')) : '';
      if (!(k in obj)) obj[k] = v;
      else if (Array.isArray(obj[k])) obj[k].push(v);
      else obj[k] = [obj[k], v];
    }
    return obj;
  },
};
__querystring.decode = __querystring.parse;
__querystring.encode = __querystring.stringify;
// WHATWG UTF-8 decode (one U+FFFD per maximal invalid subpart, like Node's Buffer#toString)
function __utf8Decode(b, start, end) {
  let s = '';
  let i = start || 0;
  end = end === undefined ? b.length : end;
  while (i < end) {
    const c = b[i];
    if (c < 0x80) { s += String.fromCharCode(c); i++; continue; }
    let need = 0, cp = 0, lo = 0x80, hi = 0xbf;
    if (c >= 0xc2 && c <= 0xdf) { need = 1; cp = c & 0x1f; }
    else if (c >= 0xe0 && c <= 0xef) { need = 2; cp = c & 0xf; if (c === 0xe0) lo = 0xa0; if (c === 0xed) hi = 0x9f; }
    else if (c >= 0xf0 && c <= 0xf4) { need = 3; cp = c & 0x7; if (c === 0xf0) lo = 0x90; if (c === 0xf4) hi = 0x8f; }
    else { s += '\ufffd'; i++; continue; }
    let j = i + 1, k = 0;
    for (; k < need && j < end; k++, j++) {
      const x = b[j];
      if (x < lo || x > hi) break;
      lo = 0x80; hi = 0xbf;
      cp = (cp << 6) | (x & 0x3f);
    }
    s += k < need ? '\ufffd' : String.fromCodePoint(cp);
    i = j;
  }
  return s;
}
// Length of the incomplete-but-valid UTF-8 sequence at the end of b (0 if none).
function __utf8Tail(b) {
  for (let back = 1; back <= 3 && back <= b.length; back++) {
    const c = b[b.length - back];
    if ((c & 0xc0) === 0x80) continue; // continuation byte: keep looking for the lead
    const need = c >= 0xc2 && c <= 0xdf ? 1 : c >= 0xe0 && c <= 0xef ? 2 : c >= 0xf0 && c <= 0xf4 ? 3 : 0;
    return need >= back ? back : 0;
  }
  return 0;
}
const __sdEncodings = { utf8: 'utf8', 'utf-8': 'utf8', utf16le: 'utf16le', 'utf-16le': 'utf16le', ucs2: 'utf16le', 'ucs-2': 'utf16le', latin1: 'latin1', binary: 'latin1', ascii: 'ascii', hex: 'hex', base64: 'base64', base64url: 'base64url' };
class __StringDecoder {
  constructor(encoding) {
    const enc = encoding == null ? 'utf8' : __sdEncodings[String(encoding).toLowerCase()];
    if (!enc) { const e = new TypeError('Unknown encoding: ' + encoding); e.code = 'ERR_UNKNOWN_ENCODING'; throw e; }
    this.encoding = enc;
    this._rest = new Uint8Array(0);
  }
  _bytes(buf) {
    const b = typeof buf === 'string' ? new globalThis.TextEncoder().encode(buf) : buf instanceof Uint8Array ? buf : ArrayBuffer.isView(buf) ? new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength) : new Uint8Array(0);
    if (!this._rest.length) return b;
    const all = new Uint8Array(this._rest.length + b.length);
    all.set(this._rest); all.set(b, this._rest.length);
    return all;
  }
  _decode(b, final) {
    switch (this.encoding) {
      case 'utf8': {
        const keep = final ? 0 : __utf8Tail(b);
        this._rest = b.slice(b.length - keep);
        return __utf8Decode(b, 0, b.length - keep);
      }
      case 'utf16le': {
        let n = b.length - (b.length % 2);
        if (!final && n >= 2) { const last = b[n - 2] | (b[n - 1] << 8); if (last >= 0xd800 && last <= 0xdbff) n -= 2; }
        this._rest = final ? new Uint8Array(0) : b.slice(n);
        let s = '';
        for (let i = 0; i < n; i += 2) s += String.fromCharCode(b[i] | (b[i + 1] << 8));
        return s;
      }
      case 'base64':
      case 'base64url': {
        const n = final ? b.length : b.length - (b.length % 3);
        this._rest = b.slice(n);
        let bin = '';
        for (let i = 0; i < n; i++) bin += String.fromCharCode(b[i]);
        const s = n ? globalThis.btoa(bin) : '';
        return this.encoding === 'base64' ? s : s.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
      }
      default: {
        this._rest = new Uint8Array(0);
        let s = '';
        for (const c of b) s += this.encoding === 'hex' ? c.toString(16).padStart(2, '0') : String.fromCharCode(this.encoding === 'ascii' ? c & 0x7f : c);
        return s;
      }
    }
  }
  write(buf) { return this._decode(this._bytes(buf), false); }
  end(buf) { const s = this._decode(this._bytes(buf === undefined ? new Uint8Array(0) : buf), true); this._rest = new Uint8Array(0); return s; }
}
// ---- assert / assert/strict ----
function __fmtv(v) { try { return typeof v === 'string' ? JSON.stringify(v) : typeof v === 'bigint' ? v + 'n' : (v && typeof v === 'object') ? JSON.stringify(v) : String(v); } catch (e) { return String(v); } }
function __deepEq(a, b, strict, seen) {
  if (a === b) return a !== 0 || !strict || Object.is(a, b);
  if (a === null || typeof a !== 'object' || b === null || typeof b !== 'object') {
    if ((a === null || typeof a !== 'object') && (b === null || typeof b !== 'object')) return strict ? Object.is(a, b) : (a == b || (a !== a && b !== b));
    return false;
  }
  if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
  const tag = Object.prototype.toString.call(a);
  if (tag !== Object.prototype.toString.call(b)) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  seen = seen || new Map();
  const memo = seen.get(a);
  if (memo && memo.has(b)) return true;
  if (memo) memo.add(b); else seen.set(a, new Set([b]));
  if (a instanceof Date) { if (a.getTime() !== b.getTime()) return false; }
  else if (a instanceof RegExp) { if (a.source !== b.source || a.flags !== b.flags || a.lastIndex !== b.lastIndex) return false; }
  else if (a instanceof Error) { if (a.message !== b.message || a.name !== b.name) return false; }
  else if (ArrayBuffer.isView(a)) {
    if (a.length !== b.length || a.byteLength !== b.byteLength) return false;
    for (let i = 0; i < a.length; i++) if (strict ? !Object.is(a[i], b[i]) : a[i] != b[i]) return false;
  } else if (tag === '[object Number]' || tag === '[object String]' || tag === '[object Boolean]' || tag === '[object BigInt]' || tag === '[object Symbol]') {
    if (!Object.is(a.valueOf(), b.valueOf())) return false;
  }
  if (a instanceof Map) {
    if (a.size !== b.size) return false;
    for (const [k, v] of a) {
      if (b.has(k)) { if (!__deepEq(v, b.get(k), strict, seen)) return false; continue; }
      // an object key (or, loosely, a primitive one) can still match a deep-equal key
      if (strict && (k === null || typeof k !== 'object')) return false;
      let hit = false;
      for (const [k2, v2] of b) if (__deepEq(k, k2, strict, seen) && __deepEq(v, v2, strict, seen)) { hit = true; break; }
      if (!hit) return false;
    }
  } else if (a instanceof Set) {
    if (a.size !== b.size) return false;
    for (const v of a) {
      if (b.has(v)) continue;
      if (strict && (v === null || typeof v !== 'object')) return false;
      let hit = false;
      for (const v2 of b) if (__deepEq(v, v2, strict, seen)) { hit = true; break; }
      if (!hit) return false;
    }
  }
  const own = (o) => { const ks = Object.keys(o); if (strict) for (const s of Object.getOwnPropertySymbols(o)) if (Object.prototype.propertyIsEnumerable.call(o, s)) ks.push(s); return ks; };
  const ka = own(a), kb = own(b);
  if (ka.length !== kb.length) return false;
  for (const k of ka) {
    if (!Object.prototype.propertyIsEnumerable.call(b, k)) return false;
    if (!__deepEq(a[k], b[k], strict, seen)) return false;
  }
  return true;
}
class __AssertionError extends Error {
  constructor(options) {
    options = options || {};
    super(options.message != null ? String(options.message) : 'Failed');
    this.name = 'AssertionError';
    this.code = 'ERR_ASSERTION';
    this.actual = options.actual;
    this.expected = options.expected;
    this.operator = options.operator;
    this.generatedMessage = options.message == null;
  }
}
function __mkAssert(strict) {
  const fail = (actual, expected, message, operator, generated) => {
    if (message instanceof Error) throw message;
    throw new __AssertionError({ actual, expected, operator, message: message != null ? message : generated });
  };
  const okFn = (...args) => {
    if (args.length === 0) throw new __AssertionError({ message: 'No value argument passed to `assert.ok()`', operator: '==' });
    if (!args[0]) fail(args[0], true, args[1], '==', 'The expression evaluated to a falsy value:\n\n  assert.ok(' + __fmtv(args[0]) + ')\n');
  };
  const looseEq = (a, b) => a == b || (a !== a && b !== b);
  // error validation for throws/rejects: a class, a RegExp, a validation function or an object
  const matches = (err, expected, message, operator) => {
    if (expected === undefined) return;
    if (typeof expected === 'function') {
      if (expected.prototype !== undefined && err instanceof expected) return;
      if (Error.isPrototypeOf(expected) || expected === Error) fail(err, expected, message, operator, 'The error is expected to be an instance of "' + expected.name + '". Received "' + (err && err.constructor && err.constructor.name) + '"');
      const res = expected.call({}, err);
      if (res !== true) fail(err, expected, message, operator, 'The validation function is expected to return "true". Received ' + __fmtv(res));
      return;
    }
    if (expected instanceof RegExp) { if (expected.test(String(err))) return; fail(err, expected, message, operator, 'The input did not match the regular expression ' + String(expected) + '. Input:\n\n' + __fmtv(String(err)) + '\n'); }
    if (expected && typeof expected === 'object') {
      for (const k of Object.keys(expected)) {
        const want = expected[k], got = err == null ? undefined : err[k];
        const ok = want instanceof RegExp && typeof got === 'string' ? want.test(got) : __deepEq(got, want, true);
        if (!ok) fail(err, expected, message, operator, 'Expected values to be strictly deep-equal: ' + k + ' ' + __fmtv(got) + ' !== ' + __fmtv(want));
      }
    }
  };
  // doesNotThrow / doesNotReject: an error that does not match `expected` propagates as is
  const unwanted = (e, expected, message, operator, what) => {
    if (expected !== undefined) {
      if (typeof expected !== 'function' && !(expected instanceof RegExp)) { const t = new TypeError('The "expected" argument must be of type function or an instance of RegExp'); t.code = 'ERR_INVALID_ARG_TYPE'; throw t; }
      const hit = expected instanceof RegExp ? expected.test(String(e)) : (expected.prototype !== undefined && e instanceof expected) || (!Error.isPrototypeOf(expected) && expected !== Error && expected.call({}, e) === true);
      if (!hit) throw e;
    }
    fail(e, expected, message, operator, 'Got unwanted ' + what + (message ? ': ' + message : '.') + '\nActual message: "' + (e && e.message) + '"');
  };
  const split = (expected, message) => (typeof expected === 'string' && message === undefined ? [undefined, expected] : [expected, message]);
  const a = function assert(...args) { okFn(...args); };
  Object.assign(a, {
    ok: okFn,
    fail: (message) => { if (message instanceof Error) throw message; throw new __AssertionError({ message: message === undefined ? 'Failed' : message, operator: 'fail' }); },
    equal: (x, y, m) => { if (!(strict ? Object.is(x, y) : looseEq(x, y))) fail(x, y, m, strict ? 'strictEqual' : '==', __fmtv(x) + (strict ? ' !== ' : ' == ') + __fmtv(y)); },
    notEqual: (x, y, m) => { if (strict ? Object.is(x, y) : looseEq(x, y)) fail(x, y, m, strict ? 'notStrictEqual' : '!=', __fmtv(x) + (strict ? ' === ' : ' != ') + __fmtv(y)); },
    strictEqual: (x, y, m) => { if (!Object.is(x, y)) fail(x, y, m, 'strictEqual', 'Expected values to be strictly equal: ' + __fmtv(x) + ' !== ' + __fmtv(y)); },
    notStrictEqual: (x, y, m) => { if (Object.is(x, y)) fail(x, y, m, 'notStrictEqual', 'Expected "actual" to be strictly unequal to: ' + __fmtv(y)); },
    deepEqual: (x, y, m) => { if (!__deepEq(x, y, strict)) fail(x, y, m, strict ? 'deepStrictEqual' : 'deepEqual', 'Expected values to be ' + (strict ? 'strictly ' : 'loosely ') + 'deep-equal: ' + __fmtv(x) + ' vs ' + __fmtv(y)); },
    notDeepEqual: (x, y, m) => { if (__deepEq(x, y, strict)) fail(x, y, m, strict ? 'notDeepStrictEqual' : 'notDeepEqual', 'Expected "actual" not to be ' + (strict ? 'strictly ' : 'loosely ') + 'deep-equal to: ' + __fmtv(y)); },
    deepStrictEqual: (x, y, m) => { if (!__deepEq(x, y, true)) fail(x, y, m, 'deepStrictEqual', 'Expected values to be strictly deep-equal: ' + __fmtv(x) + ' vs ' + __fmtv(y)); },
    notDeepStrictEqual: (x, y, m) => { if (__deepEq(x, y, true)) fail(x, y, m, 'notDeepStrictEqual', 'Expected "actual" not to be strictly deep-equal to: ' + __fmtv(y)); },
    match: (s, re, m) => { if (typeof s !== 'string' || !re.test(s)) fail(s, re, m, 'match', 'The input did not match the regular expression ' + String(re) + '. Input:\n\n' + __fmtv(s) + '\n'); },
    doesNotMatch: (s, re, m) => { if (typeof s !== 'string' || re.test(s)) fail(s, re, m, 'doesNotMatch', 'The input was expected to not match the regular expression ' + String(re) + '. Input:\n\n' + __fmtv(s) + '\n'); },
    ifError: (v) => { if (v !== null && v !== undefined) throw new __AssertionError({ actual: v, expected: null, operator: 'ifError', message: 'ifError got unwanted exception: ' + (v instanceof Error ? v.message : __fmtv(v)) }); },
    throws: (fn, expected, message) => {
      [expected, message] = split(expected, message);
      let threw = false, err;
      try { fn(); } catch (e) { threw = true; err = e; }
      if (!threw) fail(undefined, expected, message, 'throws', 'Missing expected exception' + (message ? ': ' + message : '.'));
      matches(err, expected, message, 'throws');
    },
    doesNotThrow: (fn, expected, message) => {
      [expected, message] = split(expected, message);
      try { fn(); } catch (e) { unwanted(e, expected, message, 'doesNotThrow', 'exception'); }
    },
    rejects: async (p, expected, message) => {
      [expected, message] = split(expected, message);
      let threw = false, err;
      try { await (typeof p === 'function' ? p() : p); } catch (e) { threw = true; err = e; }
      if (!threw) fail(undefined, expected, message, 'rejects', 'Missing expected rejection' + (message ? ': ' + message : '.'));
      matches(err, expected, message, 'rejects');
    },
    doesNotReject: async (p, expected, message) => {
      [expected, message] = split(expected, message);
      try { await (typeof p === 'function' ? p() : p); } catch (e) { unwanted(e, expected, message, 'doesNotReject', 'rejection'); }
    },
    AssertionError: __AssertionError,
  });
  return a;
}
const __assert = __mkAssert(false);
const __assertStrict = __mkAssert(true);
__assert.strict = __assertStrict;
__assertStrict.strict = __assertStrict;
// ---- buffer / fs/promises / process / timers ----
const __buffer = {
  Buffer: globalThis.Buffer,
  atob: (s) => globalThis.atob(s),
  btoa: (s) => globalThis.btoa(s),
  get Blob() { return globalThis.Blob; },
  get File() { return globalThis.File; },
};
// fs.constants: the values that are the same on every platform turbo-test ships for (no O_CREAT
// etc., which differ between Linux and macOS)
const __fsConstants = {
  F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1, O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2,
  S_IFMT: 0o170000, S_IFREG: 0o100000, S_IFDIR: 0o040000, S_IFLNK: 0o120000,
  COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4,
};
__fs.constants = __fsConstants;
__fs.promises.constants = __fsConstants;
// os.constants: libuv's priorities everywhere; POSIX signal / errno numbers that Linux and macOS
// share (left empty on Windows, whose numbering differs)
__os.constants = {
  priority: { PRIORITY_LOW: 19, PRIORITY_BELOW_NORMAL: 10, PRIORITY_NORMAL: 0, PRIORITY_ABOVE_NORMAL: -7, PRIORITY_HIGH: -14, PRIORITY_HIGHEST: -20 },
  signals: {},
  errno: {},
};
if ((globalThis.__ttPlatform || 'darwin') !== 'win32') {
  __os.constants.signals = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGILL: 4, SIGTRAP: 5, SIGABRT: 6, SIGIOT: 6, SIGFPE: 8, SIGKILL: 9, SIGSEGV: 11, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 };
  const names = ['EPERM', 'ENOENT', 'ESRCH', 'EINTR', 'EIO', 'ENXIO', 'E2BIG', 'ENOEXEC', 'EBADF', 'ECHILD', null, 'ENOMEM', 'EACCES', 'EFAULT', 'ENOTBLK', 'EBUSY', 'EEXIST', 'EXDEV', 'ENODEV', 'ENOTDIR', 'EISDIR', 'EINVAL', 'ENFILE', 'EMFILE', 'ENOTTY', 'ETXTBSY', 'EFBIG', 'ENOSPC', 'ESPIPE', 'EROFS', 'EMLINK', 'EPIPE', 'EDOM', 'ERANGE'];
  names.forEach((n, i) => { if (n) __os.constants.errno[n] = i + 1; });
}
const __constants = Object.assign({}, __os.constants.errno, __os.constants.priority, __os.constants.signals, __fsConstants, __crypto.constants);
// the global timers (vi.useFakeTimers drives these same functions)
const __abortError = (signal) => { const e = new Error('The operation was aborted'); e.name = 'AbortError'; e.code = 'ABORT_ERR'; if (signal && signal.reason !== undefined) e.cause = signal.reason; return e; };
const __timerPromise = (schedule, clear, value, options) => new Promise((resolve, reject) => {
  const signal = options && options.signal;
  if (signal && signal.aborted) { reject(__abortError(signal)); return; }
  const onAbort = () => { clear(id); reject(__abortError(signal)); };
  const id = schedule(() => { if (signal) signal.removeEventListener('abort', onAbort); resolve(value); });
  if (signal) signal.addEventListener('abort', onAbort);
});
const __timersPromises = {
  setTimeout: (delay, value, options) => __timerPromise((cb) => globalThis.setTimeout(cb, delay), (id) => globalThis.clearTimeout(id), value, options),
  setImmediate: (value, options) => __timerPromise((cb) => globalThis.setImmediate(cb), (id) => globalThis.clearImmediate(id), value, options),
  // async iterator yielding `value` once per tick (ticks the consumer hasn't taken yet queue up)
  setInterval: (delay, value, options) => {
    const signal = options && options.signal;
    let ticks = 0, wake = null, done = false, error = null;
    const notify = () => { if (wake) { const w = wake; wake = null; w(); } };
    const id = globalThis.setInterval(() => { ticks++; notify(); }, delay);
    const finish = () => { done = true; globalThis.clearInterval(id); if (signal) signal.removeEventListener('abort', onAbort); };
    const onAbort = () => { error = __abortError(signal); finish(); notify(); };
    if (signal) { if (signal.aborted) onAbort(); else signal.addEventListener('abort', onAbort); }
    return {
      [Symbol.asyncIterator]() { return this; },
      async next() {
        while (!ticks && !done) await new Promise((r) => { wake = r; });
        if (error) { const e = error; error = null; throw e; }
        if (done) return { done: true, value: undefined };
        ticks--;
        return { done: false, value };
      },
      async return() { finish(); return { done: true, value: undefined }; },
    };
  },
  scheduler: {
    wait: (delay, options) => __timersPromises.setTimeout(delay, undefined, options),
    yield: () => __timersPromises.setImmediate(),
  },
};
const __timers = {
  get setTimeout() { return globalThis.setTimeout; },
  get clearTimeout() { return globalThis.clearTimeout; },
  get setInterval() { return globalThis.setInterval; },
  get clearInterval() { return globalThis.clearInterval; },
  get setImmediate() { return globalThis.setImmediate; },
  get clearImmediate() { return globalThis.clearImmediate; },
  promises: __timersPromises,
};
// ---- vm / worker_threads / inspector: what one V8 context in one thread can truthfully offer;
// the rest throws a clear error instead of pretending ----
const __unavailable = (what, why) => () => { throw new Error('turbo-test: ' + what + ' is not available (' + why + ')'); };
const __oneContext = 'tests run in a single V8 context';
class __VmScript {
  constructor(code) { this._code = String(code); }
  runInThisContext() { return (0, eval)(this._code); }
}
__VmScript.prototype.runInContext = __unavailable('vm.Script.runInContext', __oneContext);
__VmScript.prototype.runInNewContext = __unavailable('vm.Script.runInNewContext', __oneContext);
const __vm = {
  Script: __VmScript,
  runInThisContext: (code) => (0, eval)(String(code)),
  compileFunction: (code, params) => new Function(...(params || []), String(code)),
  isContext: (o) => {
    if (o === null || typeof o !== 'object') { const e = new TypeError('The "object" argument must be of type object'); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
    return false; // no other contexts can exist here
  },
  createContext: __unavailable('vm.createContext', __oneContext),
  runInContext: __unavailable('vm.runInContext', __oneContext),
  runInNewContext: __unavailable('vm.runInNewContext', __oneContext),
};
const __envData = new Map();
const __untransferable = new WeakSet();
const __worker_threads = {
  isMainThread: true,
  parentPort: null,
  threadId: 0,
  workerData: null,
  resourceLimits: {},
  SHARE_ENV: Symbol.for('nodejs.worker_threads.SHARE_ENV'),
  Worker: class Worker { constructor() { __unavailable('worker_threads.Worker', 'tests run on one thread')(); } },
  get MessageChannel() { return globalThis.MessageChannel; },
  get MessagePort() { return globalThis.MessagePort; },
  getEnvironmentData: (k) => __envData.get(k),
  setEnvironmentData: (k, v) => { if (v === undefined) __envData.delete(k); else __envData.set(k, v); },
  markAsUntransferable: (o) => { if (o !== null && (typeof o === 'object' || typeof o === 'function')) __untransferable.add(o); },
  isMarkedAsUntransferable: (o) => o !== null && (typeof o === 'object' || typeof o === 'function') && __untransferable.has(o),
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
globalThis.__nodeBuiltins = {
  fs: __fs,
  'node:fs': __fs,
  path: __path,
  'node:path': __path,
  module: __module,
  'node:module': __module,
  os: __os,
  'node:os': __os,
  child_process: { execSync: () => '', exec: () => {}, spawnSync: () => ({ status: 0 }) },
  'node:child_process': { execSync: () => '' },
  url: { fileURLToPath: globalThis.__fileURLToPath, pathToFileURL: globalThis.__pathToFileURL, URL: globalThis.URL },
  util: __util,
  'node:util': __util,
  crypto: __crypto,
  'node:crypto': __crypto,
  events: __events,
  'node:events': __events,
  stream: __stream,
  'node:stream': __stream,
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
  http2: __http2,
  'node:http2': __http2,
  dgram: __dgram,
  'node:dgram': __dgram,
  readline: __readline,
  'node:readline': __readline,
  v8: __v8,
  'node:v8': __v8,
  diagnostics_channel: __dc,
  'node:diagnostics_channel': __dc,
  async_hooks: __async_hooks,
  'node:async_hooks': __async_hooks,
  assert: __assert,
  'assert/strict': __assertStrict,
  perf_hooks: { performance: globalThis.performance || { now: () => 0 } },
  buffer: __buffer,
  'fs/promises': __fs.promises,
  get process() { return globalThis.process; },
  timers: __timers,
  'timers/promises': __timersPromises,
  vm: __vm,
  worker_threads: __worker_threads,
  querystring: __querystring,
  string_decoder: { StringDecoder: __StringDecoder },
  constants: __constants,
  inspector: __inspector,
};

// ---- loader helpers (used by the CJS wrapper) ----
globalThis.__keys = (o) =>
  o && (typeof o === 'object' || typeof o === 'function') ? Object.keys(o) : [];
// Node's MODULE_NOT_FOUND error for `spec`: the message lists the require stack (the requiring
// module, the modules loading it, the test file)
globalThis.__ttModuleNotFound = (spec, importer) => {
  const stack = [];
  const add = (p) => { if (p && !stack.includes(p)) stack.push(p); };
  add(importer);
  for (const p of globalThis.__ttRequireStack()) add(p);
  add(globalThis.__ttFile);
  const e = new Error(`Cannot find module '${spec}'` + (stack.length ? `\nRequire stack:\n- ${stack.join('\n- ')}` : ''));
  e.code = 'MODULE_NOT_FOUND';
  e.requireStack = stack;
  return e;
};
// require.cache, shared by every require (Node's Module._cache): a module loaded from a file,
// by its path; deleting one makes the next require() load it again
const __requireCacheEntries = new Map();
function __requireCacheEntry(path) {
  if (globalThis.__ttCache('get', path) === undefined) { __requireCacheEntries.delete(path); return undefined; }
  let e = __requireCacheEntries.get(path);
  if (!e) {
    e = { id: path, filename: path, loaded: true, children: [], paths: [] };
    Object.defineProperty(e, 'exports', {
      get: () => globalThis.__ttCache('get', path),
      set: (v) => globalThis.__ttCache('set', path, v),
      enumerable: true,
      configurable: true,
    });
    __requireCacheEntries.set(path, e);
  }
  return e;
}
const __requireCache = new Proxy({}, {
  get: (_t, k) => (typeof k === 'string' ? __requireCacheEntry(k) : undefined),
  has: (_t, k) => typeof k === 'string' && __requireCacheEntry(k) !== undefined,
  set: (_t, k, v) => { if (typeof k === 'string') globalThis.__ttCache('set', k, v && v.exports); return true; },
  deleteProperty: (_t, k) => {
    if (typeof k === 'string') { globalThis.__ttCache('delete', k); __requireCacheEntries.delete(k); }
    return true;
  },
  ownKeys: () => globalThis.__ttCache('keys'),
  getOwnPropertyDescriptor: (_t, k) => {
    const e = typeof k === 'string' ? __requireCacheEntry(k) : undefined;
    return e && { value: e, writable: true, enumerable: true, configurable: true };
  },
});
globalThis.__mkRequire = (dir, isEsm, importer) => {
  const req = (spec) => globalThis.__nativeRequire(dir, spec, isEsm, importer);
  // as Node: the resolved path (a builtin stays its name); MODULE_NOT_FOUND when nothing resolves
  req.resolve = (spec) => {
    const r = globalThis.__ttRequireResolve(dir, String(spec), isEsm === undefined ? true : !!isEsm);
    if (r === undefined) throw globalThis.__ttModuleNotFound(String(spec), importer);
    return r;
  };
  req.cache = __requireCache;
  req.extensions = {};
  req.main = undefined;
  return req;
};

// ---- asymmetric matchers (expect.objectContaining et al) ----
class Asymmetric {
  constructor(kind, sample, negate) { this.__asymmetric = kind; this.sample = sample; this.__negate = !!negate; }
  matches(actual) { return this.__negate ? !this.__matchesInner(actual) : this.__matchesInner(actual); }
  __matchesInner(actual) {
    switch (this.__asymmetric) {
      case 'any':
        if (this.sample === String) return typeof actual === 'string';
        if (this.sample === Number) return typeof actual === 'number';
        if (this.sample === Boolean) return typeof actual === 'boolean';
        if (this.sample === Function) return typeof actual === 'function';
        if (this.sample === Object) return typeof actual === 'object' && actual !== null;
        if (this.sample === Array) return Array.isArray(actual);
        return actual instanceof this.sample;
      case 'anything': return actual !== null && actual !== undefined;
      case 'objectContaining':
        return actual && typeof actual === 'object' &&
          Object.keys(this.sample).every((k) => deepEqual(actual[k], this.sample[k]));
      case 'arrayContaining':
        return Array.isArray(actual) && this.sample.every((s) => actual.some((a) => deepEqual(a, s)));
      case 'stringContaining': return typeof actual === 'string' && actual.includes(this.sample);
      case 'stringMatching': {
        const r = this.sample instanceof RegExp ? this.sample : new RegExp(this.sample);
        return typeof actual === 'string' && r.test(actual);
      }
      default: return false;
    }
  }
}

// ---- deep equality (asymmetric-aware) ----
// `strict` mirrors jest/vitest: toEqual/toHaveBeenCalledWith (strict=false) treat an own
// property whose value is `undefined` as ABSENT ({a:1,b:undefined} equals {a:1}); toStrictEqual
// (strict=true) keeps undefined-valued keys significant. Array indices are NEVER stripped in
// either mode — a hole/undefined element changes length, so [1,undefined] != [1].
function deepEqual(a, b, strict = false) {
  if (b instanceof Asymmetric) return b.matches(a);
  if (a instanceof Asymmetric) return a.matches(b);
  if (Object.is(a, b)) return true;
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i], strict)) return false;
    return true;
  }
  if (strict) {
    const ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => (k in b) && deepEqual(a[k], b[k], true));
  }
  // non-strict: drop own keys whose value is `undefined` on both sides before comparing.
  const ka = Object.keys(a).filter((k) => a[k] !== undefined);
  const kb = Object.keys(b).filter((k) => b[k] !== undefined);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEqual(a[k], b[k], false));
}

function fmt(v) {
  try { return typeof v === 'string' ? JSON.stringify(v) : (Array.isArray(v) || (v && typeof v === 'object')) ? JSON.stringify(v) : String(v); }
  catch { return String(v); }
}

// ---- snapshots ----
// Per-test state set by runSuite before each test: the full `describe > it` name and a
// per-(test-name) counter so multiple toMatchSnapshot() calls in one test get distinct keys
// (`<full name> 1`, `<full name> 2`, …), matching vitest's keying.
const __snap = {
  testName: '',
  counters: Object.create(null), // testName -> next index
  // assertion enforcement (expect.assertions / hasAssertions); reset per test.
  assertCount: 0,
  expectedAssertions: null, // number | null
  hasAssertions: false,
};
function __snapNextKey(name) {
  const n = (__snap.counters[name] || 0) + 1;
  __snap.counters[name] = n;
  return `${name} ${n}`;
}
function __snapFile() {
  const f = globalThis.__ttFile;
  if (!f) return null;
  const slash = f.lastIndexOf('/');
  const dir = slash >= 0 ? f.slice(0, slash) : '.';
  const base = slash >= 0 ? f.slice(slash + 1) : f;
  return { dir: `${dir}/__snapshots__`, path: `${dir}/__snapshots__/${base}.snap` };
}
// vitest's snapshot serializer (turbo-test#73): pretty-format with vitest's snapshotFormat
// (printBasicPrototype false, escapeString false), printFunctionName false, escapeRegex true and
// the mock-function serializer; CRLF normalized.
function __serialize(v) {
  return __pf(v, { indent: 2, basic: false, escape: false, fnName: false, escapeRegex: true, mocks: true }).replace(/\r\n|\r/g, '\n');
}
// Read a .snap file (vitest's or turbo-test's older one: both are `exports[...] = ...;` JS).
function __parseSnapFile(text) {
  const out = Object.create(null);
  try { new Function('exports', text)(out); } catch (e) {}
  return out;
}
// natural-compare-lite, the order vitest writes snapshot keys in.
function __naturalCompare(a, b) {
  let i, codeA, codeB = 1, posA = 0, posB = 0;
  const code = (str, pos, num) => {
    if (num) {
      for (i = pos; (num = code(str, i)), num < 76 && num > 65;) ++i;
      return +str.slice(pos - 1, i);
    }
    const c = str.charCodeAt(pos) || 0;
    return c < 45 || c > 127 ? c : c < 46 ? 65 : c < 48 ? c - 1 : c < 58 ? c + 18 : c < 65 ? c - 11 : c < 91 ? c + 11 : c < 97 ? c - 37 : c < 123 ? c + 5 : c - 63;
  };
  if ((a += '') !== (b += '')) {
    for (; codeB;) {
      codeA = code(a, posA++);
      codeB = code(b, posB++);
      if (codeA < 76 && codeB < 76 && codeA > 66 && codeB > 66) {
        codeA = code(a, posA, posA);
        codeB = code(b, posB, (posA = i));
        posB = i;
      }
      if (codeA !== codeB) return codeA < codeB ? -1 : 1;
    }
  }
  return 0;
}
function __serializeSnapFile(map) {
  const bt = (str) => `\`${str.replace(/`|\\|\$\{/g, '\\$&')}\``;
  const entries = Object.keys(map).sort(__naturalCompare).map((k) => `exports[${bt(k)}] = ${bt(map[k].replace(/\r\n|\r/g, '\n'))};`);
  return `// Vitest Snapshot v1, https://vitest.dev/guide/snapshot.html\n\n${entries.join('\n\n')}\n`;
}
// a multi-line snapshot is stored with a line break at either end (vitest addExtraLineBreaks)
function __snapBody(serialized) { return serialized.includes('\n') ? `\n${serialized}\n` : serialized; }
// vitest deepMergeSnapshot: the property matchers replace the received values they name
function __snapMerge(target, source) {
  const isObj = (x) => x != null && typeof x === 'object' && !Array.isArray(x) && !(x instanceof Asymmetric);
  if (Array.isArray(target) && Array.isArray(source)) {
    const out = Array.from(target);
    source.forEach((s, i) => { out[i] = isObj(s) && isObj(out[i]) ? __snapMerge(out[i], s) : Array.isArray(s) ? __snapMerge(out[i], s) : s; });
    return out;
  }
  if (!isObj(target) || !isObj(source)) return source === undefined ? target : source;
  const out = { ...target };
  for (const k of Object.keys(source)) {
    out[k] = isObj(source[k]) && isObj(target[k]) ? __snapMerge(target[k], source[k]) : Array.isArray(source[k]) ? __snapMerge(target[k], source[k]) : source[k];
  }
  return out;
}

// The file's snapshot state, as @vitest/snapshot's SnapshotState (turbo-test#73): the update mode
// (`all` under -u, `none` on CI — a missing snapshot fails —, else `new`), the snapshot data read
// once, the keys not yet checked (obsolete at the end of the file), and the counts the JSON
// report's `snapshot` summary adds up. The file is written once, when the file's tests are done.
const __snapState = { loaded: false };
function __snapMode() {
  return globalThis.__TT_UPDATE_SNAPSHOTS ? 'all' : globalThis.__TT_CI ? 'none' : 'new';
}
function __snapLoad() {
  const st = __snapState;
  if (st.loaded) return st;
  const sf = __snapFile();
  st.loaded = true;
  st.sf = sf;
  st.exists = !!sf && globalThis.__fs_existsSync(sf.path);
  st.data = st.exists ? __parseSnapFile(globalThis.__fs_readFileSync(sf.path, 'utf8')) : Object.create(null);
  st.unchecked = new Set(Object.keys(st.data));
  st.dirty = false;
  st.inline = false;
  st.inlineWrites = [];
  st.raw = false;
  st.added = 0; st.matched = 0; st.unmatched = 0; st.updated = 0;
  return st;
}
// @vitest/snapshot `_reconcile`: whether the snapshot passes, counting it and adding / updating
// it (`add`) when the mode allows.
function __snapReconcile(st, pass, hasSnapshot, add) {
  const mode = __snapMode();
  if ((hasSnapshot && mode === 'all') || (!hasSnapshot && mode !== 'none')) {
    if (mode === 'all') {
      if (!pass) { if (hasSnapshot) st.updated++; else st.added++; add(); }
      else st.matched++;
    } else { add(); st.added++; }
    return true;
  }
  if (!pass) { st.unmatched++; return false; }
  st.matched++;
  return true;
}
// @vitest/snapshot stripSnapshotIndentation
function __stripSnapIndent(snap) {
  const m = snap.match(/^([^\S\n]*)\S/m);
  if (!m || !m[1]) return snap;
  const indent = m[1];
  const lines = snap.split(/\n/g);
  if (lines.length <= 2 || lines[0].trim() !== '' || lines[lines.length - 1].trim() !== '') return snap;
  for (let i = 1; i < lines.length - 1; i++) {
    if (lines[i] !== '') {
      if (lines[i].indexOf(indent) !== 0) return snap;
      lines[i] = lines[i].substring(indent.length);
    }
  }
  lines[lines.length - 1] = '';
  return lines.join('\n');
}
// @vitest/snapshot prepareSnapString: the snapshot as a template literal indented like its line
function __snapLiteral(snap, code, index) {
  const lineStart = code.lastIndexOf('\n', index - 1) + 1;
  const indent = code.slice(lineStart).match(/^\s*/)[0].split('\n')[0];
  const next = indent.includes('\t') ? `${indent}\t` : `${indent}  `;
  const lines = snap.trim().replace(/\\/g, '\\\\').split(/\n/g);
  const esc = (x) => x.replace(/`/g, '\\`').replace(/\$\{/g, '\\${');
  if (lines.length <= 1) return `\`${esc(lines.join('\n'))}\``;
  return `\`\n${esc(lines.map((l) => (l ? next + l : '')).join('\n'))}\n${indent}\``;
}
// @vitest/snapshot replaceInlineSnap / replaceObjectSnap on `code` at `index` (the method name):
// the splice [start, end, text] that writes `snap` as the call's snapshot argument.
function __inlineSplice(code, index, snap, name) {
  const at = code.slice(index);
  const ws = String.raw`\s*\(\s*(?:\/\*[\s\S]*\*\/\s*|\/\/.*(?:[\n\r  ]\s*|[\t\v\f \xA0  -   　﻿]))*`;
  const start = new RegExp(`${name}${ws}[\\w$]*(['"\`)])`).exec(at);
  const first = at.indexOf(name);
  if (!start || start.index !== first) {
    // toMatchInlineSnapshot({ …properties }) [, `…`]
    const obj = new RegExp(`${name}${ws}\\{`).exec(at);
    if (!obj) return null;
    let depth = 0, i = obj.index + obj[0].length - 1, end = -1;
    for (; i < at.length; i++) { if (at[i] === '{') depth++; else if (at[i] === '}' && --depth === 0) { end = i + 1; break; } }
    let callEnd = -1;
    for (let j = obj.index + name.length, d = 0; j < at.length; j++) { if (at[j] === '(') d++; else if (at[j] === ')' && --d === 0) { callEnd = j; break; } }
    if (end < 0 || callEnd < 0) return null;
    const lit = `, ${__snapLiteral(snap, code, index)}`;
    return end === callEnd ? [index + callEnd, index + callEnd, lit] : [index + end, index + callEnd, lit];
  }
  const quote = start[1];
  const startIndex = index + start.index + start[0].length;
  const lit = __snapLiteral(snap, code, index);
  if (quote === ')') return [startIndex - 1, startIndex - 1, lit];
  const endMatch = new RegExp(`(?:^|[^\\\\])${quote}`).exec(code.slice(startIndex));
  if (!endMatch) return null;
  return [startIndex - 1, startIndex + endMatch.index + endMatch[0].length, lit];
}
// Write the file's new / updated inline snapshots back into its source (vitest saveInlineSnapshots).
function __writeInlineSnapshots(writes) {
  const file = globalThis.__ttFile;
  if (!writes.length || !file) return;
  const code = globalThis.__fs_readFileSync(file, 'utf8');
  const starts = [0];
  for (let i = 0; i < code.length; i++) if (code[i] === '\n') starts.push(i + 1);
  // UTF-16 columns are string indices in JS
  const splices = writes
    .map((w) => __inlineSplice(code, starts[w.line - 1] + w.column - 1, w.snapshot, w.name))
    .filter(Boolean)
    .sort((x, y) => y[0] - x[0]);
  let out = code;
  for (const [a, b, text] of splices) out = out.slice(0, a) + text + out.slice(b);
  if (out !== code) globalThis.__fs_writeFileSync(file, out);
}
function __snapMismatch(key, expected, actual) {
  const err = new Error(`Snapshot \`${key}\` mismatched`);
  err.expected = expected;
  err.actual = actual;
  return err;
}
// End of the file: mark the snapshots of skipped tests checked, drop the obsolete ones under -u,
// save (or, under -u, delete an emptied file) and report the counts.
function __snapFinish(skipped) {
  const st = __snapState;
  if (!st.loaded) {
    const sf = __snapFile();
    if (!sf || !globalThis.__fs_existsSync(sf.path)) return undefined;
    __snapLoad();
  }
  for (const name of skipped) {
    for (const k of st.unchecked) if (k.startsWith(name) && / \d+$| > /.test(k.slice(name.length))) st.unchecked.delete(k);
  }
  const mode = __snapMode();
  const uncheckedKeys = [...st.unchecked];
  if (uncheckedKeys.length && mode === 'all') {
    for (const k of uncheckedKeys) delete st.data[k];
    st.dirty = true;
    st.unchecked.clear();
  }
  __writeInlineSnapshots(st.inlineWrites);
  const external = Object.keys(st.data).length;
  let deleted = false;
  if ((st.dirty || st.unchecked.size) && (external || st.inline || st.raw)) {
    if (external) {
      globalThis.__fs_mkdirSync(st.sf.dir);
      globalThis.__fs_writeFileSync(st.sf.path, __serializeSnapFile(st.data));
    }
  } else if (!external && st.exists) {
    if (mode === 'all') globalThis.__fs_rmSync(st.sf.path);
    deleted = true;
  }
  return {
    added: st.added, matched: st.matched, unmatched: st.unmatched, updated: st.updated,
    unchecked: deleted ? 0 : uncheckedKeys.length, unchecked_keys: uncheckedKeys, file_deleted: deleted,
  };
}

// ---- expect ----
function makeExpect(actual, negated = false) {
  // `msg` is a thunk: the failure message (and the formatting of the operands in it) is built
  // only when the assertion fails, as vitest does — formatting can throw (a native class
  // prototype whose members check their receiver) and must not fail a passing assertion.
  const ok = (pass, msg) => {
    if (negated ? pass : !pass) throw new __ChaiAssertionError(msg());
  };
  // chai's assert with vitest's templates (turbo-test#71)
  const c = (pass, msg, notMsg, exp, act = actual) => __chaiAssert(negated, pass, msg, notMsg, actual, exp, act);
  const spyOf = () => {
    if (!(typeof actual === 'function' && actual._isMockFunction === true)) throw new TypeError(`${__inspect(actual)} is not a spy or a call to a spy!`);
    return actual;
  };
  const numbers = (x, label) => {
    if (typeof x !== 'number' && typeof x !== 'bigint') throw new TypeError(`${label} value must be number or bigint, received "${typeof x}"`);
  };
  const argsEqual = (a, b) => a.length === b.length && a.every((x, i) => deepEqual(x, b[i]));
  const api = {
    get not() { return makeExpect(actual, !negated); },
    toBeTypeOf(t) { c(typeof actual === t, 'expected #{this} to be type of #{exp}', 'expected #{this} not to be type of #{exp}', t, typeof actual); },
    // a jest-style matcher in vitest: a plain Error with its matcher hint
    toSatisfy(fn, message) {
      const pass = !!fn(actual);
      if (negated ? !pass : pass) return;
      const shown = message || __stringify(fn);
      throw new Error(negated
        ? `expect(received).not.toSatisfy()\n\nExpected value to not satisfy:\n${shown}\nReceived:\n${__stringify(actual)}`
        : `expect(received).toSatisfy()\n\nExpected value to satisfy:\n${shown}\n\nReceived:\n${__stringify(actual)}`);
    },
    toHaveBeenCalledExactlyOnceWith(...a) {
      const spy = spyOf();
      const name = spy.getMockName();
      const pass = spy.mock.calls.length === 1 && spy.mock.calls.some((x) => argsEqual(x, a));
      if (negated ? !pass : pass) return;
      const msg = __chaiMsg(negated ? `expected "${name}" to not be called once with arguments: #{exp}` : `expected "${name}" to be called once with arguments: #{exp}`, actual, a, actual);
      throw new __ChaiAssertionError(__formatCalls(spy, msg, a));
    },
    // async matchers: `await expect(promise).resolves.toX(...)`, `.rejects.toX(...)`, and
    // `.resolves.not.toX(...)` / `.rejects.not.toX(...)` (the `.not` returns a negated proxy).
    get resolves() {
      const mk = (neg) => new Proxy({}, { get: (_, m) => {
        if (m === 'not') return mk(!neg);
        return (...args) => Promise.resolve(actual).then(
          (v) => makeExpect(v, neg)[m](...args),
          (e) => { const err = new __ChaiAssertionError(`promise rejected "${__inspect(e)}" instead of resolving`, { showDiff: false }); err.cause = e; throw err; },
        );
      } });
      return mk(negated);
    },
    get rejects() {
      const mk = (neg) => new Proxy({}, { get: (_, m) => {
        if (m === 'not') return mk(!neg);
        return (...args) => Promise.resolve(actual).then(
          (v) => { throw new __ChaiAssertionError(`promise resolved "${__inspect(v)}" instead of rejecting`, { actual: v, expected: new Error('rejected promise') }); },
          // `.rejects.toThrow()` checks the rejection as a thrown error (vitest wraps it in a thrower)
          (e) => (m === 'toThrow' || m === 'toThrowError' ? makeExpect(() => { throw e; }, neg) : makeExpect(e, neg))[m](...args),
        );
      } });
      return mk(negated);
    },
    toBe(e) {
      const pass = Object.is(actual, e);
      let msg = 'expected #{this} to be #{exp} // Object.is equality';
      if (!pass && !negated) {
        const deep = deepEqual(actual, e, true) ? 'toStrictEqual' : deepEqual(actual, e) ? 'toEqual' : '';
        if (deep) msg += `\n\nIf it should pass with deep equality, replace "toBe" with "${deep}"\n\nExpected: #{this}\nReceived: serializes to the same string\n`;
      }
      c(pass, msg, 'expected #{this} not to be #{exp} // Object.is equality', e);
    },
    toEqual(e) { c(deepEqual(actual, e), 'expected #{this} to deeply equal #{exp}', 'expected #{this} to not deeply equal #{exp}', e); },
    toStrictEqual(e) { c(deepEqual(actual, e, true), 'expected #{this} to strictly equal #{exp}', 'expected #{this} to not strictly equal #{exp}', e); },
    toBeTruthy() { c(!!actual, 'expected #{this} to be truthy', 'expected #{this} to not be truthy', true); },
    toBeFalsy() { c(!actual, 'expected #{this} to be falsy', 'expected #{this} to not be falsy', false); },
    toBeNull() { c(actual === null, 'expected #{this} to be null', 'expected #{this} not to be null', null); },
    toBeUndefined() { c(actual === undefined, 'expected #{this} to be undefined', 'expected #{this} not to be undefined', undefined); },
    toBeDefined() { c(actual !== undefined, 'expected #{this} to be defined', 'expected #{this} to be undefined', actual); },
    toBeNaN() { c(Number.isNaN(actual), 'expected #{this} to be NaN', 'expected #{this} not to be NaN', NaN); },
    toBeGreaterThan(n) { numbers(actual, 'actual'); numbers(n, 'expected'); c(actual > n, `expected ${actual} to be greater than ${n}`, `expected ${actual} to be not greater than ${n}`, n); },
    toBeGreaterThanOrEqual(n) { numbers(actual, 'actual'); numbers(n, 'expected'); c(actual >= n, `expected ${actual} to be greater than or equal to ${n}`, `expected ${actual} to be not greater than or equal to ${n}`, n); },
    toBeLessThan(n) { numbers(actual, 'actual'); numbers(n, 'expected'); c(actual < n, `expected ${actual} to be less than ${n}`, `expected ${actual} to be not less than ${n}`, n); },
    toBeLessThanOrEqual(n) { numbers(actual, 'actual'); numbers(n, 'expected'); c(actual <= n, `expected ${actual} to be less than or equal to ${n}`, `expected ${actual} to be not less than or equal to ${n}`, n); },
    toBeCloseTo(n, d = 2) {
      let pass = false, expectedDiff = 0, receivedDiff = 0;
      if ((n === Infinity && actual === Infinity) || (n === -Infinity && actual === -Infinity)) pass = true;
      else { expectedDiff = 10 ** -d / 2; receivedDiff = Math.abs(actual - n); pass = receivedDiff < expectedDiff; }
      c(pass, `expected #{this} to be close to #{exp}, received difference is ${receivedDiff}, but expected ${expectedDiff}`, `expected #{this} to not be close to #{exp}, received difference is ${receivedDiff}, but expected ${expectedDiff}`, n);
    },
    toContain(x) {
      if (typeof Node !== 'undefined' && actual instanceof Node) {
        if (!(x instanceof Node)) throw new TypeError(`toContain() expected a DOM node as the argument, but got ${typeof x}`);
        return c(actual.contains(x), 'expected #{this} to contain element #{exp}', 'expected #{this} not to contain element #{exp}', x);
      }
      if (typeof actual === 'string' && typeof x === 'string') return c(actual.includes(x), 'expected #{this} to contain #{exp}', 'expected #{this} not to contain #{exp}', x);
      // chai's include on the value made indexable (vitest: Array.from)
      const list = actual != null && typeof actual !== 'string' ? Array.from(actual) : actual;
      const pass = list != null && list.includes(x);
      __chaiAssert(negated, pass, 'expected #{this} to include #{exp}', 'expected #{this} to not include #{exp}', list, x, list);
    },
    toContainEqual(x) { c(actual != null && Array.from(actual).some((a) => deepEqual(a, x)), 'expected #{this} to deep equally contain #{exp}', 'expected #{this} to not deep equally contain #{exp}', x); },
    toBeInstanceOf(ctor) {
      if (typeof ctor !== 'function') {
        const t = ctor === null ? 'null' : Object.prototype.toString.call(ctor).slice(8, -1);
        throw new __ChaiAssertionError(`The instanceof assertion needs a constructor but ${t} was given.`);
      }
      c(actual instanceof ctor, `expected #{this} to be an instance of ${ctor.name}`, `expected #{this} to not be an instance of ${ctor.name}`, ctor);
    },
    toMatchObject(obj) {
      const pass = __subsetMatches(actual, obj);
      if (negated ? !pass : pass) return;
      const msg = __chaiMsg(negated ? 'expected #{this} to not match object #{exp}' : 'expected #{this} to match object #{exp}', actual, obj, actual);
      const n = __strippedCount(actual, obj);
      throw new __ChaiAssertionError(n === 0 ? msg : `${msg}\n(${n} matching ${n === 1 ? 'property' : 'properties'} omitted from actual)`, { expected: obj });
    },
    toThrowError(e) { return api.toThrow(e); },
    // File snapshot: serialize `actual`, key by `<full test name> <counter>` under
    // `__snapshots__/<testfile>.snap`. Missing key or update-mode (-u) → write + pass; else compare.
    // vitest: toMatchSnapshot(propertiesOrHint?, hint?) — key `<test> [> <hint>] <n>`
    toMatchSnapshot(propertiesOrHint, hint) {
      const sf = __snapFile();
      if (!sf) throw new Error('toMatchSnapshot: no test file path available');
      const props = typeof propertiesOrHint === 'object' && propertiesOrHint !== null ? propertiesOrHint : undefined;
      const h = typeof propertiesOrHint === 'string' ? propertiesOrHint : hint;
      const key = __snapNextKey(h ? `${__snap.testName} > ${h}` : __snap.testName);
      let received = actual;
      if (props) {
        if (typeof received !== 'object' || !received) throw new Error('Received value must be an object when the matcher has properties');
        if (!__subsetMatches(received, props)) throw new Error('Snapshot properties mismatched');
        received = __snapMerge(received, props);
      }
      const body = __snapBody(__serialize(received));
      const st = __snapLoad();
      st.unchecked.delete(key);
      const expected = st.data[key];
      const pass = expected !== undefined && String(expected).trim() === body.trim();
      // a passing snapshot is re-stored as freshly serialized (vitest: the file round-trip)
      if (pass) st.data[key] = body;
      const ok = __snapReconcile(st, pass, expected !== undefined, () => { st.data[key] = body; st.dirty = true; });
      // vitest: a plain Error naming the snapshot (the diff is its expected / actual)
      if (!ok) throw __snapMismatch(key, expected, body);
    },
    // vitest's toMatchFileSnapshot(path, hint?): the value (a string as is, else serialized)
    // against the whole file at `path` (relative to the test file); async, like vitest's.
    toMatchFileSnapshot(file, hint) {
      return Promise.resolve().then(() => {
        const sf = __snapFile();
        if (!sf) throw new Error('toMatchFileSnapshot: no test file path available');
        const testDir = sf.dir.replace(/\/__snapshots__$/, '');
        const parts = [];
        for (const seg of (String(file).startsWith('/') ? String(file) : `${testDir}/${file}`).split('/')) {
          if (seg === '..') parts.pop();
          else if (seg && seg !== '.') parts.push(seg);
        }
        const path = `/${parts.join('/')}`;
        const key = __snapNextKey(hint ? `${__snap.testName} > ${hint}` : __snap.testName);
        const body = typeof actual === 'string' ? actual : __serialize(actual);
        const st = __snapLoad();
        st.raw = true;
        const exists = globalThis.__fs_existsSync(path);
        const expected = exists ? globalThis.__fs_readFileSync(path, 'utf8').replace(/\r\n/g, '\n') : undefined;
        const pass = expected === body;
        const write = () => {
          const slash = path.lastIndexOf('/');
          if (slash > 0) globalThis.__fs_mkdirSync(path.slice(0, slash));
          globalThis.__fs_writeFileSync(path, body);
        };
        if (!__snapReconcile(st, pass, expected !== undefined, write)) throw __snapMismatch(key, expected, body);
      });
    },
    // vitest: toMatchInlineSnapshot(propertiesOrInlineSnapshot?, inlineSnapshotOrHint?, hint?);
    // a new or (-u) changed one is written back into the test file when its tests are done.
    toMatchInlineSnapshot(a, b, c, assertionName = 'toMatchInlineSnapshot') {
      let props, inline, hint;
      if (typeof a === 'string') { inline = __stripSnapIndent(a); hint = b; }
      else { props = a; if (b) inline = __stripSnapIndent(b); hint = c; }
      const key = __snapNextKey(hint ? `${__snap.testName} > ${hint}` : __snap.testName);
      let received = actual;
      if (props && typeof props === 'object') {
        if (typeof received !== 'object' || !received) throw new Error('Received value must be an object when the matcher has properties');
        if (!__subsetMatches(received, props)) throw new Error('Snapshot properties mismatched');
        received = __snapMerge(received, props);
      }
      const body = __snapBody(__serialize(received));
      const st = __snapLoad();
      st.inline = true;
      const pass = inline !== undefined && inline.trim() === body.trim();
      const site = globalThis.__ttCallSite && globalThis.__ttCallSite();
      const write = () => { if (site) st.inlineWrites.push({ line: site[0], column: site[1], snapshot: body, name: assertionName }); st.dirty = true; };
      if (!__snapReconcile(st, pass, inline !== undefined, write)) throw __snapMismatch(key, inline, body);
    },
    // vitest snapshots the thrown value itself (`[TypeError: boom]`)
    toThrowErrorMatchingSnapshot(hint) {
      let thrown;
      if (typeof actual === 'function') { try { actual(); } catch (e) { thrown = e; } }
      else thrown = actual;
      makeExpect(thrown).toMatchSnapshot(hint);
    },
    toThrowErrorMatchingInlineSnapshot(expected) {
      let thrown;
      if (typeof actual === 'function') { try { actual(); } catch (e) { thrown = e; } }
      else thrown = actual;
      makeExpect(thrown).toMatchInlineSnapshot(expected, undefined, undefined, 'toThrowErrorMatchingInlineSnapshot');
    },
    toStrictEqual_alias() {},
    toHaveLength(n) {
      if (actual == null || actual.length === undefined) throw new __ChaiAssertionError(__chaiMsg("expected #{this} to have property 'length'", actual));
      c(actual.length === n, 'expected #{this} to have a length of #{exp} but got #{act}', 'expected #{this} to not have a length of #{exp}', n, actual.length);
    },
    toHaveProperty(path, value) {
      let cur = actual, present;
      // A literal key (incl. one containing dots, e.g. a CSS selector) takes precedence over
      // dot-path traversal — matches jest/vitest behavior.
      if (!Array.isArray(path) && actual != null && (path in Object(actual))) {
        cur = actual[path]; present = true;
      } else {
        const keys = Array.isArray(path) ? path : String(path).split('.');
        present = true;
        for (const k of keys) {
          if (cur != null && (k in Object(cur))) cur = cur[k]; else { present = false; break; }
        }
      }
      const pass = arguments.length < 2 ? present : (present && deepEqual(cur, value));
      const name = Array.isArray(path) ? path.map((k) => String(k).replace(/([.[\]])/g, '\\$1')).join('.') : path;
      const withValue = arguments.length < 2 ? '' : ` with value ${__objDisplay(value)}`;
      c(pass, `expected #{this} to have property "${name}"${withValue}`, `expected #{this} to not have property "${name}"${withValue}`, value, present ? cur : undefined);
    },
    toMatch(re) {
      if (typeof actual !== 'string') throw new TypeError(`.toMatch() expects to receive a string, but got ${typeof actual}`);
      c(typeof re === 'string' ? actual.includes(re) : re.test(actual), 'expected #{this} to match #{exp}', 'expected #{this} not to match #{exp}', re);
    },
    toThrow(expected) {
      let threw = false, err;
      if (typeof actual === 'function') { try { actual(); } catch (e) { threw = true; err = e; } }
      else if (actual instanceof Error) { threw = true; err = actual; }
      const fnShown = typeof actual === 'function' ? actual : () => {};
      // chai's `throws` (no argument, a string, a RegExp): vitest's messages
      if (expected === undefined || typeof expected === 'string' || expected instanceof RegExp) {
        const shown = (x) => (x instanceof Error ? x.toString() : typeof x === 'string' ? x : x && x.constructor && x.constructor.name);
        if (expected === undefined) {
          return __chaiAssert(negated, threw, 'expected #{this} to throw an error', 'expected #{this} to not throw an error but #{act} was thrown', fnShown, undefined, shown(err));
        }
        if (!threw) {
          if (negated) return;
          throw new __ChaiAssertionError(__chaiMsg('expected #{this} to throw an error', fnShown));
        }
        const message = err instanceof Error ? err.message : typeof err === 'string' ? err : String(err);
        const re = expected === '' ? /^$/ : expected;
        const matches = re instanceof RegExp ? re.test(message) : message.includes(re);
        const verb = re instanceof RegExp ? 'matching' : 'including';
        return __chaiAssert(negated, matches, `expected #{this} to throw error ${verb} #{exp} but got #{act}`, `expected #{this} to throw error not ${verb} #{exp}`, fnShown, re, message);
      }
      if (!threw && !negated) throw new __ChaiAssertionError("expected function to throw an error, but it didn't", { showDiff: false });
      if (typeof expected === 'function') {
        const name = expected.name || expected.prototype.constructor.name;
        return c(threw && err instanceof expected, `expected error to be instance of ${name}`, `expected error not to be instance of ${name}`, expected, err);
      }
      if (expected instanceof Error) {
        return c(threw && err instanceof Error && err.message === expected.message, 'expected a thrown error to be #{exp}', 'expected a thrown error not to be #{exp}', expected, err);
      }
      if (expected instanceof Asymmetric) {
        return c(threw && expected.matches(err), 'expected error to match asymmetric matcher', 'expected error not to match asymmetric matcher', expected, err);
      }
      const matches = threw && err && (expected.message == null || String(err.message || '').includes(expected.message));
      c(matches, 'expected a thrown value to equal #{exp}', 'expected a thrown value not to equal #{exp}', expected, err);
    },
    toHaveBeenCalled() {
      const spy = spyOf();
      const name = spy.getMockName();
      const n = spy.mock.calls.length;
      if (negated ? n === 0 : n > 0) return;
      if (!negated) throw new __ChaiAssertionError(`expected "${name}" to be called at least once`);
      throw new __ChaiAssertionError(__formatCalls(spy, `expected "${name}" to not be called at all, but actually been called ${n} times`));
    },
    toHaveBeenCalledTimes(times) {
      const spy = spyOf();
      const n = spy.mock.calls.length;
      c(n === times, `expected "${spy.getMockName()}" to be called #{exp} times, but got ${n} times`, `expected "${spy.getMockName()}" to not be called #{exp} times`, times, n);
    },
    toHaveBeenCalledWith(...a) {
      const spy = spyOf();
      const name = spy.getMockName();
      const pass = spy.mock.calls.some((x) => argsEqual(x, a));
      if (negated ? !pass : pass) return;
      const msg = __chaiMsg(negated ? `expected "${name}" to not be called with arguments: #{exp}` : `expected "${name}" to be called with arguments: #{exp}`, actual, a, actual);
      throw new __ChaiAssertionError(__formatCalls(spy, msg, a));
    },
    toHaveBeenCalledOnce() {
      const spy = spyOf();
      const n = spy.mock.calls.length;
      c(n === 1, `expected "${spy.getMockName()}" to be called once, but got ${n} times`, `expected "${spy.getMockName()}" to not be called once`, 1, n);
    },
    toHaveBeenLastCalledWith(...a) {
      const spy = spyOf();
      const last = spy.mock.calls[spy.mock.calls.length - 1];
      c(!!last && argsEqual(last, a), `expected last "${spy.getMockName()}" call to have been called with #{exp}`, `expected last "${spy.getMockName()}" call to not have been called with #{exp}`, a, last);
    },
    toHaveBeenNthCalledWith(times, ...a) {
      const spy = spyOf();
      const nth = spy.mock.calls[times - 1];
      const n = spy.mock.calls.length;
      c(!!nth && argsEqual(nth, a), `expected ${__ordinal(times)} "${spy.getMockName()}" call to have been called with #{exp}${times <= n ? '' : `, but called only ${n} times`}`, `expected ${__ordinal(times)} "${spy.getMockName()}" call to not have been called with #{exp}`, a, nth);
    },
    toHaveReturned() {
      const spy = spyOf();
      const pass = spy.mock.calls.length > 0 && spy.mock.results.some((r) => r.type !== 'throw');
      c(pass, `expected "${spy.getMockName()}" to be successfully called at least once`, `expected "${spy.getMockName()}" to not be successfully called`, pass, !pass);
    },
    toHaveReturnedWith(v) {
      const spy = spyOf();
      const pass = spy.mock.results.some((r) => r.type === 'return' && deepEqual(v, r.value));
      if (negated ? !pass : pass) return;
      const name = spy.getMockName();
      const msg = __chaiMsg(negated ? `expected "${name}" to not return with: #{exp}` : `expected "${name}" to return with: #{exp} at least once`, actual, v, actual);
      throw new __ChaiAssertionError(__formatReturns(spy, msg, v));
    },
  };
  // custom matchers registered via expect.extend (e.g. @testing-library/jest-dom)
  for (const name of Object.keys(__customMatchers)) {
    api[name] = (...args) => {
      const ctx = {
        isNot: negated,
        promise: '',
        equals: (a, b) => deepEqual(a, b),
        utils: __matcherUtils,
      };
      let res;
      try { res = __customMatchers[name].call(ctx, actual, ...args); }
      catch (e) { throw e; }
      const pass = res && res.pass;
      if (negated ? pass : !pass) {
        // vitest calls the matcher's `message` function (anything else is a TypeError)
        const m = res && res.message;
        if (typeof m !== 'function') throw new TypeError('message is not a function');
        throw new Error(m());
      }
      return api;
    };
  }
  return api;
}
const __matcherUtils = {
  printReceived: (x) => fmt(x),
  printExpected: (x) => fmt(x),
  matcherHint: (name) => String(name || ''),
  stringify: (x) => fmt(x),
  diff: (a, b) => `- Expected: ${fmt(a)}\n+ Received: ${fmt(b)}`,
  RECEIVED_COLOR: (s) => s,
  EXPECTED_COLOR: (s) => s,
  BOLD_WEIGHT: (s) => s,
  DIM_COLOR: (s) => s,
  EXPECTED_LABEL: 'Expected',
  RECEIVED_LABEL: 'Received',
  pluralize: (w, n) => `${n} ${w}${n === 1 ? '' : 's'}`,
  highlightTrailingWhitespace: (s) => s,
};
const __customMatchers = {};
globalThis.expect = (actual) => { __snap.assertCount++; return makeExpect(actual); };
globalThis.expect.extend = (matchers) => { Object.assign(__customMatchers, matchers || {}); };
// expect.assertions(n) / expect.hasAssertions(): record the expectation; runSuite verifies the
// count at test end (see __snap.assertCount, reset per test). These calls do NOT count themselves.
globalThis.expect.assertions = (n) => { __snap.expectedAssertions = n; };
globalThis.expect.hasAssertions = () => { __snap.hasAssertions = true; };
globalThis.expect.soft = (actual) => { __snap.assertCount++; return makeExpect(actual); };
globalThis.expect.any = (ctor) => new Asymmetric('any', ctor);
globalThis.expect.anything = () => new Asymmetric('anything');
globalThis.expect.objectContaining = (s) => new Asymmetric('objectContaining', s);
globalThis.expect.arrayContaining = (s) => new Asymmetric('arrayContaining', s);
globalThis.expect.not = {
  arrayContaining: (s) => new Asymmetric('arrayContaining', s, true),
  objectContaining: (s) => new Asymmetric('objectContaining', s, true),
  stringContaining: (s) => new Asymmetric('stringContaining', s, true),
  stringMatching: (s) => new Asymmetric('stringMatching', s, true),
};
globalThis.expect.stringContaining = (s) => new Asymmetric('stringContaining', s);
globalThis.expect.stringMatching = (s) => new Asymmetric('stringMatching', s);

// ---- expectTypeOf / assertType (type-level assertions) ----
// Type assertions are checked by tsc and erased at compile time — they have no runtime behavior in
// vitest either. What has to exist at run time is the CHAIN, so `expectTypeOf<T>().toEqualTypeOf<U>()`
// (and `.not.toHaveProperty(...)`, `.items.toBeString()`, …) loads and runs. A proxy answers every
// property with itself, so any current or future member of the chain works — with `then` excluded so
// an accidental `await` can't see the chain as a thenable and hang forever.
const __typeChain = new Proxy(function () {}, {
  get(_t, p) {
    if (p === 'then' || p === 'catch' || p === 'finally') return undefined;
    if (p === Symbol.toPrimitive) return () => 'expectTypeOf';
    if (p === Symbol.toStringTag) return 'ExpectTypeOf';
    if (p === 'toString') return () => 'expectTypeOf';
    return __typeChain;
  },
  apply() { return __typeChain; },
  construct() { return __typeChain; },
});
globalThis.expectTypeOf = () => __typeChain;
globalThis.assertType = () => undefined;

// ---- vi (mocks/spies) ----
function makeSpy(impl) {
  const onceQueue = [];
  const fn = function (...args) {
    fn.mock.calls.push(args);
    fn.mock.instances.push(this);
    fn.mock.lastCall = args;
    let ret;
    // a throwing call is recorded as vitest records it (`{ type: 'throw', value }`)
    try {
      // constructor call (e.g. vi.spyOn(mod, 'default') on a class, then `new spy()`): construct
      // the original with `new` — calling a class via .apply throws "cannot be invoked without new".
      if (new.target) {
        ret = typeof fn.__impl === 'function' ? Reflect.construct(fn.__impl, args, new.target) : this;
      } else if (onceQueue.length) ret = onceQueue.shift().apply(this, args);
      else if (fn.__returnValue !== undefined) ret = fn.__returnValue;
      else if (fn.__impl) ret = fn.__impl.apply(this, args);
    } catch (e) {
      fn.mock.results.push({ type: 'throw', value: e });
      throw e;
    }
    fn.mock.results.push({ type: 'return', value: ret });
    return ret;
  };
  fn.mock = { calls: [], results: [], instances: [], lastCall: undefined };
  fn._isMockFunction = true;
  fn.__impl = impl;
  fn.mockReturnValue = (v) => { fn.__returnValue = v; return fn; };
  fn.mockReturnThis = () => { fn.__impl = function () { return this; }; fn.__returnValue = undefined; return fn; };
  fn.mockImplementation = (f) => { fn.__impl = f; fn.__returnValue = undefined; return fn; };
  fn.mockResolvedValue = (v) => { fn.__impl = () => Promise.resolve(v); fn.__returnValue = undefined; return fn; };
  fn.mockRejectedValue = (v) => { fn.__impl = () => Promise.reject(v); fn.__returnValue = undefined; return fn; };
  fn.mockReturnValueOnce = (v) => { onceQueue.push(() => v); return fn; };
  fn.mockImplementationOnce = (f) => { onceQueue.push(typeof f === 'function' ? f : () => f); return fn; };
  fn.mockResolvedValueOnce = (v) => { onceQueue.push(() => Promise.resolve(v)); return fn; };
  fn.mockRejectedValueOnce = (v) => { onceQueue.push(() => Promise.reject(v)); return fn; };
  fn.mockClear = () => { fn.mock.calls = []; fn.mock.results = []; fn.mock.instances = []; fn.mock.lastCall = undefined; return fn; };
  fn.mockReset = () => { fn.mockClear(); fn.__impl = undefined; fn.__returnValue = undefined; onceQueue.length = 0; return fn; };
  fn.mockRestore = () => {};
  fn.getMockName = () => fn.__name || 'vi.fn()';
  fn.mockName = (n) => { fn.__name = n; return fn; };
  return fn;
}
// vitest automock (factory-less vi.mock, both loader paths): functions become vi.fn() mocks
// returning undefined, classes keep mocked statics and prototype methods (so instances get mock
// methods), objects and class instances are mocked deeply, arrays are emptied, primitives kept.
// A prototype chain is walked up to (not into) the built-in prototypes (Object, Error, ...).
function __isBuiltinProto(p) {
  if (p === Object.prototype || p === Function.prototype) return true;
  try { return typeof p.constructor === 'function' && /\{\s*\[native code\]\s*\}$/.test(Function.prototype.toString.call(p.constructor)); } catch (e) { return true; }
}
function __automockProtoMethods(target, proto) {
  for (let p = proto; p && !__isBuiltinProto(p); p = Object.getPrototypeOf(p)) {
    for (const k of Object.getOwnPropertyNames(p)) {
      if (k === 'constructor' || Object.prototype.hasOwnProperty.call(target, k)) continue;
      const d = Object.getOwnPropertyDescriptor(p, k);
      if (typeof d.value === 'function') { const s = makeSpy(); __spies.push(s); target[k] = s; }
    }
  }
}
function __automock(mod, depth) {
  depth = depth || 0;
  if (typeof mod === 'function') {
    const m = makeSpy(); __spies.push(m);
    if (depth < 3) {
      // own statics (class static methods are non-enumerable), not the function's built-ins
      for (const k of Object.getOwnPropertyNames(mod)) {
        if (k === 'length' || k === 'name' || k === 'prototype' || k === 'caller' || k === 'arguments') continue;
        try { m[k] = __automock(mod[k], depth + 1); } catch (e) {}
      }
      // class / constructor function: `new m()` returns an instance of m.prototype, whose
      // methods (inherited ones included) are mocks
      if (mod.prototype && mod.prototype.constructor === mod) __automockProtoMethods(m.prototype, mod.prototype);
    }
    return m;
  }
  if (Array.isArray(mod)) return [];
  if (mod && typeof mod === 'object') {
    const out = {};
    for (const k of Object.keys(mod)) {
      let v; try { v = mod[k]; } catch (e) { continue; }
      out[k] = depth < 4 ? __automock(v, depth + 1) : v;
    }
    // a class instance: its prototype methods become mocks too
    __automockProtoMethods(out, Object.getPrototypeOf(mod));
    return out;
  }
  return mod;
}
const __spies = [];
// Record (specifier, evaluated-exports) into a queue the native loader drains after the
// module runs, registering the mock keyed by the resolved absolute path. The factory runs
// here, in the calling module's scope (so closures over setup-file vars work). `vitest` is
// false for jest.mock/doMock (jest semantics where they differ).
const __ttMockErrorSpecs = new Set();
// vitest: reading a name the factory did not return throws (an ES import of it fails on access
// rather than reading undefined). Probes that interop/thenable checks make (`then`, `__esModule`),
// symbols and inherited members never throw. Keys (dir + spec) last mocked with jest.mock/doMock
// read undefined instead (jest semantics); checked on access, because the mock prepass registers
// every factory via vi.mock and an isolate reuse keeps that first exports object.
const __ttLenientMockKeys = new Set();
function __ttStrictMock(mod, key, spec) {
  if (mod === null || typeof mod !== 'object' || __ttMockErrors.has(mod)) return mod;
  return new Proxy(mod, {
    get: (t, k, r) => {
      if (typeof k === 'symbol' || k in t || k === 'then' || k === '__esModule' || __ttLenientMockKeys.has(key)) return Reflect.get(t, k, r);
      throw new Error(`[vitest] No "${k}" export is defined on the "${spec}" mock. Did you want to return the actual module?`);
    },
  });
}
// Modules (resolved absolute paths) the entry's mock prepass registered for the current file.
// vitest calls a hoisted factory ONCE; the module's own hoisted vi.mock call for such a module
// keeps the prepass registration instead of running the factory a second time. Consumed by
// that call (a later vi.mock/doMock of the module runs normally) and cleared between files.
const __ttPrepassMocks = new Set();
globalThis.__ttPrepassMocks = __ttPrepassMocks;
function __ttPrepassKey(spec) {
  const dir = globalThis.__ttDir || globalThis.__ttEntryDir;
  return (dir && globalThis.__ttResolveSpec(dir, String(spec))) || '\0' + String(spec);
}
// Pending async vi.doMock / jest.doMock factories, by the module's mock-registry key (its
// resolved path, `?query` kept): each is registered once its factory settles, and an import() of
// the module waits for that first (vitest awaits the factory; turbo-test#42 item 6). A later
// mock / unmock of the module drops its entry, so the stale factory never registers.
const __ttDoMockWaits = new Map();
globalThis.__ttDoMockWaits = __ttDoMockWaits;
function __ttDoMockKey(dir, spec) {
  return (__ttDoMockWaits.size && dir && globalThis.__ttResolveSpec(dir, String(spec))) || undefined;
}
// The pending registration an import() of `spec` from `dir` must wait for, if any (the ESM
// loader's import() asks it) ...
globalThis.__ttImportWait = (dir, spec) => {
  const k = __ttDoMockKey(dir, spec);
  return k && __ttDoMockWaits.get(k);
};
// ... and the promise the module-runner's lowered import() chains its require() on:
// Promise.resolve() when nothing is pending, so the timing is unchanged.
globalThis.__ttImportReady = (dir, spec) => globalThis.__ttImportWait(dir, spec) || Promise.resolve();
// Register `exports` as the mock of `spec` from `dir` now; false when it cannot be (no loader
// registry yet).
function __ttRegisterMockNow(dir, spec, exports, key) {
  // An error stand-in replaces (never merges into) a module already registered under this
  // isolate; a later working factory for the same spec replaces the stand-in the same way.
  if (__ttMockErrors.has(exports) || __ttMockErrorSpecs.has(key)) {
    globalThis.vi.unmock(spec);
    if (__ttMockErrors.has(exports)) __ttMockErrorSpecs.add(key); else __ttMockErrorSpecs.delete(key);
  }
  try { globalThis.__ttRegisterMock(dir, String(spec), exports); return true; } catch (e) { return false; }
}
// An async doMock factory's result: registered once it settles (a rejection registers the
// factory-error stand-in), unless a later mock / unmock of the module came first.
function __ttAwaitDoMock(dir, spec, exports, key, vitest) {
  const k = dir && globalThis.__ttResolveSpec(dir, String(spec));
  if (!k) return false;
  const done = Promise.resolve(exports).then(
    (v) => __ttStrictMock(v, key, String(spec)),
    (e) => __ttFactoryErrorMock(e, vitest),
  ).then((v) => {
    if (__ttDoMockWaits.get(k) !== done) return;
    __ttDoMockWaits.delete(k);
    if (!__ttRegisterMockNow(dir, spec, v, key)) {
      (globalThis.__pendingMocks || (globalThis.__pendingMocks = [])).push({ spec: String(spec), exports: v, vitest, key });
    }
  });
  __ttDoMockWaits.set(k, done);
  return true;
}
function __ttMock(spec, factory, vitest, isDoMock) {
  let exports;
  const key = (globalThis.__ttDir || '') + '\0' + String(spec);
  // The calling module's dir: the module-runner's __ttDir, else (ESM graph, where modules have
  // none) the entry file's dir, so a runtime vi.doMock registers immediately and a later import()
  // of the module sees it.
  const dir = globalThis.__ttDir || globalThis.__ttEntryDir;
  if (!globalThis.__ttMockPrepass) {
    if (vitest) __ttLenientMockKeys.delete(key); else __ttLenientMockKeys.add(key);
    if (__ttPrepassMocks.size && __ttPrepassMocks.delete(__ttPrepassKey(spec))) return;
    // this mock supersedes a still-pending async doMock of the module
    const pending = __ttDoMockKey(dir, spec);
    if (pending) __ttDoMockWaits.delete(pending);
  }
  // vitest passes `importOriginal` (a fn returning the real module, as a Promise) as the
  // factory's first arg — `vi.mock('x', async (importOriginal) => ({...await importOriginal(), y}))`.
  const importOriginal = () => Promise.resolve(globalThis.__ttImportActual(dir, String(spec)));
  if (factory === undefined) {
    // AUTOMOCK: no factory -> auto-generate a mock from the real module (functions -> vi.fn()).
    // A real module that fails to load is left unmocked: importing it reports its own error.
    try { exports = __automock(globalThis.__ttImportActual(dir, String(spec))); }
    catch (e) { return; }
  } else {
    try { exports = typeof factory === 'function' ? factory(importOriginal) : factory; }
    catch (e) {
      // The entry's mock prepass (__ttMockPrepass) runs factories outside the module's scope; a
      // throw there is not registered — the module's own vi.mock call re-runs it in scope.
      if (globalThis.__ttMockPrepass) return;
      exports = __ttFactoryErrorMock(e, vitest);
    }
  }
  // Sync factory: register IMMEDIATELY in the calling module's dir (vi.mock is hoisted above
  // the module's consumer requires, so the mock is in place before they load — and the factory
  // ran in THIS module's scope, so it shares the test's bindings). Async (Promise) factory:
  // queue for __resolvePendingMocks (run_entry_mocks prepass / drain).
  if (globalThis.__ttMockPrepass) __ttPrepassMocks.add(__ttPrepassKey(spec));
  if (exports && typeof exports.then === 'function') {
    // a doMock (not hoisted: it applies to later imports) waits on its own; a hoisted vi.mock is
    // awaited by the loader's resolve + drain
    if (isDoMock && !globalThis.__ttMockPrepass && globalThis.__ttRegisterMock && __ttAwaitDoMock(dir, spec, exports, key, vitest)) return;
    (globalThis.__pendingMocks || (globalThis.__pendingMocks = [])).push({ spec: String(spec), exports, vitest, key });
    return;
  }
  if (factory !== undefined) exports = __ttStrictMock(exports, key, String(spec));
  if (globalThis.__ttRegisterMock && dir && __ttRegisterMockNow(dir, spec, exports, key)) return;
  (globalThis.__pendingMocks || (globalThis.__pendingMocks = [])).push({ spec: String(spec), exports, vitest, key });
}
// vi.dynamicImportSettled (vitest's waitForImportsToResolve): wait a macrotask on the real
// clock (vitest's safe timers, so fake timers do not hold it), then for every import() still
// pending — the loader's import() hook tracks them — and again until none is left.
const __ttPendingImports = new Set();
globalThis.__ttTrackImport = (p) => {
  __ttPendingImports.add(p);
  const done = () => { __ttPendingImports.delete(p); };
  p.then(done, done);
  return p;
};
async function __ttImportsSettled() {
  await new Promise((resolve) => __scheduleInternal(resolve, 0));
  if (!__ttPendingImports.size) return;
  await Promise.allSettled([...__ttPendingImports]);
  return __ttImportsSettled();
}
globalThis.vi = {
  fn: (impl) => { const s = makeSpy(impl); __spies.push(s); return s; },
  spyOn: (obj, key) => {
    const orig = obj[key];
    // Do NOT bind orig to obj — for a prototype-method spy (spyOn(Cls.prototype, 'm')) the
    // original must run with the CALL's `this` (the instance), not the prototype. makeSpy's
    // fn applies the call-time `this` to __impl, preserving it.
    const s = makeSpy(orig);
    // vitest names a spy after its method
    s.__name = String(key);
    // Use defineProperty so it replaces a (configurable) live-export getter, not just a
    // writable data prop — module-runner exports are configurable getters.
    const define = (v) => { try { Object.defineProperty(obj, key, { configurable: true, enumerable: true, writable: true, value: v }); } catch (e) { try { obj[key] = v; } catch (e2) {} } };
    s.mockRestore = () => define(orig);
    define(s);
    __spies.push(s);
    return s;
  },
  // Record (specifier, evaluated-exports) into a queue the native loader drains after the
  // module runs, registering the mock keyed by the resolved absolute path. The factory runs
  // here, in the calling module's scope (so closures over setup-file vars work).
  mock: (spec, factory) => __ttMock(spec, factory, true),
  unmock: (spec) => {
    if (globalThis.__pendingMocks) globalThis.__pendingMocks = globalThis.__pendingMocks.filter((m) => m.spec !== String(spec));
    const pending = __ttDoMockKey(globalThis.__ttDir || globalThis.__ttEntryDir, spec);
    if (pending) __ttDoMockWaits.delete(pending);
    const dir = globalThis.__ttDir || globalThis.__cwd;
    if (globalThis.__ttUnmock && dir) { try { globalThis.__ttUnmock(dir, String(spec)); } catch (e) {} }
  },
  doMock: (spec, factory) => __ttMock(spec, factory, true, true),
  doUnmock: (spec) => globalThis.vi.unmock(spec),
  dynamicImportSettled: () => __ttImportsSettled(),
  clearAllMocks: () => { __spies.forEach((s) => s.mockClear()); },
  resetAllMocks: () => { __spies.forEach((s) => s.mockReset()); },
  restoreAllMocks: () => { __spies.forEach((s) => { if (s.mockRestore) s.mockRestore(); }); },
  stubGlobal: (k, v) => {
    const m = globalThis.__stubbedGlobals || (globalThis.__stubbedGlobals = new Map());
    if (!m.has(k)) m.set(k, { had: k in globalThis, val: globalThis[k] });
    globalThis[k] = v;
  },
  stubEnv: (k, v) => {
    const m = globalThis.__stubbedEnvs || (globalThis.__stubbedEnvs = new Map());
    if (!m.has(k)) m.set(k, { had: k in globalThis.process.env, val: globalThis.process.env[k] });
    globalThis.process.env[k] = v;
  },
  isMockFunction: (f) => typeof f === 'function' && f._isMockFunction === true,
  // fake timers
  useFakeTimers: () => {
    __loop.fake = true;
    if (__loop.systemTime === 0) __loop.systemTime = __loop.realNow();
    __installFakeDate();
    // Mark the global timer fns so @testing-library's jestFakeTimersAreEnabled() (checks
    // `setTimeout.clock` / `_isMockFunction`) detects fake timers — otherwise waitFor polls with the
    // frozen real clock and hangs. testing-library then drives jest.advanceTimersByTime() to advance.
    try { globalThis.setTimeout.clock = __loop; globalThis.setInterval.clock = __loop; } catch (e) {}
    return globalThis.vi;
  },
  useRealTimers: () => {
    __loop.fake = false; __restoreDate();
    try { delete globalThis.setTimeout.clock; delete globalThis.setInterval.clock; } catch (e) {}
    return globalThis.vi;
  },
  isFakeTimers: () => __loop.fake,
  setSystemTime: (t) => {
    const ms = t instanceof __loop.RealDate ? t.getTime()
      : typeof t === 'number' ? t : new __loop.RealDate(t).getTime();
    __loop.systemTime = ms;
    __loop.now = 0;
    // vitest fakes Date on setSystemTime even without useFakeTimers — install the fake Date so
    // `new Date()` / Date.now() reflect the set time (timer faking stays separate).
    __installFakeDate();
    return globalThis.vi;
  },
  getMockedSystemTime: () => (__loop.fake ? new __loop.RealDate(__loop.systemTime + __loop.now) : null),
  waitFor: async (fn, opts) => {
    const timeout = (opts && opts.timeout) || 1000;
    const interval = (opts && opts.interval) || 50;
    let last;
    for (let elapsed = 0; elapsed <= timeout; elapsed += interval) {
      try { const r = await fn(); if (r !== false && r !== null && r !== undefined) return r; if (r === undefined) return r; }
      catch (e) { last = e; }
      await new Promise((res) => setTimeout(res, interval));
    }
    throw last || new Error('vi.waitFor timed out');
  },
  waitUntil: async (fn, opts) => {
    const timeout = (opts && opts.timeout) || 1000;
    const interval = (opts && opts.interval) || 50;
    for (let elapsed = 0; elapsed <= timeout; elapsed += interval) {
      try { const r = await fn(); if (r) return r; } catch (e) {}
      await new Promise((res) => setTimeout(res, interval));
    }
    throw new Error('vi.waitUntil timed out');
  },
  advanceTimersByTime: (ms) => { __loop.advance(ms); return globalThis.vi; },
  advanceTimersByTimeAsync: async (ms) => { __loop.advance(ms); return globalThis.vi; },
  advanceTimersToNextTimer: () => {
    const n = __loop.macro.slice().sort((a, b) => a.due - b.due || a.id - b.id)[0];
    if (n) __loop.advance(n.due - __loop.now);
    return globalThis.vi;
  },
  runAllTimers: () => { __loop.runAll(); return globalThis.vi; },
  runAllTimersAsync: async () => { __loop.runAll(); return globalThis.vi; },
  runOnlyPendingTimers: () => { __loop.runOnlyPending(); return globalThis.vi; },
  runOnlyPendingTimersAsync: async () => { __loop.runOnlyPending(); return globalThis.vi; },
  advanceTimersToNextTimerAsync: async () => { const n = __loop.macro.slice().sort((a, b) => a.due - b.due || a.id - b.id)[0]; if (n) __loop.advance(n.due - __loop.now); return globalThis.vi; },
  getTimerCount: () => __loop.macro.length,
  clearAllTimers: () => { __loop.macro = []; return globalThis.vi; },
  mocked: (f) => {
    if (typeof f === 'function' && !f.mock) {
      f.mock = { calls: [], results: [] };
      f.mockClear = () => { f.mock.calls = []; f.mock.results = []; return f; };
      f.mockReset = () => f;
      f.mockRestore = () => {};
      f.mockReturnValue = () => f;
      f.mockReturnValueOnce = () => f;
      f.mockImplementation = () => f;
      f.mockImplementationOnce = () => f;
      f.mockResolvedValue = () => f;
      f.mockRejectedValue = () => f;
    }
    return f;
  },
  // vi.hoisted must return the SAME value in the mock-prepass and the entry module (the mock
  // closure and the test share it). Cache by call-index: the prepass fills the cache, then the
  // entry (index reset to 0) hits it — so `const x = vi.hoisted(...)` is one object everywhere.
  hoisted: (f) => {
    const cache = globalThis.__hoistedCache || (globalThis.__hoistedCache = []);
    const i = globalThis.__hoistedIdx || 0;
    globalThis.__hoistedIdx = i + 1;
    if (i < cache.length) return cache[i];
    const v = f();
    cache[i] = v;
    return v;
  },
  importActual: (p) => { try { return Promise.resolve(globalThis.__ttImportActual(globalThis.__ttDir, String(p))); } catch (e) { return Promise.resolve({}); } },
  importMock: (p) => { try { return Promise.resolve(globalThis.__nativeRequire(globalThis.__ttDir, String(p))); } catch (e) { return Promise.resolve({}); } },
  unstubAllEnvs: () => {
    const m = globalThis.__stubbedEnvs;
    if (m) { m.forEach(({ had, val }, k) => { if (had) globalThis.process.env[k] = val; else delete globalThis.process.env[k]; }); m.clear(); }
  },
  unstubAllGlobals: () => {
    const m = globalThis.__stubbedGlobals;
    if (m) { m.forEach(({ had, val }, k) => { if (had) globalThis[k] = val; else delete globalThis[k]; }); m.clear(); }
  },
  resetModules: () => { if (globalThis.__ttResetModules) globalThis.__ttResetModules(); return globalThis.vi; },
  setConfig: () => {},
  stubEnv: (k, v) => {
    const m = globalThis.__stubbedEnvs || (globalThis.__stubbedEnvs = new Map());
    if (!m.has(k)) m.set(k, { had: k in globalThis.process.env, val: globalThis.process.env[k] });
    globalThis.process.env[k] = v;
  },
};

// ---- legacy decorator + emitDecoratorMetadata helpers ----
// The oxc decorator-metadata transform emits `babelHelpers.decorate / decorateParam /
// decorateMetadata(...)` (External helper mode). Provide them with standard tslib semantics so
// NestJS/Mongoose decorators (`@Injectable`, `@Prop`, `@Controller`) run. No Reflect.metadata
// polyfill: such projects depend on the real `reflect-metadata`, which their import graph
// (`@nestjs/common`) loads before any decorator runs. A partial polyfill would trip
// reflect-metadata's "already installed" guard, leaving it missing methods (getOwnMetadataKeys
// etc.). `decorateMetadata` no-ops cleanly when Reflect.metadata is absent (non-Nest projects).
globalThis.babelHelpers = globalThis.babelHelpers || {};
globalThis.babelHelpers.decorate = function (decorators, target, key, desc) {
  var c = arguments.length,
    r = c < 3 ? target : desc === null ? (desc = Object.getOwnPropertyDescriptor(target, key)) : desc,
    d;
  if (typeof Reflect === 'object' && typeof Reflect.decorate === 'function') {
    r = Reflect.decorate(decorators, target, key, desc);
  } else {
    for (var i = decorators.length - 1; i >= 0; i--) {
      if ((d = decorators[i])) r = (c < 3 ? d(r) : c > 3 ? d(target, key, r) : d(target, key)) || r;
    }
  }
  return c > 3 && r && Object.defineProperty(target, key, r), r;
};
globalThis.babelHelpers.decorateParam = function (paramIndex, decorator) {
  return function (target, key) { decorator(target, key, paramIndex); };
};
globalThis.babelHelpers.decorateMetadata = function (metadataKey, metadataValue) {
  if (typeof Reflect === 'object' && typeof Reflect.metadata === 'function') {
    return Reflect.metadata(metadataKey, metadataValue);
  }
};

// ---- jest compatibility shim ----
// Jest's `jest` global maps onto the same machinery as `vi` (drop-in for jest projects, e.g.
// NestJS backends using ts-jest). Most methods are identical; the jest-only ones (sync
// requireActual/requireMock, isolateModules, setTimeout) are bridged here. Type-only members
// (jest.Mock, jest.Mocked, jest.SpyInstance, ...) are erased by the transform — no runtime needed.
// Defined unconditionally: it doesn't touch vitest behavior (vitest suites never reference `jest`).
globalThis.jest = {
  fn: (impl) => globalThis.vi.fn(impl),
  spyOn: (obj, key) => globalThis.vi.spyOn(obj, key),
  mock: (spec, factory) => __ttMock(spec, factory, false),
  unmock: (spec) => globalThis.vi.unmock(spec),
  doMock: (spec, factory) => __ttMock(spec, factory, false, true),
  dontMock: (spec) => globalThis.vi.unmock(spec),
  mocked: (item) => globalThis.vi.mocked(item),
  clearAllMocks: () => globalThis.vi.clearAllMocks(),
  resetAllMocks: () => globalThis.vi.resetAllMocks(),
  restoreAllMocks: () => globalThis.vi.restoreAllMocks(),
  isMockFunction: (f) => globalThis.vi.isMockFunction(f),
  // jest.requireActual / requireMock are SYNCHRONOUS (return the module, not a Promise) —
  // unlike vi.importActual. Bridge to the native sync loader directly.
  requireActual: (p) => { try { return globalThis.__ttImportActual(globalThis.__ttDir, String(p)); } catch (e) { return {}; } },
  requireMock: (p) => { try { return globalThis.__nativeRequire(globalThis.__ttDir, String(p)); } catch (e) { return {}; } },
  // jest.isolateModules(fn): run `fn` with a fresh module registry. Approximate with a
  // resetModules around the (sync) callback — enough for the common "re-require with new env".
  isolateModules: (fn) => { globalThis.vi.resetModules(); try { fn(); } finally { globalThis.vi.resetModules(); } },
  isolateModulesAsync: async (fn) => { globalThis.vi.resetModules(); try { await fn(); } finally { globalThis.vi.resetModules(); } },
  resetModules: () => { globalThis.vi.resetModules(); return globalThis.jest; },
  // timers
  useFakeTimers: (cfg) => { globalThis.vi.useFakeTimers(cfg); return globalThis.jest; },
  useRealTimers: () => { globalThis.vi.useRealTimers(); return globalThis.jest; },
  setSystemTime: (t) => { globalThis.vi.setSystemTime(t); return globalThis.jest; },
  getRealSystemTime: () => __loop.realNow(),
  now: () => (globalThis.vi.isFakeTimers() ? globalThis.vi.getMockedSystemTime().getTime() : __loop.realNow()),
  advanceTimersByTime: (ms) => { globalThis.vi.advanceTimersByTime(ms); return globalThis.jest; },
  advanceTimersByTimeAsync: async (ms) => { await globalThis.vi.advanceTimersByTimeAsync(ms); return globalThis.jest; },
  advanceTimersToNextTimer: () => { globalThis.vi.advanceTimersToNextTimer(); return globalThis.jest; },
  runAllTimers: () => { globalThis.vi.runAllTimers(); return globalThis.jest; },
  runAllTimersAsync: async () => { await globalThis.vi.runAllTimersAsync(); return globalThis.jest; },
  runOnlyPendingTimers: () => { globalThis.vi.runOnlyPendingTimers(); return globalThis.jest; },
  clearAllTimers: () => { globalThis.vi.clearAllTimers(); return globalThis.jest; },
  getTimerCount: () => globalThis.vi.getTimerCount(),
  // per-test config knobs — no-ops (turbo-test has no per-file timeout/retry gate yet)
  setTimeout: () => globalThis.jest,
  retryTimes: () => globalThis.jest,
  // env stubbing parity (rarely used via jest, but harmless)
  replaceProperty: (obj, key, val) => { const orig = obj[key]; obj[key] = val; return { restore: () => { obj[key] = orig; } }; },
};

// ---- test.each / describe.each titles (turbo-test#67) ----
// vitest's `formatTitle` (@vitest/runner): `%#` / `%$` (index / index + 1), printf placeholders
// (`%s %d %i %f %j %o %O %c %%`, each formatting ONE row item as @vitest/utils `format` does) and
// `$key` / `$a.b` / `$0` attributes shown as `objDisplay`. Values are rendered the way vitest's
// loupe `inspect` renders them (`'str'`, `[ 1, 2 ]`, `{ a: 1 }`, `[Function f]`, `+0`).
const __inspectEsc = /['\u0000-\u001f\u007f-\u009f\u00ad\u0600-\u0604\u070f\u17b4\u17b5\u200c-\u200f\u2028-\u202f\u2060-\u206f\ufeff\ufff0-\uffff]/g;
const __inspectEscMap = { '\b': '\\b', '\t': '\\t', '\n': '\\n', '\f': '\\f', '\r': '\\r', "'": "\\'" };
function __inspectTrunc(str, len) {
  str = String(str);
  if (1 > len && str.length > 1) return '…';
  if (str.length > len && str.length > 1) {
    let end = len - 1;
    if (end > 0 && str[end - 1] >= '\ud800' && str[end - 1] <= '\udbff') end--;
    return `${str.slice(0, end)}…`;
  }
  return str;
}
function __inspectString(str, o) {
  str = String(str).replace(__inspectEsc, (c) => __inspectEscMap[c] || `\\u${`0000${c.charCodeAt(0).toString(16)}`.slice(-4)}`);
  return `'${__inspectTrunc(str, o.truncate - 2)}'`;
}
function __inspectNumber(n, o) {
  if (Number.isNaN(n)) return 'NaN';
  if (n === Infinity) return 'Infinity';
  if (n === -Infinity) return '-Infinity';
  if (n === 0) return 1 / n === Infinity ? '+0' : '-0';
  return __inspectTrunc(String(n), o.truncate);
}
// loupe's `inspectList`: items joined with `sep`, cut with `…(n)` once `o.truncate` runs out.
function __inspectList(list, o, item = __inspect, sep = ', ') {
  if (!list.length) return '';
  const orig = o.truncate;
  let out = '';
  let peek = '';
  let cut = '';
  for (let i = 0; i < list.length; i++) {
    const last = i + 1 === list.length;
    const second = i + 2 === list.length;
    cut = `…(${list.length - i})`;
    o.truncate = orig - out.length - (last ? 0 : sep.length);
    const str = peek || item(list[i], o) + (last ? '' : sep);
    const next = out.length + str.length;
    const cutLen = next + cut.length;
    if (last && next > orig && out.length + cut.length <= orig) break;
    if (!last && !second && cutLen > orig) break;
    peek = last ? '' : item(list[i + 1], o) + (second ? '' : sep);
    if (!last && second && cutLen > orig && next + peek.length > orig) break;
    out += str;
    if (!last && !second && next + peek.length >= orig) { cut = `…(${list.length - i - 1})`; break; }
    cut = '';
  }
  return `${out}${cut}`;
}
function __inspectProp([key, value], o) {
  o.truncate -= 2;
  if (typeof key === 'string') {
    key = /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : JSON.stringify(key).replace(/'/g, "\\'").replace(/\\"/g, '"').replace(/(^"|"$)/g, "'");
  } else if (typeof key !== 'number') {
    key = `[${__inspect(key, o)}]`;
  }
  o.truncate -= key.length;
  return `${key}: ${__inspect(value, o)}`;
}
function __inspectObject(obj, o) {
  const props = Object.getOwnPropertyNames(obj);
  const syms = Object.getOwnPropertySymbols(obj);
  if (!props.length && !syms.length) return '{}';
  o.truncate -= 4;
  if (o.seen.includes(obj)) return '[Circular]';
  o.seen.push(obj);
  const p = __inspectList(props.map((k) => [k, obj[k]]), o, __inspectProp);
  const q = __inspectList(syms.map((k) => [k, obj[k]]), o, __inspectProp);
  o.seen.pop();
  return `{ ${p}${p && q ? ', ' : ''}${q} }`;
}
// vitest's loupe `inspect` (the value rendering of titles, `%o`/`%O`/`%s` objects); `o.truncate`
// (default unlimited) bounds the output as loupe does.
function __inspect(v, o = { truncate: Infinity, seen: [] }) {
  if (v === null) return 'null';
  switch (typeof v) {
    case 'undefined': return 'undefined';
    case 'boolean': return String(v);
    case 'number': return __inspectNumber(v, o);
    case 'bigint': { const n = __inspectTrunc(v.toString(), o.truncate - 1); return n === '…' ? n : `${n}n`; }
    case 'string': return __inspectString(v, o);
    case 'symbol': return v.description ? `Symbol(${v.description})` : 'Symbol()';
    case 'function': {
      const tag = v[Symbol.toStringTag] || 'Function';
      return v.name ? `[${tag} ${__inspectTrunc(v.name, o.truncate - 11)}]` : `[${tag}]`;
    }
  }
  const type = Object.prototype.toString.call(v).slice(8, -1);
  switch (type) {
    case 'Boolean': return String(v);
    case 'Number': return __inspectNumber(Number(v), o);
    case 'String': return __inspectString(v, o);
    case 'Array': {
      const extra = Object.keys(v).slice(v.length);
      if (!v.length && !extra.length) return '[]';
      o.truncate -= 4;
      const items = __inspectList(Array.from({ length: v.length }, (_, i) => v[i]), o);
      o.truncate -= items.length;
      const props = extra.length ? __inspectList(extra.map((k) => [k, v[k]]), o, __inspectProp) : '';
      return `[ ${items}${props ? `, ${props}` : ''} ]`;
    }
    case 'Date': {
      const j = v.toJSON();
      if (j === null) return 'Invalid Date';
      const [d, t] = j.split('T');
      return `${d}T${__inspectTrunc(t, o.truncate - d.length - 1)}`;
    }
    case 'Map':
      if (!v.size) return 'Map{}';
      o.truncate -= 7;
      return `Map{ ${__inspectList([...v], o, ([k, x], oo) => { oo.truncate -= 4; const ks = __inspect(k, oo); oo.truncate -= ks.length; return `${ks} => ${__inspect(x, oo)}`; })} }`;
    case 'Set':
      if (!v.size) return 'Set{}';
      o.truncate -= 7;
      return `Set{ ${__inspectList([...v], o)} }`;
    case 'RegExp': return `/${__inspectTrunc(v.source, o.truncate - (2 + v.flags.length))}/${v.flags}`;
    case 'Promise': return 'Promise{…}';
    case 'WeakSet': return 'WeakSet{…}';
    case 'WeakMap': return 'WeakMap{…}';
    case 'Arguments':
      if (!v.length) return 'Arguments[]';
      o.truncate -= 13;
      return `Arguments[ ${__inspectList([...v], o)} ]`;
    case 'Generator': case 'DataView': case 'ArrayBuffer': return '';
    case 'Error': {
      const skip = ['stack', 'line', 'column', 'name', 'message', 'fileName', 'lineNumber', 'columnNumber', 'number', 'description', 'cause'];
      const keys = Object.getOwnPropertyNames(v).filter((k) => !skip.includes(k));
      o.truncate -= v.name.length;
      let msg = '';
      if (typeof v.message === 'string') msg = __inspectTrunc(v.message, o.truncate);
      else keys.unshift('message');
      msg = msg ? `: ${msg}` : '';
      o.truncate -= msg.length + 5;
      if (o.seen.includes(v)) return '[Circular]';
      o.seen.push(v);
      const props = __inspectList(keys.map((k) => [k, v[k]]), o, __inspectProp);
      return `${v.name}${msg}${props ? ` { ${props} }` : ''}`;
    }
  }
  if (ArrayBuffer.isView(v)) {
    const name = (typeof Buffer === 'function' && v instanceof Buffer) ? 'Buffer' : (v[Symbol.toStringTag] || v.constructor.name);
    if (!v.length) return `${name}[]`;
    o.truncate -= name.length + 4;
    let out = '';
    for (let i = 0; i < v.length; i++) {
      const str = `${__inspectTrunc(v[i], o.truncate)}${i === v.length - 1 ? '' : ', '}`;
      o.truncate -= str.length;
      if (v[i] !== v.length && o.truncate <= 3) { out += `…(${v.length - v[i] + 1})`; break; }
      out += str;
    }
    return `${name}[ ${out} ]`;
  }
  // vitest's asymmetric matchers inspect as their minimal pretty-format (`Any<Number>`, …)
  if (v instanceof Asymmetric) {
    const r = __pf(v, { min: true });
    return r.length <= o.truncate ? r : `${__asymName(v)}{…}`;
  }
  const chai = v[Symbol.for('chai/inspect')];
  if (typeof chai === 'function') return String(chai.call(v, o));
  const custom = v[Symbol.for('nodejs.util.inspect.custom')];
  if (typeof custom === 'function') { const r = custom.call(v, 2, o, __inspect); return typeof r === 'string' ? r : __inspect(r, o); }
  if (typeof v.inspect === 'function') { const r = v.inspect(2, o); return typeof r === 'string' ? r : __inspect(r, o); }
  const proto = Object.getPrototypeOf(v);
  if (proto === Object.prototype || proto === null) return __inspectObject(v, o);
  let name = v[Symbol.toStringTag] || (v.constructor && v.constructor.name);
  if (!name || name === '_class') name = '<Anonymous Class>';
  o.truncate -= name.length;
  return `${name}${__inspectObject(v, o)}`;
}
// @vitest/utils `objDisplay` (truncateThreshold 40): loupe-truncated, and a rendering still that
// long collapses to a summary.
function __objDisplay(v) {
  const str = __inspect(v, { truncate: 40, seen: [] });
  if (str.length < 40) return str;
  const type = Object.prototype.toString.call(v);
  if (type === '[object Function]') return v.name ? `[Function: ${v.name}]` : '[Function]';
  if (type === '[object Array]') return `[ Array(${v.length}) ]`;
  if (type === '[object Object]') {
    const keys = Object.keys(v);
    return `{ Object (${keys.length > 2 ? `${keys.slice(0, 2).join(', ')}, ...` : keys.join(', ')}) }`;
  }
  return str;
}
// ---- vitest's assertion errors and messages (turbo-test#71) ----
// A failed built-in matcher throws chai's `AssertionError` with vitest's message: chai templates
// whose `#{this}` / `#{exp}` / `#{act}` show values as chai's `objDisplay` (loupe, 40 chars), and for
// the spy matchers vitest's call / return listings with @vitest/utils' `diff` of pretty-format
// output (no colours, as with NO_COLOR).
class __ChaiAssertionError extends Error {
  constructor(message, props) {
    super(message);
    this.name = 'AssertionError';
    this.showDiff = !(props && props.showDiff === false);
    if (props && 'actual' in props) this.actual = props.actual;
    if (props && 'expected' in props) this.expected = props.expected;
  }
}
// chai `getMessage`: fill a template's placeholders.
function __chaiMsg(tpl, obj, exp, act) {
  return tpl.replace(/#\{this\}/g, () => __objDisplay(obj)).replace(/#\{act\}/g, () => __objDisplay(act)).replace(/#\{exp\}/g, () => __objDisplay(exp));
}
// chai `assert(pass, msg, negatedMsg, expected, actual)` for an assertion on `obj`.
function __chaiAssert(negated, pass, msg, notMsg, obj, exp, act) {
  if (negated ? !pass : pass) return;
  throw new __ChaiAssertionError(__chaiMsg(negated ? notMsg : msg, obj, exp, act), { actual: act, expected: exp });
}
// The asymmetric matchers' vitest names and samples.
function __asymName(m) {
  const not = m.__negate;
  switch (m.__asymmetric) {
    case 'any': return 'Any';
    case 'anything': return 'Anything';
    case 'objectContaining': return not ? 'ObjectNotContaining' : 'ObjectContaining';
    case 'arrayContaining': return not ? 'ArrayNotContaining' : 'ArrayContaining';
    case 'stringContaining': return not ? 'StringNotContaining' : 'StringContaining';
    case 'stringMatching': return not ? 'StringNotMatching' : 'StringMatching';
    default: return String(m.__asymmetric);
  }
}
// @vitest/pretty-format: `o` = { indent, min, basic (printBasicPrototype), maxDepth, escape
// (escapeString), toJSON (callToJSON), errors (the Error plugin) }.
function __pf(val, o) {
  const cfg = {
    indent: o.min ? '' : ' '.repeat(o.indent == null ? 2 : o.indent),
    outer: o.min ? '' : '\n',
    inner: o.min ? ' ' : '\n',
    min: !!o.min,
    basic: o.basic !== false,
    maxDepth: o.maxDepth == null ? Infinity : o.maxDepth,
    escape: o.escape !== false,
    toJSON: o.toJSON !== false,
    errors: !!o.errors,
    fnName: o.fnName !== false,
    escapeRegex: !!o.escapeRegex,
    mocks: !!o.mocks,
  };
  return __pfPrint(val, cfg, '', 0, [], false);
}
function __pfCtorName(v) {
  return (typeof v.constructor === 'function' && v.constructor.name) || 'Object';
}
function __pfItems(items, cfg, ind, depth, refs, print) {
  if (!items.length) return '';
  const next = ind + cfg.indent;
  let out = cfg.outer;
  items.forEach((it, i) => {
    out += next + print(it, next);
    if (i < items.length - 1) out += `,${cfg.inner}`;
    else if (!cfg.min) out += ',';
  });
  return out + cfg.outer + ind;
}
function __pfPrint(v, cfg, ind, depth, refs, calledToJSON) {
  const p = (x, i) => __pfPrint(x, cfg, i, depth, refs, false);
  if (v instanceof Asymmetric) {
    const name = __asymName(v);
    const d = depth + 1;
    const pp = (x, i) => __pfPrint(x, cfg, i, d, refs, false);
    switch (v.__asymmetric) {
      case 'any': return `Any<${v.sample && v.sample.name}>`;
      case 'anything': return 'Anything';
      case 'arrayContaining': return d > cfg.maxDepth ? `[${name}]` : `${name} [${__pfItems(Array.from(v.sample), cfg, ind, d, refs, pp)}]`;
      case 'objectContaining': return d > cfg.maxDepth ? `[${name}]` : `${name} {${__pfProps(v.sample, cfg, ind, d, refs)}}`;
      default: return `${name} ${p(v.sample, ind)}`;
    }
  }
  // vitest's snapshot serializer for mock functions: `[MockFunction name] { calls, results }`
  if (cfg.mocks && typeof v === 'function' && v._isMockFunction) {
    const name = v.getMockName();
    let calls = '';
    if (v.mock.calls.length) {
      const next = ind + cfg.indent;
      calls = ` {${cfg.outer}${next}"calls": ${__pfPrint(v.mock.calls, cfg, next, depth, refs, false)}${cfg.min ? ', ' : ','}${cfg.outer}${next}"results": ${__pfPrint(v.mock.results, cfg, next, depth, refs, false)}${cfg.min ? '' : ','}${cfg.outer}${ind}}`;
    }
    return `[MockFunction${name === 'vi.fn()' ? '' : ` ${name}`}]${calls}`;
  }
  if (cfg.errors && v instanceof Error) {
    if (refs.includes(v)) return '[Circular]';
    const r = [...refs, v];
    const d = depth + 1;
    const entries = { message: v.message };
    if (v.cause !== undefined) entries.cause = v.cause;
    for (const k of Object.keys(v)) if (k !== 'message' && k !== 'cause') entries[k] = v[k];
    const name = v.name !== 'Error' ? v.name : __pfCtorName(v);
    if (d > cfg.maxDepth) return `[${name}]`;
    return `${name} {${__pfEntries(Object.entries(entries), cfg, ind, d, r, ': ')}}`;
  }
  if (v === true || v === false) return `${v}`;
  if (v === undefined) return 'undefined';
  if (v === null) return 'null';
  switch (typeof v) {
    case 'number': return Object.is(v, -0) ? '-0' : String(v);
    case 'bigint': return `${v}n`;
    case 'string': return cfg.escape ? `"${v.replace(/"|\\/g, '\\$&')}"` : `"${v}"`;
    case 'function': return cfg.fnName ? `[Function ${v.name || 'anonymous'}]` : '[Function]';
    case 'symbol': return String(v).replace(/^Symbol\((.*)\)(.*)$/, 'Symbol($1)');
  }
  const tag = Object.prototype.toString.call(v);
  if (tag === '[object WeakMap]') return 'WeakMap {}';
  if (tag === '[object WeakSet]') return 'WeakSet {}';
  if (tag === '[object Date]') return Number.isNaN(+v) ? 'Date { NaN }' : v.toISOString();
  if (tag === '[object Error]' || v instanceof Error) return `[${Error.prototype.toString.call(v)}]`;
  if (tag === '[object RegExp]') {
    const r = RegExp.prototype.toString.call(v);
    return cfg.escapeRegex ? r.replace(/[$()*+.?[\\\]^{|}]/g, '\\$&') : r;
  }
  if (refs.includes(v)) return '[Circular]';
  refs = [...refs, v];
  const d = depth + 1;
  const hit = d > cfg.maxDepth;
  if (cfg.toJSON && !hit && typeof v.toJSON === 'function' && !calledToJSON) {
    return __pfPrint(v.toJSON(), cfg, ind, d, refs, true);
  }
  const pp = (x, i) => __pfPrint(x, cfg, i, d, refs, false);
  if (tag === '[object Arguments]') return hit ? '[Arguments]' : `${cfg.min ? '' : 'Arguments '}[${__pfItems(Array.from(v), cfg, ind, d, refs, pp)}]`;
  if (Array.isArray(v) || ArrayBuffer.isView(v)) {
    const name = v.constructor.name;
    if (hit) return `[${name}]`;
    const prefix = cfg.min ? '' : !cfg.basic && name === 'Array' ? '' : `${name} `;
    return `${prefix}[${__pfItems(Array.from({ length: v.length }, (_, i) => v[i]), cfg, ind, d, refs, pp)}]`;
  }
  if (tag === '[object Map]') return hit ? '[Map]' : `Map {${__pfEntries([...v.entries()], cfg, ind, d, refs, ' => ')}}`;
  if (tag === '[object Set]') return hit ? '[Set]' : `Set {${__pfItems([...v.values()], cfg, ind, d, refs, pp)}}`;
  const name = __pfCtorName(v);
  if (hit) return `[${name}]`;
  const prefix = cfg.min ? '' : !cfg.basic && name === 'Object' ? '' : `${name} `;
  return `${prefix}{${__pfProps(v, cfg, ind, d, refs)}}`;
}
function __pfEntries(entries, cfg, ind, depth, refs, sep) {
  if (!entries.length) return '';
  const next = ind + cfg.indent;
  let out = cfg.outer;
  entries.forEach(([k, x], i) => {
    out += next + __pfPrint(k, cfg, next, depth, refs, false) + sep + __pfPrint(x, cfg, next, depth, refs, false);
    if (i < entries.length - 1) out += `,${cfg.inner}`;
    else if (!cfg.min) out += ',';
  });
  return out + cfg.outer + ind;
}
function __pfProps(v, cfg, ind, depth, refs) {
  const keys = Object.keys(v).sort();
  for (const s of Object.getOwnPropertySymbols(v)) if (Object.getOwnPropertyDescriptor(v, s).enumerable) keys.push(s);
  return __pfEntries(keys.map((k) => [k, v[k]]), cfg, ind, depth, refs, ': ');
}
// @vitest/utils `stringify` (the call listings without a diff).
function __stringify(v) {
  return __pf(v, { maxDepth: 10, escape: false });
}
// A line diff (longest common subsequence): [op, line] with op -1 delete, 1 insert, 0 common.
function __lineDiff(a, b) {
  const n = a.length;
  const m = b.length;
  if (n * m > 4e6) return [...a.map((l) => [-1, l]), ...b.map((l) => [1, l])];
  const lcs = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const out = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { out.push([0, i, j]); i++; j++; }
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) { out.push([-1, i]); i++; }
    else { out.push([1, j]); j++; }
  }
  while (i < n) out.push([-1, i++]);
  while (j < m) out.push([1, j++]);
  return out;
}
// jest-diff `joinAlignedDiffsNoExpand` (5 context lines, `@@` patch marks).
function __joinDiffs(diffs) {
  const ctx = 5;
  const ctx2 = ctx + ctx;
  const line = (ind, l, first) => (l.length ? `${ind} ${l}` : ind !== ' ' ? ind : '');
  const len = diffs.length;
  let jLength = len;
  let excessEdge = false;
  let excessBetween = 0;
  let i = 0;
  while (i !== len) {
    const start = i;
    while (i !== len && diffs[i][0] === 0) i++;
    if (start !== i) {
      if (start === 0) { if (i > ctx) { jLength -= i - ctx; excessEdge = true; } }
      else if (i === len) { const k = i - start; if (k > ctx) { jLength -= k - ctx; excessEdge = true; } }
      else { const k = i - start; if (k > ctx2) { jLength -= k - ctx2; excessBetween++; } }
    }
    while (i !== len && diffs[i][0] !== 0) i++;
  }
  const patch = excessBetween !== 0 || excessEdge;
  if (excessBetween !== 0) jLength += excessBetween + 1;
  else if (excessEdge) jLength += 1;
  const lines = [];
  let mark = 0;
  if (patch) lines.push('');
  let aStart = 0, bStart = 0, aEnd = 0, bEnd = 0;
  const common = (l) => { lines.push(line(' ', l)); aEnd++; bEnd++; };
  const del = (l) => { lines.push(line('-', l)); aEnd++; };
  const ins = (l) => { lines.push(line('+', l)); bEnd++; };
  const patchMark = () => `@@ -${aStart + 1},${aEnd - aStart} +${bStart + 1},${bEnd - bStart} @@`;
  i = 0;
  while (i !== len) {
    let start = i;
    while (i !== len && diffs[i][0] === 0) i++;
    if (start !== i) {
      if (start === 0) {
        if (i > ctx) { start = i - ctx; aStart = start; bStart = start; aEnd = aStart; bEnd = bStart; }
        for (let c = start; c !== i; c++) common(diffs[c][1]);
      } else if (i === len) {
        const end = i - start > ctx ? start + ctx : i;
        for (let c = start; c !== end; c++) common(diffs[c][1]);
      } else {
        const k = i - start;
        if (k > ctx2) {
          for (let c = start; c !== start + ctx; c++) common(diffs[c][1]);
          lines[mark] = patchMark();
          mark = lines.length;
          lines.push('');
          const omit = k - ctx2;
          aStart = aEnd + omit; bStart = bEnd + omit; aEnd = aStart; bEnd = bStart;
          for (let c = i - ctx; c !== i; c++) common(diffs[c][1]);
        } else {
          for (let c = start; c !== i; c++) common(diffs[c][1]);
        }
      }
    }
    while (i !== len && diffs[i][0] === -1) { del(diffs[i][1]); i++; }
    while (i !== len && diffs[i][0] === 1) { ins(diffs[i][1]); i++; }
  }
  if (patch) lines[mark] = patchMark();
  return lines.join('\n');
}
// jest-diff `diffLinesUnified2`: align by the compare lines, show the display lines.
function __diffLines(aDisp, bDisp, aCmp = aDisp, bCmp = bDisp) {
  const empty = (l) => l.length === 1 && l[0].length === 0;
  if (empty(aDisp) && empty(aCmp)) { aDisp = []; aCmp = []; }
  if (empty(bDisp) && empty(bCmp)) { bDisp = []; bCmp = []; }
  if (aDisp.length !== aCmp.length || bDisp.length !== bCmp.length) { aCmp = aDisp; bCmp = bDisp; }
  const diffs = __lineDiff(aCmp, bCmp).map((d) => (d[0] === -1 ? [-1, aDisp[d[1]]] : d[0] === 1 ? [1, bDisp[d[1]]] : [0, bDisp[d[2]]]));
  return __joinDiffs(diffs);
}
function __diffType(v) {
  if (v === undefined) return 'undefined';
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'array';
  if (typeof v !== 'object') return typeof v;
  if (v.constructor === RegExp) return 'regexp';
  if (v.constructor === Map) return 'map';
  if (v.constructor === Set) return 'set';
  if (v.constructor === Date) return 'date';
  return 'object';
}
const __NO_DIFF = 'Compared values have no visual difference.';
// @vitest/utils `diff(expected, received, { omitAnnotationLines: true })`.
function __vdiff(a, b) {
  if (Object.is(a, b)) return '';
  let type = __diffType(a);
  if (a instanceof Asymmetric) {
    const t = { any: a.sample === String ? 'string' : a.sample === Number ? 'number' : a.sample === Boolean ? 'boolean' : undefined, stringContaining: 'string', stringMatching: 'string', objectContaining: 'object', arrayContaining: 'array' }[a.__asymmetric];
    if (!t) return undefined;
    if (t === 'string') return undefined;
    type = t;
  }
  const fmt = { indent: 2, basic: false, maxDepth: 20, errors: true };
  const fallback = { ...fmt, maxDepth: 8, toJSON: false };
  if (type !== __diffType(b)) {
    return `- Expected:\n${__pf(a, fallback)}\n\n+ Received:\n${__pf(b, fallback)}`;
  }
  const objects = (opts) => {
    const ac = __pf(a, { ...opts, indent: 0 });
    const bc = __pf(b, { ...opts, indent: 0 });
    if (ac === bc) return __NO_DIFF;
    return __diffLines(__pf(a, opts).split('\n'), __pf(b, opts).split('\n'), ac.split('\n'), bc.split('\n'));
  };
  const sortMap = (x) => new Map([...x.entries()].sort());
  const sortSet = (x) => new Set([...x.values()].sort());
  switch (type) {
    case 'string': return __diffLines(a.split('\n'), b.split('\n'));
    case 'boolean':
    case 'number': {
      const af = __pf(a, fmt);
      const bf = __pf(b, fmt);
      return af === bf ? '' : __diffLines(af.split('\n'), bf.split('\n'));
    }
    case 'map': a = sortMap(a); b = sortMap(b); break;
    case 'set': a = sortSet(a); b = sortSet(b); break;
  }
  let d;
  let threw = false;
  try { d = objects(fmt); } catch (e) { threw = true; }
  if (d === undefined || d === __NO_DIFF) {
    d = objects(fallback);
    if (d !== __NO_DIFF && !threw) d = `Compared values serialize to the same structure.\nPrinting internal object structure without calling \`toJSON\` instead.\n\n${d}`;
  }
  return d;
}
function __ordinal(n) {
  const j = n % 10;
  const k = n % 100;
  if (j === 1 && k !== 11) return `${n}st`;
  if (j === 2 && k !== 12) return `${n}nd`;
  if (j === 3 && k !== 13) return `${n}rd`;
  return `${n}th`;
}
// vitest's `formatCalls` / `formatReturns`: each call (or return) diffed against `shown`, or listed.
function __formatCalls(spy, msg, shown) {
  const calls = spy.mock.calls;
  if (calls.length) {
    msg += `\n\nReceived:\n\n${calls.map((args, i) => {
      let s = `  ${__ordinal(i + 1)} ${spy.getMockName()} call:\n\n`;
      s += shown ? __vdiff(shown, args) : __stringify(args).split('\n').map((l) => `    ${l}`).join('\n');
      return `${s}\n`;
    }).join('\n')}`;
  }
  return `${msg}\n\nNumber of calls: ${calls.length}\n`;
}
function __formatReturns(spy, msg, shown) {
  const results = spy.mock.results;
  if (results.length) {
    msg += `\n\nReceived:\n\n${results.map((r, i) => {
      let s = `  ${__ordinal(i + 1)} ${spy.getMockName()} call return:\n\n`;
      s += shown ? __vdiff(shown, r.value) : __stringify(r).split('\n').map((l) => `    ${l}`).join('\n');
      return `${s}\n`;
    }).join('\n')}`;
  }
  return `${msg}\n\nNumber of calls: ${spy.mock.calls.length}\n`;
}
// vitest's `getObjectSubset` count: the properties of `actual` that `subset` does not name.
function __strippedCount(actual, subset, seen = new WeakSet()) {
  const isObj = (x) => x != null && typeof x === 'object' && !Array.isArray(x);
  const keys = (x) => [...Object.keys(x), ...Object.getOwnPropertySymbols(x).filter((s) => Object.getOwnPropertyDescriptor(x, s).enumerable)];
  if (Array.isArray(actual)) {
    if (Array.isArray(subset) && subset.length === actual.length) return actual.reduce((n, x, i) => n + __strippedCount(x, subset[i], seen), 0);
    return 0;
  }
  if (actual instanceof Date || !isObj(actual) || !isObj(subset)) return 0;
  if (deepEqual(actual, subset) || __subsetMatches(actual, subset)) return 0;
  seen.add(actual);
  let n = 0;
  for (const k of keys(actual)) {
    if (k in subset) {
      if (!seen.has(actual[k])) n += __strippedCount(actual[k], subset[k], seen);
    } else if (!seen.has(actual[k])) {
      n += 1;
      if (isObj(actual[k])) n += keys(actual[k]).length;
    }
  }
  return n;
}
function __subsetMatches(a, e) {
  if (e instanceof Asymmetric) return e.matches(a);
  if (e == null || typeof e !== 'object') return deepEqual(a, e);
  if (Array.isArray(e)) return Array.isArray(a) && a.length === e.length && e.every((x, i) => __subsetMatches(a[i], x));
  return a != null && typeof a === 'object' && Object.keys(e).every((k) => __subsetMatches(a[k], e[k]));
}

// @vitest/utils `format(spec, item)` for one placeholder.
function __formatOne(spec, item) {
  const v = item;
  let consumed = true;
  let out;
  switch (spec) {
    case '%%': consumed = false; out = '%'; break;
    case '%s':
      if (typeof v === 'bigint') out = `${v}n`;
      else if (typeof v === 'number' && v === 0 && 1 / v < 0) out = '-0';
      else if (typeof v === 'object' && v !== null) out = (typeof v.toString === 'function' && v.toString !== Object.prototype.toString) ? v.toString() : __inspect(v);
      else out = String(v);
      break;
    case '%d': out = typeof v === 'bigint' ? `${v}n` : typeof v === 'symbol' ? 'NaN' : Number(v).toString(); break;
    case '%i': out = typeof v === 'bigint' ? `${v}n` : Number.parseInt(String(v)).toString(); break;
    case '%f': out = Number.parseFloat(String(v)).toString(); break;
    case '%o': case '%O': out = __inspect(v); break;
    case '%c': out = ''; break;
    case '%j':
      try { out = JSON.stringify(v); } catch (e) {
        const m = String(e && e.message);
        if (/circular structure|cyclic structures|cyclic object/.test(m)) out = '[Circular]'; else throw e;
      }
      break;
  }
  // an unconsumed item is appended, as `format` appends extra arguments
  if (!consumed) out += ` ${v === null || typeof v !== 'object' ? (typeof v === 'symbol' ? v.toString() : v) : __inspect(v)}`;
  return out;
}
function __objectAttr(source, path, dflt) {
  let r = source;
  for (const k of path.replace(/\[(\d+)\]/g, '.$1').split('.')) {
    r = new Object(r)[k];
    if (r === undefined) return dflt;
  }
  return r;
}
function __negZeroOrNaN(v) {
  if (Object.is(v, -0)) return true;
  if (!Number.isNaN(v) || typeof v !== 'number') return false;
  const f = new Float64Array([v]);
  return new Uint32Array(f.buffer)[1] >>> 31 === 1;
}
function __formatTitle(template, items, idx) {
  if (template.includes('%#') || template.includes('%$')) {
    template = template.replace(/%%/g, '__tt_escaped_%__').replace(/%#/g, `${idx}`).replace(/%\$/g, `${idx + 1}`).replace(/__tt_escaped_%__/g, '%%');
  }
  const count = template.split('%').length - 1;
  if (template.includes('%f')) {
    (template.match(/%f/g) || []).forEach((_, i) => {
      if (__negZeroOrNaN(items[i])) {
        let n = 0;
        template = template.replace(/%f/g, (m) => (++n === i + 1 ? '-%f' : m));
      }
    });
  }
  const isObjectItem = items[0] != null && typeof items[0] === 'object' && !Array.isArray(items[0]);
  const attrs = (str) => str.replace(/\$([$\w.]+)/g, (_, key) => {
    const isArrayKey = /^\d+$/.test(key);
    if (!isObjectItem && !isArrayKey) return `$${key}`;
    const el = isArrayKey ? __objectAttr(items, key) : undefined;
    return __objDisplay(isObjectItem ? __objectAttr(items[0], key, el) : el);
  });
  let out = '';
  let last = 0;
  let i = 0;
  for (const m of template.matchAll(/%[sdjifoOc%]/g)) {
    if (last < m.index) out += attrs(template.slice(last, m.index));
    out += i < count ? __formatOne(m[0], items[i++]) : m[0];
    last = m.index + m[0].length;
  }
  if (last < template.length) out += attrs(template.slice(last));
  return out;
}
function __formatName(name) {
  return typeof name === 'string' ? name : typeof name === 'function' ? name.name || '<anonymous>' : String(name);
}
// A tagged-template table (`` test.each`a | b ${1} | ${2}` ``) as one object per row.
function __eachCases(cases, targs) {
  if (!Array.isArray(cases) || !targs.length) return cases;
  const header = cases.join('').trim().replace(/ /g, '').split('\n').map((l) => l.split('|'))[0];
  const rows = [];
  for (let r = 0; r < Math.floor(targs.length / header.length); r++) {
    const row = {};
    header.forEach((h, c) => { row[h] = targs[r * header.length + c]; });
    rows.push(row);
  }
  return rows;
}
// `.each`: one task per row, the row spread into the handler when every row is an array.
function __each(register, cases, targs) {
  cases = __eachCases(cases, targs);
  return (name, a, b) => {
    const title = __formatName(name);
    const fnFirst = typeof a === 'function';
    const handler = fnFirst ? a : b;
    const opts = fnFirst ? b : a;
    const spread = cases.every(Array.isArray);
    cases.forEach((row, idx) => {
      const items = Array.isArray(row) ? row : [row];
      const args = spread ? items : [row];
      // `ctx` is only defined for an it.extend test (its resolved fixtures, passed last)
      const fn = (ctx) => (typeof handler === 'function' ? (ctx === undefined ? handler(...args) : handler(...args, ctx)) : undefined);
      register(__formatTitle(title, items, idx), fn, opts);
    });
  };
}

// `.for` (turbo-test#68): like `.each`, but the row is passed as ONE argument (never spread),
// followed by the test context for a test (its fixtures under it.extend; otherwise a minimal
// context carrying `expect`). A `describe.for` body gets the row only.
function __for(register, cases, targs, isSuite) {
  cases = __eachCases(cases, targs);
  return (name, a, b) => {
    const title = __formatName(name);
    const fnFirst = typeof a === 'function';
    const handler = fnFirst ? a : b;
    const opts = fnFirst ? b : a;
    cases.forEach((item, idx) => {
      const ctxOf = (ctx) => (ctx === undefined ? { expect: globalThis.expect } : ctx);
      const fn = (ctx) => (typeof handler !== 'function' ? undefined : isSuite ? handler(item) : handler(item, ctxOf(ctx)));
      register(__formatTitle(title, item == null ? [] : Array.isArray(item) ? item : [item], idx), fn, opts);
    });
  };
}

// ---- the test file's scope for its hoisted vi.mock factories (turbo-test#72, mock_scope.rs) ----
// The prepass runs the factories inside `with (__ttScope)`: the file's top-level names the
// factories mention (__ttScopeNames) resolve through getters / setters the file itself registers
// when it runs (__ttScopeBind); read before that, a name is in its temporal dead zone.
const __scopeGet = new Map();
const __scopeSet = new Map();
let __scopeNames = new Set();
globalThis.__ttScopeNames = (names) => { __scopeNames = new Set(names); __scopeGet.clear(); __scopeSet.clear(); };
globalThis.__ttScopeBind = (get, set) => {
  for (const k of Object.keys(get)) __scopeGet.set(k, get[k]);
  for (const k of Object.keys(set)) __scopeSet.set(k, set[k]);
};
const __tdz = (k) => new ReferenceError(`Cannot access '${k}' before initialization`);
globalThis.__ttScope = new Proxy(Object.create(null), {
  has: (_, k) => __scopeNames.has(k),
  get: (_, k) => {
    if (typeof k !== 'string') return undefined; // `with` asks for Symbol.unscopables
    const g = __scopeGet.get(k);
    if (!g) throw __tdz(k);
    return g();
  },
  set: (_, k, v) => {
    const set = __scopeSet.get(k);
    if (set) { set(v); return true; }
    throw __scopeGet.has(k) ? new TypeError('Assignment to constant variable.') : __tdz(k);
  },
});

// ---- collector: describe / it / hooks ----
// A suite's `tasks` hold its tests and child suites (`suite: true`) in declaration order: vitest
// runs and reports them in that order (turbo-test#64).
const root = { name: '', tasks: [], suite: true, hooks: { be: [], ae: [], ba: [], aa: [] }, parent: null };
let current = root;

function describe(name, a, b) {
  const fn = typeof a === 'function' ? a : (typeof b === 'function' ? b : () => {});
  const suite = { name, tasks: [], suite: true, hooks: { be: [], ae: [], ba: [], aa: [] }, parent: current };
  current.tasks.push(suite);
  const prev = current;
  current = suite;
  fn();
  current = prev;
  return suite;
}
// A skipped/todo suite registers no tests, but it IS a collected task for vitest's "No test suite
// found in file" check (see __tt.run) — count it so such a file is not reported as empty.
// describe.skip / describe.todo collect their block (vitest runs the callback at collection) in
// `skip` / `todo` mode: its tests are reported skipped / todo and nothing in it runs (#63).
describe.skip = (name, a, b) => { describe(name, a, b).mode = 'skip'; };
describe.only = (name, a, b) => { __tt.hasOnly = true; describe(name, a, b).only = true; };
describe.todo = (name, a, b) => { describe(name, a, b).mode = 'todo'; };
describe.skipIf = (cond) => (cond ? describe.skip : describe);
describe.runIf = (cond) => (cond ? describe : describe.skip);
describe.concurrent = describe; // accepted; a file's tests still run sequentially
describe.each = (cases, ...targs) => __each(describe, cases, targs);
describe.for = (cases, ...targs) => __for(describe, cases, targs, true);
// Supports both vitest signatures: it(name, fn, opts) and it(name, opts, fn).
function __normTest(a, b) {
  let fn, o;
  if (typeof a === 'function') { fn = a; o = (typeof b === 'object' && b ? b : (typeof b === 'number' ? { timeout: b } : {})); }
  else if (typeof a === 'object' && a) { o = a; fn = (typeof b === 'function' ? b : () => {}); }
  else { fn = (typeof b === 'function' ? b : () => {}); o = {}; }
  return { fn, o };
}
// Default per-test timeout (ms): `--testTimeout` (host → globalThis.__TT_DEFAULT_TIMEOUT) else
// vitest's 5000ms default. A test's own `{ timeout }` overrides this in runSuite.
function __defaultTimeout() {
  const n = Number(globalThis.__TT_DEFAULT_TIMEOUT);
  return Number.isFinite(n) && n > 0 ? n : 5000;
}
// Collect a test into the current suite. Under `includeTaskLocation` it records where it was
// registered from (`[line, column]` in the test file, native `__ttCallSite`, #63).
function __addTest(t) {
  if (globalThis.__TT_TASK_LOCATION) t.location = globalThis.__ttCallSite();
  current.tasks.push(t);
}
function it(name, a, b) {
  const { fn, o } = __normTest(a, b);
  __addTest({ name, fn, skip: false, only: false, retry: o.retry || 0, timeout: o.timeout });
}
it.skip = (name, a, b) => __addTest({ name, fn: __normTest(a, b).fn, skip: true });
it.todo = (name) => __addTest({ name, fn: () => {}, skip: true, todo: true });
it.only = (name, a, b) => { __tt.hasOnly = true; const { fn, o } = __normTest(a, b); __addTest({ name, fn, skip: false, only: true, retry: o.retry || 0, timeout: o.timeout }); };
it.concurrent = it; // accepted; this runner executes a file's tests sequentially
it.each = (cases, ...targs) => __each(it, cases, targs);
it.for = (cases, ...targs) => __for(it, cases, targs);
it.skipIf = (cond) => (name, fn) => __addTest({ name, fn, skip: !!cond, only: false, retry: 0 });
it.runIf = (cond) => (name, fn) => __addTest({ name, fn, skip: !cond, only: false, retry: 0 });
// it.fails: the test PASSES iff its body throws (runSuite inverts the outcome via `fails`).
it.fails = (name, a, b) => { const { fn, o } = __normTest(a, b); __addTest({ name, fn, skip: false, only: false, retry: o.retry || 0, fails: true }); };
// it.extend({...}) — test-context fixtures (best-effort). vitest passes a merged context object as
// the test fn's first arg; fixtures defined as plain values or `async ({}, use) => use(value)`
// functions are resolved here and provided. Returns a new test fn with these fixtures baked in.
it.extend = (fixtures) => {
  const resolveCtx = async () => {
    const ctx = {};
    for (const k of Object.keys(fixtures || {})) {
      const f = fixtures[k];
      if (typeof f === 'function') {
        // vitest fixture fn: ({...deps}, use) => { ... await use(value) }. Capture the used value.
        let captured;
        const use = (v) => { captured = v; return Promise.resolve(); };
        const r = f(ctx, use);
        if (r && typeof r.then === 'function') await r;
        ctx[k] = captured;
      } else {
        ctx[k] = f;
      }
    }
    return ctx;
  };
  const wrap = (userFn) => async () => { const ctx = await resolveCtx(); return userFn(ctx); };
  const ext = (name, a, b) => { const { fn, o } = __normTest(a, b); __addTest({ name, fn: wrap(fn), skip: false, only: false, retry: o.retry || 0 }); };
  ext.skip = (name, a, b) => __addTest({ name, fn: wrap(__normTest(a, b).fn), skip: true });
  ext.only = (name, a, b) => { __tt.hasOnly = true; const { fn, o } = __normTest(a, b); __addTest({ name, fn: wrap(fn), skip: false, only: true, retry: o.retry || 0 }); };
  ext.each = (cases, ...targs) => __each(ext, cases, targs);
  ext.for = (cases, ...targs) => __for(ext, cases, targs);
  ext.extend = (more) => it.extend({ ...fixtures, ...more });
  return ext;
};
globalThis.describe = describe;
globalThis.it = it;
globalThis.test = it;
globalThis.beforeEach = (fn) => current.hooks.be.push(fn);
globalThis.afterEach = (fn) => current.hooks.ae.push(fn);
globalThis.beforeAll = (fn) => current.hooks.ba.push(fn);
globalThis.afterAll = (fn) => current.hooks.aa.push(fn);

// ---- runner ----
// One per-test record for the reporters (`summary.tests`): the `describe > test` label, the
// title and its enclosing describe names (vitest's `title` / `ancestorTitles`), the status
// (`passed` / `failed` / `skipped` / `todo`), and for a test that ran its real start time and
// duration (ms) and its failure messages (each error's `stack`, else its `message`, as vitest's
// JSON reporter) (#63).
function __testRecord(t, chain, status, extra) {
  return {
    name: chain.map((s) => s.name).filter(Boolean).concat(t.name).join(' > '),
    title: t.name,
    ancestors: chain.slice(1).map((s) => s.name),
    status,
    duration_ms: 0,
    location: t.location,
    ...extra,
  };
}

// Whether a test of `suite` (or of a nested suite) will run: not skip / todo / filtered by -t /
// left out by `.only`. vitest marks a suite none of whose tasks run `skip`: its hooks do not run.
function __runnable(suite, chain) {
  if (suite.mode) return false;
  const inner = [...chain, suite];
  return suite.tasks.some((t) => (t.suite
    ? __runnable(t, inner)
    : !t.skip && !t.todo && !t.notOnly && !(__tt.namePattern && !__tt.namePattern.test(inner.slice(1).map((s) => s.name).concat(t.name).join(' ')))));
}

// Returns whether the suite failed (a failed test, or a hook / empty-suite error, in it or a
// nested suite). `mode` is the enclosing mode: '' (run), 'skip', 'todo' or 'hook' (an enclosing
// beforeAll failed).
async function runSuite(suite, ancestors, summary, mode = '') {
  // vitest (turbo-test#69): a describe with no task fails with "No test found in suite <name>";
  // a throwing beforeAll fails the suite (a file-level failure, not a failed test) and skips its
  // tests and nested suites, whose hooks do not run, while its afterAll hooks still run. A
  // throwing afterAll fails the suite too. Neither rejects the run: the file still reports.
  // A describe.skip / describe.todo block (or one inside it) runs nothing (#63).
  const skipAll = mode || suite.mode || '';
  const record = (state) => { if (ancestors.length) summary.suites.push(skipAll === 'todo' ? 'todo' : state); };
  if (ancestors.length && !skipAll && !suite.tasks.length) {
    summary.failures.push(`No test found in suite ${suite.name}`);
    record('fail');
    return true;
  }
  let failed = false;
  // nothing in it runs: no hooks either
  const idle = !skipAll && !__runnable(suite, ancestors);
  let hookFailed = mode === 'hook';
  // a forbidden `describe.only` (turbo-test#76) fails the suite like a throwing beforeAll, but
  // none of its hooks run
  const forbidden = !skipAll && !!suite.forbiddenOnly;
  if (forbidden) {
    summary.failures.push(`${suite.name}: ${__ONLY_FORBIDDEN}`);
    hookFailed = true;
    failed = true;
  }
  if (!skipAll && !idle && !forbidden) {
    for (const h of suite.hooks.ba) {
      try { await h(); } catch (e) {
        summary.failures.push(`${suite.name || '<root>'} beforeAll: ${(e && e.message) || String(e)}`);
        hookFailed = true;
        failed = true;
        break;
      }
    }
  }
  const chain = [...ancestors, suite];
  for (const t of suite.tasks) {
    if (t.suite) { failed = (await runSuite(t, chain, summary, hookFailed ? 'hook' : skipAll)) || failed; continue; }
    const label = chain.map((s) => s.name).filter(Boolean).concat(t.name).join(' > ');
    // -t/--testNamePattern: skip tests whose full name does not match the regex (vitest: unanchored,
    // case-sensitive, tested against `getTaskFullName` — the enclosing describe names and the
    // test's joined with a space, turbo-test#66).
    const fullName = chain.slice(1).map((s) => s.name).concat(t.name).join(' ');
    // A test that does not run is reported `skipped` (`todo` for it.todo / inside describe.todo),
    // as vitest's JSON reporter does — -t-filtered ones included (#63).
    // Precedence as vitest's `interpretTaskModes` + runner: a failed beforeAll skips everything;
    // a describe.todo block makes its tests todo and a describe.skip block skips them (an
    // it.todo stays todo); otherwise -t, then the test's own todo / skip / `.only`.
    const filtered = !skipAll && __tt.namePattern && !__tt.namePattern.test(fullName);
    // a `.only` test inside describe.skip / describe.todo: vitest's interpretTaskModes does not
    // descend there, so it keeps mode `only`, never runs and is reported `pending` (#77)
    if (t.only && (skipAll === 'skip' || skipAll === 'todo')) {
      summary.tests.push(__testRecord(t, chain, 'pending'));
      summary.skippedNames.push(label);
      continue;
    }
    const todo = !hookFailed && (skipAll === 'todo' || (!filtered && !!t.todo));
    if (todo || hookFailed || skipAll || filtered || t.skip || t.notOnly) {
      summary.skipped++;
      // a forbidden `.only` test that does not run keeps its error (vitest)
      const extra = t.forbiddenOnly ? { failure_messages: [__onlyForbiddenError()] } : undefined;
      summary.tests.push(__testRecord(t, chain, todo ? 'todo' : 'skipped', extra));
      // vitest marks a skipped test's snapshots checked (not obsolete)
      if (!todo) summary.skippedNames.push(label);
      continue;
    }
    // a forbidden `.only` test fails without running: no hooks, no body, no duration (#76)
    if (t.forbiddenOnly) {
      failed = true;
      summary.failed++;
      summary.failures.push(`${label}: ${__ONLY_FORBIDDEN}`);
      summary.tests.push(__testRecord(t, chain, 'failed', { message: __ONLY_FORBIDDEN, failure_messages: [__onlyForbiddenError()], not_run: true }));
      continue;
    }
    // retry: per-test `{ retry }` wins; else the global default (--retry → __TT_DEFAULT_RETRY).
    const retry = (t.retry != null && t.retry > 0)
      ? t.retry
      : (Number(globalThis.__TT_DEFAULT_RETRY) > 0 ? Number(globalThis.__TT_DEFAULT_RETRY) : 0);
    const attempts = retry + 1;
    // timeout (ms): per-test `{ timeout }` wins; else the --testTimeout/5000 default.
    const timeoutMs = (typeof t.timeout === 'number' && t.timeout > 0) ? t.timeout : __defaultTimeout();
    let lastErr;
    let ok = false;
    // Real wall time for the per-test start / duration the reporters show. NOT performance.now()
    // (stubbed to a constant 0 in this runtime, see the perf stub near top of file) nor Date.now()
    // (fake timers / vi.setSystemTime replace it).
    const __t0 = __loop.realNow();
    for (let a = 0; a < attempts && !ok; a++) {
      // Reset per-test snapshot + assertion-count state (snapshot counter is keyed by test name,
      // so distinct multi-snapshot tests don't collide; assertion enforcement resets each attempt).
      __snap.testName = label;
      for (const k of Object.keys(__snap.counters)) if (k === label || k.startsWith(`${label} > `)) __snap.counters[k] = 0;
      __snap.assertCount = 0;
      __snap.expectedAssertions = null;
      __snap.hasAssertions = false;
      try {
        for (const s of chain) for (const h of s.hooks.be) await h();
        // Race t.fn() against a timeout. The timeout is an INTERNAL one-shot timer (separate
        // from the user/fake-timer queue), so the Rust drive loop advances the virtual clock to
        // it and rejects even a test that never resolves (e.g. `await new Promise(() => {})`) —
        // otherwise that hangs the whole worker — while staying invisible to vi.runAllTimers etc.
        let timer;
        const timed = new Promise((_, rej) => {
          timer = __scheduleInternal(() => rej(new Error(`test timed out in ${timeoutMs}ms`)), timeoutMs);
        });
        try {
          await Promise.race([Promise.resolve().then(() => t.fn()), timed]);
        } finally {
          __clearInternal(timer);
        }
        // expect.assertions(n) / hasAssertions() enforcement (vitest: checked after the test body).
        if (__snap.expectedAssertions != null && __snap.assertCount !== __snap.expectedAssertions) {
          throw new Error(`expected ${__snap.expectedAssertions} assertion(s) but got ${__snap.assertCount}`);
        }
        if (__snap.hasAssertions && __snap.assertCount === 0) {
          throw new Error('expected at least one assertion but got none');
        }
        ok = true;
      } catch (e) {
        lastErr = e;
      }
      // it.fails: invert the outcome — a thrown error is the success condition, a clean pass fails.
      if (t.fails) {
        if (lastErr) { ok = true; lastErr = undefined; }
        else { ok = false; lastErr = new Error('expected test to fail, but it passed'); }
      }
      // afterEach (incl. @testing-library cleanup) runs even when the test failed — otherwise a
      // failed test's mounted DOM leaks into the next test. Hook errors here don't fail a test
      // that already passed (matches the prior behavior; avoids mass cleanup-throw failures).
      for (const s of [...chain].reverse()) for (const h of s.hooks.ae) {
        try { await h(); } catch (e) {}
      }
    }
    const __dur = __loop.realNow() - __t0;
    const timing = { duration_ms: __dur, start_ms: __t0 };
    // a passing it.fails is vitest's `expected fail` in the summary footer (#78)
    if (ok) { summary.passed++; summary.tests.push(__testRecord(t, chain, 'passed', t.fails ? { ...timing, expected_fail: true } : timing)); }
    else {
      failed = true;
      summary.failed++;
      let msg = lastErr ? (lastErr.message || String(lastErr)) : String(lastErr);
      if (lastErr && lastErr.errors && lastErr.errors.length) {
        msg += ' [' + lastErr.errors.map((x) => (x && x.message) || String(x)).join('; ') + ']';
      }
      if ((!msg || msg === 'AggregateError' || msg === 'Error') && lastErr && lastErr.stack) {
        msg = String(lastErr.stack).split('\n').slice(0, 3).join(' | ');
      }
      if (globalThis.__TT_STACK && lastErr) {
        const e0 = lastErr.errors && lastErr.errors[0] ? lastErr.errors[0] : lastErr;
        if (e0 && e0.stack) msg += '\n          @ ' + String(e0.stack).split('\n').slice(0, 5).join('\n          @ ');
      }
      summary.failures.push(`${label}: ${msg}`);
      const failure = (lastErr && (lastErr.stack || lastErr.message)) || String(lastErr);
      summary.tests.push(__testRecord(t, chain, 'failed', { ...timing, message: msg, failure_messages: [String(failure)] }));
    }
  }
  if (!skipAll && !idle && !forbidden) {
    for (const h of suite.hooks.aa) {
      try { await h(); } catch (e) {
        summary.failures.push(`${suite.name || '<root>'} afterAll: ${(e && e.message) || String(e)}`);
        failed = true;
      }
    }
  }
  record(failed ? 'fail' : skipAll || idle ? 'skip' : 'pass');
  return failed;
}

// `.only` (vitest `interpretTaskModes`): with a `.only` in the file, a task runs when it or an
// enclosing suite is `.only` — except that inside a suite holding `.only` tasks only those run.
// Every other test is marked `notOnly` (skipped) (turbo-test#65). With allowOnly off
// (`forbid`), vitest's `checkAllowOnly` marks each `.only` task it reaches — not those inside a
// describe.skip / describe.todo — failed with its message, and the task does not run (#76).
function __someOnly(suite) {
  return suite.tasks.some((t) => t.only || (t.suite && __someOnly(t)));
}
const __ONLY_FORBIDDEN = '[Vitest] Unexpected .only modifier. Remove it or pass --allowOnly argument to bypass this error';
function __onlyForbiddenError() {
  const e = new Error(__ONLY_FORBIDDEN);
  return String(e.stack || `Error: ${__ONLY_FORBIDDEN}`);
}
function __interpretOnly(suite, parentIsOnly, forbid) {
  const suiteIsOnly = parentIsOnly || !!suite.only;
  const hasSomeOnly = __someOnly(suite);
  for (const t of suite.tasks) {
    const include = hasSomeOnly ? !!t.only || (!!t.suite && __someOnly(t)) : suiteIsOnly || !!t.only;
    if (forbid && t.only) t.forbiddenOnly = true;
    if (t.suite) __interpretOnly(t, include, forbid && !t.mode);
    else if (!include) t.notOnly = true;
  }
}

globalThis.__tt = {
  hasOnly: false,
  // Skipped/todo describe blocks seen in this file (they register nothing in `root`).
  // Compiled lazily from globalThis.__TT_NAME_PATTERN (set by the host from -t/--testNamePattern).
  get namePattern() {
    if (this._np !== undefined) return this._np;
    const src = globalThis.__TT_NAME_PATTERN;
    this._np = src ? (() => { try { return new RegExp(src); } catch { return null; } })() : null;
    return this._np;
  },
  async run() {
    // `suites`: one state per describe block ('pass' / 'fail' / 'skip' / 'todo'), for the JSON
    // reporter's suite counts.
    const summary = { passed: 0, failed: 0, skipped: 0, failures: [], tests: [], suites: [], skippedNames: [] };
    // vitest (runner `runFiles`): a file that collected NO task at all — no test or suite, skipped
    // and todo ones included — fails with "No test suite found in file <path>" unless
    // passWithNoTests. Reported as the file's only failure entry with no failed test; the CLI
    // turns that into a file-level error (cli.rs `file_result`).
    if (!root.tasks.length && !globalThis.__TT_PASS_WITH_NO_TESTS) {
      summary.failures.push(`No test suite found in file ${globalThis.__ttFile}`);
      return summary;
    }
    // allowOnly off (host → __TT_FORBID_ONLY: --no-allowOnly, or vitest's default on CI): each
    // `.only` task fails in place, as vitest's `checkAllowOnly` (turbo-test#76).
    if (this.hasOnly) __interpretOnly(root, false, !!globalThis.__TT_FORBID_ONLY);
    await runSuite(root, [], summary);
    summary.snapshot = __snapFinish(summary.skippedNames);
    return summary;
  },
};

// Isolate-reuse (TURBO_REUSE_ISOLATE): when a worker reuses one isolate+context across files,
// the runner calls this between files to wipe per-file framework state — collected tests, spies,
// timers, fake clock, pending mocks, vi.hoisted cache, and env/global stubs — so file N+1 starts
// as clean as a fresh isolate would. Module caches (node_modules) are kept on the Rust side.
// Snapshot the hooks registered by setup files (run once per worker). The per-file reset
// restores root.hooks to this baseline so the setup file's afterEach(cleanup) etc. survive
// across files, while each test file's own top-level hooks are dropped.
globalThis.__ttCaptureHookBaseline = () => {
  globalThis.__ttHookBaseline = {
    be: root.hooks.be.slice(),
    ae: root.hooks.ae.slice(),
    ba: root.hooks.ba.slice(),
    aa: root.hooks.aa.slice(),
  };
  // Spies created during setup (e.g. the analytics trackEvent vi.fn()) are the first entries in
  // __spies and must persist; everything appended by a test file is per-file and must be dropped
  // on reset. Record the setup count as the truncation point.
  globalThis.__ttSpyBaseline = __spies.length;
};

// Per-file reset, modeled on vitest's isolate:false loop (vitest/dist .../base*.js): with
// isolation OFF, vitest does NOT reset modules, the DOM, spies, timers or globals between files
// — it only calls `vi.resetConfig()` + `vi.restoreAllMocks()`, and its runner scopes test
// collection per file. We mirror that minimally. Over-resetting (clearing DOM, truncating the
// spy registry, unstubbing globals, poking React internals) was what broke interaction tests
// ~100 files into a shared isolate — vitest proves none of that is needed.
globalThis.__ttResetForNextFile = () => {
  // turbo-test collects tests into a single global `root` (vitest scopes per file via
  // startTests([file])) — so we must drop the prior file's collected tests/suites. Setup files
  // run once per worker, so root.hooks only holds setup hooks; leave them in place.
  root.tasks = [];
  // Reset snapshot counters between files (keyed by test name; cleared to avoid cross-file drift).
  __snap.counters = Object.create(null);
  __snapState.loaded = false;
  // NOTE: root.hooks is intentionally LEFT in place. Restoring it to a post-global-setup baseline
  // drops @testing-library's auto-cleanup afterEach for most files (it registers during a test
  // file's first render, not during global setup) → DOM accumulates → mass failure. The downside
  // (a test-file-imported setup module's root afterEach leaking, e.g. marketing analytics-test-
  // setup) is the lesser evil and handled elsewhere.
  current = root;
  globalThis.__tt.hasOnly = false;
  // mock-prepass scratch (turbo-test specific) — must be empty before the next file hoists.
  globalThis.__pendingMocks = [];
  __ttPrepassMocks.clear();
  __ttDoMockWaits.clear();
  globalThis.__hoistedCache = [];
  globalThis.__hoistedIdx = 0;
  // Bump the timer generation: the prior file's leaked timers (now gen < __loop.gen) get dropped,
  // unrun, the moment the loop next reaches them (see __dropStaleTimers) — without touching this
  // file's own pending one-shots. Do NOT clear macro/nextTicks or reset the clock here (that
  // broke ~40 async tests).
  __loop.gen++;
  try { globalThis.vi && globalThis.vi.resetConfig && globalThis.vi.resetConfig(); } catch (e) {}
  // NOTE: we do NOT call restoreAllMocks() here. A spyOn spy's mockRestore() redefines a
  // (possibly shared, cached) module export back to the `orig` captured at spy time. This
  // suite's own setup.ts afterEach already restores per test; calling it again between files
  // re-applies a now-stale orig onto the shared analytics module — corrupting trackEvent for
  // every later file (asserts then see the wrong/old spy). Per-file restore is the test's job.
};
