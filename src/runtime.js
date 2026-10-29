// Minimal test runtime injected before a test module loads.
// PLACEHOLDER for the real @vitest/* framework, which gets baked into the V8 snapshot
// at M3 (compatible-by-construction). This M1 version exists only to run the
// non-timer/non-DOM logic subset of the gauntlet end-to-end and report pass/fail.

// ---- console shim (V8 contexts have none; Node/browser provide it) ----
// `--silent` (host → globalThis.__TT_SILENT): suppress test console output entirely. Checked at
// call time, not eval time — the host injects __TT_SILENT AFTER this module is snapshot-evaluated.
const __noop = () => {};
// The arguments are formatted as Node's util.format (turbo-test#93). With vitest's reporters
// (`__TT_CAPTURE_LOGS`: junit, default, verbose; turbo-test#81, #94) the output is recorded
// instead (__ttLog), on the test, suite or file it belongs to, as vitest's console does.
const __emit = (tag) => (...a) => {
  if (globalThis.__TT_SILENT) return;
  if (globalThis.__TT_CAPTURE_LOGS) return __ttLog(tag === '[error]' || tag === '[warn]' ? 'stderr' : 'stdout', __nodeFormatWith({ colors: !!globalThis.__TT_COLORS }, ...a) + '\n');
  globalThis.log(tag, __nodeFormat(...a));
};
// The console output: [type, content, tick] entries of the running test (junit) and
// [type, content, tick, task] entries of the file (the human reporters; `task` is the test's or
// suite's name path, '' for the file itself). vitest buffers each stream of each task and
// flushes it once no write came for a macrotask, so writes with no macrotask between them form
// one entry; each beforeEach / afterEach hook, test function and onTestFinished / onTestFailed
// handler call starts one too (runSuite bumps the tick before each).
let __ttTestLogs = null;
let __ttFileLogs = [];
let __ttLogTask = '';
function __ttLog(type, content) {
  const tick = __loop.ticks;
  if (__ttTestLogs) {
    const last = __ttTestLogs.findLast((l) => l[0] === type);
    if (last && last[2] === tick) last[1] += content;
    else __ttTestLogs.push([type, content, tick]);
  }
  const lastF = __ttFileLogs.findLast((l) => l[0] === type && l[3] === __ttLogTask);
  if (lastF && lastF[2] === tick) lastF[1] += content;
  else __ttFileLogs.push([type, content, tick, __ttLogTask]);
}
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
  // vitest's own variables, in its order: TEST, VITEST, NODE_ENV (when the host has none), vite's
  // PROD / DEV / BASE_URL / MODE, VITEST_MODE, FORCE_TTY, VITEST_POOL_ID / VITEST_WORKER_ID and
  // SSR ('' under a DOM environment). The runner puts the host environment underneath
  // (install_host_env, turbo-test#143), sets PROD / DEV from NODE_ENV and FORCE_TTY, and the
  // per-file SSR / VITEST_POOL_ID / VITEST_WORKER_ID (set_file_env, turbo-test#123, #156).
  env: { TEST: 'true', VITEST: 'true', NODE_ENV: 'test', PROD: '', DEV: '1', BASE_URL: '/', MODE: 'test', VITEST_MODE: 'RUN', FORCE_TTY: '', VITEST_POOL_ID: '1', VITEST_WORKER_ID: '0', SSR: '1' },
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
  // the real clock (real time plus the drive loop's jumps) since the runtime started
  // (turbo-test#178; it was a constant [0, 0])
  hrtime: __realHrtime,
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
  const text = typeof chunk === 'string' ? chunk : (chunk instanceof Uint8Array ? new TextDecoder('utf-8', { ignoreBOM: true }).decode(chunk) : String(chunk));
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
// to the next due timer — deterministic, order-preserving). Fake mode (turbo-test#168): a timer
// set while fake timers are on goes on the FAKE clock (`fmacro` / `fnow`), which only vi.*
// moves; the drive loop never fires it. Timers set before useFakeTimers and the internal ones
// (test timeout) stay on the real clock, which advancing the fake clock does not move — as
// @sinonjs/fake-timers under vitest (shouldAdvanceTime off).
// ===========================================================================
function __reportUncaught(e) {
  try { globalThis.console.error('uncaught', (e && e.message) || e); } catch {}
}
// the native host (napi_fatal_exception, an exception left by an async native callback)
// reports through the same path
globalThis.__reportUncaught = __reportUncaught;
const __loop = {
  macro: [],       // real-clock user timers {id, due, cb, interval, gen}
  fmacro: [],      // fake-clock user timers (set while `fake`), due on `fnow`
  // Internal one-shot timers (test-timeout enforcement). Kept SEPARATE from `macro` so they are
  // invisible to the fake-timer API (getTimerCount / runAllTimers / advance / clearAllTimers) —
  // otherwise a test's `vi.runAllTimers()` would fire the timeout reject. Driven only by the
  // real-mode drive loop (stepMacro) so a genuinely hung async test still times out.
  internal: [],    // {id, due, cb, gen}
  nextTicks: [],
  seq: 1,
  now: 0,          // logical (real) clock (ms)
  fnow: 0,         // fake clock (ms): only vi.advanceTimersByTime / runAllTimers / ... move it
  gen: 0,          // file generation (isolate-reuse): timers from a prior file are dropped, unrun
  fake: false,
  faked: new Set(),  // what the current fake clock fakes (`toFake`)
  fstart: 0,       // fnow when the fake clock was installed (performance.now / hrtime count from it)
  epoch0: 0,       // the fake clock's start (epoch ms): performance.timeOrigin, frame boundaries
  realPerf: null,  // the global `performance` property / process.hrtime the fake ones replaced
  realHrtime: null,
  advanceId: null, // shouldAdvanceTime's real-clock interval
  tickMode: 'manual', // vi.setTimerTickMode's (sinon's tick mode), its delta and a counter that
  tickDelta: undefined, // ends a 'nextAsync' loop when the mode changes
  tickCounter: 0,
  systemTime: 0,   // wall-clock base used by Date when faking (Date.now() = systemTime + fnow)
  RealDate: Date,
  realNow: Date.now.bind(Date),
  // ms the real-mode drive loop jumped its clock instead of sleeping: a duration is the real
  // time plus the jumps in it, what the code would take on Node (turbo-test#95)
  waited: 0,
  elapsed() { return __loop.realNow() + __loop.waited; },
};
// the real clock's origin (performance.timeOrigin): when the runtime started
const __perfOrigin = __loop.realNow();
// process.hrtime on the real clock (Node's [seconds, nanoseconds]; `prev` gives the difference)
function __hrtimeOf(ms, prev) {
  const secs = Math.floor(ms / 1000);
  const nanos = Math.round((ms - secs * 1e3) * 1e6);
  if (Array.isArray(prev)) {
    if (prev[1] > 1e9) throw new TypeError("Number of nanoseconds can't exceed a billion");
    let nanoDiff = nanos - prev[1];
    let secDiff = secs - prev[0];
    if (nanoDiff < 0) { nanoDiff += 1e9; secDiff -= 1; }
    return [secDiff, nanoDiff];
  }
  return [secs, nanos];
}
function __realHrtime(prev) { return __hrtimeOf(__loop.elapsed() - __perfOrigin, prev); }
__realHrtime.bigint = () => { const [s, n] = __realHrtime(); return BigInt(s) * 1000000000n + BigInt(n); };
function __schedule(cb, delay, interval, fake, immediate) {
  const id = __loop.seq++;
  // `immediate`: a setImmediate (jest's legacy runAllImmediates)
  if (fake) __loop.fmacro.push({ id, due: __loop.fnow + Math.max(0, delay | 0), cb, interval, gen: __loop.gen, immediate });
  else __loop.macro.push({ id, due: __loop.now + Math.max(0, delay | 0), cb, interval, gen: __loop.gen });
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
  if (__loop.fmacro.length && __loop.fmacro.some((t) => t.gen < __loop.gen)) {
    __loop.fmacro = __loop.fmacro.filter((t) => t.gen >= __loop.gen);
  }
  if (__loop.internal.length && __loop.internal.some((t) => t.gen < __loop.gen)) {
    __loop.internal = __loop.internal.filter((t) => t.gen >= __loop.gen);
  }
}
function __clearTimer(id) {
  __loop.macro = __loop.macro.filter((t) => t.id !== id);
  __loop.fmacro = __loop.fmacro.filter((t) => t.id !== id);
}
// Internal one-shot timer (test-timeout). Separate queue → invisible to the fake-timer API.
function __scheduleInternal(cb, delay) {
  const id = __loop.seq++;
  __loop.internal.push({ id, due: __loop.now + Math.max(0, delay | 0), cb, gen: __loop.gen });
  return id;
}
function __clearInternal(id) { __loop.internal = __loop.internal.filter((t) => t.id !== id); }
function __dueSorted(upto) {
  return __loop.fmacro.filter((t) => t.due <= upto).sort((a, b) => a.due - b.due || a.id - b.id);
}
const __byDue = (a, b) => a.due - b.due || a.id - b.id;
// one real-clock macrotask (sinon's `originalSetTimeout(…, 0)`): what the *Async fake-timer
// functions yield between timers so promise callbacks run and can set more timers
const __realTick = () => new Promise((resolve) => __scheduleInternal(resolve, 0));
// fire one fake timer at its due time (the fake clock moves there)
function __fireFake(t) {
  __loop.fnow = Math.max(__loop.fnow, t.due);
  if (t.interval == null) __clearTimer(t.id);
  else t.due = __loop.fnow + t.interval;
  // a callback's error goes to the advance call (sinon rethrows it, turbo-test#177)
  t.cb();
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
__loop.ticks = 0;
__loop.stepMacro = function () {
  __loop.ticks++;
  __dropStaleTimers();
  __loop.macro.sort((a, b) => a.due - b.due || a.id - b.id);
  __loop.internal.sort((a, b) => a.due - b.due || a.id - b.id);
  const m = __loop.macro[0];
  const it = __loop.internal[0];
  if (!m && !it) return false;
  // earliest-due wins; ties favor the user timer (matches scheduling order intent).
  const useInternal = it && (!m || it.due < m.due);
  const due = useInternal ? it.due : m.due;
  if (due > __loop.now) __loop.waited += due - __loop.now;
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
// Fake mode: advance the fake clock, firing the fake timers in order (synchronous).
// sinon's tick: every timer due in the range runs, also when one throws; the first error is
// rethrown once the clock reached the target (turbo-test#177)
__loop.advance = function (ms) {
  __dropStaleTimers();
  const target = __loop.fnow + Math.max(0, ms | 0);
  let guard = 0;
  let first = null;
  while (guard++ < 1e6) {
    const next = __dueSorted(target)[0];
    if (!next) break;
    try { __fireFake(next); } catch (e) { if (!first) first = { e }; }
    __loop.drainNextTicks();
  }
  __loop.fnow = target;
  if (first) throw first.e;
};
// sinon's tickAsync: a real macrotask before the first timer and after each one, so the
// promise callbacks a timer starts run (and may set timers due within the range)
__loop.advanceAsync = async function (ms) {
  await __realTick();
  __dropStaleTimers();
  const target = __loop.fnow + Math.max(0, ms | 0);
  let guard = 0;
  let first = null;
  while (guard++ < 1e6) {
    const next = __dueSorted(target)[0];
    if (!next) break;
    try { __fireFake(next); } catch (e) { if (!first) first = { e }; }
    await __realTick();
  }
  __loop.fnow = Math.max(__loop.fnow, target);
  if (first) throw first.e;
};
// sinon's runAll / runAllAsync: fire the earliest timer until none is left, at most `loopLimit`
// of them (vitest's default 10000, `useFakeTimers({ loopLimit })`): a timer chain or an
// interval that never ends aborts (turbo-test#172)
__loop.loopLimit = 10000;
const __loopLimitError = () => new Error(`Aborting after running ${__loop.loopLimit} timers, assuming an infinite loop!`);
__loop.runAll = function () {
  for (let i = 0; i < __loop.loopLimit; i++) {
    if (!__loop.fmacro.length) return;
    __fireFake(__loop.fmacro.sort(__byDue)[0]);
    __loop.drainNextTicks();
  }
  if (__loop.fmacro.length) throw __loopLimitError();
};
__loop.runAllAsync = async function () {
  for (let i = 0; i < __loop.loopLimit; i++) {
    await __realTick();
    if (!__loop.fmacro.length) return;
    __fireFake(__loop.fmacro.sort(__byDue)[0]);
  }
  await __realTick();
  if (__loop.fmacro.length) throw __loopLimitError();
};
// sinon's runToLast: tick to the latest timer pending now — every timer due up to it runs,
// intervals and timers set on the way included (turbo-test#177)
__loop.runOnlyPending = function () {
  const last = __loop.fmacro.slice().sort(__byDue).pop();
  if (last) __loop.advance(last.due - __loop.fnow);
};
// sinon's runToLastAsync: tickAsync to the latest timer pending now
__loop.runOnlyPendingAsync = async function () {
  const last = __loop.fmacro.slice().sort(__byDue).pop();
  if (last) await __loop.advanceAsync(last.due - __loop.fnow);
};
// sinon's next() (+ vitest's tick(0)): the fake clock moves to the earliest fake timer and
// fires every timer due then
// (next() throws the timer's error before tick(0) runs the others)
__loop.toNext = function () {
  const n = __loop.fmacro.slice().sort(__byDue)[0];
  if (!n) return;
  __fireFake(n);
  __loop.drainNextTicks();
  __loop.advance(0);
};
__loop.toNextAsync = async function () {
  await __realTick();
  const n = __loop.fmacro.slice().sort(__byDue)[0];
  if (!n) return;
  let err = null;
  try { __fireFake(n); } catch (e) { err = { e }; }
  await __realTick();
  if (err) throw err.e;
  __loop.advance(0);
};
// drop the fake clock's timers (useRealTimers / a fresh useFakeTimers: sinon's uninstall)
__loop.dropFake = function () { __loop.fmacro = []; };
// sinon's runToFrame: to the next 16ms frame since the clock was installed
__loop.toFrame = function () {
  const since = __loop.systemTime + __loop.fnow - __loop.epoch0;
  __loop.advance(16 - (((since % 16) + 16) % 16));
};

// timer globals (bare V8 has none). `fake` picks the clock: vi.useFakeTimers swaps the globals
// for the fake-clock set and useRealTimers puts the real ones back (sinon's hijack /
// uninstall), so a reference taken before (node:timers, a library's captured setImmediate)
// stays on the real clock, as in vitest (turbo-test#168).
function __timerFns(fake) {
  return {
    setTimeout: (cb, d, ...a) => __schedule(() => cb(...a), d, null, fake),
    setInterval: (cb, d, ...a) => __schedule(() => cb(...a), d, Math.max(1, d | 0), fake),
    setImmediate: (cb, ...a) => __schedule(() => cb(...a), 0, null, fake, true),
    clearTimeout: (id) => __clearTimer(id),
    clearInterval: (id) => __clearTimer(id),
    clearImmediate: (id) => __clearTimer(id),
  };
}
const __realTimers = __timerFns(false);
const __fakeTimers = __timerFns(true);
// @testing-library's jestFakeTimersAreEnabled() checks `setTimeout.clock` (sinon sets it on
// every faked function)
for (const k of Object.keys(__fakeTimers)) __fakeTimers[k].clock = __loop;
function __installTimers(set) {
  for (const k of Object.keys(set)) globalThis[k] = set[k];
}
__installTimers(__realTimers);
// Fake `process.hrtime` / `performance` (turbo-test#172): sinon's, counting the fake clock's
// time since install (setSystemTime does not move them).
function __fakeHrtime(prev) { return __hrtimeOf(__loop.fnow - __loop.fstart, prev); }
__fakeHrtime.bigint = () => { const [s, n] = __fakeHrtime(); return BigInt(s) * 1000000000n + BigInt(n); };
__fakeHrtime.clock = __loop;
// sinon's fake performance: `now()` on the fake clock, `timeOrigin` the clock's start, the other
// methods no-ops (`getEntries*` return [], `mark` / `measure` a bare entry)
function __fakePerformance(real) {
  const perf = Object.create(null);
  const names = new Set();
  for (let o = real; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) {
    for (const k of Object.getOwnPropertyNames(o)) names.add(k);
  }
  for (const k of names) {
    if (k === 'constructor' || k === 'now') continue;
    let v;
    try { v = real[k]; } catch (e) { continue; }
    if (typeof v === 'function') perf[k] = k.indexOf('getEntries') === 0 ? () => [] : () => {};
  }
  const entry = (name, entryType, startTime, duration) => ({ name, entryType, startTime, duration, toJSON() { return { name, entryType, startTime, duration }; } });
  perf.mark = (name) => entry(name, 'mark', 0, 0);
  perf.measure = (name) => entry(name, 'measure', 0, 100);
  perf.now = () => { const [s, n] = __fakeHrtime(); return s * 1000 + n / 1e6; };
  perf.timeOrigin = __loop.epoch0;
  return perf;
}
// vitest's default `toFake`: every timer @sinonjs/fake-timers knows but nextTick and
// queueMicrotask (requestAnimationFrame & co. and Intl are not faked here)
const __TO_FAKE = ['setTimeout', 'clearTimeout', 'setImmediate', 'clearImmediate', 'setInterval', 'clearInterval', 'Date', 'hrtime', 'performance'];
const __epochOf = (t) => (t == null ? 0 : typeof t === 'number' ? t : t instanceof __loop.RealDate || (t && typeof t.getTime === 'function') ? t.getTime() : Number(t) || 0);
// vitest's runtime config, as far as the runtime reads it (vi.setConfig / vi.resetConfig):
// `fakeTimers` is vitest's default (`loopLimit: 10_000`, `shouldClearNativeTimers`) under the
// project's `test.fakeTimers` (turbo-test#180); `clearMocks` / `mockReset` / `restoreMocks` /
// `unstubEnvs` / `unstubGlobals` undo mocks and stubs before each test (turbo-test#183). The
// runner applies the project's config per file (__ttApplyConfig).
const __ttConfigDefaults = () => ({ fakeTimers: { loopLimit: 10000, shouldClearNativeTimers: true }, clearMocks: false, mockReset: false, restoreMocks: false, unstubEnvs: false, unstubGlobals: false });
let __ttConfig = __ttConfigDefaults();
let __ttConfigSaved = null;
globalThis.__ttApplyConfig = (cfg) => {
  if (!cfg || typeof cfg !== 'object') return;
  const { fakeTimers, ...rest } = cfg;
  if (fakeTimers && typeof fakeTimers === 'object') __ttConfig.fakeTimers = Object.assign(__ttConfigDefaults().fakeTimers, fakeTimers);
  Object.assign(__ttConfig, rest);
};
// vitest's clearModuleMocks (onBeforeTryTask): before each attempt of a test, its beforeEach too
function __clearModuleMocks() {
  const vi = globalThis.vi;
  if (__ttConfig.restoreMocks) vi.restoreAllMocks();
  if (__ttConfig.mockReset) vi.resetAllMocks();
  if (__ttConfig.clearMocks) vi.clearAllMocks();
  if (__ttConfig.unstubEnvs) vi.unstubAllEnvs();
  if (__ttConfig.unstubGlobals) vi.unstubAllGlobals();
}
// vi.useFakeTimers(config): a fresh fake clock (sinon's install with vitest's `now: Date.now()`);
// a config without `loopLimit` (vi.setConfig replaced the default one) gets sinon's 1000
function __installFake(cfg) {
  const c = Object.assign({}, cfg || {});
  const start = c.now !== undefined ? __epochOf(c.now) : globalThis.Date.now();
  if (__loop.fake) __uninstallFake();
  __loop.dateMocked = false;
  const toFake = Array.isArray(c.toFake) && c.toFake.length ? c.toFake : __TO_FAKE;
  __loop.fake = true;
  __loop.faked = new Set(toFake);
  __loop.loopLimit = c.loopLimit || 1000;
  __loop.systemTime = start - __loop.fnow;
  __loop.fstart = __loop.fnow;
  __loop.epoch0 = start;
  if (__loop.faked.has('Date')) __installFakeDate();
  else __restoreDate();
  for (const k of Object.keys(__fakeTimers)) globalThis[k] = __loop.faked.has(k) ? __fakeTimers[k] : __realTimers[k];
  if (__loop.faked.has('performance') && globalThis.performance) {
    __loop.realPerf = Object.getOwnPropertyDescriptor(globalThis, 'performance');
    Object.defineProperty(globalThis, 'performance', { value: __fakePerformance(globalThis.performance), writable: true, configurable: true, enumerable: !!(__loop.realPerf && __loop.realPerf.enumerable) });
  }
  if (__loop.faked.has('hrtime') && globalThis.process && typeof globalThis.process.hrtime === 'function') {
    __loop.realHrtime = globalThis.process.hrtime;
    globalThis.process.hrtime = __fakeHrtime;
  }
  // shouldAdvanceTime: a real-clock interval ticks the fake clock by advanceTimeDelta (20ms)
  if (c.shouldAdvanceTime === true) __setTickMode('interval', c.advanceTimeDelta);
}
// sinon's setTickMode (vi.setTimerTickMode, shouldAdvanceTime): 'interval' is a real-clock
// interval moving the fake clock by `delta` (20ms) each time; 'nextAsync' fires the next fake
// timer after each real macrotask until the mode changes; 'manual' stops both
function __setTickMode(mode, delta) {
  if (mode === __loop.tickMode && delta === __loop.tickDelta) return;
  if (__loop.advanceId != null) { __realTimers.clearInterval(__loop.advanceId); __loop.advanceId = null; }
  const counter = ++__loop.tickCounter;
  __loop.tickMode = mode;
  __loop.tickDelta = delta;
  if (mode === 'interval') {
    const d = delta || 20;
    __loop.advanceId = __realTimers.setInterval(() => __loop.advance(d), d);
  } else if (mode === 'nextAsync') {
    (async () => {
      while (__loop.tickCounter === counter) {
        await __realTick();
        await __realTick();
        if (__loop.tickCounter !== counter) return;
        const n = __loop.fmacro.slice().sort(__byDue)[0];
        if (n) { try { __fireFake(n); } catch (e) { __reportUncaught(e); } __loop.drainNextTicks(); }
      }
    })();
  }
}
// sinon's uninstall: the real functions back, the fake clock's pending timers dropped
function __uninstallFake() {
  __loop.fake = false;
  __loop.dateMocked = false;
  __loop.dropFake();
  __restoreDate();
  __installTimers(__realTimers);
  if (__loop.realPerf) { Object.defineProperty(globalThis, 'performance', __loop.realPerf); __loop.realPerf = null; }
  if (__loop.realHrtime) { globalThis.process.hrtime = __loop.realHrtime; __loop.realHrtime = null; }
  __setTickMode('manual');
  __loop.loopLimit = 10000;
}
// vitest's _checkFakeTimers (turbo-test#171)
function __checkFake() {
  if (!__loop.fake) throw new Error('A function to advance timers was called but the timers APIs are not mocked. Call `vi.useFakeTimers()` in the test file first.');
}
if (typeof globalThis.queueMicrotask !== 'function') {
  globalThis.queueMicrotask = (cb) => Promise.resolve().then(cb);
}

// Date faking
function __installFakeDate() {
  const Real = __loop.RealDate;
  function FakeDate(...args) {
    if (!(this instanceof FakeDate)) return new Real(__loop.systemTime + __loop.fnow).toString();
    if (args.length === 0) return new Real(__loop.systemTime + __loop.fnow);
    return new Real(...args);
  }
  FakeDate.prototype = Real.prototype;
  FakeDate.now = () => __loop.systemTime + __loop.fnow;
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
// one VDate for the run: restoring the real Date gives back the same function, as Node's
let __vdate = null;
function __installVirtualDate() {
  if (__vdate) { globalThis.Date = __vdate; return; }
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
  __vdate = VDate;
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
      try { m.exports = __ttStrictMock(await m.exports, m.key, m.spec, m.vitest !== false); }
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
// WHATWG Encoding, checked row by row against Node 26 (turbo-test#118): lone surrogates encode
// as U+FFFD, invalid UTF-8 decodes to U+FFFD per maximal subpart (or throws with `fatal`), a
// leading BOM is stripped unless `ignoreBOM`, `stream` keeps a partial sequence. TextDecoder's
// other encodings (turbo-test#119) decode natively through encoding_rs (src/text_decoder.rs).
const __codecError = (Ctor, code, msg) => { const e = new Ctor(msg); e.code = code; return e; };
// The one WHATWG UTF-8 decoder (turbo-test#121): b[start..end) (any indexable bytes) to a string,
// one U+FFFD per maximal invalid subpart (or `onBad()`'s throw, for `fatal`). With `stream`, an
// incomplete-but-valid sequence at the end is left undecoded; returns [string, where it starts].
function __utf8Run(b, start, end, stream, onBad) {
  const units = []; let out = '';
  const unit = (u) => { units.push(u); if (units.length >= 8192) { out += String.fromCharCode.apply(null, units); units.length = 0; } };
  const bad = () => { if (onBad) onBad(); unit(0xfffd); };
  let i = start;
  while (i < end) {
    const c = b[i];
    if (c < 0x80) { unit(c); i++; continue; }
    let need; let cp; let lo = 0x80; let hi = 0xbf;
    if (c >= 0xc2 && c <= 0xdf) { need = 1; cp = c & 0x1f; }
    else if (c >= 0xe0 && c <= 0xef) { need = 2; cp = c & 0xf; if (c === 0xe0) lo = 0xa0; if (c === 0xed) hi = 0x9f; }
    else if (c >= 0xf0 && c <= 0xf4) { need = 3; cp = c & 0x7; if (c === 0xf0) lo = 0x90; if (c === 0xf4) hi = 0x8f; }
    else { bad(); i++; continue; }
    let j = i + 1;
    for (; j <= i + need && j < end; j++) {
      if (b[j] < lo || b[j] > hi) break;
      cp = (cp << 6) | (b[j] & 0x3f); lo = 0x80; hi = 0xbf;
    }
    if (j > i + need) {
      if (cp < 0x10000) unit(cp);
      else { cp -= 0x10000; unit(0xd800 + (cp >> 10)); unit(0xdc00 + (cp & 0x3ff)); }
    } else if (j === end && stream) break; // valid so far: wait for more
    else bad(); // one U+FFFD for the maximal subpart b[i..j)
    i = j;
  }
  return [out + String.fromCharCode.apply(null, units), i];
}
// UTF-8 decode without BOM handling (Buffer#toString, querystring, URLSearchParams, StringDecoder)
function __utf8Decode(b, start = 0, end = b.length) { return __utf8Run(b, start, end, false, null)[0]; }
if (typeof globalThis.TextEncoder === 'undefined') {
  // UTF-8 bytes of s[i..] into out[j..] while they fit; returns [code units read, bytes written]
  const encodeTo = (s, out) => {
    const n = s.length; const max = out.length; let i = 0; let j = 0;
    while (i < n) {
      let c = s.charCodeAt(i); let units = 1;
      if (c >= 0xd800 && c <= 0xdfff) {
        const d = c < 0xdc00 && i + 1 < n ? s.charCodeAt(i + 1) : 0;
        if (d >= 0xdc00 && d <= 0xdfff) { c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00); units = 2; }
        else c = 0xfffd;
      }
      const len = c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
      if (j + len > max) break;
      if (len === 1) out[j++] = c;
      else if (len === 2) { out[j++] = 0xc0 | (c >> 6); out[j++] = 0x80 | (c & 0x3f); }
      else if (len === 3) { out[j++] = 0xe0 | (c >> 12); out[j++] = 0x80 | ((c >> 6) & 0x3f); out[j++] = 0x80 | (c & 0x3f); }
      else { out[j++] = 0xf0 | (c >> 18); out[j++] = 0x80 | ((c >> 12) & 0x3f); out[j++] = 0x80 | ((c >> 6) & 0x3f); out[j++] = 0x80 | (c & 0x3f); }
      i += units;
    }
    return [i, j];
  };
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') {
      if (!(this instanceof TextEncoder)) throw new TypeError('Receiver must be an instance of class TextEncoder');
      const s = `${input}`;
      const out = new Uint8Array(s.length * 3);
      return out.slice(0, encodeTo(s, out)[1]);
    }
    encodeInto(src, dest) {
      if (!(this instanceof TextEncoder)) throw new TypeError('Receiver must be an instance of class TextEncoder');
      if (typeof src !== 'string') throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "src" argument must be of type string');
      if (!(dest instanceof Uint8Array)) throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "dest" argument must be an instance of Uint8Array');
      const [read, written] = encodeTo(src, dest);
      return { read, written };
    }
  };
  Object.defineProperty(globalThis.TextEncoder.prototype, Symbol.toStringTag, { value: 'TextEncoder', configurable: true });
}
if (typeof globalThis.TextDecoder === 'undefined') {
  const LABELS = ['unicode-1-1-utf-8', 'unicode11utf8', 'unicode20utf8', 'utf-8', 'utf8', 'x-unicode20utf8'];
  const abLen = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength').get;
  // SharedArrayBuffer is looked up per call: it may be installed after this file runs
  const sabLen = () => typeof SharedArrayBuffer === 'function' && Object.getOwnPropertyDescriptor(SharedArrayBuffer.prototype, 'byteLength').get;
  const isBuffer = (x, get) => { if (!get) return false; try { get.call(x); return true; } catch { return false; } };
  const bytesOf = (input) => {
    if (input === undefined) return new Uint8Array(0);
    if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    if (isBuffer(input, abLen) || isBuffer(input, sabLen())) return new Uint8Array(input);
    throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "input" argument must be an instance of SharedArrayBuffer, ArrayBuffer or ArrayBufferView.');
  };
  const options = (o) => {
    if (o == null) return {};
    if (typeof o !== 'object' && typeof o !== 'function') throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be of type object.');
    return o;
  };
  // Node's error for a text longer than V8's string limit (turbo-test#187)
  const tooLong = () => __codecError(Error, 'ERR_STRING_TOO_LONG', 'Cannot create a string longer than 0x1fffffe8 characters');
  const state = new WeakMap();
  const self = (d) => {
    const st = state.get(d);
    if (!st) throw __codecError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type TextDecoder');
    return st;
  };
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = 'utf-8', opts = undefined) {
      const name = `${label}`.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, '').replace(/[A-Z]+/g, (c) => c.toLowerCase());
      // UTF-8 here; any other WHATWG label names its encoding natively (undefined: not a label)
      const encoding = LABELS.includes(name) ? 'utf-8' : globalThis.__ttTextEncoding(name);
      if (encoding === undefined) throw __codecError(RangeError, 'ERR_ENCODING_NOT_SUPPORTED', `The "${label}" encoding is not supported`);
      if (encoding === 'replacement') throw __codecError(RangeError, 'ERR_ENCODING_NOT_SUPPORTED', 'The "replacement" encoding is not supported');
      const o = options(opts);
      const id = encoding === 'utf-8' ? 0 : globalThis.__ttTextDecoderId();
      state.set(this, { encoding, id, fatal: !!o.fatal, ignoreBOM: !!o.ignoreBOM, pending: null, bomSeen: false });
    }
    get encoding() { return self(this).encoding; }
    get fatal() { return self(this).fatal; }
    get ignoreBOM() { return self(this).ignoreBOM; }
    decode(input = undefined, opts = undefined) {
      const st = self(this);
      let b = bytesOf(input);
      const stream = !!options(opts).stream;
      if (st.id) {
        const text = globalThis.__ttTextDecode(st.id, st.encoding, st.ignoreBOM, st.fatal, stream, b);
        if (text === null) throw __codecError(TypeError, 'ERR_ENCODING_INVALID_ENCODED_DATA', `The encoded data was not valid for encoding ${st.encoding}`);
        if (text === undefined) throw tooLong();
        return text;
      }
      if (st.pending) { const m = new Uint8Array(st.pending.length + b.length); m.set(st.pending); m.set(b, st.pending.length); b = m; st.pending = null; }
      const fail = () => { st.pending = null; st.bomSeen = false; throw __codecError(TypeError, 'ERR_ENCODING_INVALID_ENCODED_DATA', 'The encoded data was not valid for encoding utf-8'); };
      let decoded, rest;
      try { [decoded, rest] = __utf8Run(b, 0, b.length, stream, st.fatal ? fail : null); } catch (e) { throw e instanceof RangeError ? tooLong() : e; }
      if (rest < b.length) st.pending = b.slice(rest);
      let out = decoded;
      if (!st.ignoreBOM && !st.bomSeen && out.length > 0) {
        st.bomSeen = true;
        if (out.charCodeAt(0) === 0xfeff) out = out.slice(1);
      }
      if (!stream) st.bomSeen = false; // end of stream: reset (a partial sequence already became U+FFFD)
      return out;
    }
  };
  Object.defineProperty(globalThis.TextDecoder.prototype, Symbol.toStringTag, { value: 'TextDecoder', configurable: true });
}

// ---- Web platform globals (bare V8 lacks these) ----
if (typeof globalThis.URLSearchParams === 'undefined') {
  // WHATWG URLSearchParams, checked row by row against Node 26 (turbo-test#117). Names and values
  // are USVStrings: String() (a Symbol throws TypeError), lone surrogates -> U+FFFD.
  const usv = (v) => {
    if (typeof v === 'symbol') throw new TypeError('Cannot convert a Symbol value to a string');
    return String(v).toWellFormed();
  };
  const need = (n, got, what) => { if (got < n) throw new TypeError(`The ${what} must be specified`); };
  // percent-decode (a `%` not followed by two hex digits stays as written), then UTF-8 decode.
  // Raw characters are whole UTF-8 sequences, so each run of %XX bytes decodes on its own.
  const pctDecode = (s) => s.replace(/(?:%[0-9A-Fa-f]{2})+/g, (run) => {
    const b = [];
    for (let i = 0; i < run.length; i += 3) b.push(parseInt(run.slice(i + 1, i + 3), 16));
    return __utf8Decode(b);
  });
  // application/x-www-form-urlencoded parser: pairs split on '&' (empty ones dropped), name and
  // value at the first '=', '+' is a space
  const parse = (s) => s.split('&').filter(Boolean).map((p) => {
    const i = p.indexOf('=');
    const dec = (x) => pctDecode(x.replace(/\+/g, ' '));
    return [dec(i < 0 ? p : p.slice(0, i)), dec(i < 0 ? '' : p.slice(i + 1))];
  });
  // form-urlencoded serializer: only ASCII alphanumerics and *-._ stay; space is '+'; everything
  // else is its UTF-8 bytes as %XX (encodeURIComponent also leaves !'()~, so those are escaped)
  const enc = (s) => encodeURIComponent(s)
    .replace(/[!'()~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase()).replace(/%20/g, '+');
  const IterProto = Object.create(Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
  // live, index-based iterator over the list (as Node's): pairs added later are still visited
  Object.defineProperties(IterProto, {
    next: {
      value: function next() {
        const list = this._p._;
        if (this._i >= list.length) return { done: true, value: undefined };
        const [k, v] = list[this._i++];
        return { done: false, value: this._k === 'keys' ? k : this._k === 'values' ? v : [k, v] };
      },
      writable: true, configurable: true,
    },
    [Symbol.toStringTag]: { value: 'URLSearchParams Iterator', configurable: true },
  });
  const iter = (p, k) => Object.defineProperties(Object.create(IterProto), {
    _p: { value: p }, _k: { value: k }, _i: { value: 0, writable: true },
  });
  globalThis.URLSearchParams = class URLSearchParams {
    constructor(init = undefined) {
      // list of [name, value] pairs; not enumerable, as a real URLSearchParams has no own keys
      // (pretty-format prints `URLSearchParams {}`)
      Object.defineProperty(this, '_', { value: [], writable: true, enumerable: false, configurable: true });
      if (init === undefined) return;
      if ((typeof init === 'object' && init !== null) || typeof init === 'function') {
        const m = init[Symbol.iterator];
        if (m !== undefined && m !== null) {
          // sequence<sequence<USVString>>: every pair is an iterable of exactly two
          if (typeof m !== 'function') throw new TypeError('Query pairs must be iterable');
          for (const pair of { [Symbol.iterator]: () => m.call(init) }) {
            if ((typeof pair !== 'object' || pair === null) && typeof pair !== 'function') {
              throw new TypeError('Each query pair must be an iterable [name, value] tuple');
            }
            const kv = [...pair];
            if (kv.length !== 2) throw new TypeError('Each query pair must be an iterable [name, value] tuple');
            this._.push([usv(kv[0]), usv(kv[1])]);
          }
        } else {
          // record<USVString, USVString>: own enumerable keys; a key repeated after conversion
          // keeps its first position and takes the last value
          const rec = new Map();
          for (const key of Reflect.ownKeys(init)) {
            const d = Reflect.getOwnPropertyDescriptor(init, key);
            if (d && d.enumerable) rec.set(usv(key), usv(init[key]));
          }
          for (const kv of rec) this._.push(kv);
        }
        return;
      }
      const s = usv(init);
      this._ = parse(s.startsWith('?') ? s.slice(1) : s);
    }
    get size() { return this._.length; }
    get(name) { need(1, arguments.length, '"name" argument'); const n = usv(name); const e = this._.find((p) => p[0] === n); return e ? e[1] : null; }
    getAll(name) { need(1, arguments.length, '"name" argument'); const n = usv(name); return this._.filter((p) => p[0] === n).map((p) => p[1]); }
    has(name, value = undefined) {
      need(1, arguments.length, '"name" argument');
      const n = usv(name);
      if (value === undefined) return this._.some((p) => p[0] === n);
      const v = usv(value);
      return this._.some((p) => p[0] === n && p[1] === v);
    }
    // the first pair of the name takes the value in place and the others go; a new name is
    // appended (WHATWG; turbo-test#116)
    set(name, value) {
      need(2, arguments.length, '"name" and "value" arguments');
      const n = usv(name); const v = usv(value);
      const i = this._.findIndex((p) => p[0] === n);
      if (i < 0) this._.push([n, v]);
      else { this._[i][1] = v; this._ = this._.filter((p, j) => j <= i || p[0] !== n); }
      this._update();
    }
    append(name, value) { need(2, arguments.length, '"name" and "value" arguments'); this._.push([usv(name), usv(value)]); this._update(); }
    delete(name, value = undefined) {
      need(1, arguments.length, '"name" argument');
      const n = usv(name);
      if (value === undefined) this._ = this._.filter((p) => p[0] !== n);
      else { const v = usv(value); this._ = this._.filter((p) => p[0] !== n || p[1] !== v); }
      this._update();
    }
    // live: pairs appended during the walk are visited, a deleted one shifts the rest (Node)
    forEach(callback, thisArg = undefined) {
      if (typeof callback !== 'function') throw new TypeError('The "callback" argument must be of type function');
      for (let i = 0; i < this._.length; i++) { const [k, v] = this._[i]; callback.call(thisArg, v, k, this); }
    }
    keys() { return iter(this, 'keys'); }
    values() { return iter(this, 'values'); }
    entries() { return iter(this, 'entries'); }
    // stable, by UTF-16 code units (Array#sort is stable and `<` compares code units)
    sort() { this._.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); this._update(); }
    // a URL's searchParams writes its serialization back as the URL's query (WHATWG update steps:
    // an empty list leaves no query)
    _update() { if (this._url) { const q = this.toString(); this._url._query = q === '' ? null : q; } }
    toString() { return this._.map(([k, v]) => `${enc(k)}=${enc(v)}`).join('&'); }
    get [Symbol.toStringTag]() { return 'URLSearchParams'; }
    // Node's util.inspect rendering (vitest's loupe hands it the value): `URLSearchParams { 'a' => '1' }`,
    // one pair per line past `breakLength`.
    [Symbol.for('nodejs.util.inspect.custom')](depth, opts) {
      const o = __inspectCustomOpts(opts);
      if (typeof depth === 'number' && depth < 0) return __sty(o, '[Object]', 'special');
      if (typeof depth === 'number') o.depth = depth - 1;
      const out = this._.map(([k, v]) => `${__nodeInspect(k, o)} => ${__nodeInspect(v, o)}`);
      const len = out.reduce((n, x) => n + __stripAnsi(x).length + 2, -2);
      const name = (this.constructor && this.constructor.name) || 'URLSearchParams';
      if (len > o.breakLength) return `${name} {\n  ${out.join(',\n  ')} }`;
      return out.length ? `${name} { ${out.join(', ')} }` : `${name} {}`;
    }
  };
  Object.defineProperty(globalThis.URLSearchParams.prototype, Symbol.iterator, {
    value: globalThis.URLSearchParams.prototype.entries, writable: true, configurable: true,
  });
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
        // `user:pass@` before the last '@' is the userinfo, percent-encoded (WHATWG; turbo-test#114)
        const at = m[2].lastIndexOf('@');
        const info = at < 0 ? '' : m[2].slice(0, at);
        const hostport = at < 0 ? m[2] : m[2].slice(at + 1);
        const colon = info.indexOf(':');
        this.username = URL._encode(colon < 0 ? info : info.slice(0, colon), URL._USERINFO);
        this.password = colon < 0 ? '' : URL._encode(info.slice(colon + 1), URL._USERINFO);
        // a file URL's `localhost` host is the empty host (WHATWG)
        this.host = this.protocol.toLowerCase() === 'file:' && /^localhost$/i.test(hostport) ? '' : hostport;
        this.hostname = this.host.split(':')[0];
        this.port = hostport.split(':')[1] || '';
        this.pathname = m[3] || '/';
        searchStr = m[4] || '';
        this.hash = m[5] || '';
        this.origin = this.protocol.toLowerCase() === 'file:' ? 'null' : this.protocol + '//' + this.host;
      } else {
        // opaque URL (no authority): everything after the scheme is the path (sans query/hash).
        this._hierarchical = false;
        this.username = ''; this.password = '';
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
      // the query as parsed, percent-encoded with the query set (null: none); searchParams is
      // parsed from it and writes back to it when changed (turbo-test#115)
      this._query = searchStr ? URL._encode(searchStr.slice(1), URL._querySet(this.protocol)) : null;
      this._sp = URL._params(this._query || '', this);
      // the parts live in non-enumerable slots: a real URL has no own enumerable keys
      for (const k of Object.keys(this)) Object.defineProperty(this, k, { enumerable: false });
    }
    // `user:pass@` in href (empty parts left out, as WHATWG serializes them)
    get _userinfo() { return this.username || this.password ? this.username + (this.password ? ':' + this.password : '') + '@' : ''; }
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
    get search() { return this._query ? '?' + this._query : ''; }
    set search(v) {
      v = String(v);
      const input = v.startsWith('?') ? v.slice(1) : v;
      this._query = v === '' ? null : URL._encode(input, URL._querySet(this.protocol));
      this._sp._ = v === '' ? [] : new globalThis.URLSearchParams(input)._;
    }
    // a searchParams linked to its URL (the same object for the URL's lifetime)
    static _params(query, url) {
      const sp = new globalThis.URLSearchParams(query);
      Object.defineProperty(sp, '_url', { value: url, writable: true, enumerable: false, configurable: true });
      return sp;
    }
    // WHATWG's query percent-encode set; a special scheme's also encodes '
    static _querySet(protocol) { return /^(https?|wss?|ftp|file):$/i.test(protocol) ? '\x7f "#<>\'' : '\x7f "#<>'; }
    get href() {
      const auth = this._hierarchical ? this.protocol + '//' + this._userinfo + this.host : this.protocol;
      return auth + this._path + (this._query === null ? '' : '?' + this._query) + this._hash;
    }
    set href(v) { const u = new globalThis.URL(v); this.protocol = u.protocol; this.username = u.username; this.password = u.password; this.host = u.host; this.hostname = u.hostname; this.port = u.port; this._path = u._path; this._hash = u._hash; this.origin = u.origin; this._hierarchical = u._hierarchical; this._query = u._query; this._sp._ = u._sp._; }
    toString() { return this.href; }
    // JSON.stringify and pretty-format (vitest's diffs) show the href, as Node's URL
    toJSON() { return this.href; }
    // `[object URL]`, as Node's: chai's objDisplay keeps the rendering instead of summarising it
    get [Symbol.toStringTag]() { return 'URL'; }
    // Node's util.inspect rendering (vitest's loupe hands it the value, turbo-test#112):
    // `URL { href: …, origin: …, …, searchParams: URLSearchParams {…}, hash: … }`.
    [Symbol.for('nodejs.util.inspect.custom')](depth, opts) {
      const name = (this.constructor && this.constructor.name) || 'URL';
      if (typeof depth === 'number' && depth < 0) return `${name} {}`;
      const obj = {
        href: this.href, origin: this.origin, protocol: this.protocol, username: this.username, password: this.password,
        host: this.host, hostname: this.hostname, port: this.port, pathname: this.pathname, search: this.search,
        searchParams: this.searchParams, hash: this.hash,
      };
      const o = __inspectCustomOpts(opts);
      delete o.depth;
      return `${name} ${__nodeInspect(obj, o)}`;
    }
    // WHATWG relative-reference resolution against a parsed base (Node's `new URL(rel, base)`):
    // scheme-relative `//h/p`, absolute path `/p`, query-only `?q`, fragment-only `#f`, empty,
    // and a relative path merged with the base path's directory. Dot segments are removed by
    // the caller's pathname normalization. An opaque base (`mailto:x`) only takes a fragment.
    static _resolve(rel, b) {
      const auth = b.protocol + '//' + b._userinfo + b.host;
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
    static get _USERINFO() { return '\x7f "#<>?^`{}/:;=@[\\]|'; }
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
// Blob / File as Node 26's (turbo-test#120, #128): the parts' bytes are copied at construction
// (strings as UTF-8, `endings: 'native'` turning CR / CRLF into LF), one chunk per non-empty
// part, nested Blobs contributing their chunks; size / type / name / lastModified are prototype
// accessors; slice() takes [Clamp]ed, negative-relative bounds and its own type.
if (typeof globalThis.Blob === 'undefined') {
  const thisErr = (what) => __codecError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type ' + what);
  // a type with any character outside U+0020..U+007E is '', otherwise it is lower-cased
  const normType = (t) => { const s = String(t); return /^[\x20-\x7e]*$/.test(s) ? s.toLowerCase() : ''; };
  // WebIDL [Clamp] long long: NaN -> 0, ties to even
  const clamp = (v) => {
    const n = Number(v);
    if (Number.isNaN(n)) return 0;
    if (!Number.isFinite(n)) return n;
    const f = Math.floor(n), d = n - f;
    return d < 0.5 ? f : d > 0.5 ? f + 1 : (f % 2 === 0 ? f : f + 1);
  };
  const received = (v) => typeof v === 'string' ? `type string ('${v}')` : `type ${typeof v} (${String(v)})`;
  const concat = (chunks, size) => {
    const out = new Uint8Array(size);
    let at = 0;
    for (const c of chunks) { out.set(c, at); at += c.length; }
    return out;
  };
  globalThis.Blob = class Blob {
    #chunks = [];
    #size = 0;
    #type = '';
    constructor(blobParts = undefined, options = undefined) {
      if (blobParts !== undefined && (blobParts === null || (typeof blobParts !== 'object' && typeof blobParts !== 'function') || typeof blobParts[Symbol.iterator] !== 'function'))
        throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'Value cannot be converted to sequence.');
      if (options != null && typeof options !== 'object' && typeof options !== 'function')
        throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be a dictionary. Received ' + received(options));
      const o = options == null ? {} : options;
      const endings = o.endings === undefined ? 'transparent' : String(o.endings);
      if (endings !== 'transparent' && endings !== 'native')
        throw __codecError(TypeError, 'ERR_INVALID_ARG_VALUE', `The property 'options.endings' is invalid. Received '${endings}'`);
      this.#type = o.type === undefined ? '' : normType(o.type);
      const add = (c) => { if (c.length) { this.#chunks.push(c); this.#size += c.length; } };
      for (const p of blobParts === undefined ? [] : blobParts) {
        if (p instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && p instanceof SharedArrayBuffer)) add(new Uint8Array(p.slice(0)));
        else if (ArrayBuffer.isView(p)) add(new Uint8Array(p.buffer, p.byteOffset, p.byteLength).slice());
        else if (p !== null && typeof p === 'object' && #chunks in p) p.#chunks.forEach(add);
        else {
          let s = String(p);
          if (endings === 'native') s = s.replace(/\r\n?/g, '\n');
          add(new globalThis.TextEncoder().encode(s));
        }
      }
    }
    get type() { if (!(#chunks in this)) throw thisErr('Blob'); return this.#type; }
    get size() { if (!(#chunks in this)) throw thisErr('Blob'); return this.#size; }
    slice(start = undefined, end = undefined, contentType = undefined) {
      if (!(#chunks in this)) throw thisErr('Blob');
      const size = this.#size;
      const rel = (v, dflt) => { if (v === undefined) return dflt; const n = clamp(v); return n < 0 ? Math.max(size + n, 0) : Math.min(n, size); };
      const from = rel(start, 0), to = rel(end, size);
      const out = new Blob([], contentType === undefined ? undefined : { type: contentType });
      let at = 0;
      for (const c of this.#chunks) {
        const lo = Math.max(from - at, 0), hi = Math.min(to - at, c.length);
        if (lo < hi) { out.#chunks.push(lo === 0 && hi === c.length ? c : c.subarray(lo, hi)); out.#size += hi - lo; }
        at += c.length;
      }
      return out;
    }
    arrayBuffer() {
      if (!(#chunks in this)) return Promise.reject(thisErr('Blob'));
      return Promise.resolve(concat(this.#chunks, this.#size).buffer);
    }
    // WHATWG "UTF-8 decode" of all the bytes at once (turbo-test#120): a sequence split across
    // parts is one character and one leading BOM is stripped, as Node's
    text() {
      if (!(#chunks in this)) return Promise.reject(thisErr('Blob'));
      return Promise.resolve(new globalThis.TextDecoder().decode(concat(this.#chunks, this.#size)));
    }
    bytes() {
      if (!(#chunks in this)) return Promise.reject(thisErr('Blob'));
      return Promise.resolve(concat(this.#chunks, this.#size));
    }
    // a ReadableStream of one Uint8Array (a copy) per chunk
    stream() {
      if (!(#chunks in this)) throw thisErr('Blob');
      const chunks = this.#chunks;
      let i = 0;
      return new globalThis.ReadableStream({
        pull(c) {
          if (i < chunks.length) c.enqueue(new Uint8Array(chunks[i++]));
          if (i >= chunks.length) c.close();
        },
      });
    }
    // the chunks UTF-8-decoded as one stream (a sequence split across chunks is one character),
    // empty pieces skipped
    textStream() {
      if (!(#chunks in this)) throw thisErr('Blob');
      const chunks = this.#chunks;
      const dec = new globalThis.TextDecoder();
      let i = 0;
      return new globalThis.ReadableStream({
        pull(c) {
          while (i < chunks.length) {
            const s = dec.decode(chunks[i++], { stream: true });
            if (s) { c.enqueue(s); if (i < chunks.length) return; }
          }
          const rest = dec.decode();
          if (rest) c.enqueue(rest);
          c.close();
        },
      });
    }
  };
  Object.defineProperty(globalThis.Blob.prototype, Symbol.toStringTag, { value: 'Blob', configurable: true });
}
if (typeof globalThis.File === 'undefined') {
  globalThis.File = class File extends globalThis.Blob {
    #name;
    #lastModified;
    constructor(fileBits, fileName, options = undefined) {
      if (arguments.length < 2) throw __codecError(TypeError, 'ERR_MISSING_ARGS', 'The "fileBits" and "fileName" arguments must be specified');
      super(fileBits, options);
      this.#name = String(fileName);
      const lm = options == null ? undefined : options.lastModified;
      this.#lastModified = lm === undefined ? Date.now() : Number(lm);
    }
    get name() { if (!(#name in this)) throw __codecError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type File'); return this.#name; }
    get lastModified() { if (!(#name in this)) throw __codecError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type File'); return this.#lastModified; }
  };
  Object.defineProperty(globalThis.File.prototype, Symbol.toStringTag, { value: 'File', configurable: true });
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
// FileReader as jsdom's (turbo-test#132, #139): a read reads the Blob's bytes, sets readyState
// LOADING at once, then (asynchronously) fires loadstart, progress (a non-empty Blob), load and
// loadend; abort() while LOADING fires abort and loadend instead. Listeners are kept in order
// (addEventListener deduplicated, on<type> handlers holding the slot of their first set).
if (typeof globalThis.FileReader === 'undefined') {
  const EVENTS = ['loadstart', 'progress', 'load', 'abort', 'error', 'loadend'];
  const invalidState = () => {
    const e = new globalThis.DOMException('The object is in an invalid state.', 'InvalidStateError');
    Object.defineProperty(e, 'code', { value: 11, configurable: true });
    return e;
  };
  // the encoding readAsText decodes with: a UTF-8 / UTF-16 BOM wins, then the label, then UTF-8
  const textOf = (b, label) => {
    let enc = 'utf-8';
    if (b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf) enc = 'utf-8';
    else if (b[0] === 0xff && b[1] === 0xfe) enc = 'utf-16le';
    else if (b[0] === 0xfe && b[1] === 0xff) enc = 'utf-16be';
    else if (label !== undefined) { try { enc = new globalThis.TextDecoder(String(label)).encoding; } catch {} }
    return new globalThis.TextDecoder(enc).decode(b);
  };
  globalThis.FileReader = class FileReader {
    #listeners = Object.create(null); // type -> [{ fn }] (an on<type> handler's entry has `on: true`)
    #gen = 0;                          // bumped by every read and abort; a stale read stops
    constructor() { this.result = null; this.error = null; this.readyState = 0; }
    addEventListener(type, fn) {
      if (fn == null) return;
      const list = this.#listeners[type] || (this.#listeners[type] = []);
      if (!list.some((l) => !l.on && l.fn === fn)) list.push({ fn });
    }
    removeEventListener(type, fn) {
      const list = this.#listeners[type];
      if (list) this.#listeners[type] = list.filter((l) => l.on || l.fn !== fn);
    }
    dispatchEvent(ev) {
      for (const l of (this.#listeners[ev.type] || []).slice()) {
        try {
          if (typeof l.fn === 'function') l.fn.call(this, ev);
          else if (l.fn && typeof l.fn.handleEvent === 'function') l.fn.handleEvent(ev);
        } catch (e) { console.error(e); } // reported, as jsdom's virtual console does
      }
      return !ev.defaultPrevented;
    }
    #handler(type) { const l = (this.#listeners[type] || []).find((x) => x.on); return l ? l.fn : null; }
    #setHandler(type, fn) {
      const list = this.#listeners[type] || (this.#listeners[type] = []);
      const at = list.findIndex((x) => x.on);
      if (typeof fn !== 'function' && (fn === null || typeof fn !== 'object')) { if (at >= 0) list.splice(at, 1); return; }
      if (at >= 0) list[at].fn = fn; else list.push({ fn, on: true });
    }
    #fire(type, loaded = 0, total = 0, lengthComputable = false) {
      const ev = new globalThis.Event(type);
      for (const [k, v] of [['target', this], ['currentTarget', this], ['loaded', loaded], ['total', total], ['lengthComputable', lengthComputable]])
        Object.defineProperty(ev, k, { value: v, configurable: true, enumerable: true });
      this.dispatchEvent(ev);
    }
    #read(blob, kind, label) {
      if (blob === null || typeof blob !== 'object' || typeof blob.arrayBuffer !== 'function')
        throw new TypeError(`Failed to execute '${kind}' on 'FileReader': parameter 1 is not of type 'Blob'.`);
      if (this.readyState === 1) throw invalidState();
      const gen = ++this.#gen;
      this.readyState = 1;
      this.result = null;
      this.error = null;
      Promise.resolve().then(async () => {
        let bytes;
        try {
          // the Blob's bytes, not char codes of its decoded text (turbo-test#132)
          bytes = typeof blob.bytes === 'function' ? await blob.bytes() : new Uint8Array(await blob.arrayBuffer());
        } catch (e) {
          if (gen !== this.#gen) return;
          this.readyState = 2; this.error = e;
          this.#fire('loadstart'); this.#fire('error'); this.#fire('loadend');
          return;
        }
        if (gen !== this.#gen) return;
        this.#fire('loadstart');
        if (gen !== this.#gen) return;
        if (bytes.length) { this.#fire('progress', bytes.length, bytes.length, true); if (gen !== this.#gen) return; }
        if (kind === 'readAsDataURL') this.result = 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + globalThis.__b64encodeBytes(bytes);
        else if (kind === 'readAsArrayBuffer') this.result = bytes.slice().buffer;
        else if (kind === 'readAsBinaryString') { let s = ''; for (let i = 0; i < bytes.length; i += 8192) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192)); this.result = s; }
        else this.result = textOf(bytes, label);
        this.readyState = 2;
        this.#fire('load');
        this.#fire('loadend');
      });
    }
    readAsArrayBuffer(blob) { this.#read(blob, 'readAsArrayBuffer'); }
    readAsBinaryString(blob) { this.#read(blob, 'readAsBinaryString'); }
    readAsDataURL(blob) { this.#read(blob, 'readAsDataURL'); }
    readAsText(blob, encoding = undefined) { this.#read(blob, 'readAsText', encoding); }
    abort() {
      if (this.readyState === 0 || this.readyState === 2) { this.result = null; return; }
      this.#gen++;
      this.readyState = 2;
      this.result = null;
      this.#fire('abort');
      this.#fire('loadend');
    }
    static #accessors = (() => {
      for (const type of EVENTS) Object.defineProperty(FileReader.prototype, 'on' + type, {
        get() { return this.#handler(type); },
        set(fn) { this.#setHandler(type, fn); },
        configurable: true, enumerable: true,
      });
    })();
  };
  ['EMPTY', 'LOADING', 'DONE'].forEach((k, i) => {
    Object.defineProperty(globalThis.FileReader, k, { value: i, enumerable: true });
    Object.defineProperty(globalThis.FileReader.prototype, k, { value: i, enumerable: true });
  });
}
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
      return new globalThis.TextDecoder('utf-8', { ignoreBOM: true }).decode(this); // Node keeps a BOM
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
          // a real-clock task: Node's MessagePort is not a faked timer
          if (o) __realTimers.setTimeout(() => {
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
  // now(): ms on the real clock since timeOrigin, the runtime's start (turbo-test#178; it was a
  // constant 0)
  globalThis.performance = { now: () => __loop.elapsed() - __perfOrigin, timeOrigin: __perfOrigin, mark() {}, measure() {}, getEntriesByName: () => [], clearMarks() {}, clearMeasures() {} };
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
// FileReader is defined above EventTarget; make it one (turbo-test#139)
if (globalThis.FileReader && !(globalThis.FileReader.prototype instanceof globalThis.EventTarget)) {
  Object.setPrototypeOf(globalThis.FileReader.prototype, globalThis.EventTarget.prototype);
  Object.setPrototypeOf(globalThis.FileReader, globalThis.EventTarget);
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
// win32 (turbo-test#199): native drive-letter / UNC paths, like Node's `{ windows: true }`; the
// platform is read at call time.
const __ttWin = () => globalThis.__ttPlatform === 'win32';
globalThis.__ttIsAbsolutePath = (p) => (__ttWin() ? /^([a-zA-Z]:)?[\\/]/.test(p) : String(p).startsWith('/'));
globalThis.__fileURLToPath = (u) => {
  const url = typeof u === 'string' ? new globalThis.URL(u) : u;
  if (url.protocol !== 'file:') throw __urlError('ERR_INVALID_URL_SCHEME', 'The URL must be of scheme file');
  if (__ttWin()) {
    if (/%2f|%5c/i.test(url.pathname)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded \\ or / characters');
    const path = decodeURIComponent(url.pathname).replace(/\//g, '\\');
    if (url.hostname !== '') return `\\\\${url.hostname}${path}`;
    if (!/^\\[a-zA-Z]:/.test(path)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must be absolute');
    return path.slice(1);
  }
  if (url.hostname !== '') throw __urlError('ERR_INVALID_FILE_URL_HOST', `File URL host must be "localhost" or empty on ${globalThis.__ttPlatform || 'darwin'}`);
  if (/%2f/i.test(url.pathname)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded / characters');
  return decodeURIComponent(url.pathname);
};
// win32: [host, root ('/C:' or ''), rest] with `/` separators, made absolute against the cwd
const __ttWinSplit = (p) => {
  let path = String(p).replace(/\\/g, '/');
  const unc = /^\/\/([^/]+)(\/.*)?$/.exec(path);
  if (unc) return [unc[1], '', unc[2] || '/'];
  if (!/^[a-zA-Z]:\//.test(path)) {
    const cwd = String(globalThis.process.cwd()).replace(/\\/g, '/');
    path = path.startsWith('/') ? cwd.slice(0, 2) + path : cwd + '/' + path;
  }
  return ['', '/' + path.slice(0, 2), path.slice(2)];
};
globalThis.__pathToFileURL = (p) => {
  let host = '';
  let root = '';
  let path = String(p);
  // Node keeps a trailing separator only when the given path ends in one ('' is the cwd)
  const trailing = /\/$/.test(path) || (__ttWin() && path.endsWith('\\')) ? '/' : '';
  if (__ttWin()) [host, root, path] = __ttWinSplit(path);
  else if (!path.startsWith('/')) path = globalThis.process.cwd() + '/' + path;
  const out = [];
  for (const seg of path.split('/')) { if (seg === '..') out.pop(); else if (seg && seg !== '.') out.push(seg); }
  const resolved = root + '/' + out.join('/') + (out.length ? trailing : '');
  const url = new globalThis.URL(`file://${host}/`);
  // Node's set: controls, space, "#%<>?[\]^`{|}~ and DEL (the setter encodes non-ASCII)
  url.pathname = resolved.replace(/[\x00-\x20"#%<>?[\\\]^`{|}~\x7f]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));
  return url;
};
globalThis.__dirnameOf = (p) => {
  if (!__ttWin()) { const s = String(p).replace(/\/+$/, ''); const i = s.lastIndexOf('/'); return i <= 0 ? '/' : s.slice(0, i); }
  // path.win32.dirname for drive paths: the root (`C:\`) is its own dirname
  const s = String(p);
  const rootLen = /^[a-zA-Z]:[\\/]/.test(s) ? 3 : /^[\\/]/.test(s) ? 1 : 0;
  const t = s.replace(/[\\/]+$/, '');
  const i = Math.max(t.lastIndexOf('/'), t.lastIndexOf('\\'));
  return i < rootLen ? (rootLen ? s.slice(0, rootLen) : '.') : t.slice(0, i);
};
// import.meta (turbo-test#123): the module runner's CJS output (native emit + dep bundler, esbuild
// transforms) calls __ttImportMeta(<module path>) where the source said `import.meta` — one object
// per module path, like vitest's; the ESM graph's host callback fills V8's object with
// __ttFillImportMeta. vitest's import.meta.env: process.env, read and written live, DEV / PROD /
// SSR as booleans (vitest module-evaluator createImportMetaEnvProxy).
const __ttMetaBool = ['DEV', 'PROD', 'SSR'];
globalThis.__ttImportMetaEnv = new Proxy({}, {
  get: (_, k) => (typeof k !== 'string' ? undefined : __ttMetaBool.includes(k) ? !!globalThis.process.env[k] : globalThis.process.env[k]),
  set: (_, k, v) => {
    if (typeof k === 'string') globalThis.process.env[k] = __ttMetaBool.includes(k) ? (v ? '1' : '') : v;
    return true;
  },
  has: (_, k) => k in globalThis.process.env,
  deleteProperty: (_, k) => delete globalThis.process.env[k],
  ownKeys: () => Reflect.ownKeys(globalThis.process.env),
  getOwnPropertyDescriptor: (_, k) => {
    if (!Object.prototype.hasOwnProperty.call(globalThis.process.env, k)) return undefined;
    return { value: globalThis.__ttImportMetaEnv[k], writable: true, enumerable: true, configurable: true };
  },
});
// vitest 4.1.12's members (vite createNodeImportMeta, turbo-test#130), in its order: filename,
// dirname, url, env, resolve (Node's import.meta.resolve from the module), glob (vite replaces a
// full-name call at transform time, src/import_glob.rs; any other call throws), main (false), and
// in the test file the non-enumerable `vitest` getter (in-source testing's vitest module).
const __ttMetaGlob = { glob() { throw new Error('[module runner] "import.meta.glob" is statically replaced during file transformation. Make sure to reference it by the full name.'); } }.glob;
// Node's import.meta.resolve(specifier, parent): relative / absolute specifiers and URLs against
// the parent URL (no file check), builtins as `node:` URLs, packages by the ESM conditions.
const __ttMetaResolve = (specifier, parent) => {
  const spec = String(specifier);
  if (/^\.{0,2}\//.test(spec) || spec === '.' || spec === '..') return new globalThis.URL(spec, parent).href;
  if (/^[a-zA-Z][a-zA-Z\d+.-]*:/.test(spec) && !spec.startsWith('node:')) {
    try { return new globalThis.URL(spec).href; } catch (e) { /* not a URL: a package name */ }
  }
  let from;
  try { from = globalThis.__fileURLToPath(parent); } catch (e) { from = String(parent); }
  const r = globalThis.__ttRequireResolve(globalThis.__dirnameOf(from), spec, true);
  if (typeof r === 'string' && globalThis.__ttIsAbsolutePath(r)) return globalThis.__pathToFileURL(r).href;
  if (typeof r === 'string' && spec !== 'vitest' && spec !== '@jest/globals') return 'node:' + r.replace(/^node:/, '');
  const pkg = spec.startsWith('@') ? spec.split('/').slice(0, 2).join('/') : spec.split('/')[0];
  const e = new Error(`Cannot find package '${pkg}' imported from ${from}`);
  e.code = 'ERR_MODULE_NOT_FOUND';
  throw e;
};
globalThis.__ttFillImportMeta = (m, filename) => {
  const url = globalThis.__pathToFileURL(filename).href;
  const dirname = globalThis.__dirnameOf(filename);
  m.filename = filename;
  m.dirname = dirname;
  m.url = url;
  m.env = globalThis.__ttImportMetaEnv;
  m.resolve = { resolve(specifier, parent) { return __ttMetaResolve(specifier, parent ?? url); } }.resolve;
  m.glob = __ttMetaGlob;
  m.main = false;
  __ttMetas.set(filename, m);
  return __ttMetaVitest(m, filename);
};
// the test file's `vitest` (its import.meta can be made before the file is the current one: the
// entry's vi.mock prepass)
const __ttMetaVitest = (m, filename) => {
  if (filename === globalThis.__ttFile && !Object.prototype.hasOwnProperty.call(m, 'vitest')) {
    Object.defineProperty(m, 'vitest', { get: () => globalThis.__nativeRequire(m.dirname, 'vitest', true) });
  }
  return m;
};
const __ttMetas = new Map();
globalThis.__ttImportMeta = (filename) => __ttMetaVitest(__ttMetas.get(filename) || globalThis.__ttFillImportMeta({}, filename), filename);
// the runner names the test file once it is loaded: its import.meta, made while loading, gets
// `vitest` then
globalThis.__ttMetaOfTestFile = () => { const m = __ttMetas.get(globalThis.__ttFile); if (m) __ttMetaVitest(m, globalThis.__ttFile); };
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
  // a require(): the `require` export condition (turbo-test#134)
  createRequire: (url) => globalThis.__mkRequire(globalThis.__dirnameOf(String(url).startsWith('file:') ? globalThis.__fileURLToPath(url) : String(url)), false),
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
// ---- Node's util.inspect / util.format (Node 26; turbo-test#93) ----
// The shape of Node's formatValue / formatRaw / reduceToSingleString / groupArrayElements:
// depth 2, breakLength 80, compact 3, maxArrayLength 100; console.log formats through it.
// `colors` (vitest's console under its coloured reporters, turbo-test#94) stylizes as Node's
// stylizeWithColor with inspect.styles, RegExps through Node's highlightRegExp.
const __inspectStyles = { special: [36, 39], number: [33, 39], bigint: [33, 39], boolean: [33, 39], undefined: [90, 39], null: [1, 22], string: [32, 39], symbol: [32, 39], date: [35, 39], module: [4, 24] };
function __sty(ctx, str, type) {
  if (!ctx.colors) return str;
  if (type === 'regexp') return __highlightRegExp(str);
  const c = __inspectStyles[type];
  return c ? `\x1b[${c[0]}m${str}\x1b[${c[1]}m` : str;
}
const __stripAnsi = (s) => s.replace(/\x1b\[[0-9;]*m/g, '');
// Node's highlightRegExp: the pattern's parts in the depth palette green, red, yellow, cyan,
// magenta (groups, classes, quantifiers, escapes, assertions), the flags after the closing `/`.
function __highlightRegExp(re) {
  const palette = [[32, 39], [31, 39], [33, 39], [36, 39], [35, 39]];
  let out = '';
  let i = 0;
  let depth = 0;
  let inClass = false;
  const write = (str) => { const c = palette[depth % palette.length] ?? palette[0]; out += `\x1b[${c[0]}m${str}\x1b[${c[1]}m`; };
  const writeDepth = (str, incDepth, incI) => { depth += incDepth; write(str); depth -= incDepth; i += incI; };
  const writeGroup = (start, end, decreaseDepth = 1) => {
    let seq = '';
    i++;
    while (i < re.length && re[i] !== end) seq += re[i++];
    if (i < re.length) {
      depth -= decreaseDepth;
      write(start);
      writeDepth(seq, 1, 1);
      write(end);
      depth += decreaseDepth;
    } else {
      writeDepth(start, 1, -seq.length);
    }
  };
  write('/');
  depth++;
  i = 1;
  while (i < re.length) {
    const ch = re[i];
    if (inClass) {
      if (ch === '\\') {
        let seq = '\\';
        i++;
        if (i < re.length) {
          seq += re[i++];
          const next = seq[1];
          if (next === 'u' && re[i] === '{') { writeGroup(`${seq}{`, '}', 0); continue; }
          else if ((next === 'p' || next === 'P') && re[i] === '{') { writeGroup(`${seq}{`, '}', 0); continue; }
          else if (seq[1] === 'x') { seq += re.slice(i, i + 2); i += 2; }
        }
        write(seq);
      } else if (ch === ']') {
        depth--; write(']'); i++; inClass = false;
      } else if (ch === '-' && re[i - 1] !== '[' && i + 1 < re.length && re[i + 1] !== ']') {
        writeDepth('-', 1, 1);
      } else {
        write(ch); i++;
      }
    } else if (ch === '[') {
      write('['); depth++; i++; inClass = true;
    } else if (ch === '(') {
      write('('); depth++; i++;
      if (i < re.length && re[i] === '?') {
        i++;
        const a = i < re.length ? re[i] : '';
        if (a === ':' || a === '=' || a === '!') {
          writeDepth(`?${a}`, -1, 1);
        } else {
          const b = i + 1 < re.length ? re[i + 1] : '';
          if (a === '<' && (b === '=' || b === '!')) {
            writeDepth(`?<${b}`, -1, 2);
          } else if (a === '<') {
            i++;
            const start = i;
            while (i < re.length && re[i] !== '>') i++;
            const name = re.slice(start, i);
            if (i < re.length && re[i] === '>') {
              depth--; write('?<'); writeDepth(name, 1, 0); write('>'); depth++; i++;
            } else {
              writeDepth('?<', -1, 0); write(name);
            }
          } else {
            write('?');
          }
        }
      }
    } else if (ch === ')') {
      depth--; write(')'); i++;
    } else if (ch === '\\') {
      let seq = '\\';
      i++;
      if (i < re.length) {
        seq += re[i++];
        const next = seq[1];
        if (i < re.length) {
          if (next === 'u' && re[i] === '{') { writeGroup(`${seq}{`, '}', 0); continue; }
          else if (next === 'x') { seq += re.slice(i, i + 2); i += 2; }
          else if (next >= '0' && next <= '9') { while (i < re.length && re[i] >= '0' && re[i] <= '9') seq += re[i++]; }
          else if (next === 'k' && re[i] === '<') { writeGroup(`${seq}<`, '>'); continue; }
          else if ((next === 'p' || next === 'P') && re[i] === '{') { writeGroup(`${seq}{`, '}', 0); continue; }
        }
      }
      writeDepth(seq, 1, 0);
    } else if (ch === '|' || ch === '+' || ch === '*' || ch === '?' || ch === ',' || ch === '^' || ch === '$') {
      writeDepth(ch, 3, 1);
    } else if (ch === '{') {
      i++;
      let digits = '';
      while (i < re.length && re[i] >= '0' && re[i] <= '9') digits += re[i++];
      if (digits) { write('{'); depth++; writeDepth(digits, 1, 0); }
      if (i < re.length) {
        if (re[i] === ',') {
          if (!digits) { write('{'); depth++; }
          write(','); i++;
        } else if (!digits) {
          depth += 1; write('{'); depth -= 1;
          continue;
        }
      }
      let digits2 = '';
      while (i < re.length && re[i] >= '0' && re[i] <= '9') digits2 += re[i++];
      if (digits2) writeDepth(digits2, 1, 0);
      if (i < re.length && re[i] === '}') { depth--; write('}'); i++; }
      if (i < re.length && re[i] === '?') writeDepth('?', 3, 1);
    } else if (ch === '.') {
      writeDepth(ch, 2, 1);
    } else if (ch === '/') {
      break;
    } else {
      writeDepth(ch, 1, 1);
    }
  }
  writeDepth('/', -1, 1);
  if (i < re.length) write(re.slice(i));
  return out;
}
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
// formatPrimitive; `plain` is Node's stylizeNoColor (boxed primitives)
function __inspectPrimitive(v, ctx, plain) {
  const sty = (str, type) => (plain ? str : __sty(ctx, str, type));
  switch (typeof v) {
    case 'string': {
      const trailer = '';
      if (v.length > 16 && v.length > 80 - ctx.indentationLvl - 4) {
        const parts = v.split(/(?<=\n)/);
        if (parts.length > 1) return parts.map((l) => sty(__strEscape(l), 'string')).join(` +\n${' '.repeat(ctx.indentationLvl + 2)}`) + trailer;
      }
      return sty(__strEscape(v), 'string') + trailer;
    }
    case 'number': return sty(Object.is(v, -0) ? '-0' : String(v), 'number');
    case 'bigint': return sty(`${v}n`, 'bigint');
    case 'boolean': return sty(String(v), 'boolean');
    case 'undefined': return sty('undefined', 'undefined');
    case 'symbol': return sty(v.toString(), 'symbol');
  }
  return String(v);
}
function __formatValue(ctx, value, recurseTimes, typedArray) {
  if (typeof value !== 'object' && typeof value !== 'function') return __inspectPrimitive(value, ctx);
  if (value === null) return __sty(ctx, 'null', 'null');
  // a [util.inspect.custom](depth, options, inspect) method formats its own value
  const custom = value[Symbol.for('nodejs.util.inspect.custom')];
  if (typeof custom === 'function' && custom !== __util.inspect) {
    const r = custom.call(value, ctx.depth - recurseTimes, { depth: ctx.depth - recurseTimes, breakLength: ctx.breakLength, compact: ctx.compact, colors: ctx.colors }, __util.inspect);
    // a multi-line result is indented to where it is printed, as Node does
    if (r !== value) return typeof r === 'string' ? r.replace(/\n/g, `\n${' '.repeat(ctx.indentationLvl)}`) : __formatValue(ctx, r, recurseTimes);
  }
  if (ctx.seen.includes(value)) {
    let index = 1;
    if (ctx.circular === undefined) { ctx.circular = new Map(); ctx.circular.set(value, index); }
    else {
      index = ctx.circular.get(value);
      if (index === undefined) { index = ctx.circular.size + 1; ctx.circular.set(value, index); }
    }
    return __sty(ctx, `[Circular *${index}]`, 'special');
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
    str = __sty(ctx, desc.set !== undefined ? '[Getter/Setter]' : '[Getter]', 'special');
  } else if (desc.set !== undefined) {
    str = __sty(ctx, '[Setter]', 'special');
  } else {
    str = __sty(ctx, 'undefined', 'undefined');
  }
  if (type === 1) return str; // an array element (2: an array's extra key)
  if (typeof key === 'symbol') name = __sty(ctx, key.toString(), 'symbol');
  else if (desc.enumerable === false) name = `[${key}]`;
  else name = /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : __sty(ctx, __strEscape(key), 'string');
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
    const len = ctx.colors ? __stripAnsi(output[i]).length : output[i].length;
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
    totalLength += ctx.colors ? __stripAnsi(output[i]).length : output[i].length;
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
        if (holes) { out.push(__sty(ctx, `<${holes} empty item${holes > 1 ? 's' : ''}>`, 'undefined')); holes = 0; }
        out.push(__formatProperty(ctx, value, recurseTimes, i, 1));
      }
      if (holes) out.push(__sty(ctx, `<${holes} empty item${holes > 1 ? 's' : ''}>`, 'undefined'));
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
      if (keys.length === 0) return __sty(ctx, base, 'special');
      braces = ['{', '}'];
    } else if (value instanceof RegExp) {
      base = __sty(ctx, RegExp.prototype.toString.call(value), 'regexp');
      if (keys.length === 0 || recurseTimes > ctx.depth) return base;
      braces = ['{', '}'];
    } else if (value instanceof Date) {
      base = Number.isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
      if (keys.length === 0) return __sty(ctx, base, 'date');
      braces = ['{', '}'];
    } else if (value instanceof Error) {
      base = value.stack ? String(value.stack) : `[${Error.prototype.toString.call(value)}]`;
      keys = keys.filter((k) => k !== 'stack' && k !== 'message');
      if (keys.length === 0) return base;
      braces = ['{', '}'];
    } else if (value instanceof Number || value instanceof String || value instanceof Boolean) {
      const type = value instanceof Number ? 'Number' : value instanceof String ? 'String' : 'Boolean';
      base = `[${type}: ${__inspectPrimitive(value.valueOf(), ctx, true)}]`;
      if (value instanceof String) keys = keys.filter((k) => !/^(0|[1-9][0-9]*)$/.test(k));
      if (keys.length === 0) return __sty(ctx, base, type.toLowerCase());
      braces = ['{', '}'];
    } else if (value instanceof Promise) {
      braces = [`${prefix('Promise', '')}{`, '}'];
      formatter = () => [__sty(ctx, '<pending>', 'special')];
    } else if (value instanceof WeakMap || value instanceof WeakSet) {
      braces = [`${prefix(value instanceof WeakMap ? 'WeakMap' : 'WeakSet', '')}{`, '}'];
      formatter = () => [__sty(ctx, '<items unknown>', 'special')];
    } else {
      const p = ctor === 'Object' && !(typeof tag === 'string' && tag !== '') ? '' : prefix('Object', '');
      braces = [`${p}{`, '}'];
      if (keys.length === 0) return `${p}{}`;
    }
  }
  if (recurseTimes > ctx.depth) {
    const name = (ctor || tag || 'Object');
    return __sty(ctx, Array.isArray(value) ? '[Array]' : `[${name}]`, 'special');
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
    if (index !== undefined) res = `${__sty(ctx, `<ref *${index}>`, 'special')} ${res}`;
  }
  void noIterator;
  return res;
}
// The options a [util.inspect.custom] method passes on to its inner inspect calls (Node's
// `breakLength` 80 when the caller gives none).
function __inspectCustomOpts(opts) {
  const o = { breakLength: 80, colors: false };
  if (opts && typeof opts.breakLength === 'number') o.breakLength = opts.breakLength;
  if (opts && opts.compact !== undefined) o.compact = opts.compact;
  if (opts && opts.colors) o.colors = true;
  if (opts && (typeof opts.depth === 'number' || opts.depth === null)) o.depth = opts.depth;
  return o;
}
function __nodeInspect(value, opts) {
  const ctx = { seen: [], indentationLvl: 0, currentDepth: 0, depth: 2, breakLength: 80, compact: 3, maxArrayLength: 100, circular: undefined, colors: false };
  if (opts) {
    if (typeof opts.depth === 'number' || opts.depth === null) ctx.depth = opts.depth === null ? Infinity : opts.depth;
    if (typeof opts.breakLength === 'number') ctx.breakLength = opts.breakLength;
    if (opts.compact !== undefined) ctx.compact = opts.compact;
    if (opts.colors) ctx.colors = true;
  }
  return __formatValue(ctx, value, 0);
}
// util.format: %s %d %i %f %j %o %O %c %%, the rest inspected (strings as they are)
function __nodeFormat(...args) {
  return __nodeFormatWith({}, ...args);
}
// util.formatWithOptions: `o.colors` colours %o / %O and the inspected rest (not %s / %d / …)
function __nodeFormatWith(o, ...args) {
  const colors = !!(o && o.colors);
  const first = args[0];
  const insp = (v) => (typeof v === 'string' ? v : __nodeInspect(v, { colors }));
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
      case 'o': s = __nodeInspect(a, { depth: 4, colors }); break;
      case 'O': s = __nodeInspect(a, { colors }); break;
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
  formatWithOptions: (o, ...a) => __nodeFormatWith(o, ...a),
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
  // on the real clock: node:timers/promises is never faked (turbo-test#168)
  setTimeout: (delay, value, options) => __timerPromise((cb) => __realTimers.setTimeout(cb, delay), (id) => __realTimers.clearTimeout(id), value, options),
  setImmediate: (value, options) => __timerPromise((cb) => __realTimers.setImmediate(cb), (id) => __realTimers.clearImmediate(id), value, options),
  // async iterator yielding `value` once per tick (ticks the consumer hasn't taken yet queue up)
  setInterval: (delay, value, options) => {
    const signal = options && options.signal;
    let ticks = 0, wake = null, done = false, error = null;
    const notify = () => { if (wake) { const w = wake; wake = null; w(); } };
    const id = __realTimers.setInterval(() => { ticks++; notify(); }, delay);
    const finish = () => { done = true; __realTimers.clearInterval(id); if (signal) signal.removeEventListener('abort', onAbort); };
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
  // node:timers is never faked (vitest fakes the globals only)
  ...__realTimers,
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
// an app ES module's namespace in vitest's order (turbo-test#141) -> the V8 namespace
const __ttOrderedTargets = new WeakMap();
const __ttOrderedViews = new WeakMap();
// The ESM graph's view of an app ES module's namespace: its names in `order` (source order),
// then the others (`export *`'s) as V8 sorts them. Reads and writes are the namespace's.
globalThis.__ttOrderedNs = (ns, order) => {
  let v = __ttOrderedViews.get(ns);
  if (!v) {
    const at = new Map(order.map((k, i) => [k, i]));
    const rank = (k) => (at.has(k) ? at.get(k) : order.length);
    v = new Proxy(ns, {
      ownKeys: (t) => {
        const keys = Reflect.ownKeys(t);
        const names = keys.filter((k) => typeof k === 'string');
        // a stable sort keeps V8's order among the names `order` does not know
        names.sort((a, b) => rank(a) - rank(b));
        return [...names, ...keys.filter((k) => typeof k !== 'string')];
      },
    });
    __ttOrderedViews.set(ns, v);
    __ttOrderedTargets.set(v, ns);
  }
  return v;
};
const __ttNsTarget = (ns) => __ttOrderedTargets.get(ns) || ns;
// Spy-able ESM-graph namespaces (esm_spy.rs, turbo-test#131, #146), as vitest's exports objects:
// an app module registers its namespace (its own export names, its re-exports by name and its
// `export *` sources); vi.spyOn sets an override (__ttNsSet) that importers' reads (__ttR) and
// namespace views (__ttNs) see, while the module's own references keep the original binding.
const __ttNsInfo = new WeakMap();
const __ttNsViews = new WeakMap();
const __ttNsRaw = new WeakMap();
// keyed by the V8 namespace: a module reaches it through J5's ordered view (turbo-test#141) or
// as V8's own (__ttNsTarget)
globalThis.__ttNsRegister = (ns, own, re, stars) => {
  const t = __ttNsTarget(ns);
  if (!__ttNsInfo.has(t)) __ttNsInfo.set(t, { over: new Map(), own: new Set(own), re, stars });
};
const __ttNsLookup = (ns, k) => {
  const info = __ttNsInfo.get(__ttNsTarget(ns));
  if (info === undefined || typeof k !== 'string') return undefined;
  if (info.over.has(k)) return { v: info.over.get(k) };
  // a name the namespace does not list (a CommonJS source's `__esModule`) is not re-exported
  if (!info.own.has(k) && k in ns) {
    const r = Object.prototype.hasOwnProperty.call(info.re, k) ? info.re[k] : undefined;
    if (r !== undefined) return { v: globalThis.__ttR(r[0], r[1]) };
    if (k !== 'default') for (const s of info.stars) if (k in s) return { v: globalThis.__ttR(s, k) };
  }
  return undefined;
};
globalThis.__ttR = (ns, k) => {
  const found = __ttNsLookup(ns, k);
  return found === undefined ? ns[k] : found.v;
};
globalThis.__ttNs = (ns) => {
  if (ns === null || typeof ns !== 'object' || !__ttNsInfo.has(__ttNsTarget(ns))) return ns;
  let view = __ttNsViews.get(ns);
  if (view === undefined) {
    view = new Proxy(ns, {
      get: (t, k) => globalThis.__ttR(t, k),
      getOwnPropertyDescriptor: (t, k) => {
        const d = Reflect.getOwnPropertyDescriptor(t, k);
        if (d !== undefined && 'value' in d) d.value = globalThis.__ttR(t, k);
        return d;
      },
    });
    __ttNsViews.set(ns, view);
    __ttNsRaw.set(view, ns);
  }
  return view;
};
// set export `k` of a registered namespace (or its view) to `v`; false when `ns` is not one or
// has no such export. Setting the module's own value back drops the override.
globalThis.__ttNsSet = (ns, k, v) => {
  const raw = (ns !== null && typeof ns === 'object' && __ttNsRaw.get(ns)) || ns;
  const info = raw !== null && typeof raw === 'object' ? __ttNsInfo.get(__ttNsTarget(raw)) : undefined;
  if (info === undefined || typeof k !== 'string' || !(k in raw)) return false;
  info.over.delete(k);
  if (globalThis.__ttR(raw, k) !== v) info.over.set(k, v);
  return true;
};
// require() of an ES module from CommonJS (`isEsm` false) gives Node's require(esm) object: one
// with a default export and no `__esModule` of its own reads an enumerable `__esModule: true`
// (turbo-test#133).
const __ttRequireEsm = (m) => {
  if (m && typeof m === 'object' && m[Symbol.for('turbo-test.esm')] && 'default' in m) {
    const d = Object.getOwnPropertyDescriptor(m, '__esModule');
    if (d && d.configurable && !d.enumerable) Object.defineProperty(m, '__esModule', { value: true, enumerable: true, configurable: true });
  }
  return m;
};
globalThis.__mkRequire = (dir, isEsm, importer) => {
  const req = isEsm === false
    ? (spec) => __ttRequireEsm(globalThis.__nativeRequire(dir, spec, false, importer))
    : (spec) => globalThis.__nativeRequire(dir, spec, isEsm, importer);
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

// ---- asymmetric matchers: @vitest/expect's classes (turbo-test#140, #153, #154, #155) ----
// The members are vitest 4.1.12's: `$$typeof` (Symbol.for('jest.asymmetricMatcher')), `sample`,
// `inverse`, getMatcherContext, asymmetricMatch(other, customTesters), toString (the printed
// name), getExpectedType (the diff's expected type) and, where vitest has it, toAsymmetricMatcher.
// `equals` takes any object with an asymmetricMatch method as one (vitest's isAsymmetric).
class Asymmetric {
  constructor(sample, inverse = false) {
    this.$$typeof = Symbol.for('jest.asymmetricMatcher');
    this.sample = sample;
    this.inverse = inverse;
  }
  getMatcherContext() {
    return { ...globalThis.expect.getState(), equals: __equals, isNot: this.inverse, customTesters: __customTesters, utils: __matcherUtilsFull() };
  }
}
class __StringContaining extends Asymmetric {
  constructor(sample, inverse = false) {
    if (!__isA('String', sample)) throw new Error('Expected is not a string');
    super(sample, inverse);
  }
  asymmetricMatch(other) { const r = __isA('String', other) && other.includes(this.sample); return this.inverse ? !r : r; }
  toString() { return `String${this.inverse ? 'Not' : ''}Containing`; }
  getExpectedType() { return 'string'; }
}
class __Anything extends Asymmetric {
  asymmetricMatch(other) { return other != null; }
  toString() { return 'Anything'; }
  toAsymmetricMatcher() { return 'Anything'; }
}
class __ObjectContaining extends Asymmetric {
  constructor(sample, inverse = false) { super(sample, inverse); }
  getPrototype(obj) {
    if (Object.getPrototypeOf) return Object.getPrototypeOf(obj);
    if (obj.constructor.prototype === obj) return null;
    return obj.constructor.prototype;
  }
  hasProperty(obj, property) {
    if (!obj) return false;
    if (Object.hasOwn(obj, property)) return true;
    return this.hasProperty(this.getPrototype(obj), property);
  }
  getProperties(obj) {
    return [...Object.keys(obj), ...Object.getOwnPropertySymbols(obj).filter((s) => Object.getOwnPropertyDescriptor(obj, s)?.enumerable)];
  }
  asymmetricMatch(other, customTesters) {
    if (typeof this.sample !== 'object') throw new TypeError(`You must provide an object to ${this.toString()}, not '${typeof this.sample}'.`);
    let result = true;
    for (const property of this.getProperties(this.sample)) {
      if (!this.hasProperty(other, property) || !__equals(this.sample[property], other[property], customTesters)) { result = false; break; }
    }
    return this.inverse ? !result : result;
  }
  toString() { return `Object${this.inverse ? 'Not' : ''}Containing`; }
  getExpectedType() { return 'object'; }
}
class __ArrayContaining extends Asymmetric {
  constructor(sample, inverse = false) { super(sample, inverse); }
  asymmetricMatch(other, customTesters) {
    if (!Array.isArray(this.sample)) throw new TypeError(`You must provide an array to ${this.toString()}, not '${typeof this.sample}'.`);
    const r = this.sample.length === 0 || (Array.isArray(other) && this.sample.every((item) => other.some((another) => __equals(item, another, customTesters))));
    return this.inverse ? !r : r;
  }
  toString() { return `Array${this.inverse ? 'Not' : ''}Containing`; }
  getExpectedType() { return 'array'; }
}
class __Any extends Asymmetric {
  constructor(sample) {
    if (typeof sample === 'undefined') throw new TypeError('any() expects to be passed a constructor function. Please pass one or use anything() to match any object.');
    super(sample);
  }
  fnNameFor(func) {
    if (func.name) return func.name;
    const m = Function.prototype.toString.call(func).match(/^(?:async)?\s*function\s*(?:\*\s*)?([\w$]+)\s*\(/);
    return m ? m[1] : '<anonymous>';
  }
  asymmetricMatch(other) {
    const s = this.sample;
    if (s === String) return typeof other == 'string' || other instanceof String;
    if (s === Number) return typeof other == 'number' || other instanceof Number;
    if (s === Function) return typeof other == 'function';
    if (s === Boolean) return typeof other == 'boolean' || other instanceof Boolean;
    if (s === BigInt) return typeof other == 'bigint' || other instanceof BigInt;
    if (s === Symbol) return typeof other == 'symbol' || other instanceof Symbol;
    if (s === Object) return typeof other == 'object';
    return other instanceof s;
  }
  toString() { return 'Any'; }
  getExpectedType() {
    const s = this.sample;
    if (s === String) return 'string';
    if (s === Number) return 'number';
    if (s === Function) return 'function';
    if (s === Object) return 'object';
    if (s === Boolean) return 'boolean';
    return this.fnNameFor(s);
  }
  toAsymmetricMatcher() { return `Any<${this.fnNameFor(this.sample)}>`; }
}
class __StringMatching extends Asymmetric {
  constructor(sample, inverse = false) {
    if (!__isA('String', sample) && !__isA('RegExp', sample)) throw new Error('Expected is not a String or a RegExp');
    super(new RegExp(sample), inverse);
  }
  asymmetricMatch(other) { const r = __isA('String', other) && this.sample.test(other); return this.inverse ? !r : r; }
  toString() { return `String${this.inverse ? 'Not' : ''}Matching`; }
  getExpectedType() { return 'string'; }
}
class __CloseTo extends Asymmetric {
  constructor(sample, precision = 2, inverse = false) {
    if (!__isA('Number', sample)) throw new Error('Expected is not a Number');
    if (!__isA('Number', precision)) throw new Error('Precision is not a Number');
    super(sample);
    this.precision = undefined;
    this.inverse = inverse;
    this.precision = precision;
  }
  asymmetricMatch(other) {
    if (!__isA('Number', other)) return false;
    let r = false;
    if (other === Infinity && this.sample === Infinity) r = true;
    else if (other === -Infinity && this.sample === -Infinity) r = true;
    else r = Math.abs(this.sample - other) < 10 ** -this.precision / 2;
    return this.inverse ? !r : r;
  }
  toString() { return `Number${this.inverse ? 'Not' : ''}CloseTo`; }
  getExpectedType() { return 'number'; }
  toAsymmetricMatcher() { return [this.toString(), this.sample, `(${this.precision} digit${this.precision === 1 ? '' : 's'})`].join(' '); }
}
// a Standard Schema (`~standard.validate`), synchronous only, as vitest's schemaMatching
const __isStandardSchema = (o) => !!o && (typeof o === 'object' || typeof o === 'function') && o['~standard'] && typeof o['~standard'].validate === 'function';
class __SchemaMatching extends Asymmetric {
  constructor(sample, inverse = false) {
    if (!__isStandardSchema(sample)) throw new TypeError('SchemaMatching expected to receive a Standard Schema.');
    super(sample, inverse);
    this.result = undefined;
  }
  asymmetricMatch(other) {
    const result = this.sample['~standard'].validate(other);
    if (result instanceof Promise) throw new TypeError('Async schema validation is not supported in asymmetric matchers.');
    this.result = result;
    const pass = !this.result.issues || this.result.issues.length === 0;
    return this.inverse ? !pass : pass;
  }
  toString() { return `Schema${this.inverse ? 'Not' : ''}Matching`; }
  getExpectedType() { return 'object'; }
  toAsymmetricMatcher() {
    const issues = (this.result && this.result.issues) || [];
    return issues.length > 0 ? `${this.toString()} ${__pf(this.result, { maxDepth: 10, escape: false, basic: false })}` : this.toString();
  }
}
// The asymmetric form vitest gives every jest-style matcher (expect.extend's and the built-in
// toSatisfy / toBeOneOf): expect.<name>(...sample) / expect.not.<name>(...sample) run the matcher
// against the other value with its matcher context.
function __customAsymmetric(name, matcher) {
  return class CustomMatcher extends Asymmetric {
    constructor(inverse = false, ...sample) { super(sample, inverse); }
    asymmetricMatch(other) {
      const { pass } = matcher.call(this.getMatcherContext(), other, ...this.sample);
      return this.inverse ? !pass : pass;
    }
    toString() { return `${this.inverse ? 'not.' : ''}${name}`; }
    getExpectedType() { return 'any'; }
    toAsymmetricMatcher() { return `${this.toString()}<${this.sample.map((item) => __stringify(item)).join(', ')}>`; }
  };
}
// vitest's isAsymmetric: any object with an asymmetricMatch method
function __isAsym(o) { return !!o && typeof o === 'object' && 'asymmetricMatch' in o && typeof o.asymmetricMatch === 'function'; }
// @vitest/utils getType: the Object.prototype.toString class name
function __getType(v) { return Object.prototype.toString.apply(v).slice(8, -1); }
// @vitest/expect isA: the Object.prototype.toString class
function __isA(type, v) { return Object.prototype.toString.apply(v) === `[object ${type}]`; }

// vitest's jest-style built-in matchers (selectively ported from jest-extended): called with the
// matcher state as `this`, they return { pass, message } as an expect.extend matcher does.
const __jestMatchers = {
  toSatisfy(actual, expected, message) {
    const pass = expected(actual);
    return {
      pass,
      message: () => (pass
        ? `expect(received).not.toSatisfy()\n\nExpected value to not satisfy:\n${message || __stringify(expected)}\nReceived:\n${__stringify(actual)}`
        : `expect(received).toSatisfy()\n\nExpected value to satisfy:\n${message || __stringify(expected)}\n\nReceived:\n${__stringify(actual)}`),
    };
  },
  // vitest's toBeOneOf (turbo-test#140): `actual` equals one of an array's / Set's items (a Set by
  // `has` first) with `equals` and the custom testers; an empty array / Set always passes.
  toBeOneOf(actual, expected) {
    const { equals, customTesters } = this;
    let pass;
    if (Array.isArray(expected)) pass = expected.length === 0 || expected.some((item) => equals(item, actual, customTesters));
    else if (expected instanceof Set) pass = expected.size === 0 || expected.has(actual) || [...expected].some((item) => equals(item, actual, customTesters));
    else throw new TypeError(`You must provide an array or set to expect(received).toBeOneOf(expected), not '${typeof expected}'.`);
    return {
      pass,
      message: () => (pass
        ? `expect(received).not.toBeOneOf()\n\nExpected value to not be one of:\n${__stringify(expected)}\nReceived:\n${__stringify(actual)}`
        : `expect(received).toBeOneOf()\n\nExpected value to be one of:\n${__stringify(expected)}\n\nReceived:\n${__stringify(actual)}`),
    };
  },
};

// ---- deep equality: @vitest/expect's `equals` and its testers (turbo-test#103) ----
// `equals(a, b, testers, strictCheck)` is jasmine's `eq` as vitest runs it: asymmetric matchers,
// then the custom testers (the first defined answer wins), URLs by href, Object.is, the
// `Object.prototype.toString` class (boxed primitives by value, Dates by time — invalid Dates are
// equal —, RegExps by source and flags), DOM nodes by isEqualNode, circular structures (equal
// when both cycle at the same depth), Errors by name / message / cause / AggregateError errors /
// enumerable properties, then enumerable own string keys and enumerable symbol keys. Without
// `strictCheck` an own key whose value is `undefined` counts as absent (string keys only).
// Each matcher passes vitest's testers: toEqual and the spy matchers [iterableEquality];
// toStrictEqual adds typeEquality, sparseArrayEquality, arrayBufferEquality with strictCheck;
// toMatchObject [iterableEquality, subsetEquality]; toContainEqual / toHaveProperty none.
function __hasKey(o, k) { return Object.prototype.hasOwnProperty.call(o, k); }
function __hasDefinedKey(o, k) { return __hasKey(o, k) && o[k] !== undefined; }
function __eqKeys(o, hasKey) {
  const out = [];
  for (const k in o) if (hasKey(o, k)) out.push(k);
  return out.concat(Object.getOwnPropertySymbols(o).filter((s) => Object.getOwnPropertyDescriptor(o, s).enumerable));
}
function __isError(v) {
  if (typeof Error.isError === 'function') return Error.isError(v);
  const t = Object.prototype.toString.call(v);
  return t === '[object Error]' || t === '[object Exception]' || t === '[object DOMException]' || v instanceof Error;
}
function __isDomNode(o) {
  // `in` first, as vitest: a mocked module's namespace throws on a read of a missing export
  return o !== null && typeof o === 'object' && 'nodeType' in o && typeof o.nodeType === 'number' &&
    'nodeName' in o && typeof o.nodeName === 'string' && 'isEqualNode' in o && typeof o.isEqualNode === 'function';
}
function __equals(a, b, testers, strictCheck) {
  return __eq(a, b, [], [], testers || [], strictCheck ? __hasKey : __hasDefinedKey);
}
function __eq(a, b, aStack, bStack, testers, hasKey) {
  const asymA = __isAsym(a), asymB = __isAsym(b);
  if (asymA && !asymB) return a.asymmetricMatch(b, testers);
  if (asymB && !asymA) return b.asymmetricMatch(a, testers);
  const ctx = { equals: __equals };
  for (const t of testers) {
    const r = t.call(ctx, a, b, testers);
    if (r !== undefined) return r;
  }
  if (typeof URL === 'function' && a instanceof URL && b instanceof URL) return a.href === b.href;
  if (Object.is(a, b)) return true;
  if (a === null || b === null) return a === b;
  const cls = Object.prototype.toString.call(a);
  if (cls !== Object.prototype.toString.call(b)) return false;
  switch (cls) {
    case '[object Boolean]':
    case '[object String]':
    case '[object Number]':
      if (typeof a !== typeof b) return false;
      if (typeof a !== 'object') return Object.is(a, b);
      return Object.is(a.valueOf(), b.valueOf());
    case '[object Date]': {
      const x = +a, y = +b;
      return x === y || (Number.isNaN(x) && Number.isNaN(y));
    }
    case '[object RegExp]': return a.source === b.source && a.flags === b.flags;
  }
  if (typeof a !== 'object' || typeof b !== 'object') return false;
  if (__isDomNode(a) && __isDomNode(b)) return a.isEqualNode(b);
  for (let i = aStack.length; i--;) {
    if (aStack[i] === a) return bStack[i] === b;
    if (bStack[i] === b) return false;
  }
  aStack.push(a);
  bStack.push(b);
  if (cls === '[object Array]' && a.length !== b.length) return false;
  if (__isError(a) && __isError(b)) {
    try {
      let r = Object.prototype.toString.call(a) === Object.prototype.toString.call(b) && a.name === b.name && a.message === b.message;
      if (typeof b.cause !== 'undefined') r = r && __eq(a.cause, b.cause, aStack, bStack, testers, hasKey);
      if (a instanceof AggregateError && b instanceof AggregateError) r = r && __eq(a.errors, b.errors, aStack, bStack, testers, hasKey);
      return r && __eq({ ...a }, { ...b }, aStack, bStack, testers, hasKey);
    } finally {
      aStack.pop();
      bStack.pop();
    }
  }
  const aKeys = __eqKeys(a, hasKey);
  if (__eqKeys(b, hasKey).length !== aKeys.length) return false;
  for (let i = aKeys.length; i--;) {
    const k = aKeys[i];
    if (!(hasKey(b, k) && __eq(a[k], b[k], aStack, bStack, testers, hasKey))) return false;
  }
  aStack.pop();
  bStack.pop();
  return true;
}
// vitest iterableEquality: two non-array iterables of the same constructor — Sets by members and
// Maps by entries (in any order, deep-equal members match), others in iteration order plus their
// enumerable own entries.
function __iterableEquality(a, b, testers = [], aStack = [], bStack = []) {
  if (typeof a !== 'object' || typeof b !== 'object' || Array.isArray(a) || Array.isArray(b) ||
    a == null || b == null || !a[Symbol.iterator] || !b[Symbol.iterator]) return undefined;
  if (a.constructor !== b.constructor) return false;
  for (let i = aStack.length; i--;) if (aStack[i] === a) return bStack[i] === b;
  aStack.push(a);
  bStack.push(b);
  const withStack = (x, y) => __iterableEquality(x, y, [...testers], [...aStack], [...bStack]);
  const inner = [...testers.filter((t) => t !== __iterableEquality), withStack];
  const isA = (t, v) => Object.prototype.toString.call(v) === `[object ${t}]`;
  if (a.size !== undefined) {
    if (a.size !== b.size) return false;
    if (isA('Set', a)) {
      let all = true;
      for (const av of a) {
        if (!b.has(av)) {
          let has = false;
          for (const bv of b) if (__equals(av, bv, inner) === true) has = true;
          if (!has) { all = false; break; }
        }
      }
      aStack.pop(); bStack.pop();
      return all;
    }
    if (isA('Map', a)) {
      let all = true;
      for (const [ak, av] of a) {
        if (!b.has(ak) || !__equals(av, b.get(ak), inner)) {
          let has = false;
          for (const [bk, bv] of b) if (__equals(ak, bk, inner) === true && __equals(av, bv, inner) === true) has = true;
          if (!has) { all = false; break; }
        }
      }
      aStack.pop(); bStack.pop();
      return all;
    }
  }
  const bi = b[Symbol.iterator]();
  for (const av of a) {
    const nb = bi.next();
    if (nb.done || !__equals(av, nb.value, inner)) return false;
  }
  if (!bi.next().done) return false;
  if (!__equals(Object.entries(a), Object.entries(b), inner)) return false;
  aStack.pop();
  bStack.pop();
  return true;
}
// vitest typeEquality: toStrictEqual tells apart values whose `constructor` differs (a class
// instance never strictly equals a plain object; turbo-test#98).
function __typeEquality(a, b) {
  return a == null || b == null || a.constructor === b.constructor ? undefined : false;
}
// vitest sparseArrayEquality: a hole is not an `undefined` element under toStrictEqual.
function __sparseArrayEquality(a, b, testers = []) {
  if (!Array.isArray(a) || !Array.isArray(b)) return undefined;
  return __equals(a, b, testers.filter((t) => t !== __sparseArrayEquality), true) && __equals(Object.keys(a), Object.keys(b));
}
// vitest arrayBufferEquality: ArrayBuffers / DataViews by their bytes.
function __arrayBufferEquality(a, b) {
  let x = a, y = b;
  if (!(a instanceof DataView && b instanceof DataView)) {
    if (!(a instanceof ArrayBuffer) || !(b instanceof ArrayBuffer)) return undefined;
    try { x = new DataView(a); y = new DataView(b); } catch { return undefined; }
  }
  if (x.byteLength !== y.byteLength) return false;
  for (let i = 0; i < x.byteLength; i++) if (x.getUint8(i) !== y.getUint8(i)) return false;
  return true;
}
// vitest subsetEquality (toMatchObject, snapshot property matchers): every key of an object
// subset is a property of the object (own or inherited, short of Object.prototype) whose value
// matches, recursively.
function __subsetEquality(object, subset, testers = []) {
  const inner = testers.filter((t) => t !== __subsetEquality);
  const inObj = (o, k) => !!o && typeof o === 'object' && o !== Object.prototype && (__hasKey(o, k) || inObj(Object.getPrototypeOf(o), k));
  const withSeen = (seen = new WeakMap()) => (obj, sub) => {
    if (!(sub !== null && typeof sub === 'object' && !__isError(sub) && !Array.isArray(sub) &&
      !(sub instanceof Date) && !(sub instanceof Set) && !(sub instanceof Map))) return undefined;
    return Object.keys(sub).every((k) => {
      if (sub[k] != null && typeof sub[k] === 'object') {
        if (seen.has(sub[k])) return __equals(obj[k], sub[k], inner);
        seen.set(sub[k], true);
      }
      const r = obj != null && inObj(obj, k) && __equals(obj[k], sub[k], [...inner, withSeen(seen)]);
      seen.delete(sub[k]);
      return r;
    });
  };
  return withSeen()(object, subset);
}
// toEqual (strict=false) / toStrictEqual (strict=true) with vitest's testers for each.
// toEqual / toStrictEqual's testers: expect.addEqualityTesters' first (vitest's customTesters)
function deepEqual(a, b, strict = false) {
  return strict
    ? __equals(a, b, [...__customTesters, __iterableEquality, __typeEquality, __sparseArrayEquality, __arrayBufferEquality], true)
    : __equals(a, b, [...__customTesters, __iterableEquality]);
}
// expect.addEqualityTesters (turbo-test#155): every equality matcher and asymmetric matcher tries
// them first, as vitest's getCustomEqualityTesters()
const __customTesters = [];


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
  // the test running now (its hooks included), null between tests: expect.poll's
  currentTest: null,
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
// expect.addSnapshotSerializer's serializers come first, the newest first (turbo-test#162).
function __serialize(v) {
  return __pf(v, { indent: 2, basic: false, escape: false, fnName: false, escapeRegex: true, mocks: true, plugins: __snapSerializers }).replace(/\r\n|\r/g, '\n');
}
const __snapSerializers = [];
// the config's snapshotSerializers (turbo-test#167), as vitest's loadSnapshotSerializers: each
// file's default export, checked, then added in order (a file once per worker)
const __snapSerializerFiles = new Set();
globalThis.__ttLoadSnapshotSerializers = (files) => {
  const loaded = files.filter((file) => !__snapSerializerFiles.has(file)).map((file) => {
    const mo = globalThis.__ttToESM(globalThis.__ttImportActual('/', file));
    if (!mo || typeof mo.default !== 'object' || mo.default === null) throw new Error(`invalid snapshot serializer file ${file}. Must export a default object`);
    const config = mo.default;
    if (typeof config.test !== 'function' || (typeof config.serialize !== 'function' && typeof config.print !== 'function')) throw new TypeError(`invalid snapshot serializer in ${file}. Must have a 'test' method along with either a 'serialize' or 'print' method.`);
    return [file, config];
  });
  for (const [file, config] of loaded) { __snapSerializerFiles.add(file); __snapSerializers.unshift(config); }
};
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
  const isObj = (x) => x != null && typeof x === 'object' && !Array.isArray(x) && !__isAsym(x);
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
  st.data = st.exists ? __parseSnapFile(globalThis.__fs_readFile(sf.path, true)) : Object.create(null);
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
  const code = globalThis.__fs_readFile(file, true);
  const starts = [0];
  for (let i = 0; i < code.length; i++) if (code[i] === '\n') starts.push(i + 1);
  // UTF-16 columns are string indices in JS
  const splices = writes
    .map((w) => __inlineSplice(code, starts[w.line - 1] + w.column - 1, w.snapshot, w.name))
    .filter(Boolean)
    .sort((x, y) => y[0] - x[0]);
  let out = code;
  for (const [a, b, text] of splices) out = out.slice(0, a) + text + out.slice(b);
  if (out !== code) globalThis.__fs_writeFile(file, out, false);
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
      globalThis.__fs_mkdir(st.sf.dir, true);
      globalThis.__fs_writeFile(st.sf.path, __serializeSnapFile(st.data), false);
    }
  } else if (!external && st.exists) {
    if (mode === 'all') globalThis.__fs_remove(st.sf.path, 'unlink');
    deleted = true;
  }
  return {
    added: st.added, matched: st.matched, unmatched: st.unmatched, updated: st.updated,
    unchecked: deleted ? 0 : uncheckedKeys.length, unchecked_keys: uncheckedKeys, file_deleted: deleted,
  };
}

// ---- expect ----
// The jest-style matchers of one call on a chai Assertion (__attachMatcher): `actual`, `negated`,
// `message` and `promise` are the assertion's object / negate / message / promise flags.
function makeExpect(actual, negated = false, message = undefined, promise = '') {
  // `msg` is a thunk: the failure message (and the formatting of the operands in it) is built
  // only when the assertion fails, as vitest does — formatting can throw (a native class
  // prototype whose members check their receiver) and must not fail a passing assertion.
  const ok = (pass, msg) => {
    if (negated ? pass : !pass) throw new __ChaiAssertionError(msg());
  };
  // chai's assert with vitest's templates (turbo-test#71)
  const c = (pass, msg, notMsg, exp, act = actual) => __chaiAssert(negated, pass, msg, notMsg, actual, exp, act);
  const cn = (pass, msg, notMsg, exp, act = actual) => __chaiAssert(negated, pass, msg, notMsg, actual, exp, act, false);
  const spyOf = () => {
    if (!(typeof actual === 'function' && actual._isMockFunction === true)) throw new TypeError(`${__inspect(actual)} is not a spy or a call to a spy!`);
    return actual;
  };
  const numbers = (x, label) => {
    if (typeof x !== 'number' && typeof x !== 'bigint') throw new TypeError(`${label} value must be number or bigint, received "${typeof x}"`);
  };
  const argsEqual = (a, b) => a.length === b.length && a.every((x, i) => deepEqual(x, b[i]));
  const testers = __customTesters;
  const api = {
    toBeTypeOf(t) { c(typeof actual === t, 'expected #{this} to be type of #{exp}', 'expected #{this} not to be type of #{exp}', t, typeof actual); },
    toHaveBeenCalledExactlyOnceWith(...a) {
      const spy = spyOf();
      const name = spy.getMockName();
      const pass = spy.mock.calls.length === 1 && spy.mock.calls.some((x) => argsEqual(x, a));
      if (negated ? !pass : pass) return;
      const msg = __chaiMsg(negated ? `expected "${name}" to not be called once with arguments: #{exp}` : `expected "${name}" to be called once with arguments: #{exp}`, actual, a, actual);
      throw new __ChaiAssertionError(__formatCalls(spy, msg, a));
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
    toBeGreaterThan(n) { numbers(actual, 'actual'); numbers(n, 'expected'); cn(actual > n, `expected ${actual} to be greater than ${n}`, `expected ${actual} to be not greater than ${n}`, n); },
    toBeGreaterThanOrEqual(n) { numbers(actual, 'actual'); numbers(n, 'expected'); cn(actual >= n, `expected ${actual} to be greater than or equal to ${n}`, `expected ${actual} to be not greater than or equal to ${n}`, n); },
    toBeLessThan(n) { numbers(actual, 'actual'); numbers(n, 'expected'); cn(actual < n, `expected ${actual} to be less than ${n}`, `expected ${actual} to be not less than ${n}`, n); },
    toBeLessThanOrEqual(n) { numbers(actual, 'actual'); numbers(n, 'expected'); cn(actual <= n, `expected ${actual} to be less than or equal to ${n}`, `expected ${actual} to be not less than or equal to ${n}`, n); },
    toBeCloseTo(n, d = 2) {
      let pass = false, expectedDiff = 0, receivedDiff = 0;
      if ((n === Infinity && actual === Infinity) || (n === -Infinity && actual === -Infinity)) pass = true;
      else { expectedDiff = 10 ** -d / 2; receivedDiff = Math.abs(actual - n); pass = receivedDiff < expectedDiff; }
      cn(pass, `expected #{this} to be close to #{exp}, received difference is ${receivedDiff}, but expected ${expectedDiff}`, `expected #{this} to not be close to #{exp}, received difference is ${receivedDiff}, but expected ${expectedDiff}`, n);
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
    toContainEqual(x) { c(actual != null && Array.from(actual).some((a) => __equals(a, x, testers)), 'expected #{this} to deep equally contain #{exp}', 'expected #{this} to not deep equally contain #{exp}', x); },
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
      // the diff's actual is vitest's getObjectSubset: the received value cut down to what the
      // expected object names (turbo-test#152)
      const { subset, stripped: n } = __objectSubset(actual, obj);
      const msg = __chaiMsg(negated ? 'expected #{this} to not match object #{exp}' : 'expected #{this} to match object #{exp}', actual, obj, subset);
      throw new __ChaiAssertionError(n === 0 ? msg : `${msg}\n(${n} matching ${n === 1 ? 'property' : 'properties'} omitted from actual)`, { showDiff: true, expected: obj, actual: subset });
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
        const expected = exists ? globalThis.__fs_readFile(path, true).replace(/\r\n/g, '\n') : undefined;
        const pass = expected === body;
        const write = () => {
          const slash = path.lastIndexOf('/');
          if (slash > 0) globalThis.__fs_mkdir(path.slice(0, slash), true);
          globalThis.__fs_writeFile(path, body, false);
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
      const pass = arguments.length < 2 ? present : (present && __equals(value, cur, testers));
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
      // an Error or any other value: vitest's `equals(thrown, expected, [iterableEquality])`
      // (Errors by name / message / cause / own props; a plain object never equals an Error)
      if (__isError(expected)) {
        return c(threw && __equals(err, expected, [...testers, __iterableEquality]), 'expected a thrown error to be #{exp}', 'expected a thrown error not to be #{exp}', expected, err);
      }
      if (__isAsym(expected)) {
        return c(threw && expected.asymmetricMatch(err), 'expected error to match asymmetric matcher', 'expected error not to match asymmetric matcher', expected, err);
      }
      c(threw && __equals(err, expected, [...testers, __iterableEquality]), 'expected a thrown value to equal #{exp}', 'expected a thrown value not to equal #{exp}', expected, err);
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
      cn(n === times, `expected "${spy.getMockName()}" to be called #{exp} times, but got ${n} times`, `expected "${spy.getMockName()}" to not be called #{exp} times`, times, n);
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
      cn(n === 1, `expected "${spy.getMockName()}" to be called once, but got ${n} times`, `expected "${spy.getMockName()}" to not be called once`, 1, n);
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
    // vitest's return / resolve families (turbo-test#155 surface): `results` for return,
    // `settledResults` for resolve
    toHaveReturned() { returnedOnce(spyOf().mock.results, (r) => r.type !== 'throw', 'called'); },
    toHaveResolved() { returnedOnce(spyOf().mock.settledResults, (r) => r.type === 'fulfilled', 'resolved', true); },
    toHaveReturnedTimes(times) { returnedTimes(spyOf().mock.results.reduce((n, r) => (r.type === 'throw' ? n : n + 1), 0), times, 'called'); },
    toHaveResolvedTimes(times) { returnedTimes(spyOf().mock.settledResults.reduce((n, r) => (r.type === 'fulfilled' ? n + 1 : n), 0), times, 'resolved'); },
    toHaveReturnedWith(v) { returnedWith(spyOf(), v, 'return', 'return'); },
    toHaveResolvedWith(v) { returnedWith(spyOf(), v, 'resolve', 'fulfilled'); },
    toHaveLastReturnedWith(v) { lastReturned(spyOf().mock.results, v, 'return', 'return'); },
    toHaveLastResolvedWith(v) { lastReturned(spyOf().mock.settledResults, v, 'resolve', 'fulfilled'); },
    toHaveNthReturnedWith(nth, v) { nthReturned(spyOf().mock.results, nth, v, 'return', 'return'); },
    toHaveNthResolvedWith(nth, v) { nthReturned(spyOf().mock.settledResults, nth, v, 'resolve', 'fulfilled'); },
    toHaveBeenCalledBefore(other, failIfNoFirstInvocation = true) { calledOrder(other, failIfNoFirstInvocation, 'before'); },
    toHaveBeenCalledAfter(other, failIfNoFirstInvocation = true) { calledOrder(other, failIfNoFirstInvocation, 'after'); },
    toBeNullable() { c(actual == null, 'expected #{this} to be nullish', 'expected #{this} not to be nullish', null); },
  };
  function returnedOnce(results, ok, action, settled) {
    const spy = actual;
    const pass = settled ? results.some(ok) : spy.mock.calls.length > 0 && results.some(ok);
    cn(pass, `expected "${spy.getMockName()}" to be successfully ${action} at least once`, `expected "${spy.getMockName()}" to not be successfully ${action}`, pass, !pass);
  }
  function returnedTimes(n, times, action) {
    const name = actual.getMockName();
    const pass = n === times;
    cn(pass, `expected "${name}" to be successfully ${action} ${times} times`, `expected "${name}" to not be successfully ${action} ${times} times`, `expected resolved times: ${times}`, `received resolved times: ${pass}`);
  }
  function returnedWith(spy, v, action, type) {
    const results = type === 'return' ? spy.mock.results : spy.mock.settledResults;
    const pass = results.some((r) => r.type === type && __equals(v, r.value));
    if (negated ? !pass : pass) return;
    const name = spy.getMockName();
    const msg = __chaiMsg(negated ? `expected "${name}" to not ${action} with: #{exp}` : `expected "${name}" to ${action} with: #{exp} at least once`, actual, v, actual);
    throw new __ChaiAssertionError(__formatReturns(spy, msg, v, results));
  }
  function lastReturned(results, v, action, type) {
    const r = results.at(-1);
    const name = actual.getMockName();
    c(Boolean(r && r.type === type && __equals(r.value, v)), `expected last "${name}" call to ${action} #{exp}`, `expected last "${name}" call to not ${action} #{exp}`, v, r?.value);
  }
  function nthReturned(results, nth, v, action, type) {
    const r = results[nth - 1];
    const name = actual.getMockName();
    const call = `${__ordinal(nth)} call`;
    c(!!(r && r.type === type && __equals(r.value, v)), `expected ${call} "${name}" call to ${action} #{exp}`, `expected ${call} "${name}" call to not ${action} #{exp}`, v, r?.value);
  }
  function calledOrder(other, failIfNoFirstInvocation, which) {
    const spy = spyOf();
    if (!(typeof other === 'function' && other._isMockFunction === true)) throw new TypeError(`${__inspect(other)} is not a spy or a call to a spy`);
    const [before, after] = which === 'before' ? [spy, other] : [other, spy];
    const b = before.mock.invocationCallOrder, a = after.mock.invocationCallOrder;
    const pass = b.length === 0 ? !failIfNoFirstInvocation : a.length === 0 ? false : b[0] < a[0];
    c(pass, `expected "${spy.getMockName()}" to have been called ${which} "${other.getMockName()}"`, `expected "${spy.getMockName()}" to not have been called ${which} "${other.getMockName()}"`, other, spy);
  }
  // vitest's aliases
  api.toBeCalled = api.toHaveBeenCalled;
  api.toBeCalledTimes = api.toHaveBeenCalledTimes;
  api.toBeCalledWith = api.toHaveBeenCalledWith;
  api.toReturn = api.toHaveReturned;
  api.toReturnTimes = api.toHaveReturnedTimes;
  api.toReturnWith = api.toHaveReturnedWith;
  // jest-style matchers — vitest's built-in toSatisfy / toBeOneOf and expect.extend's (e.g.
  // @testing-library/jest-dom) — run with vitest's matcher state as `this` and fail with its
  // JestExtendError (`message: ` already prefixed); an async result is awaited
  // (`message` is the result's own name, so a non-function throws vitest's `message is not a
  // function`)
  const customMessage = message;
  const jestMatcher = (matcher) => (...args) => {
    const result = matcher.call(__matcherState(negated || undefined, promise), actual, ...args);
    const check = ({ pass, message, actual: a, expected: e }) => {
      if ((pass && negated) || (!pass && !negated)) throw new __JestExtendError((customMessage ? `${customMessage}: ` : '') + message(), a, e);
    };
    if (result && typeof result === 'object' && typeof result.then === 'function') return result.then(check);
    check(result);
  };
  for (const name of Object.keys(__jestMatchers)) api[name] = jestMatcher(__jestMatchers[name]);
  for (const name of Object.keys(__customMatchers)) api[name] = jestMatcher(__customMatchers[name]);
  // expect(value, message): vitest prefixes `message: ` to a matcher's failure (turbo-test#138)
  if (message) {
    for (const name of Object.keys(api)) {
      const d = Object.getOwnPropertyDescriptor(api, name);
      if (typeof d.value !== 'function') continue;
      const matcher = d.value;
      api[name] = (...args) => {
        let r;
        try { r = matcher(...args); } catch (e) { throw __withExpectMessage(e, message); }
        return r && typeof r.then === 'function' ? r.then(undefined, (e) => { throw __withExpectMessage(e, message); }) : r;
      };
    }
  }
  return api;
}
// vitest's JestExtendError: a jest-style matcher's failure (name 'Error')
class __JestExtendError extends Error {
  constructor(message, actual, expected) { super(message); this.actual = actual; this.expected = expected; }
}
// `message: ` before the message (and the stack's first line) of an assertion failure: chai's
// AssertionError or a jest-style matcher's plain Error, once; any other error (a TypeError from
// a non-spy, ...) passes through
const __prefixedErrors = new WeakSet();
function __withExpectMessage(e, message) {
  if (!(e instanceof __ChaiAssertionError || (e instanceof Error && e.constructor === Error)) || __prefixedErrors.has(e)) return e;
  __prefixedErrors.add(e);
  const before = `${e.name}: ${e.message}`;
  e.message = `${message}: ${e.message}`;
  if (typeof e.stack === 'string' && e.stack.startsWith(before)) e.stack = `${e.name}: ${e.message}` + e.stack.slice(before.length);
  return e;
}
// a custom matcher's `this.utils`: vitest's getMatcherUtils() with diff / stringify /
// iterableEquality / subsetEquality (turbo-test#163), coloured as tinyrainbow colours them
const __bold = __tr('\x1b[1m', '\x1b[22m', '\x1b[22m\x1b[1m');
// @vitest/expect matcherHint
function __matcherHint(matcherName, received = 'received', expected = 'expected', options = {}) {
  const { comment = '', isDirectExpectCall = false, isNot = false, promise = '', secondArgument = '', expectedColor = __dcol.green, receivedColor = __dcol.red, secondArgumentColor = __dcol.green } = options;
  const dim = __dcol.dim;
  let hint = '';
  let dimString = 'expect';
  if (!isDirectExpectCall && received !== '') { hint += dim(`${dimString}(`) + receivedColor(received); dimString = ')'; }
  if (promise !== '') { hint += dim(`${dimString}.`) + promise; dimString = ''; }
  if (isNot) { hint += `${dim(`${dimString}.`)}not`; dimString = ''; }
  if (matcherName.includes('.')) dimString += matcherName;
  else { hint += dim(`${dimString}.`) + matcherName; dimString = ''; }
  if (expected === '') dimString += '()';
  else {
    hint += dim(`${dimString}(`) + expectedColor(expected);
    if (secondArgument) hint += dim(', ') + secondArgumentColor(secondArgument);
    dimString = ')';
  }
  if (comment !== '') dimString += ` // ${comment}`;
  if (dimString !== '') hint += dim(dimString);
  return hint;
}
// trailing spaces of each line as `·` (printReceived / printExpected)
const __trailingDots = (text) => text.replace(/\s+$/gm, (spaces) => '·'.repeat(spaces.length));
const __matcherUtils = {
  EXPECTED_COLOR: (s) => __dcol.green(s),
  RECEIVED_COLOR: (s) => __dcol.red(s),
  INVERTED_COLOR: (s) => __dcol.inverse(s),
  BOLD_WEIGHT: (s) => __bold(s),
  DIM_COLOR: (s) => __dcol.dim(s),
  // @vitest/utils diff(a, b, options): its omitAnnotationLines and a/bAnnotation
  diff: (a, b, options) => {
    const keep = __diffAnno;
    __diffAnno = [options?.aAnnotation ?? 'Expected', options?.bAnnotation ?? 'Received'];
    try { return __vdiff(a, b, !options?.omitAnnotationLines); } finally { __diffAnno = keep; }
  },
  matcherHint: __matcherHint,
  printReceived: (object) => __dcol.red(__trailingDots(__vstringify(object))),
  printExpected: (value) => __dcol.green(__trailingDots(__vstringify(value))),
  printDiffOrStringify: (received, expected) => __printDiffOrStringify(received, expected),
  printWithType: (name, value, print) => {
    const type = Object.prototype.toString.call(value).slice(8, -1);
    const hasType = type !== 'null' && type !== 'undefined' ? `${name} has type:  ${type}\n` : '';
    return `${hasType}${name} has value: ${print(value)}`;
  },
  stringify: (object, maxDepth, options) => __vstringify(object, maxDepth, options),
};
const __customMatchers = {};
// the `this` of a jest-style matcher (vitest's getMatcherState) and of an asymmetric matcher
// (getMatcherContext)
function __matcherUtilsFull() {
  return { ...__matcherUtils, iterableEquality: __iterableEquality, subsetEquality: __subsetEquality };
}
function __matcherState(isNot, promise) {
  return {
    ...globalThis.expect.getState(),
    currentTestName: __snap.testName,
    customTesters: __customTesters,
    isNot,
    utils: __matcherUtilsFull(),
    promise,
    equals: __equals,
    suppressedErrors: [],
    soft: undefined,
    poll: undefined,
  };
}
// ---- chai: vitest's expect(x) is a chai Assertion (turbo-test#160) ----
// The runner puts chai 6.3.0 (this package's dependency) into the snapshot as __ttLoadChai
// (runner.rs chai_script), loaded here. As vitest's JestChaiExpect does, every jest-style matcher is a method of
// chai's Assertion, run on the assertion's flags; chai brings its BDD chain (`to`, `be`, `deep`,
// `equal`, `include`, ...), its proxy (`Invalid Chai property`), `assert` and `should`.
const __chai = globalThis.__ttLoadChai && globalThis.__ttLoadChai();
delete globalThis.__ttLoadChai;
if (!__chai || typeof __chai.Assertion !== 'function') throw new Error('turbo-test: chai is not loaded');
const __cu = __chai.util;
// the `vitest` module's assert / chai / should exports (runner.rs vitest_exports)
Object.defineProperty(globalThis, '__ttVitestChai', { value: __chai });
Object.defineProperty(globalThis, '__ttVitestAssert', { value: __chai.assert });
Object.defineProperty(globalThis, '__ttVitestShould', { value: __chai.should });
function __attachMatcher(name) {
  __cu.addMethod(__chai.Assertion.prototype, name, function (...args) {
    const obj = __cu.flag(this, 'object');
    const promise = __cu.flag(this, 'promise') || '';
    // `.rejects.toThrow()` checks the rejection as a thrown error (vitest wraps it in a thrower)
    const actual = promise === 'rejects' && (name === 'toThrow' || name === 'toThrowError') ? () => { throw obj; } : obj;
    return makeExpect(actual, !!__cu.flag(this, 'negate'), __cu.flag(this, 'message'), promise)[name](...args);
  });
}
for (const name of Object.keys(makeExpect(undefined))) __attachMatcher(name);
// vitest's chai-style matchSnapshot and its sinon-chai-like members (ChaiStyleAssertions)
__cu.addMethod(__chai.Assertion.prototype, 'matchSnapshot', function (...args) { return __chai.Assertion.prototype.toMatchSnapshot.call(this, ...args); });
for (const [name, to, ...fixed] of [['called', 'toHaveBeenCalled'], ['calledOnce', 'toHaveBeenCalledOnce'], ['calledTwice', 'toHaveBeenCalledTimes', 2], ['calledThrice', 'toHaveBeenCalledTimes', 3]]) {
  __cu.addProperty(__chai.Assertion.prototype, name, function () { return __chai.Assertion.prototype[to].call(this, ...fixed); });
}
for (const [name, to] of [
  ['callCount', 'toHaveBeenCalledTimes'], ['calledWith', 'toHaveBeenCalledWith'], ['calledOnceWith', 'toHaveBeenCalledExactlyOnceWith'],
  ['lastCalledWith', 'toHaveBeenLastCalledWith'], ['nthCalledWith', 'toHaveBeenNthCalledWith'], ['returned', 'toHaveReturned'],
  ['returnedWith', 'toHaveReturnedWith'], ['returnedTimes', 'toHaveReturnedTimes'], ['lastReturnedWith', 'toHaveLastReturnedWith'],
  ['nthReturnedWith', 'toHaveNthReturnedWith'], ['calledBefore', 'toHaveBeenCalledBefore'], ['calledAfter', 'toHaveBeenCalledAfter'],
]) {
  __cu.addMethod(__chai.Assertion.prototype, name, function (...args) { return __chai.Assertion.prototype[to].call(this, ...args); });
}
// vitest's internal withTest / withContext (flags)
__cu.addMethod(__chai.Assertion.prototype, 'withTest', function (test) { __cu.flag(this, 'vitest-test', test); return this; });
__cu.addMethod(__chai.Assertion.prototype, 'withContext', function (context) { for (const key in context) __cu.flag(this, key, context[key]); return this; });
// vitest's chai throw / throws / Throw under .resolves / .rejects
for (const m of ['throw', 'throws', 'Throw']) {
  __cu.overwriteMethod(__chai.Assertion.prototype, m, (_super) => function (...args) {
    const promise = __cu.flag(this, 'promise');
    const object = __cu.flag(this, 'object');
    const isNot = __cu.flag(this, 'negate');
    if (promise === 'rejects') {
      __cu.flag(this, 'object', () => { throw object; });
    } else if (promise === 'resolves' && typeof object !== 'function') {
      if (!isNot) throw new __chai.AssertionError(__cu.flag(this, 'message') || "expected promise to throw an error, but it didn't", { showDiff: false }, __cu.flag(this, 'ssfi'));
      return;
    }
    _super.apply(this, args);
  });
}
// async matchers: `await expect(promise).resolves.toX(...)` / `.rejects.toX(...)`, as vitest's: a
// proxy over the assertion whose methods run on the settled value (`.rejects` calls a function
// first); anything but a thenable is vitest's TypeError, thrown when the property is read. A
// failure's stack is the one taken where `.resolves` / `.rejects` was read.
function __asyncProperty(kind) {
  __cu.addProperty(__chai.Assertion.prototype, kind, function () {
    const error = new Error(kind);
    __cu.flag(this, 'promise', kind);
    __cu.flag(this, 'error', error);
    const obj = __cu.flag(this, 'object');
    const wrapper = kind === 'rejects' && typeof obj === 'function' ? obj() : obj;
    if (typeof wrapper?.then !== 'function') throw new TypeError(`You must provide a Promise to expect() when using .${kind}, not '${typeof wrapper}'.`);
    const proxy = new Proxy(this, { get: (target, key, receiver) => {
      const result = Reflect.get(target, key, receiver);
      if (typeof result !== 'function') return result instanceof __chai.Assertion ? proxy : result;
      return (...args) => {
        __cu.flag(this, '_name', key);
        return Promise.resolve(wrapper).then(
          kind === 'resolves'
            ? (value) => { __cu.flag(this, 'object', value); return result.call(this, ...args); }
            : (value) => { throw new __chai.AssertionError(`promise resolved "${__cu.inspect(value)}" instead of rejecting`, { showDiff: true, expected: new Error('rejected promise'), actual: value }); },
          kind === 'resolves'
            ? (err) => { const e = new __chai.AssertionError(`promise rejected "${__cu.inspect(err)}" instead of resolving`, { showDiff: false }); e.cause = err; throw e; }
            : (err) => { __cu.flag(this, 'object', err); return result.call(this, ...args); },
        ).catch((err) => {
          if (err instanceof Error && error.stack) err.stack = error.stack.replace(error.message, err.message);
          throw err;
        });
      };
    } });
    return proxy;
  });
}
__asyncProperty('resolves');
__asyncProperty('rejects');
globalThis.expect = (actual, message) => { __snap.assertCount++; return __chai.expect(actual, message); };
// chai.expect's own statics (vitest: Object.assign(expect, chai.expect)) — `fail`
Object.assign(globalThis.expect, __chai.expect);
globalThis.expect.assert = __chai.assert;
// expect.extend: the matchers, and their asymmetric expect.<name> / expect.not.<name> (#153)
globalThis.expect.extend = (matchers) => {
  for (const [name, matcher] of Object.entries(matchers || {})) { __customMatchers[name] = matcher; __attachMatcher(name); __addAsymmetric(name, matcher); }
};
// expect.addEqualityTesters (turbo-test#155)
globalThis.expect.addEqualityTesters = (testers) => {
  if (!Array.isArray(testers)) throw new TypeError(`expect.customEqualityTesters: Must be set to an array of Testers. Was given "${__getType(testers)}"`);
  __customTesters.push(...testers);
};
// vitest's expect.unreachable: chai's assert.fail
globalThis.expect.unreachable = (msg) => { __chai.assert.fail(`expected${msg ? ` "${msg}" ` : ' '}not to be reached`); };
// vitest's matcher state (expect.getState / setState): the assertion counters of the running test
// live in __snap; anything else set is kept as is
const __extraState = {};
globalThis.expect.getState = () => ({
  assertionCalls: __snap.assertCount,
  isExpectingAssertions: !!__snap.hasAssertions,
  isExpectingAssertionsError: __snap.hasAssertions || null,
  expectedAssertionsNumber: __snap.expectedAssertions,
  expectedAssertionsNumberErrorGen: __snap.expectedAssertions == null ? null : () => new Error(`expected number of assertions to be ${__snap.expectedAssertions}, but got ${__snap.assertCount}`),
  currentTestName: __snap.testName,
  testPath: globalThis.__ttFile,
  environment: globalThis.__ttEnvName || 'node',
  snapshotState: __snapState,
  ...__extraState,
});
globalThis.expect.setState = (state) => {
  for (const [k, v] of Object.entries(state || {})) {
    if (k === 'assertionCalls') __snap.assertCount = v;
    else if (k === 'expectedAssertionsNumber') __snap.expectedAssertions = v;
    else if (k === 'isExpectingAssertionsError') __snap.hasAssertions = v || false;
    else __extraState[k] = v;
  }
};
// expect.assertions(n) / expect.hasAssertions(): record the expectation; runSuite verifies the
// count at test end (see __snap.assertCount, reset per test). These calls do NOT count themselves.
globalThis.expect.assertions = (n) => { __snap.expectedAssertions = n; };
// hasAssertions keeps vitest's error, made where it is called (its frame is that call site).
globalThis.expect.hasAssertions = function hasAssertions() {
  const error = new Error('expected any number of assertion, but got none');
  if (Error.captureStackTrace) Error.captureStackTrace(error, hasAssertions);
  __snap.hasAssertions = error;
};
// vitest's expect.poll(fn, { interval = 50, timeout = 1000, message }) (turbo-test#161): each
// matcher re-runs fn until it passes or the timeout ran out, then throws the last error (its
// cause `Matcher did not succeed in time.`). Its interval and timeout are internal timers on the
// real (virtual) clock, which the test's fake timers do not see; while the timers are faked,
// each retry then also advances the fake clock by the interval, as vitest's real-time wait
// followed by vi.advanceTimersByTime(interval). A matcher that is not awaited never runs and
// fails the test when it is done. As vitest's, a proxy over one chai Assertion (`expect(null)`)
// whose matchers run on fn's value as the assertion's `object`.
const __pollUnsupported = ['matchSnapshot', 'toMatchSnapshot', 'toMatchInlineSnapshot', 'toThrowErrorMatchingSnapshot', 'toThrowErrorMatchingInlineSnapshot', 'throws', 'Throw', 'throw', 'toThrow', 'toThrowError'];
globalThis.expect.poll = function poll(fn, options = {}) {
  const { interval = 50, timeout = 1000, message } = options;
  __snap.assertCount++;
  const test = __snap.currentTest;
  if (!test) throw new Error('expect.poll() must be called inside a test');
  const assertion = __chai.expect(null, message);
  fn = fn.bind(assertion);
  const proxy = new Proxy(assertion, { get(target, key, receiver) {
    if (key === 'resolves' || key === 'rejects') throw new SyntaxError(`expect.poll() is not supported in combination with .${key}`);
    const matcher = Reflect.get(target, key, receiver);
    if (typeof matcher !== 'function') return matcher instanceof __chai.Assertion ? proxy : matcher;
    if (key === 'assert') return matcher;
    if (typeof key === 'string' && __pollUnsupported.includes(key)) throw new SyntaxError(`expect.poll() is not supported in combination with .${key}(). Use vi.waitFor() if your assertion condition is unstable.`);
    return function __VITEST_POLL_CHAIN__(...args) {
      const STACK_TRACE_ERROR = new Error('STACK_TRACE_ERROR');
      const withStack = (err) => {
        if (err && typeof err === 'object' && STACK_TRACE_ERROR.stack !== undefined) err.stack = STACK_TRACE_ERROR.stack.replace(STACK_TRACE_ERROR.message, err.message);
        return err;
      };
      const run = async () => {
        let timedOut = false;
        const timer = __scheduleInternal(() => { timedOut = true; }, timeout);
        try {
          while (true) {
            const last = timedOut;
            try {
              const obj = await fn();
              __cu.flag(assertion, 'object', obj);
              return await matcher.call(assertion, ...args);
            } catch (err) {
              if (last) {
                if (err && typeof err === 'object' && err.cause == null) err.cause = new Error('Matcher did not succeed in time.');
                throw withStack(err);
              }
              await new Promise((resolve) => __scheduleInternal(resolve, interval));
              if (__loop.fake) __loop.advance(interval);
            }
          }
        } finally {
          __clearInternal(timer);
        }
      };
      let awaited = false;
      let result;
      test.onFinished.push(() => {
        if (awaited) return;
        const shown = `expect.poll(assertion).${__cu.flag(assertion, 'negate') ? 'not.' : ''}${String(key)}()`;
        throw withStack(new Error(`${shown} was not awaited. This assertion is asynchronous and must be awaited; otherwise, it is not executed to avoid unhandled rejections:\n\nawait ${shown}\n`));
      });
      return {
        then(onFulfilled, onRejected) { awaited = true; return (result ||= run()).then(onFulfilled, onRejected); },
        catch(onRejected) { awaited = true; return (result ||= run()).catch(onRejected); },
        finally(onFinally) { awaited = true; return (result ||= run()).finally(onFinally); },
        [Symbol.toStringTag]: 'Promise',
      };
    };
  } });
  return proxy;
};
// expect.addSnapshotSerializer: a pretty-format plugin every later snapshot uses (turbo-test#162)
globalThis.expect.addSnapshotSerializer = (plugin) => { __snapSerializers.unshift(plugin); };
// expect.addSnapshotSerializer: a pretty-format plugin every later snapshot uses (turbo-test#162),
// added with chai's addMethod as vitest adds it
__cu.addMethod(globalThis.expect, 'addSnapshotSerializer', (plugin) => { __snapSerializers.unshift(plugin); });
globalThis.expect.soft = (actual, message) => globalThis.expect(actual, message).withContext({ soft: true });
// vitest adds these with chai's addMethod (on chai.expect, then copied): called on `expect`, they
// leave chai's `__flags` on it
for (const [name, make] of Object.entries({
  anything: () => new __Anything(),
  any: (expected) => new __Any(expected),
  stringContaining: (expected) => new __StringContaining(expected),
  objectContaining: (expected) => new __ObjectContaining(expected),
  arrayContaining: (expected) => new __ArrayContaining(expected),
  stringMatching: (expected) => new __StringMatching(expected),
  closeTo: (expected, precision) => new __CloseTo(expected, precision),
  schemaMatching: (expected) => new __SchemaMatching(expected),
})) __cu.addMethod(globalThis.expect, name, make);
globalThis.expect.not = {
  stringContaining: (expected) => new __StringContaining(expected, true),
  objectContaining: (expected) => new __ObjectContaining(expected, true),
  arrayContaining: (expected) => new __ArrayContaining(expected, true),
  stringMatching: (expected) => new __StringMatching(expected, true),
  closeTo: (expected, precision) => new __CloseTo(expected, precision, true),
  schemaMatching: (expected) => new __SchemaMatching(expected, true),
};
// every jest-style matcher (built-in and expect.extend) gets expect.<name> / expect.not.<name>
function __addAsymmetric(name, matcher) {
  const CustomMatcher = __customAsymmetric(name, matcher);
  Object.defineProperty(globalThis.expect, name, { configurable: true, enumerable: true, writable: true, value: (...sample) => new CustomMatcher(false, ...sample) });
  Object.defineProperty(globalThis.expect.not, name, { configurable: true, enumerable: true, writable: true, value: (...sample) => new CustomMatcher(true, ...sample) });
}
for (const name of Object.keys(__jestMatchers)) __addAsymmetric(name, __jestMatchers[name]);

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
// vitest's invocationCallOrder: one counter for every mock in the worker, from 1
let __mockCallOrder = 0;
// A mock function as @vitest/spy 4.1.12 makes one (vi.fn, vi.spyOn, the automocker): its state
// is `fn.mock` ({ calls, contexts, instances, invocationCallOrder, settledResults, results,
// lastCall }), its implementation, once-queue and name live in a closure (no own internals), it is
// named after the original (`Mock` without one) with the implementation's length, and
// mockReturnValue & co. are implementations. `opts`: `original` (spyOn's method, called while no
// implementation is set), `restore` (spyOn's, run by mockRestore / vi.restoreAllMocks),
// `keepName` (spyOn: mockReset keeps the method's name).
const __mockRestores = new Set();
const __mockStatic = new Set(['length', 'name', 'prototype', Symbol.for('nodejs.util.promisify.custom')]);
function __mockDescriptor(o, k) {
  for (let p = o; p != null; p = Object.getPrototypeOf(p)) {
    const d = Object.getOwnPropertyDescriptor(p, k);
    if (d) return [p, d];
  }
  return undefined;
}
function __mockConstructorError(shorthand) {
  return new TypeError(`Cannot use \`${shorthand}\` when called with \`new\`. Use \`mockImplementation\` with a \`class\` keyword instead. See https://vitest.dev/api/mock#class-support for more information.`);
}
function makeSpy(impl, opts) {
  opts = opts || {};
  const original = opts.original;
  if (opts.restore) __mockRestores.add(opts.restore);
  const config = { impl: undefined, once: [], name: 'vi.fn()' };
  const state = {
    calls: [], contexts: [], instances: [], invocationCallOrder: [], settledResults: [], results: [],
    get lastCall() { return state.calls[state.calls.length - 1]; },
  };
  const name = (original && original.name) || 'Mock';
  const fn = { [name]: function (...args) {
    state.calls.push(args);
    state.invocationCallOrder.push(++__mockCallOrder);
    const result = { type: 'incomplete', value: undefined };
    // vitest's settledResults: 'incomplete' until the returned promise settles
    const settled = { type: 'incomplete', value: undefined };
    state.results.push(result);
    state.settledResults.push(settled);
    const context = new.target ? undefined : this;
    const at = state.instances.push(context);
    state.contexts.push(context);
    const implementation = config.once.shift() || config.impl || original || function () {};
    let ret;
    try {
      // `new spy()` constructs the implementation (a class cannot be called with .apply)
      ret = new.target ? Reflect.construct(implementation, args, new.target) : implementation.apply(this, args);
    } catch (e) {
      result.type = 'throw';
      result.value = e;
      settled.type = 'rejected';
      settled.value = e;
      if (e instanceof TypeError && String(e.message).includes('is not a constructor')) {
        globalThis.console.warn(`[vitest] The ${fn.getMockName()} mock did not use 'function' or 'class' in its implementation, see https://vitest.dev/api/vi#vi-spyon for examples.`);
      }
      throw e;
    }
    result.type = 'return';
    result.value = ret;
    if (new.target) {
      state.contexts[at - 1] = ret;
      state.instances[at - 1] = ret;
    }
    if (ret instanceof Promise) {
      ret.then((v) => { settled.type = 'fulfilled'; settled.value = v; }, (e) => { settled.type = 'rejected'; settled.value = e; });
    } else {
      settled.type = 'fulfilled';
      settled.value = ret;
    }
    return ret;
  } }[name];
  // the original's (or implementation's) own and inherited statics, as vitest copies them
  for (let src = original || impl; src && src !== Object.prototype && src !== Function.prototype; src = Object.getPrototypeOf(src)) {
    for (const k of [...Object.getOwnPropertyNames(src), ...Object.getOwnPropertySymbols(src)]) {
      if (__mockStatic.has(k) || __mockDescriptor(fn, k)) continue;
      try { Object.defineProperty(fn, k, Object.getOwnPropertyDescriptor(src, k)); } catch (e) {}
    }
  }
  Object.defineProperty(fn, 'length', { writable: true, enumerable: false, configurable: true, value: (impl || original) ? (impl || original).length : 0 });
  if (opts.keepName) config.name = fn.name || 'vi.fn()';
  fn._isMockFunction = true;
  // the implementation the next call runs (vitest: a queued once first)
  fn.getMockImplementation = () => config.once[0] || config.impl;
  Object.defineProperty(fn, 'mock', { configurable: false, enumerable: true, writable: false, value: state });
  fn.mockImplementation = (f) => { config.impl = f; return fn; };
  fn.mockImplementationOnce = (f) => { config.once.push(f); return fn; };
  // runs `cb` with `f` as the implementation (no once-queue), then puts both back; a promise
  // `cb` result restores when it settles and resolves to the mock
  fn.withImplementation = (f, cb) => {
    const prevImpl = config.impl;
    const prevOnce = config.once;
    const reset = () => { config.impl = prevImpl; config.once = prevOnce; };
    config.impl = f;
    config.once = [];
    const r = cb();
    if (typeof r === 'object' && r !== null && typeof r.then === 'function') return r.then(() => { reset(); return fn; });
    reset();
    return fn;
  };
  fn.mockReturnThis = () => fn.mockImplementation(function () { return this; });
  const value = (shorthand, make) => function () { if (new.target) throw __mockConstructorError(shorthand); return make(); };
  fn.mockReturnValue = (v) => fn.mockImplementation(value('mockReturnValue', () => v));
  fn.mockReturnValueOnce = (v) => fn.mockImplementationOnce(value('mockReturnValueOnce', () => v));
  fn.mockThrow = (v) => fn.mockImplementation(() => { throw v; });
  fn.mockThrowOnce = (v) => fn.mockImplementationOnce(() => { throw v; });
  fn.mockResolvedValue = (v) => fn.mockImplementation(value('mockResolvedValue', () => Promise.resolve(v)));
  fn.mockResolvedValueOnce = (v) => fn.mockImplementationOnce(value('mockResolvedValueOnce', () => Promise.resolve(v)));
  fn.mockRejectedValue = (v) => fn.mockImplementation(value('mockRejectedValue', () => Promise.reject(v)));
  fn.mockRejectedValueOnce = (v) => fn.mockImplementationOnce(value('mockRejectedValueOnce', () => Promise.reject(v)));
  fn.mockClear = () => {
    state.calls = [];
    state.contexts = [];
    state.instances = [];
    state.invocationCallOrder = [];
    state.results = [];
    state.settledResults = [];
    return fn;
  };
  // back to vi.fn(impl)'s implementation (a spy calls its original again)
  fn.mockReset = () => {
    fn.mockClear();
    config.impl = opts.resetTo;
    config.name = opts.keepName ? fn.name || 'vi.fn()' : 'vi.fn()';
    config.once = [];
    return fn;
  };
  fn.mockRestore = () => { fn.mockReset(); return opts.restore ? opts.restore() : undefined; };
  fn.mockName = (n) => { if (typeof n === 'string') config.name = n; return fn; };
  fn.getMockName = () => config.name || 'vi.fn()';
  if (Symbol.dispose) fn[Symbol.dispose] = () => fn.mockRestore();
  if (impl) fn.mockImplementation(impl);
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
function __automockProtoMethods(target, proto, spy) {
  for (let p = proto; p && !__isBuiltinProto(p); p = Object.getPrototypeOf(p)) {
    for (const k of Object.getOwnPropertyNames(p)) {
      if (k === 'constructor' || Object.prototype.hasOwnProperty.call(target, k)) continue;
      const d = Object.getOwnPropertyDescriptor(p, k);
      if (typeof d.value === 'function') { target[k] = __trackSpy(makeSpy(undefined, { original: spy ? d.value : undefined })); }
    }
  }
}
// A mock's names an ES import may name although the mock lacks them (the ESM graph's synthetic
// module exports them, undefined): an automock's of a package's CommonJS module.
const __ttMockNames = new WeakMap();
globalThis.__ttMockNames = (m) => (m && (typeof m === 'object' || typeof m === 'function') ? __ttMockNames.get(m) : undefined);
// `spy` (vi.mockObject's `{ spy: true }`, vitest's autospy): functions call their originals and
// arrays are kept.
function __automock(mod, depth, spy) {
  depth = depth || 0;
  if (typeof mod === 'function') {
    const m = __trackSpy(makeSpy(undefined, { original: spy ? mod : undefined }));
    // vitest's automocker keeps the function's own name (`Comp.name`, not the spy's) (turbo-test#136)
    try { Object.defineProperty(m, 'name', { value: mod.name, configurable: true }); } catch (e) {}
    if (depth < 3) {
      // own statics (class static methods are non-enumerable), not the function's built-ins
      for (const k of Object.getOwnPropertyNames(mod)) {
        if (k === 'length' || k === 'name' || k === 'prototype' || k === 'caller' || k === 'arguments') continue;
        try { m[k] = __automock(mod[k], depth + 1, spy); } catch (e) {}
      }
      // class / constructor function: `new m()` returns an instance of m.prototype, whose
      // methods (inherited ones included) are mocks
      if (mod.prototype && mod.prototype.constructor === mod) __automockProtoMethods(m.prototype, mod.prototype, spy);
    }
    return m;
  }
  if (Array.isArray(mod)) return spy ? mod.slice() : [];
  if (mod && typeof mod === 'object') {
    const out = {};
    for (const k of Object.keys(mod)) {
      let v; try { v = mod[k]; } catch (e) { continue; }
      out[k] = depth < 4 ? __automock(v, depth + 1, spy) : v;
    }
    // a class instance: its prototype methods become mocks too
    __automockProtoMethods(out, Object.getPrototypeOf(mod), spy);
    // an ES module's automock is still an ES module: its `default` is the mocked default
    if (mod[Symbol.for('turbo-test.esm')]) Object.defineProperty(out, Symbol.for('turbo-test.esm'), { value: true });
    return out;
  }
  return mod;
}
// vitest's autospy (vi.mockObject(value, { spy: true })): in place, each function (nested
// objects' too) becomes a spy on it; arrays and other values stay.
function __autospy(v, depth, seen) {
  if (typeof v === 'function') return __automock(v, depth, true);
  if (v === null || typeof v !== 'object' || Array.isArray(v) || seen.has(v) || depth > 4) return v;
  seen.add(v);
  for (const k of Object.keys(v)) {
    let x;
    try { x = v[k]; } catch (e) { continue; }
    if (typeof x === 'function' || (x && typeof x === 'object')) {
      try { v[k] = __autospy(x, depth + 1, seen); } catch (e) {}
    }
  }
  return v;
}
// Registry behind vi.clearAllMocks/resetAllMocks/restoreAllMocks, held WEAKLY (turbo-test#124,
// from alphajew420's miaskiewicz/turbo-test#107). Each spy keeps every call's arguments, receiver
// and result (`mock.calls` / `instances` / `results`); a strong never-truncated array kept the
// props, React elements and DOM nodes of every call of every finished file reachable for the
// whole life of a reused isolate (#79: the heap grew until V8 aborted). A spy something else still
// references (an installed spyOn, a setup file's vi.fn, a kept module's helper) stays registered
// with its history, as in vitest; one only the registry knew about can no longer be observed.
let __spyRefs = [];
let __spyCompactAt = 4096;
function __trackSpy(s) {
  __spyRefs.push(new WeakRef(s));
  if (__spyRefs.length > __spyCompactAt) {
    __spyRefs = __spyRefs.filter((r) => r.deref() !== undefined);
    __spyCompactAt = Math.max(4096, __spyRefs.length * 2);
  }
  return s;
}
function __eachSpy(f) {
  for (const r of __spyRefs.slice()) { const s = r.deref(); if (s) f(s); }
}
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
// The strict mock proxies (to their key and whether vi.mock made them): their `default` is the factory's own (`__ttToESM` adds
// none).
const __ttStrictMocks = new WeakMap();
function __ttStrictMock(mod, key, spec, vitest) {
  if (mod === null || typeof mod !== 'object' || __ttMockErrors.has(mod)) return mod;
  const proxy = new Proxy(mod, {
    get: (t, k, r) => {
      if (typeof k === 'symbol' || k in t || k === 'then' || k === '__esModule') return Reflect.get(t, k, r);
      // jest: a factory without `default` is its own default (babel's interop)
      if (__ttLenientMockKeys.has(key)) return k === 'default' ? r : Reflect.get(t, k, r);
      const e = new Error(`[vitest] No "${k}" export is defined on the "${spec}" mock. Did you forget to return it from "vi.mock"?
If you need to partially mock a module, you can use "importOriginal" helper inside:
`);
      e.codeFrame = `vi.mock(import("${spec}"), async (importOriginal) => {
  const actual = await importOriginal()
  return {
    ...actual,
    // your mocked methods
  }
})`;
      throw e;
    },
  });
  __ttStrictMocks.set(proxy, { key, vitest: vitest !== false });
  return proxy;
}
// The ES-graph loader's default export of a mock without its own `default` (synth_eval). A
// synthetic module's export is a value, so it cannot throw when read as vitest's does; this
// stand-in throws vitest's error on its first use instead — a property read, a call, its keys —
// by reading the strict mock's `default` (turbo-test#137). A jest.mock one (lenient) is its own
// default.
function __ttMockDefault(m) {
  const s = __ttStrictMocks.get(m);
  if (!s || !s.vitest || __ttLenientMockKeys.has(s.key)) return m;
  const d = () => m.default;
  // a bound function: callable, constructible, no own non-configurable `prototype`
  const stand = new Proxy(function () {}.bind(null), {
    get: (t, k) => Reflect.get(d(), k),
    has: (t, k) => k in d(),
    ownKeys: () => Reflect.ownKeys(d()),
    getOwnPropertyDescriptor: (t, k) => {
      const desc = Reflect.getOwnPropertyDescriptor(d(), k);
      return desc && { ...desc, configurable: true };
    },
    getPrototypeOf: () => Reflect.getPrototypeOf(d()),
    apply: (t, self, args) => Reflect.apply(d(), self, args),
    construct: (t, args) => Reflect.construct(d(), args),
    set: (t, k, v) => Reflect.set(d(), k, v),
    defineProperty: (t, k, desc) => Reflect.defineProperty(d(), k, desc),
    deleteProperty: (t, k) => Reflect.deleteProperty(d(), k),
  });
  __ttMockStandIns.set(stand, m);
  return stand;
}
globalThis.__ttMockDefault = __ttMockDefault;
// The stand-ins __ttMockDefault made, to their mock.
const __ttMockStandIns = new WeakMap();
// Whether the mock `m` (a registered mock's exports) gets the stand-in default: a vi.mock
// factory's, without its own `default`. The graph loader then guards its importers' reads.
globalThis.__ttMockGuarded = (m) => {
  const s = __ttStrictMocks.get(m);
  return !!s && s.vitest && !__ttLenientMockKeys.has(s.key) && !Object.prototype.hasOwnProperty.call(m, 'default');
};
// Every read of a guarded default import (mock_read.rs, turbo-test#151): a stand-in's is a read
// of its mock's `default` (vitest's error, or for a jest.mock the mock itself); any other value
// is itself.
globalThis.__ttMockRead = (v) => {
  const m = __ttMockStandIns.get(v);
  return m === undefined ? v : m.default;
};
// The ES-module view of a required module: what `import X from` / `import * as ns from` /
// `import()` / `vi.importActual` read once lowered to `__toESM(require(...))` (turbo-test#122,
// #133), vitest's `interopModule`. Mocks are themselves. turbo-test's own ES modules (marked by
// `__toCommonJS`) are their namespace: the exports object without its `__esModule` marker (an ES
// namespace has none). A CommonJS `module.exports` marked `__esModule` (tsc / babel output) has
// its own `default` (none: the whole object). A package's other CommonJS module is Node's
// namespace of it: `default` and `'module.exports'` are the whole `module.exports`, every other
// name reads it. An app CommonJS file (vitest inlines it) keeps its own `default`, else the whole
// object is added as `default` in place.
const __ttEsmMark = Symbol.for('turbo-test.esm');
// packages' CommonJS `module.exports` without `__esModule` (the loader adds them)
const __ttExternalCjs = new WeakSet();
globalThis.__ttExternalCjs = __ttExternalCjs;
// the export names Node's cjs-module-lexer finds in one, where they differ from its own names
const __ttCjsNames = new WeakMap();
globalThis.__ttMarkExternalCjs = (m, names) => {
  __ttExternalCjs.add(m);
  if (names) __ttCjsNames.set(m, names); else __ttCjsNames.delete(m);
};
const __ttCjsViews = new WeakMap();
function __ttCjsView(m) {
  let v = __ttCjsViews.get(m);
  if (!v) {
    v = new Proxy(m, {
      get: (t, k, r) => (k === 'default' ? t : Reflect.get(t, k, t)),
      has: (t, k) => k === 'default' || k in t,
      getOwnPropertyDescriptor: (t, k) => {
        const d = Reflect.getOwnPropertyDescriptor(t, k);
        if (k !== 'default' || (d && !d.configurable)) return d;
        return { value: t, writable: true, enumerable: d ? d.enumerable : false, configurable: true };
      },
    });
    __ttCjsViews.set(m, v);
  }
  return v;
}
// An ES module's namespace: its exports object without `__esModule` (which CommonJS code
// requiring it reads), tagged `Module` (turbo-test#141). Writes (vi.spyOn) go to the exports
// object, seen by every importer.
const __ttEsmViews = new WeakMap();
function __ttEsmNs(m) {
  const d = Object.getOwnPropertyDescriptor(m, '__esModule');
  if (!d || !d.configurable) return m;
  let v = __ttEsmViews.get(m);
  if (!v) {
    v = new Proxy(m, {
      get: (t, k) => (k === '__esModule' ? undefined : k === Symbol.toStringTag ? 'Module' : Reflect.get(t, k, t)),
      has: (t, k) => k !== '__esModule' && k in t,
      // a package's (marked "pkg"): Node's namespace, its names sorted (turbo-test#159)
      ownKeys: (t) => {
        const keys = Reflect.ownKeys(t).filter((k) => k !== '__esModule');
        if (t[__ttEsmMark] !== 'pkg') return keys;
        return [...keys.filter((k) => typeof k === 'string').sort(), ...keys.filter((k) => typeof k !== 'string')];
      },
      getOwnPropertyDescriptor: (t, k) => (k === '__esModule' ? undefined : Reflect.getOwnPropertyDescriptor(t, k)),
    });
    __ttEsmViews.set(m, v);
  }
  return v;
}
// Node's namespace of a CommonJS module (vitest imports a package through Node): `default` and
// `'module.exports'` are `module.exports`, then its own enumerable names; any other name reads
// `module.exports` too (vitest's interop). Writes go to `module.exports`.
const __ttNodeViews = new WeakMap();
function __ttNodeNs(m) {
  let v = __ttNodeViews.get(m);
  if (!v) {
    const keys = () => {
      const ks = new Set(['default', 'module.exports']);
      for (const k of __ttCjsNames.get(m) || Object.keys(m)) ks.add(k);
      return [...ks].sort();
    };
    const read = (k) => (k === 'default' || k === 'module.exports' ? m : m[k]);
    v = new Proxy(Object.create(null), {
      get: (t, k) => read(k),
      has: (t, k) => k === 'default' || k === 'module.exports' || k in m,
      ownKeys: () => keys(),
      getOwnPropertyDescriptor: (t, k) => (typeof k === 'string' && keys().includes(k)
        ? { value: read(k), writable: true, enumerable: true, configurable: true }
        : undefined),
      defineProperty: (t, k, d) => Reflect.defineProperty(m, k, d),
      set: (t, k, val) => Reflect.set(m, k, val),
      deleteProperty: (t, k) => Reflect.deleteProperty(m, k),
    });
    __ttNodeViews.set(m, v);
  }
  return v;
}
const __ttIsNs = (m) => Object.prototype.toString.call(m) === '[object Module]';
globalThis.__ttToESM = (m) => {
  if (!m || (typeof m !== 'object' && typeof m !== 'function') || __ttStrictMocks.has(m) || __ttIsNs(m)) return m;
  if (m[__ttEsmMark]) return __ttEsmNs(m);
  if (m.__esModule || '__esModule' in m) return ('default' in m || m.default !== undefined) ? m : __ttCjsView(m);
  if (__ttExternalCjs.has(m)) return __ttNodeNs(m);
  if ('default' in m) return m;
  try {
    Object.defineProperty(m, 'default', { value: m, configurable: true });
    return m;
  } catch (e) {
    return __ttCjsView(m);
  }
};
// `__toESM` in a package's bundle: as Node imports in an ES module that Node runs as one
// (`node`: a `.mjs` file or a `"type": "module"` package's) — CommonJS is Node's namespace of it
// (its default the whole `module.exports`); else a bundler's interop, the exports object itself
// with a `default` (a package's CommonJS with its own `default` and no `__esModule` read through
// a view, its own `default` staying what CommonJS code reads).
globalThis.__ttToESMDep = (m, node) => {
  // Node's namespace of a primitive module.exports: the value as default and 'module.exports'
  // (turbo-test#193)
  if (node && (m === null || (typeof m !== 'object' && typeof m !== 'function'))) return { default: m, 'module.exports': m };
  if (!m || (typeof m !== 'object' && typeof m !== 'function') || __ttStrictMocks.has(m) || __ttIsNs(m) || m[__ttEsmMark]) return m;
  if (node) return __ttNodeNs(m);
  if (m.__esModule || '__esModule' in m) return ('default' in m || m.default !== undefined) ? m : __ttCjsView(m);
  if ('default' in m) return m.default === m || !__ttExternalCjs.has(m) ? m : __ttCjsView(m);
  try {
    Object.defineProperty(m, 'default', { value: m, configurable: true });
    return m;
  } catch (e) {
    return __ttCjsView(m);
  }
};
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
    (v) => __ttStrictMock(v, key, String(spec), vitest),
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
  const importOriginal = () => Promise.resolve(globalThis.__ttToESM(globalThis.__ttImportActual(dir, String(spec))));
  if (factory === undefined) {
    // AUTOMOCK: no factory -> auto-generate a mock from the real module (functions -> vi.fn()).
    // A real module that fails to load is left unmocked: importing it reports its own error.
    // vitest automocks the module's namespace: a package's CommonJS one is Node's (`default`,
    // `module.exports` and the names cjs-module-lexer finds; turbo-test#136, #133)
    try {
      const actual = globalThis.__ttImportActual(dir, String(spec));
      exports = __automock(globalThis.__ttToESM(actual));
      // its own names stay importable on the ESM graph (reading undefined, as vitest's)
      if (__ttExternalCjs.has(actual)) __ttMockNames.set(exports, Object.keys(actual));
    }
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
  if (factory !== undefined) exports = __ttStrictMock(exports, key, String(spec), vitest);
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
// vi.waitFor / vi.waitUntil (turbo-test#174): vitest's, polling on the real clock (its safe
// timers); under fake timers each check first advances them by the interval. A timeout's error
// carries the stack of the call (`copyStackTrace` from a STACK_TRACE_ERROR made there).
const __copyStackTrace = (target, source) => {
  if (source.stack !== undefined) target.stack = source.stack.replace(source.message, target.message);
  return target;
};
function __waitFor(callback, options = {}) {
  const { setTimeout, setInterval, clearTimeout, clearInterval } = __realTimers;
  const { interval = 50, timeout = 1e3 } = typeof options === 'number' ? { timeout: options } : (options || {});
  const STACK_TRACE_ERROR = new Error('STACK_TRACE_ERROR');
  return new Promise((resolve, reject) => {
    let lastError;
    let promiseStatus = 'idle';
    let timeoutId;
    let intervalId;
    const onResolve = (result) => {
      if (timeoutId) clearTimeout(timeoutId);
      if (intervalId) clearInterval(intervalId);
      resolve(result);
    };
    const handleTimeout = () => {
      if (intervalId) clearInterval(intervalId);
      let error = lastError;
      if (!error) error = __copyStackTrace(new Error('Timed out in waitFor!'), STACK_TRACE_ERROR);
      reject(error);
    };
    const checkCallback = () => {
      if (__loop.fake) __loop.advance(interval);
      if (promiseStatus === 'pending') return;
      try {
        const result = callback();
        if (result !== null && typeof result === 'object' && typeof result.then === 'function') {
          promiseStatus = 'pending';
          result.then((value) => { promiseStatus = 'resolved'; onResolve(value); },
            (error) => { promiseStatus = 'rejected'; lastError = error; });
        } else {
          onResolve(result);
          return true;
        }
      } catch (error) {
        lastError = error;
      }
    };
    if (checkCallback() === true) return;
    timeoutId = setTimeout(handleTimeout, timeout);
    intervalId = setInterval(checkCallback, interval);
  });
}
function __waitUntil(callback, options = {}) {
  const { setTimeout, setInterval, clearTimeout, clearInterval } = __realTimers;
  const { interval = 50, timeout = 1e3 } = typeof options === 'number' ? { timeout: options } : (options || {});
  const STACK_TRACE_ERROR = new Error('STACK_TRACE_ERROR');
  return new Promise((resolve, reject) => {
    let promiseStatus = 'idle';
    let timeoutId;
    let intervalId;
    const onReject = (error) => {
      if (intervalId) clearInterval(intervalId);
      if (!error) error = __copyStackTrace(new Error('Timed out in waitUntil!'), STACK_TRACE_ERROR);
      reject(error);
    };
    const onResolve = (result) => {
      if (!result) return;
      if (timeoutId) clearTimeout(timeoutId);
      if (intervalId) clearInterval(intervalId);
      resolve(result);
      return true;
    };
    const checkCallback = () => {
      if (__loop.fake) __loop.advance(interval);
      if (promiseStatus === 'pending') return;
      try {
        const result = callback();
        if (result !== null && typeof result === 'object' && typeof result.then === 'function') {
          promiseStatus = 'pending';
          result.then((value) => { promiseStatus = 'resolved'; onResolve(value); },
            (error) => { promiseStatus = 'rejected'; onReject(error); });
        } else {
          return onResolve(result);
        }
      } catch (error) {
        onReject(error);
      }
    };
    if (checkCallback() === true) return;
    timeoutId = setTimeout(onReject, timeout);
    intervalId = setInterval(checkCallback, interval);
  });
}
// vitest's argument check of vi.doMock / vi.doUnmock
function __viPath(name, path) {
  if (typeof path !== 'string') throw new TypeError(`vi.${name}() expects a string path, but received a ${typeof path}`);
}
globalThis.vi = {
  // a mock given a mock is that mock (vitest)
  fn: (impl) => (impl != null && globalThis.vi.isMockFunction(impl) ? impl : __trackSpy(makeSpy(impl, { resetTo: impl }))),
  // @vitest/spy's spyOn: `accessor` 'get' / 'set' spies on an accessor; a getter holding a
  // function (a live re-export) is spied through it; the original descriptor comes back on
  // restore (a method found on a prototype is deleted from the object again)
  spyOn: (obj, key, accessor) => {
    if (obj == null) throw new Error('The vi.spyOn() function could not find an object to spy upon. The first argument must be defined.');
    if (typeof obj !== 'object' && typeof obj !== 'function') throw new Error('Vitest cannot spy on a primitive value.');
    // a registered ESM-graph namespace: the spy is its override (esm_spy.rs, turbo-test#131,
    // #146); its current value (a spy already set on it included) is the original
    const nsRaw = typeof obj === 'object' ? __ttNsRaw.get(obj) || obj : obj;
    let isNs = false;
    try { isNs = !accessor && typeof key === 'string' && __ttNsInfo.has(__ttNsTarget(nsRaw)) && key in nsRaw; } catch (e) {}
    const found = isNs ? undefined : __mockDescriptor(obj, key);
    const [descObj, desc] = found || [];
    let has = !!desc;
    if (!has) { try { has = key in obj; } catch (e) { has = true; } }
    if (!has) throw new Error(`The property "${String(key)}" is not defined on the ${typeof obj}.`);
    let accessType = accessor || 'value';
    let ssr = false;
    if (accessType === 'value' && desc && desc.value == null && desc.get) { accessType = 'get'; ssr = true; }
    let original;
    if (isNs) original = globalThis.__ttR(nsRaw, key);
    else if (desc) {
      original = desc[accessType];
      if (original == null && accessType === 'value') original = obj[key];
    } else if (accessType !== 'value') original = () => obj[key];
    else original = obj[key];
    const originalImpl = ssr && original ? original.call(obj) : original;
    if (!(typeof originalImpl === 'function' || (accessType !== 'value' && original == null))) {
      throw new Error(`vi.spyOn() can only spy on a function. Received ${typeof originalImpl}.`);
    }
    if (globalThis.vi.isMockFunction(originalImpl)) return originalImpl;
    const reassign = (cb) => {
      if (isNs) { globalThis.__ttNsSet(obj, key, cb); return; }
      const d = Object.assign({}, desc || { configurable: true, writable: true });
      delete d.value;
      if (accessType !== 'value') delete d.writable;
      d[accessType] = cb;
      Object.defineProperty(obj, key, d);
    };
    const restore = () => {
      if (!isNs && descObj !== obj) Reflect.deleteProperty(obj, key);
      else if (!isNs && desc && !original) Object.defineProperty(obj, key, desc);
      else reassign(original);
    };
    const s = makeSpy(undefined, { original: originalImpl, restore, keepName: true });
    reassign(ssr ? () => s : s);
    return __trackSpy(s);
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
  // vitest's: a string path or a TypeError; the result disposes the mock (`using`)
  doMock: (spec, factory) => {
    __viPath('doMock', spec);
    __ttMock(spec, factory, true, true);
    const r = {};
    if (Symbol.dispose) r[Symbol.dispose] = () => globalThis.vi.doUnmock(spec);
    return r;
  },
  doUnmock: (spec) => { __viPath('doUnmock', spec); globalThis.vi.unmock(spec); },
  dynamicImportSettled: () => __ttImportsSettled(),
  clearAllMocks: () => { __eachSpy((s) => s.mockClear()); return globalThis.vi; },
  resetAllMocks: () => { __eachSpy((s) => s.mockReset()); return globalThis.vi; },
  // vitest 4: puts back what vi.spyOn replaced (each once); vi.fn() mocks keep their state
  restoreAllMocks: () => {
    for (const restore of __mockRestores) restore();
    __mockRestores.clear();
    return globalThis.vi;
  },
  // the global's own descriptor comes back on unstubAllGlobals (none: it is deleted)
  stubGlobal: (k, v) => {
    const m = globalThis.__stubbedGlobals || (globalThis.__stubbedGlobals = new Map());
    if (!m.has(k)) m.set(k, Object.getOwnPropertyDescriptor(globalThis, k));
    Object.defineProperty(globalThis, k, { value: v, writable: true, configurable: true, enumerable: true });
    return globalThis.vi;
  },
  // import.meta.env (process.env): a string, DEV / PROD / SSR as '1' / '', undefined deletes
  stubEnv: (k, v) => {
    const env = globalThis.process.env;
    const m = globalThis.__stubbedEnvs || (globalThis.__stubbedEnvs = new Map());
    if (!m.has(k)) m.set(k, env[k]);
    if (__ttMetaBool.includes(k)) env[k] = v ? '1' : '';
    else if (v === undefined) delete env[k];
    else env[k] = String(v);
    return globalThis.vi;
  },
  isMockFunction: (f) => typeof f === 'function' && f._isMockFunction === true,
  // fake timers
  // a fresh fake clock at the current Date or `config.now` (vitest installs a new sinon clock;
  // the previous one's timers are dropped with it), faking `config.toFake` (default: every
  // timer function, Date, performance, process.hrtime). The fake set carries `.clock`, so
  // @testing-library's jestFakeTimersAreEnabled() drives jest.advanceTimersByTime().
  // the config's `fakeTimers` (test.fakeTimers / vi.setConfig) with `cfg` over it (turbo-test#180)
  useFakeTimers: (cfg) => { __installFake(cfg ? Object.assign({}, __ttConfig.fakeTimers, cfg) : __ttConfig.fakeTimers); return globalThis.vi; },
  useRealTimers: () => { __uninstallFake(); return globalThis.vi; },
  isFakeTimers: () => __loop.fake,
  setSystemTime: (t) => {
    const ms = t instanceof __loop.RealDate ? t.getTime()
      : typeof t === 'number' ? t : new __loop.RealDate(t).getTime();
    // the fake clock keeps its time: timers stay due relative to it (sinon's setSystemTime)
    __loop.systemTime = ms - __loop.fnow;
    // vitest fakes Date on setSystemTime even without useFakeTimers — install the fake Date so
    // `new Date()` / Date.now() reflect the set time (timer faking stays separate). Under fake
    // timers whose `toFake` leaves Date out, Date stays real.
    if (!__loop.fake || __loop.faked.has('Date')) __installFakeDate();
    if (!__loop.fake) __loop.dateMocked = true;
    return globalThis.vi;
  },
  // the fake clock's time, the time vi.setSystemTime mocked Date to without fake timers, or null
  getMockedSystemTime: () => (__loop.fake || __loop.dateMocked ? new __loop.RealDate(__loop.systemTime + __loop.fnow) : null),
  waitFor: (fn, opts) => __waitFor(fn, opts),
  waitUntil: (fn, opts) => __waitUntil(fn, opts),
  // each throws vitest's error without fake timers (turbo-test#171; the async ones reject)
  advanceTimersByTime: (ms) => { __checkFake(); __loop.advance(ms); return globalThis.vi; },
  advanceTimersByTimeAsync: async (ms) => { __checkFake(); await __loop.advanceAsync(ms); return globalThis.vi; },
  advanceTimersToNextTimer: () => { __checkFake(); __loop.toNext(); return globalThis.vi; },
  advanceTimersToNextFrame: () => { __checkFake(); __loop.toFrame(); return globalThis.vi; },
  runAllTimers: () => { __checkFake(); __loop.runAll(); return globalThis.vi; },
  runAllTimersAsync: async () => { __checkFake(); await __loop.runAllAsync(); return globalThis.vi; },
  runOnlyPendingTimers: () => { __checkFake(); __loop.runOnlyPending(); return globalThis.vi; },
  runOnlyPendingTimersAsync: async () => { __checkFake(); await __loop.runOnlyPendingAsync(); return globalThis.vi; },
  advanceTimersToNextTimerAsync: async () => { __checkFake(); await __loop.toNextAsync(); return globalThis.vi; },
  // process.nextTick / queueMicrotask are not faked (vitest's default): nothing to run
  runAllTicks: () => { __checkFake(); return globalThis.vi; },
  getTimerCount: () => { __checkFake(); return __loop.fmacro.length; },
  clearAllTimers: () => { __loop.dropFake(); return globalThis.vi; },
  // the real clock, also under fake timers or a mocked Date (turbo-test#179)
  getRealSystemTime: () => __loop.realNow(),
  // sinon's tick modes (turbo-test#179): 'manual' (the default), 'nextTimerAsync' (each real
  // macrotask fires the next fake timer), 'interval' (a real interval moves the clock by `delta`)
  setTimerTickMode: (mode, delta) => {
    __checkFake();
    if (mode === 'manual') __setTickMode('manual');
    else if (mode === 'nextTimerAsync') __setTickMode('nextAsync');
    else if (mode === 'interval') __setTickMode('interval', delta);
    else throw new Error(`Invalid tick mode: ${mode}`);
    return globalThis.vi;
  },
  // a helper whose frames vitest leaves out of an error's location (turbo-test#179): it runs `fn`
  // with the call's `this` and a thenable result is awaited by a new promise
  defineHelper: (fn) => function __VITEST_HELPER__(...args) {
    const result = fn.apply(this, args);
    if (result && typeof result === 'object' && typeof result.then === 'function') return (async function __VITEST_HELPER__() { return await result; })();
    return result;
  },
  // the automocker on a value (turbo-test#179): a deep copy whose functions are mocks returning
  // undefined (arrays emptied) or, with `{ spy: true }`, the value itself with its functions
  // replaced by spies calling the originals
  mockObject: (value, options) => (options && options.spy ? __autospy(value, 0, new Set()) : __automock(value, 0)),
  // a type helper: the value itself
  mocked: (item) => item,
  // vi.hoisted must return the SAME value in the mock-prepass and the entry module (the mock
  // closure and the test share it). Cache by call-index: the prepass fills the cache, then the
  // entry (index reset to 0) hits it — so `const x = vi.hoisted(...)` is one object everywhere.
  hoisted: (f) => {
    if (typeof f !== 'function') throw new TypeError(`"vi.hoisted" factory value must be function, received "${f === null ? 'null' : typeof f}"`);
    const cache = globalThis.__hoistedCache || (globalThis.__hoistedCache = []);
    const i = globalThis.__hoistedIdx || 0;
    globalThis.__hoistedIdx = i + 1;
    if (i < cache.length) return cache[i];
    const v = f();
    cache[i] = v;
    return v;
  },
  importActual: (p) => { try { return Promise.resolve(globalThis.__ttToESM(globalThis.__ttImportActual(globalThis.__ttDir || globalThis.__ttEntryDir, String(p)))); } catch (e) { return Promise.resolve({}); } },
  importMock: (p) => { try { return Promise.resolve(globalThis.__nativeRequire(globalThis.__ttDir || globalThis.__ttEntryDir, String(p))); } catch (e) { return Promise.resolve({}); } },
  unstubAllEnvs: () => {
    const m = globalThis.__stubbedEnvs;
    if (m) { m.forEach((val, k) => { if (val === undefined) delete globalThis.process.env[k]; else globalThis.process.env[k] = val; }); m.clear(); }
    return globalThis.vi;
  },
  unstubAllGlobals: () => {
    const m = globalThis.__stubbedGlobals;
    if (m) { m.forEach((d, k) => { if (d) Object.defineProperty(globalThis, k, d); else Reflect.deleteProperty(globalThis, k); }); m.clear(); }
    return globalThis.vi;
  },
  resetModules: () => { if (globalThis.__ttResetModules) globalThis.__ttResetModules(); return globalThis.vi; },
  // vitest's runtime config (turbo-test#179, #180): setConfig assigns over it (the first call
  // keeps a copy), resetConfig puts that copy back. `fakeTimers` is what vi.useFakeTimers reads.
  setConfig: (config) => {
    if (!__ttConfigSaved) __ttConfigSaved = Object.assign({}, __ttConfig);
    Object.assign(__ttConfig, config);
  },
  resetConfig: () => {
    if (__ttConfigSaved) Object.assign(__ttConfig, __ttConfigSaved);
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
// jest.useFakeTimers(config) as jest-environment's modern fake timers (turbo-test#181): jest
// fakes every timer API but those in `doNotFake`, stops runAllTimers after `timerLimit` (100000)
// timers, and `advanceTimers` (true, or ms per real ms) moves the clock with real time.
// `legacyFakeTimers` approximates jest's legacy timers: Date, performance and hrtime stay real
// and runAllImmediates works.
let __jestLegacy = false;
const __jestSeed = Math.floor(Math.random() * 0xffffffff) - 0x80000000;
function __jestInstallFake(cfg) {
  const c = cfg || {};
  const legacy = c.legacyFakeTimers === true;
  const skip = new Set(legacy ? ['Date', 'performance', 'hrtime'] : Array.isArray(c.doNotFake) ? c.doNotFake : []);
  const at = c.now === undefined ? undefined : c.now;
  const delta = typeof c.advanceTimers === 'number' ? c.advanceTimers : undefined;
  __installFake({
    now: legacy ? undefined : at,
    toFake: __TO_FAKE.filter((k) => !skip.has(k)),
    loopLimit: c.timerLimit || 100000,
    shouldAdvanceTime: !legacy && (c.advanceTimers === true || delta !== undefined),
    advanceTimeDelta: delta,
  });
  __jestLegacy = legacy;
}
// jest's _checkFakeTimers: a warning, not vitest's error
function __jestCheckFake() {
  if (__loop.fake) return true;
  const stack = String(new Error().stack || '').split('\n').slice(2).join('\n');
  globalThis.console.warn(`A function to advance timers was called but the timers APIs are not replaced with fake timers. Call \`jest.useFakeTimers()\` in this test file or enable fake timers for all tests by setting 'fakeTimers': {'enableGlobally': true} in Jest configuration file.\nStack Trace:\n${stack}`);
  return false;
}

// Jest's `jest` global maps onto the same machinery as `vi` (drop-in for jest projects, e.g.
// NestJS backends using ts-jest). Most methods are identical; the jest-only ones (sync
// requireActual/requireMock, isolateModules, setTimeout) are bridged here. Type-only members
// (jest.Mock, jest.Mocked, jest.SpyInstance, ...) are erased by the transform — no runtime needed.
// Defined unconditionally: it doesn't touch vitest behavior (vitest suites never reference `jest`).
globalThis.jest = {
  fn: (impl) => globalThis.vi.fn(impl),
  spyOn: (obj, key, accessType) => globalThis.vi.spyOn(obj, key, accessType),
  mock: (spec, factory) => __ttMock(spec, factory, false),
  unmock: (spec) => globalThis.vi.unmock(spec),
  doMock: (spec, factory) => __ttMock(spec, factory, false, true),
  dontMock: (spec) => globalThis.vi.unmock(spec),
  // jest's other module-registry members, on the same machinery (turbo-test#179): setMock is a
  // doMock of the given exports, unstable_mockModule / unstable_unmockModule the ESM forms,
  // deepUnmock an unmock, createMockFromModule the automock of the real module
  deepUnmock: (spec) => { globalThis.vi.unmock(spec); return globalThis.jest; },
  setMock: (spec, exports) => { __ttMock(spec, () => exports, false, true); return globalThis.jest; },
  unstable_mockModule: (spec, factory) => { __ttMock(spec, factory, false, true); return globalThis.jest; },
  unstable_unmockModule: (spec) => { globalThis.vi.unmock(spec); return globalThis.jest; },
  createMockFromModule: (spec) => __automock(globalThis.jest.requireActual(spec)),
  // the run's seed (jest: a 32-bit integer, one per run) and whether the file's environment is gone
  getSeed: () => __jestSeed,
  isEnvironmentTornDown: () => false,
  mocked: (item) => globalThis.vi.mocked(item),
  clearAllMocks: () => { globalThis.vi.clearAllMocks(); return globalThis.jest; },
  resetAllMocks: () => { globalThis.vi.resetAllMocks(); return globalThis.jest; },
  restoreAllMocks: () => { globalThis.vi.restoreAllMocks(); return globalThis.jest; },
  isMockFunction: (f) => globalThis.vi.isMockFunction(f),
  // jest.requireActual / requireMock are SYNCHRONOUS (return the module, not a Promise) —
  // unlike vi.importActual. Bridge to the native sync loader directly.
  requireActual: (p) => { try { return globalThis.__ttImportActual(globalThis.__ttDir || globalThis.__ttEntryDir, String(p)); } catch (e) { return {}; } },
  requireMock: (p) => { try { return globalThis.__nativeRequire(globalThis.__ttDir || globalThis.__ttEntryDir, String(p)); } catch (e) { return {}; } },
  // jest.isolateModules(fn): run `fn` with a fresh module registry. Approximate with a
  // resetModules around the (sync) callback — enough for the common "re-require with new env".
  isolateModules: (fn) => { globalThis.vi.resetModules(); try { fn(); } finally { globalThis.vi.resetModules(); } },
  isolateModulesAsync: async (fn) => { globalThis.vi.resetModules(); try { await fn(); } finally { globalThis.vi.resetModules(); } },
  resetModules: () => { globalThis.vi.resetModules(); return globalThis.jest; },
  // timers (turbo-test#181): jest's modern fake timers. useFakeTimers reads jest's options
  // (`now`, `doNotFake`, `timerLimit` (100000), `advanceTimers: true | ms`, `legacyFakeTimers`);
  // vitest's toFake / loopLimit mean nothing to it. Without fake timers each advance function
  // warns as jest does and does nothing; they return undefined.
  useFakeTimers: (cfg) => { __jestInstallFake(cfg); return globalThis.jest; },
  useRealTimers: () => { __uninstallFake(); __jestLegacy = false; return globalThis.jest; },
  setSystemTime: (t) => { if (__jestCheckFake()) globalThis.vi.setSystemTime(t === undefined ? __loop.realNow() : t); },
  getRealSystemTime: () => __loop.realNow(),
  now: () => (__loop.fake ? __loop.systemTime + __loop.fnow : __loop.realNow()),
  advanceTimersByTime: (ms) => { if (__jestCheckFake()) __loop.advance(ms); },
  advanceTimersByTimeAsync: async (ms) => { if (__jestCheckFake()) await __loop.advanceAsync(ms); },
  advanceTimersToNextTimer: (steps) => { if (__jestCheckFake()) for (let i = steps === undefined ? 1 : steps; i > 0 && __loop.fmacro.length; i--) __loop.toNext(); },
  advanceTimersToNextTimerAsync: async (steps) => { if (__jestCheckFake()) for (let i = steps === undefined ? 1 : steps; i > 0 && __loop.fmacro.length; i--) await __loop.toNextAsync(); },
  advanceTimersToNextFrame: () => { if (__jestCheckFake()) __loop.toFrame(); },
  runAllTimers: () => { if (__jestCheckFake()) __loop.runAll(); },
  runAllTimersAsync: async () => { if (__jestCheckFake()) await __loop.runAllAsync(); },
  runOnlyPendingTimers: () => { if (__jestCheckFake()) __loop.runOnlyPending(); },
  runOnlyPendingTimersAsync: async () => { if (__jestCheckFake()) await __loop.runOnlyPendingAsync(); },
  runAllTicks: () => { __jestCheckFake(); },
  // legacy fake timers only (jest's TypeError under the modern ones)
  runAllImmediates: () => {
    if (!__jestLegacy) throw new TypeError('`jest.runAllImmediates()` is only available when using legacy fake timers.');
    for (const t of __loop.fmacro.filter((x) => x.immediate).sort(__byDue)) if (__loop.fmacro.includes(t)) { __clearTimer(t.id); try { t.cb(); } catch (e) { __reportUncaught(e); } }
  },
  clearAllTimers: () => { __loop.dropFake(); },
  getTimerCount: () => (__jestCheckFake() ? __loop.fmacro.length : 0),
  // `{ mode: 'manual' | 'nextAsync' | 'interval', delta }`
  setTimerTickMode: (o) => {
    if (__jestCheckFake()) __setTickMode(o && o.mode, o && o.delta);
    return globalThis.jest;
  },
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
    return r.length <= o.truncate ? r : `${v.toString()}{…}`;
  }
  const chai = v[Symbol.for('chai/inspect')];
  if (typeof chai === 'function') return String(chai.call(v, o));
  const custom = v[Symbol.for('nodejs.util.inspect.custom')];
  // loupe hands the method its normalised options (depth 2, breakLength Infinity, no colours)
  if (typeof custom === 'function') { const r = custom.call(v, 2, Object.assign({ depth: 2, breakLength: Infinity, colors: false, showHidden: false }, o), __inspect); return typeof r === 'string' ? r : __inspect(r, o); }
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
// (chai's AssertionError itself, turbo-test#160: `instanceof chai.AssertionError` holds, as in vitest)
class __ChaiAssertionError extends __chai.AssertionError {
  constructor(message, props) {
    super(message);
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
// `showDiff` false: vitest's comparison and call-count matchers pass it (no diff is printed);
// chai also drops the diff when both values are undefined.
function __chaiAssert(negated, pass, msg, notMsg, obj, exp, act, showDiff = true) {
  if (negated ? !pass : pass) return;
  throw new __ChaiAssertionError(__chaiMsg(negated ? notMsg : msg, obj, exp, act), { actual: act, expected: exp, showDiff: showDiff && !(exp === undefined && act === undefined) });
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
    plugins: o.plugins || [],
  };
  return __pfPrint(val, cfg, '', 0, [], false);
}
// pretty-format plugins (expect.addSnapshotSerializer, turbo-test#162): a plugin is
// `{ test, serialize }` or the older `{ test, print }`, tried before any built-in printing.
// A plugin sees pretty-format's config; `__pfStubs` stand for vitest's 7 built-in snapshot
// plugins in its `plugins` (printed natively here: the mock function and asymmetric matchers).
const __PrettyFormatPluginError = class PrettyFormatPluginError extends Error {
  constructor(message, stack) { super(message); this.stack = stack; this.name = this.constructor.name; }
};
const __pfStubs = Array.from({ length: 7 }, () => ({ test: () => false, serialize: () => '' }));
const __pfCfgOf = new WeakMap();
function __pfConfig(cfg) {
  if (cfg.pretty) return cfg.pretty;
  const none = () => ({ close: '', open: '' });
  const pretty = {
    callToJSON: cfg.toJSON,
    colors: { comment: none(), content: none(), prop: none(), tag: none(), value: none() },
    compareKeys: undefined,
    escapeRegex: cfg.escapeRegex,
    escapeString: cfg.escape,
    indent: cfg.indent,
    maxDepth: cfg.maxDepth,
    maxWidth: Infinity,
    min: cfg.min,
    plugins: cfg.mocks ? [...cfg.plugins, ...__pfStubs] : cfg.plugins.slice(),
    printBasicPrototype: cfg.basic,
    printFunctionName: cfg.fnName,
    printShadowRoot: true,
    spacingInner: cfg.inner,
    spacingOuter: cfg.outer,
    maxOutputLength: 1e6,
    _outputLengthPerDepth: [],
  };
  cfg.pretty = pretty;
  __pfCfgOf.set(pretty, cfg);
  return pretty;
}
// pretty-format's `printer` as a plugin calls it: with the config it was given, or its own copy
function __pfPrinter(val, config, indentation, depth, refs, hasCalledToJSON) {
  let cfg = __pfCfgOf.get(config);
  if (!cfg) {
    const plugins = config.plugins || [];
    cfg = {
      indent: config.indent, outer: config.spacingOuter, inner: config.spacingInner, min: !!config.min,
      basic: config.printBasicPrototype !== false, maxDepth: config.maxDepth, escape: config.escapeString !== false,
      toJSON: config.callToJSON !== false, errors: false, fnName: config.printFunctionName !== false,
      escapeRegex: !!config.escapeRegex, mocks: plugins.some((p) => __pfStubs.includes(p)),
      plugins: plugins.filter((p) => !__pfStubs.includes(p)),
    };
    cfg.pretty = config;
    __pfCfgOf.set(config, cfg);
  }
  return __pfPrint(val, cfg, indentation, depth, refs, !!hasCalledToJSON);
}
function __pfPlugin(plugin, val, cfg, indentation, depth, refs) {
  const config = __pfConfig(cfg);
  let printed;
  try {
    printed = plugin.serialize != null
      ? plugin.serialize(val, config, indentation, depth, refs, __pfPrinter)
      : plugin.print(val, (valChild) => __pfPrinter(valChild, config, indentation, depth, refs), (str) => {
        const indentationNext = indentation + config.indent;
        return indentationNext + str.replaceAll(/\n/g, `\n${indentationNext}`);
      }, { edgeSpacing: config.spacingOuter, min: config.min, spacing: config.spacingInner }, config.colors);
  } catch (error) {
    throw new __PrettyFormatPluginError(error.message, error.stack);
  }
  if (typeof printed !== 'string') throw new TypeError(`pretty-format: Plugin must return type "string" but instead returned "${typeof printed}".`);
  return printed;
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
  for (const plugin of cfg.plugins) {
    let hit;
    try { hit = plugin.test(v); } catch (error) { throw new __PrettyFormatPluginError(error.message, error.stack); }
    if (hit) return __pfPlugin(plugin, v, cfg, ind, depth, refs);
  }
  const p = (x, i) => __pfPrint(x, cfg, i, depth, refs, false);
  // @vitest/pretty-format's AsymmetricMatcher plugin: anything with vitest's `$$typeof`, printed by
  // its toString name (containing / matching samples) or its toAsymmetricMatcher()
  if (v && v.$$typeof === Symbol.for('jest.asymmetricMatcher')) {
    const name = v.toString();
    const d = depth + 1;
    const pp = (x, i) => __pfPrint(x, cfg, i, d, refs, false);
    if (name === 'ArrayContaining' || name === 'ArrayNotContaining') return d > cfg.maxDepth ? `[${name}]` : `${name} [${__pfItems(Array.from(v.sample), cfg, ind, d, refs, pp)}]`;
    if (name === 'ObjectContaining' || name === 'ObjectNotContaining') return d > cfg.maxDepth ? `[${name}]` : `${name} {${__pfProps(v.sample, cfg, ind, d, refs)}}`;
    if (name === 'StringMatching' || name === 'StringNotMatching' || name === 'StringContaining' || name === 'StringNotContaining') return `${name} ${p(v.sample, ind)}`;
    if (typeof v.toAsymmetricMatcher !== 'function') throw new TypeError(`Asymmetric matcher ${v.constructor.name} does not implement toAsymmetricMatcher()`);
    return v.toAsymmetricMatcher();
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
// @vitest/utils `stringify(object, maxDepth = 10, { maxLength, ...prettyFormatOptions })`: an
// output of maxLength (10000) or more is printed again at half the depth
function __vstringify(object, maxDepth = 10, { maxLength, ...options } = {}) {
  const max = maxLength ?? 1e4;
  const o = { maxDepth, escapeString: false, ...options };
  const pf = (callToJSON) => __pf(object, {
    indent: o.indent, min: o.min, basic: o.printBasicPrototype, maxDepth: o.maxDepth, escape: o.escapeString,
    toJSON: callToJSON ?? o.callToJSON, fnName: o.printFunctionName, escapeRegex: o.escapeRegex,
  });
  let result;
  try { result = pf(); } catch { result = pf(false); }
  return result.length >= max && maxDepth > 1 ? __vstringify(object, Math.floor(Math.min(maxDepth, Number.MAX_SAFE_INTEGER) / 2), { maxLength, ...options }) : result;
}
// tinyrainbow's colours for the diffs vitest's worker prints with its colours on (turbo-test#94:
// `__TT_COLORS`, the coloured human reporters); identity otherwise.
function __tr(open, close, replace = open) {
  return (input) => {
    const s = `${input}`;
    if (!globalThis.__TT_COLORS) return s;
    let i = s.indexOf(close, open.length);
    if (i === -1) return open + s + close;
    let out = '';
    let last = 0;
    do { out += s.substring(last, i) + replace; last = i + close.length; i = s.indexOf(close, last); } while (i !== -1);
    return open + out + s.substring(last) + close;
  };
}
const __dcol = {
  green: __tr('\x1b[32m', '\x1b[39m'),
  red: __tr('\x1b[31m', '\x1b[39m'),
  yellow: __tr('\x1b[33m', '\x1b[39m'),
  dim: __tr('\x1b[2m', '\x1b[22m', '\x1b[22m\x1b[2m'),
  inverse: __tr('\x1b[7m', '\x1b[27m'),
};
// diff-sequences (jest): the common subsequences of a[0, aLength) and b[0, bLength) by Myers'
// linear-space bidirectional search, reported in order as foundSubsequence(nCommon, aCommon,
// bCommon) — the edit script jest-diff and vitest's diff build on.
function __diffSequences(aLength, bLength, isCommon, foundSubsequence) {
  const countF = (aIndex, aEnd, bIndex, bEnd, common) => {
    let n = 0;
    while (aIndex < aEnd && bIndex < bEnd && common(aIndex, bIndex)) { aIndex++; bIndex++; n++; }
    return n;
  };
  const countR = (aStart, aIndex, bStart, bIndex, common) => {
    let n = 0;
    while (aStart <= aIndex && bStart <= bIndex && common(aIndex, bIndex)) { aIndex--; bIndex--; n++; }
    return n;
  };
  const extendF = (d, aEnd, bEnd, bF, common, aIndexesF, iMaxF) => {
    let iF = 0;
    let kF = -d;
    let aFirst = aIndexesF[iF];
    let aIndexPrev1 = aFirst;
    aIndexesF[iF] += countF(aFirst + 1, aEnd, bF + aFirst - kF + 1, bEnd, common);
    const nF = d < iMaxF ? d : iMaxF;
    for (iF += 1, kF += 2; iF <= nF; iF += 1, kF += 2) {
      if (iF !== d && aIndexPrev1 < aIndexesF[iF]) aFirst = aIndexesF[iF];
      else {
        aFirst = aIndexPrev1 + 1;
        if (aEnd <= aFirst) return iF - 1;
      }
      aIndexPrev1 = aIndexesF[iF];
      aIndexesF[iF] = aFirst + countF(aFirst + 1, aEnd, bF + aFirst - kF + 1, bEnd, common);
    }
    return iMaxF;
  };
  const extendR = (d, aStart, bStart, bR, common, aIndexesR, iMaxR) => {
    let iR = 0;
    let kR = d;
    let aFirst = aIndexesR[iR];
    let aIndexPrev1 = aFirst;
    aIndexesR[iR] -= countR(aStart, aFirst - 1, bStart, bR + aFirst - kR - 1, common);
    const nR = d < iMaxR ? d : iMaxR;
    for (iR += 1, kR -= 2; iR <= nR; iR += 1, kR -= 2) {
      if (iR !== d && aIndexesR[iR] < aIndexPrev1) aFirst = aIndexesR[iR];
      else {
        aFirst = aIndexPrev1 - 1;
        if (aFirst < aStart) return iR - 1;
      }
      aIndexPrev1 = aIndexesR[iR];
      aIndexesR[iR] = aFirst - countR(aStart, aFirst - 1, bStart, bR + aFirst - kR - 1, common);
    }
    return iMaxR;
  };
  const overlapF = (d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division) => {
    const bF = bStart - aStart;
    const baDeltaLength = (bEnd - bStart) - (aEnd - aStart);
    const kMin = -baDeltaLength - (d - 1);
    const kMax = -baDeltaLength + (d - 1);
    let aIndexPrev1 = 0;
    const nF = d < iMaxF ? d : iMaxF;
    for (let iF = 0, kF = -d; iF <= nF; iF += 1, kF += 2) {
      const insert = iF === 0 || (iF !== d && aIndexPrev1 < aIndexesF[iF]);
      const aLastPrev = insert ? aIndexesF[iF] : aIndexPrev1;
      const aFirst = insert ? aLastPrev : aLastPrev + 1;
      const bFirst = bF + aFirst - kF;
      const nCommonF = countF(aFirst + 1, aEnd, bFirst + 1, bEnd, common);
      const aLast = aFirst + nCommonF;
      aIndexPrev1 = aIndexesF[iF];
      aIndexesF[iF] = aLast;
      if (kMin <= kF && kF <= kMax) {
        const iR = (d - 1 - (kF + baDeltaLength)) / 2;
        if (iR <= iMaxR && aIndexesR[iR] - 1 <= aLast) {
          const bLastPrev = bF + aLastPrev - (insert ? kF + 1 : kF - 1);
          const nCommonR = countR(aStart, aLastPrev, bStart, bLastPrev, common);
          const aEndPreceding = aLastPrev - nCommonR + 1;
          const bEndPreceding = bLastPrev - nCommonR + 1;
          division.nChangePreceding = d - 1;
          if (d - 1 === aEndPreceding + bEndPreceding - aStart - bStart) { division.aEndPreceding = aStart; division.bEndPreceding = bStart; }
          else { division.aEndPreceding = aEndPreceding; division.bEndPreceding = bEndPreceding; }
          division.nCommonPreceding = nCommonR;
          if (nCommonR !== 0) { division.aCommonPreceding = aEndPreceding; division.bCommonPreceding = bEndPreceding; }
          division.nCommonFollowing = nCommonF;
          if (nCommonF !== 0) { division.aCommonFollowing = aFirst + 1; division.bCommonFollowing = bFirst + 1; }
          const aStartFollowing = aLast + 1;
          const bStartFollowing = bFirst + nCommonF + 1;
          division.nChangeFollowing = d - 1;
          if (d - 1 === aEnd + bEnd - aStartFollowing - bStartFollowing) { division.aStartFollowing = aEnd; division.bStartFollowing = bEnd; }
          else { division.aStartFollowing = aStartFollowing; division.bStartFollowing = bStartFollowing; }
          return true;
        }
      }
    }
    return false;
  };
  const overlapR = (d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division) => {
    const bR = bEnd - aEnd;
    const baDeltaLength = (bEnd - bStart) - (aEnd - aStart);
    const kMin = baDeltaLength - d;
    const kMax = baDeltaLength + d;
    let aIndexPrev1 = 0;
    const nR = d < iMaxR ? d : iMaxR;
    for (let iR = 0, kR = d; iR <= nR; iR += 1, kR -= 2) {
      const insert = iR === 0 || (iR !== d && aIndexesR[iR] < aIndexPrev1);
      const aLastPrev = insert ? aIndexesR[iR] : aIndexPrev1;
      const aFirst = insert ? aLastPrev : aLastPrev - 1;
      const bFirst = bR + aFirst - kR;
      const nCommonR = countR(aStart, aFirst - 1, bStart, bFirst - 1, common);
      const aLast = aFirst - nCommonR;
      aIndexPrev1 = aIndexesR[iR];
      aIndexesR[iR] = aLast;
      if (kMin <= kR && kR <= kMax) {
        const iF = (d + (kR - baDeltaLength)) / 2;
        if (iF <= iMaxF && aLast - 1 <= aIndexesF[iF]) {
          const bLast = bFirst - nCommonR;
          division.nChangePreceding = d;
          if (d === aLast + bLast - aStart - bStart) { division.aEndPreceding = aStart; division.bEndPreceding = bStart; }
          else { division.aEndPreceding = aLast; division.bEndPreceding = bLast; }
          division.nCommonPreceding = nCommonR;
          if (nCommonR !== 0) { division.aCommonPreceding = aLast; division.bCommonPreceding = bLast; }
          division.nChangeFollowing = d - 1;
          if (d === 1) {
            division.nCommonFollowing = 0;
            division.aStartFollowing = aEnd;
            division.bStartFollowing = bEnd;
          } else {
            const bLastPrev = bR + aLastPrev - (insert ? kR - 1 : kR + 1);
            const nCommonF = countF(aLastPrev, aEnd, bLastPrev, bEnd, common);
            division.nCommonFollowing = nCommonF;
            if (nCommonF !== 0) { division.aCommonFollowing = aLastPrev; division.bCommonFollowing = bLastPrev; }
            const aStartFollowing = aLastPrev + nCommonF;
            const bStartFollowing = bLastPrev + nCommonF;
            if (d - 1 === aEnd + bEnd - aStartFollowing - bStartFollowing) { division.aStartFollowing = aEnd; division.bStartFollowing = bEnd; }
            else { division.aStartFollowing = aStartFollowing; division.bStartFollowing = bStartFollowing; }
          }
          return true;
        }
      }
    }
    return false;
  };
  const divide = (nChange, aStart, aEnd, bStart, bEnd, common, aIndexesF, aIndexesR, division) => {
    const bF = bStart - aStart;
    const bR = bEnd - aEnd;
    const aLength = aEnd - aStart;
    const bLength = bEnd - bStart;
    const baDeltaLength = bLength - aLength;
    let iMaxF = aLength;
    let iMaxR = aLength;
    aIndexesF[0] = aStart - 1;
    aIndexesR[0] = aEnd;
    if (baDeltaLength % 2 === 0) {
      const dMin = (nChange || baDeltaLength) / 2;
      const dMax = (aLength + bLength) / 2;
      for (let d = 1; d <= dMax; d += 1) {
        iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
        if (d < dMin) iMaxR = extendR(d, aStart, bStart, bR, common, aIndexesR, iMaxR);
        else if (overlapR(d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division)) return;
      }
    } else {
      const dMin = ((nChange || baDeltaLength) + 1) / 2;
      const dMax = (aLength + bLength + 1) / 2;
      let d = 1;
      iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
      for (d += 1; d <= dMax; d += 1) {
        iMaxR = extendR(d - 1, aStart, bStart, bR, common, aIndexesR, iMaxR);
        if (d < dMin) iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
        else if (overlapF(d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division)) return;
      }
    }
    throw new Error(`diff-sequences: no overlap aStart=${aStart} aEnd=${aEnd} bStart=${bStart} bEnd=${bEnd}`);
  };
  const find = (nChange, aStart, aEnd, bStart, bEnd, transposed, callbacks, aIndexesF, aIndexesR, division) => {
    if (bEnd - bStart < aEnd - aStart) {
      transposed = !transposed;
      if (transposed && callbacks.length === 1) {
        const { found, common } = callbacks[0];
        callbacks[1] = { found: (n, bc, ac) => found(n, ac, bc), common: (bi, ai) => common(ai, bi) };
      }
      [aStart, aEnd, bStart, bEnd] = [bStart, bEnd, aStart, aEnd];
    }
    const { found, common } = callbacks[transposed ? 1 : 0];
    divide(nChange, aStart, aEnd, bStart, bEnd, common, aIndexesF, aIndexesR, division);
    const { nChangePreceding, aEndPreceding, bEndPreceding, nCommonPreceding, aCommonPreceding, bCommonPreceding, nCommonFollowing, aCommonFollowing, bCommonFollowing, nChangeFollowing, aStartFollowing, bStartFollowing } = division;
    if (aStart < aEndPreceding && bStart < bEndPreceding) find(nChangePreceding, aStart, aEndPreceding, bStart, bEndPreceding, transposed, callbacks, aIndexesF, aIndexesR, division);
    if (nCommonPreceding !== 0) found(nCommonPreceding, aCommonPreceding, bCommonPreceding);
    if (nCommonFollowing !== 0) found(nCommonFollowing, aCommonFollowing, bCommonFollowing);
    if (aStartFollowing < aEnd && bStartFollowing < bEnd) find(nChangeFollowing, aStartFollowing, aEnd, bStartFollowing, bEnd, transposed, callbacks, aIndexesF, aIndexesR, division);
  };
  const nCommonF = countF(0, aLength, 0, bLength, isCommon);
  if (nCommonF !== 0) foundSubsequence(nCommonF, 0, 0);
  if (aLength !== nCommonF || bLength !== nCommonF) {
    const aStart = nCommonF;
    const bStart = nCommonF;
    const nCommonR = countR(aStart, aLength - 1, bStart, bLength - 1, isCommon);
    const aEnd = aLength - nCommonR;
    const bEnd = bLength - nCommonR;
    const nCommonFR = nCommonF + nCommonR;
    if (aLength !== nCommonFR && bLength !== nCommonFR) {
      const division = { aCommonFollowing: 0, aCommonPreceding: 0, aEndPreceding: 0, aStartFollowing: 0, bCommonFollowing: 0, bCommonPreceding: 0, bEndPreceding: 0, bStartFollowing: 0, nChangeFollowing: 0, nChangePreceding: 0, nCommonFollowing: 0, nCommonPreceding: 0 };
      find(0, aStart, aEnd, bStart, bEnd, false, [{ found: foundSubsequence, common: isCommon }], [0], [0], division);
    }
    if (nCommonR !== 0) foundSubsequence(nCommonR, aEnd, bEnd);
  }
}
// jest-diff `diffLinesRaw`: [op, line] with op -1 delete, 1 insert, 0 common (the b line).
function __lineDiff(a, b) {
  const out = [];
  let ai = 0;
  let bi = 0;
  __diffSequences(a.length, b.length, (i, j) => a[i] === b[j], (n, ac, bc) => {
    for (; ai !== ac; ai++) out.push([-1, ai]);
    for (; bi !== bc; bi++) out.push([1, bi]);
    for (; n !== 0; n--, ai++, bi++) out.push([0, ai, bi]);
  });
  for (; ai !== a.length; ai++) out.push([-1, ai]);
  for (; bi !== b.length; bi++) out.push([1, bi]);
  return out;
}
// jest-diff `printAnnotation` (vitest's diff options: `- Expected` green, `+ Received` red).
// the annotation names: utils.diff's aAnnotation / bAnnotation while it runs
let __diffAnno = ['Expected', 'Received'];
const __annotation = () => `${__dcol.green(`- ${__diffAnno[0]}`)}\n${__dcol.red(`+ ${__diffAnno[1]}`)}\n\n`;
// jest-diff `joinAlignedDiffsNoExpand` (5 context lines, `@@` patch marks), each line as
// printDiffLine colours it: deleted green, inserted red, common dim, the marks yellow.
function __joinDiffs(diffs) {
  const ctx = 5;
  const ctx2 = ctx + ctx;
  const color = { '-': __dcol.green, '+': __dcol.red, ' ': __dcol.dim };
  const line = (ind, l) => (l.length ? color[ind](`${ind} ${l}`) : ind !== ' ' ? color[ind](ind) : '');
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
  const patchMark = () => __dcol.yellow(`@@ -${aStart + 1},${aEnd - aStart} +${bStart + 1},${bEnd - bStart} @@`);
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
// jest-diff `diffLinesUnified2`: align by the compare lines, show the display lines; with
// `annotate` the `- Expected` / `+ Received` lines first (printDiffLines).
function __diffLines(aDisp, bDisp, aCmp = aDisp, bCmp = bDisp, annotate = false) {
  const empty = (l) => l.length === 1 && l[0].length === 0;
  if (empty(aDisp) && empty(aCmp)) { aDisp = []; aCmp = []; }
  if (empty(bDisp) && empty(bCmp)) { bDisp = []; bCmp = []; }
  if (aDisp.length !== aCmp.length || bDisp.length !== bCmp.length) { aCmp = aDisp; bCmp = bDisp; }
  const diffs = __lineDiff(aCmp, bCmp).map((d) => (d[0] === -1 ? [-1, aDisp[d[1]]] : d[0] === 1 ? [1, bDisp[d[1]]] : [0, bDisp[d[2]]]));
  return (annotate ? __annotation() : '') + __joinDiffs(diffs);
}
// diff-match-patch's cleanup, as @vitest/utils' `diffStringsRaw(a, b, true)` runs it.
function __commonPrefix(t1, t2) {
  if (!t1 || !t2 || t1.charAt(0) !== t2.charAt(0)) return 0;
  let min = 0;
  let max = Math.min(t1.length, t2.length);
  let mid = max;
  let start = 0;
  while (min < mid) {
    if (t1.substring(start, mid) === t2.substring(start, mid)) { min = mid; start = min; } else max = mid;
    mid = Math.floor((max - min) / 2 + min);
  }
  return mid;
}
function __commonSuffix(t1, t2) {
  if (!t1 || !t2 || t1.charAt(t1.length - 1) !== t2.charAt(t2.length - 1)) return 0;
  let min = 0;
  let max = Math.min(t1.length, t2.length);
  let mid = max;
  let end = 0;
  while (min < mid) {
    if (t1.substring(t1.length - mid, t1.length - end) === t2.substring(t2.length - mid, t2.length - end)) { min = mid; end = min; } else max = mid;
    mid = Math.floor((max - min) / 2 + min);
  }
  return mid;
}
function __commonOverlap(t1, t2) {
  const l1 = t1.length;
  const l2 = t2.length;
  if (l1 === 0 || l2 === 0) return 0;
  if (l1 > l2) t1 = t1.substring(l1 - l2);
  else if (l1 < l2) t2 = t2.substring(0, l1);
  const len = Math.min(l1, l2);
  if (t1 === t2) return len;
  let best = 0;
  let length = 1;
  for (;;) {
    const found = t2.indexOf(t1.substring(len - length));
    if (found === -1) return best;
    length += found;
    if (found === 0 || t1.substring(len - length) === t2.substring(0, length)) { best = length; length++; }
  }
}
function __semanticScore(one, two) {
  if (!one || !two) return 6;
  const c1 = one.charAt(one.length - 1);
  const c2 = two.charAt(0);
  const nonAlnum1 = /[^a-z0-9]/i.test(c1);
  const nonAlnum2 = /[^a-z0-9]/i.test(c2);
  const ws1 = nonAlnum1 && /\s/.test(c1);
  const ws2 = nonAlnum2 && /\s/.test(c2);
  const lb1 = ws1 && /[\r\n]/.test(c1);
  const lb2 = ws2 && /[\r\n]/.test(c2);
  const blank1 = lb1 && /\n\r?\n$/.test(one);
  const blank2 = lb2 && /^\r?\n\r?\n/.test(two);
  if (blank1 || blank2) return 5;
  if (lb1 || lb2) return 4;
  if (nonAlnum1 && !ws1 && ws2) return 3;
  if (ws1 || ws2) return 2;
  if (nonAlnum1 || nonAlnum2) return 1;
  return 0;
}
function __cleanupSemanticLossless(diffs) {
  let pointer = 1;
  while (pointer < diffs.length - 1) {
    if (diffs[pointer - 1][0] === 0 && diffs[pointer + 1][0] === 0) {
      let eq1 = diffs[pointer - 1][1];
      let edit = diffs[pointer][1];
      let eq2 = diffs[pointer + 1][1];
      const off = __commonSuffix(eq1, edit);
      if (off) {
        const common = edit.substring(edit.length - off);
        eq1 = eq1.substring(0, eq1.length - off);
        edit = common + edit.substring(0, edit.length - off);
        eq2 = common + eq2;
      }
      let bestEq1 = eq1;
      let bestEdit = edit;
      let bestEq2 = eq2;
      let bestScore = __semanticScore(eq1, edit) + __semanticScore(edit, eq2);
      while (edit.charAt(0) === eq2.charAt(0)) {
        eq1 += edit.charAt(0);
        edit = edit.substring(1) + eq2.charAt(0);
        eq2 = eq2.substring(1);
        const score = __semanticScore(eq1, edit) + __semanticScore(edit, eq2);
        if (score >= bestScore) { bestScore = score; bestEq1 = eq1; bestEdit = edit; bestEq2 = eq2; }
      }
      if (diffs[pointer - 1][1] !== bestEq1) {
        if (bestEq1) diffs[pointer - 1][1] = bestEq1;
        else { diffs.splice(pointer - 1, 1); pointer--; }
        diffs[pointer][1] = bestEdit;
        if (bestEq2) diffs[pointer + 1][1] = bestEq2;
        else { diffs.splice(pointer + 1, 1); pointer--; }
      }
    }
    pointer++;
  }
}
function __cleanupMerge(diffs) {
  diffs.push([0, '']);
  let pointer = 0;
  let nDel = 0;
  let nIns = 0;
  let tDel = '';
  let tIns = '';
  while (pointer < diffs.length) {
    switch (diffs[pointer][0]) {
      case 1: nIns++; tIns += diffs[pointer][1]; pointer++; break;
      case -1: nDel++; tDel += diffs[pointer][1]; pointer++; break;
      case 0:
        if (nDel + nIns > 1) {
          if (nDel !== 0 && nIns !== 0) {
            let n = __commonPrefix(tIns, tDel);
            if (n !== 0) {
              if (pointer - nDel - nIns > 0 && diffs[pointer - nDel - nIns - 1][0] === 0) diffs[pointer - nDel - nIns - 1][1] += tIns.substring(0, n);
              else { diffs.splice(0, 0, [0, tIns.substring(0, n)]); pointer++; }
              tIns = tIns.substring(n);
              tDel = tDel.substring(n);
            }
            n = __commonSuffix(tIns, tDel);
            if (n !== 0) {
              diffs[pointer][1] = tIns.substring(tIns.length - n) + diffs[pointer][1];
              tIns = tIns.substring(0, tIns.length - n);
              tDel = tDel.substring(0, tDel.length - n);
            }
          }
          pointer -= nDel + nIns;
          diffs.splice(pointer, nDel + nIns);
          if (tDel.length) { diffs.splice(pointer, 0, [-1, tDel]); pointer++; }
          if (tIns.length) { diffs.splice(pointer, 0, [1, tIns]); pointer++; }
          pointer++;
        } else if (pointer !== 0 && diffs[pointer - 1][0] === 0) {
          diffs[pointer - 1][1] += diffs[pointer][1];
          diffs.splice(pointer, 1);
        } else {
          pointer++;
        }
        nIns = 0; nDel = 0; tDel = ''; tIns = '';
        break;
    }
  }
  if (diffs.at(-1)?.[1] === '') diffs.pop();
  let changes = false;
  pointer = 1;
  while (pointer < diffs.length - 1) {
    if (diffs[pointer - 1][0] === 0 && diffs[pointer + 1][0] === 0) {
      const prev = diffs[pointer - 1][1];
      const cur = diffs[pointer][1];
      const next = diffs[pointer + 1][1];
      if (cur.substring(cur.length - prev.length) === prev) {
        diffs[pointer][1] = prev + cur.substring(0, cur.length - prev.length);
        diffs[pointer + 1][1] = prev + next;
        diffs.splice(pointer - 1, 1);
        changes = true;
      } else if (cur.substring(0, next.length) === next) {
        diffs[pointer - 1][1] += next;
        diffs[pointer][1] = cur.substring(next.length) + next;
        diffs.splice(pointer + 1, 1);
        changes = true;
      }
    }
    pointer++;
  }
  if (changes) __cleanupMerge(diffs);
}
function __cleanupSemantic(diffs) {
  let changes = false;
  const equalities = [];
  let nEq = 0;
  let lastEquality = null;
  let pointer = 0;
  let ins1 = 0, del1 = 0, ins2 = 0, del2 = 0;
  while (pointer < diffs.length) {
    if (diffs[pointer][0] === 0) {
      equalities[nEq++] = pointer;
      ins1 = ins2; del1 = del2; ins2 = 0; del2 = 0;
      lastEquality = diffs[pointer][1];
    } else {
      if (diffs[pointer][0] === 1) ins2 += diffs[pointer][1].length;
      else del2 += diffs[pointer][1].length;
      if (lastEquality && lastEquality.length <= Math.max(ins1, del1) && lastEquality.length <= Math.max(ins2, del2)) {
        diffs.splice(equalities[nEq - 1], 0, [-1, lastEquality]);
        diffs[equalities[nEq - 1] + 1][0] = 1;
        nEq--;
        nEq--;
        pointer = nEq > 0 ? equalities[nEq - 1] : -1;
        ins1 = 0; del1 = 0; ins2 = 0; del2 = 0;
        lastEquality = null;
        changes = true;
      }
    }
    pointer++;
  }
  if (changes) __cleanupMerge(diffs);
  __cleanupSemanticLossless(diffs);
  pointer = 1;
  while (pointer < diffs.length) {
    if (diffs[pointer - 1][0] === -1 && diffs[pointer][0] === 1) {
      const deletion = diffs[pointer - 1][1];
      const insertion = diffs[pointer][1];
      const o1 = __commonOverlap(deletion, insertion);
      const o2 = __commonOverlap(insertion, deletion);
      if (o1 >= o2) {
        if (o1 >= deletion.length / 2 || o1 >= insertion.length / 2) {
          diffs.splice(pointer, 0, [0, insertion.substring(0, o1)]);
          diffs[pointer - 1][1] = deletion.substring(0, deletion.length - o1);
          diffs[pointer + 1][1] = insertion.substring(o1);
          pointer++;
        }
      } else if (o2 >= deletion.length / 2 || o2 >= insertion.length / 2) {
        diffs.splice(pointer, 0, [0, deletion.substring(0, o2)]);
        diffs[pointer - 1][0] = 1;
        diffs[pointer - 1][1] = insertion.substring(0, insertion.length - o2);
        diffs[pointer + 1][0] = -1;
        diffs[pointer + 1][1] = deletion.substring(o2);
        pointer++;
      }
      pointer++;
    }
    pointer++;
  }
}
// @vitest/utils `diffStringsRaw(a, b, true)`: the character diff, cleaned up semantically.
function __diffStringsRaw(a, b) {
  const diffs = [];
  let ai = 0;
  let bi = 0;
  __diffSequences(a.length, b.length, (i, j) => a[i] === b[j], (n, ac, bc) => {
    if (ai !== ac) diffs.push([-1, a.slice(ai, ac)]);
    if (bi !== bc) diffs.push([1, b.slice(bi, bc)]);
    ai = ac + n;
    bi = bc + n;
    diffs.push([0, b.slice(bc, bi)]);
  });
  if (ai !== a.length) diffs.push([-1, a.slice(ai)]);
  if (bi !== b.length) diffs.push([1, b.slice(bi)]);
  __cleanupSemantic(diffs);
  return diffs;
}
// jest-diff `getAlignedDiffs`: the character diffs of two multi-line strings (a newline
// appended to each) as lines, the changed substrings of a line holding common ones inverse.
function __alignedDiffs(diffs) {
  const relevant = (op, line) => line.reduce((r, d) => r + (d[0] === 0 ? d[1] : d[0] === op && d[1].length !== 0 ? __dcol.inverse(d[1]) : ''), '');
  const changeBuffer = (op) => {
    const b = { line: [], lines: [] };
    b.pushLine = () => {
      b.lines.push(b.line.length !== 1 ? [op, relevant(op, b.line)] : b.line[0][0] === op ? b.line[0] : [op, b.line[0][1]]);
      b.line.length = 0;
    };
    b.isLineEmpty = () => b.line.length === 0;
    b.align = (d) => {
      if (d[1].includes('\n')) {
        const subs = d[1].split('\n');
        subs.forEach((sub, i) => {
          if (i < subs.length - 1) { b.line.push([op, sub]); b.pushLine(); }
          else if (sub.length !== 0) b.line.push([op, sub]);
        });
      } else b.line.push(d);
    };
    b.moveLinesTo = (lines) => { if (!b.isLineEmpty()) b.pushLine(); lines.push(...b.lines); b.lines.length = 0; };
    return b;
  };
  const del = changeBuffer(-1);
  const ins = changeBuffer(1);
  const lines = [];
  const pushChange = (d) => {
    const empty = d[1].length === 0;
    if (!empty || del.isLineEmpty()) del.line.push(d);
    if (!empty || ins.isLineEmpty()) ins.line.push(d);
  };
  const flush = () => { del.moveLinesTo(lines); ins.moveLinesTo(lines); };
  for (const d of diffs) {
    if (d[0] === -1) del.align(d);
    else if (d[0] === 1) ins.align(d);
    else if (d[1].includes('\n')) {
      const subs = d[1].split('\n');
      subs.forEach((sub, i) => {
        if (i === 0) {
          if (del.isLineEmpty() && ins.isLineEmpty()) { flush(); lines.push([0, sub]); }
          else { pushChange([0, sub]); flush(); }
        } else if (i < subs.length - 1) lines.push([0, sub]);
        else if (sub.length !== 0) pushChange([0, sub]);
      });
    } else pushChange(d);
  }
  flush();
  return lines;
}
// @vitest/utils `diffStringsUnified` for two different non-empty multi-line strings: aligned
// character diffs when they share a substring (but the appended newline), else a line diff.
function __diffStringsUnified(a, b) {
  const diffs = __diffStringsRaw(`${a}\n`, `${b}\n`);
  const last = diffs.length - 1;
  if (diffs.some((d, i) => d[0] === 0 && (i !== last || d[1] !== '\n'))) return __annotation() + __joinDiffs(__alignedDiffs(diffs));
  return __diffLines(a.split('\n'), b.split('\n'), undefined, undefined, true);
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
// @vitest/utils `diff(expected, received)`; `annotate` false is its `omitAnnotationLines: true`.
function __vdiff(a, b, annotate = false) {
  if (Object.is(a, b)) return '';
  let type = __diffType(a);
  let omitDifference = false;
  // an asymmetric matcher diffs as its expected type (vitest); a user-defined one (no vitest
  // `$$typeof`) or one without getExpectedType (expect.anything) has no diff
  if (type === 'object' && typeof a.asymmetricMatch === 'function') {
    if (a.$$typeof !== Symbol.for('jest.asymmetricMatcher')) return undefined;
    if (typeof a.getExpectedType !== 'function') return undefined;
    type = a.getExpectedType();
    omitDifference = type === 'string';
  }
  const fmt = { indent: 2, basic: false, maxDepth: 20, errors: true };
  const fallback = { ...fmt, maxDepth: 8, toJSON: false };
  if (type !== __diffType(b)) {
    return `${__dcol.green(`- ${__diffAnno[0]}:`)}\n${__pf(a, fallback)}\n\n${__dcol.red(`+ ${__diffAnno[1]}:`)}\n${__pf(b, fallback)}`;
  }
  if (omitDifference) return undefined;
  const noDiff = __dcol.dim(__NO_DIFF);
  const objects = (opts) => {
    const ac = __pf(a, { ...opts, indent: 0 });
    const bc = __pf(b, { ...opts, indent: 0 });
    if (ac === bc) return noDiff;
    return __diffLines(__pf(a, opts).split('\n'), __pf(b, opts).split('\n'), ac.split('\n'), bc.split('\n'), annotate);
  };
  const sortMap = (x) => new Map([...x.entries()].sort());
  const sortSet = (x) => new Set([...x.values()].sort());
  switch (type) {
    case 'string': return __diffLines(a.split('\n'), b.split('\n'), undefined, undefined, annotate);
    case 'boolean':
    case 'number': {
      const af = __pf(a, fmt);
      const bf = __pf(b, fmt);
      return af === bf ? '' : __diffLines(af.split('\n'), bf.split('\n'), undefined, undefined, annotate);
    }
    case 'map': a = sortMap(a); b = sortMap(b); break;
    case 'set': a = sortSet(a); b = sortSet(b); break;
  }
  let d;
  let threw = false;
  try { d = objects(fmt); } catch (e) { threw = true; }
  if (d === undefined || d === noDiff) {
    d = objects(fallback);
    if (d !== noDiff && !threw) d = `${__dcol.dim('Compared values serialize to the same structure.\nPrinting internal object structure without calling `toJSON` instead.')}\n\n${d}`;
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
function __formatReturns(spy, msg, shown, results = spy.mock.results) {
  if (results.length) {
    msg += `\n\nReceived:\n\n${results.map((r, i) => {
      let s = `  ${__ordinal(i + 1)} ${spy.getMockName()} call return:\n\n`;
      s += shown ? __vdiff(shown, r.value) : __stringify(r).split('\n').map((l) => `    ${l}`).join('\n');
      return `${s}\n`;
    }).join('\n')}`;
  }
  return `${msg}\n\nNumber of calls: ${spy.mock.calls.length}\n`;
}
// @vitest/expect getObjectSubset: `object` cut down to the keys `subset` names (the expected
// subset itself where they match), keeping the constructor; `stripped` counts what was cut.
function __objectSubset(object, subset) {
  let stripped = 0;
  const isObj = (x) => x != null && typeof x === 'object' && !Array.isArray(x);
  const keys = (x) => [...Object.keys(x), ...Object.getOwnPropertySymbols(x).filter((k) => Object.getOwnPropertyDescriptor(x, k)?.enumerable)];
  const inObj = (o, k) => !(!o || typeof o !== 'object' || o === Object.prototype) && (Object.hasOwn(o, k) || inObj(Object.getPrototypeOf(o), k));
  const withContext = (seen = new WeakMap()) => (object, subset) => {
    if (Array.isArray(object)) {
      if (Array.isArray(subset) && subset.length === object.length) return subset.map((sub, i) => withContext(seen)(object[i], sub));
    } else if (object instanceof Date) {
      return object;
    } else if (isObj(object) && isObj(subset)) {
      if (__subsetMatches(object, subset)) return subset;
      const trimmed = {};
      seen.set(object, trimmed);
      if (typeof object.constructor === 'function' && typeof object.constructor.name === 'string') {
        Object.defineProperty(trimmed, 'constructor', { enumerable: false, value: object.constructor });
      }
      for (const key of keys(object)) {
        if (inObj(subset, key)) {
          trimmed[key] = seen.has(object[key]) ? seen.get(object[key]) : withContext(seen)(object[key], subset[key]);
        } else if (!seen.has(object[key])) {
          stripped += 1;
          if (isObj(object[key])) stripped += keys(object[key]).length;
          withContext(seen)(object[key], subset[key]);
        }
      }
      if (keys(trimmed).length > 0) return trimmed;
    }
    return object;
  };
  return { subset: withContext()(object, subset), stripped };
}
// vitest's toMatchObject comparison: equals with [iterableEquality, subsetEquality].
function __subsetMatches(a, e) {
  return __equals(a, e, [...__customTesters, __iterableEquality, __subsetEquality]);
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
    // its tests are `each` tasks (vitest's `task.each`)
    const outer = __inEach;
    __inEach = true;
    cases.forEach((row, idx) => {
      const items = Array.isArray(row) ? row : [row];
      const args = spread ? items : [row];
      // the handler gets the row only, never the test context (vitest)
      const fn = () => (typeof handler === 'function' ? handler(...args) : undefined);
      register(__formatTitle(title, items, idx), fn, opts);
    });
    __inEach = outer;
  };
}

// `.for` (turbo-test#68): like `.each`, but the row is passed as ONE argument (never spread),
// followed by the test context for a test (with its fixtures under it.extend, turbo-test#96). A
// `describe.for` body gets the row only.
function __for(register, cases, targs, isSuite) {
  cases = __eachCases(cases, targs);
  return (name, a, b) => {
    const title = __formatName(name);
    const fnFirst = typeof a === 'function';
    const handler = fnFirst ? a : b;
    const opts = fnFirst ? b : a;
    cases.forEach((item, idx) => {
      const fn = (ctx) => (typeof handler !== 'function' ? undefined : isSuite ? handler(item) : handler(item, ctx));
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
let __inEach = false;
function __addTest(t) {
  if (globalThis.__TT_TASK_LOCATION) t.location = globalThis.__ttCallSite();
  // where it was registered: a timeout's stack (vitest's `STACK_TRACE_ERROR`, turbo-test#102)
  t.stackTrace = new Error('STACK_TRACE_ERROR');
  if (__inEach) t.each = true;
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
  // the fixtures are set on the test's context (turbo-test#96), which the fixture functions get
  const resolveCtx = async (ctx) => {
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
  const wrap = (userFn) => async (ctx) => userFn(await resolveCtx(ctx));
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
// beforeEach / afterEach hooks get the test's context (turbo-test#96)
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
// A failure as vitest's reporters print it (@vitest/utils processError + printErrorInner):
// name, message, the diff of expected / actual, the stack and the cause (turbo-test#81), and for
// what is not an Error with a message (turbo-test#97): a thrown primitive is `{ message }`
// (printed `Unknown Error: <value>`), a value with no message prints as its serialized form
// (`inspected`), and the serialized error's own properties are vitest's `Serialized Error`
// (`properties`). `no_name` / `no_message`: the serialized error has no `name` / `message`
// (junit leaves the `type` / `message` attribute out).
function __errorInfo(e) {
  const ser = __serializeError(e);
  let diff = '';
  const diffable = e !== null && typeof e === 'object' && (e.showDiff || (e.showDiff === undefined && e.expected !== undefined && e.actual !== undefined));
  if (diffable) {
    try { diff = __printDiffOrStringify(e.actual, e.expected); } catch (err) { diff = ''; }
  }
  const cause = e !== null && typeof e === 'object' && e.cause && typeof e.cause === 'object' && 'name' in e.cause ? __errorInfo(e.cause) : null;
  // an Error with an empty message keeps the `Name: ` line (vitest prints its serialized form
  // with its parsed frames, not modelled)
  const inspected = !ser.message && !(e instanceof Error) ? __nodeInspect(__plainOf({ ...ser, stacks: [] })) : null;
  const props = __errorProperties(ser);
  // vite's details of a module's parse / transform error (printErrorInner's
  // `__vitest_rollup_error__` lines, turbo-test#113)
  const re = e !== null && typeof e === 'object' ? e.__vitest_rollup_error__ : undefined;
  const rollup = re && typeof re === 'object'
    ? { plugin: re.plugin ? String(re.plugin) : '', id: re.id ? String(re.id) : '', loc: re.loc ? `${re.loc.line}:${re.loc.column}` : '', frame: re.frame ? String(re.frame) : '' }
    : null;
  return {
    name: ser.name ? String(ser.name) : 'Unknown Error',
    message: ser.message === undefined ? '' : String(ser.message),
    no_name: ser.name === undefined,
    no_message: ser.message === undefined,
    diff: diff || '',
    stack: typeof ser.stack === 'string' ? ser.stack : '',
    cause,
    inspected,
    properties: props === null ? null : __inspect(props),
    rollup,
    // processError's prettified actual / expected and the error's showDiff (vitest's TAP
    // block, turbo-test#170)
    show_diff: !!(e !== null && typeof e === 'object' && e.showDiff),
    actual: diffable ? __prettifyValue(e.actual) : null,
    expected: diffable ? __prettifyValue(e.expected) : null,
  };
}
// @vitest/utils prettifyValue: a string as it is, anything else pretty-formatted as the diff's
// values (FORMAT_OPTIONS)
function __prettifyValue(v) {
  if (typeof v === 'string') return v;
  try { return __pf(v, { indent: 2, basic: false, maxDepth: 20, errors: true }); } catch (err) { return String(v); }
}
// @vitest/utils processError's serialization: a primitive (or any falsy value) is `{ message }`,
// anything else serializeValue's clone.
function __serializeError(e) {
  if (!e || typeof e !== 'object') return { message: String(e) };
  try { return __serializeValue(e); } catch (err) { return __serializeValue(new Error(`Failed to fully serialize error: ${err && err.message}\nInner error message: ${e && e.message}`)); }
}
// vitest's JSON `failureMessages` entry: the serialized error's stack, else its message, else null
function __failureMessage(e) {
  const s = __serializeError(e);
  const m = s.stack || s.message;
  return m === undefined || m === null ? null : String(m);
}
const __unserializable = (err) => (err instanceof Error ? `<unserializable>: ${err.message}` : typeof err === 'string' ? `<unserializable>: ${err}` : '<unserializable>');
// @vitest/utils serializeValue: functions as `Function<name>`, objects as null-prototype clones
// of their own and inherited (up to Object.prototype) properties.
function __serializeValue(val, seen = new WeakMap()) {
  if (!val || typeof val === 'string') return val;
  if (val instanceof Error && 'toJSON' in val && typeof val.toJSON === 'function') {
    const j = val.toJSON();
    if (j && j !== val && typeof j === 'object') {
      try { if (typeof val.message === 'string') j.message ??= val.message; } catch {}
      try { if (typeof val.stack === 'string') j.stack ??= val.stack; } catch {}
      try { if (typeof val.name === 'string') j.name ??= val.name; } catch {}
      try { if (val.cause != null) j.cause ??= __serializeValue(val.cause, seen); } catch {}
    }
    return __serializeValue(j, seen);
  }
  if (typeof val === 'function') return `Function<${val.name || 'anonymous'}>`;
  if (typeof val === 'symbol') return val.toString();
  if (typeof val !== 'object') return val;
  if (typeof Buffer !== 'undefined' && val instanceof Buffer) return `<Buffer(${val.length}) ...>`;
  if (val instanceof Uint8Array) return `<Uint8Array(${val.length}) ...>`;
  if (val instanceof Promise) return 'Promise';
  if (typeof Element !== 'undefined' && val instanceof Element) return val.tagName;
  if (typeof val.toJSON === 'function') return __serializeValue(val.toJSON(), seen);
  if (seen.has(val)) return seen.get(val);
  if (Array.isArray(val)) {
    const clone = new Array(val.length);
    seen.set(val, clone);
    val.forEach((x, i) => { try { clone[i] = __serializeValue(x, seen); } catch (err) { clone[i] = __unserializable(err); } });
    return clone;
  }
  const clone = Object.create(null);
  seen.set(val, clone);
  for (let obj = val; obj && obj !== Object.prototype; obj = Object.getPrototypeOf(obj)) {
    for (const key of Object.getOwnPropertyNames(obj)) {
      if (key in clone) continue;
      try { clone[key] = __serializeValue(val[key], seen); } catch (err) { delete clone[key]; clone[key] = __unserializable(err); }
    }
  }
  if (val instanceof Error) { try { clone.message = val.message; } catch {} }
  return clone;
}
// The serialized error as the reporters see it (structured-cloned to the main process): plain
// objects and arrays.
function __plainOf(v, seen = new Map()) {
  if (v === null || typeof v !== 'object') return v;
  if (seen.has(v)) return seen.get(v);
  const out = Array.isArray(v) ? [] : {};
  seen.set(v, out);
  for (const k of Object.keys(v)) out[k] = __plainOf(v[k], seen);
  return out;
}
// vitest's getErrorProperties: the serialized error's own properties but its stack, the runner's
// keys and Error.prototype / Object.prototype names; none for an AssertionError. null for none.
const __skipErrorProperties = new Set(['cause', 'stacks', 'type', 'showDiff', 'ok', 'operator', 'diff', 'codeFrame', 'actual', 'expected', 'diffOptions', 'runnerError', 'sourceURL', 'column', 'line', 'fileName', 'lineNumber', 'columnNumber', 'VITEST_TEST_NAME', 'VITEST_TEST_PATH', '__vitest_rollup_error__', '__vitest_error_context__', ...Object.getOwnPropertyNames(Error.prototype), ...Object.getOwnPropertyNames(Object.prototype)]);
function __errorProperties(ser) {
  if (ser.name === 'AssertionError') return null;
  const out = {};
  let any = false;
  for (const key of Object.getOwnPropertyNames(ser)) {
    if (key === 'stack' ? ser[key] != null && typeof ser[key] !== 'string' : !__skipErrorProperties.has(key)) {
      out[key] = __plainOf(ser[key]);
      any = true;
    }
  }
  return any ? out : null;
}
globalThis.__ttErrorInfo = (e) => __errorInfo(e);
// @vitest/utils printDiffOrStringify(received, expected)
function __printDiffOrStringify(received, expected) {
  if (typeof expected === 'string' && typeof received === 'string' && expected.length > 0 && received.length > 0
    && expected.length <= 20000 && received.length <= 20000 && expected !== received) {
    if (expected.includes('\n') || received.includes('\n')) return __diffStringsUnified(expected, received);
    // the changed substrings inverse when the strings share any; trailing spaces as `·`
    const diffs = __diffStringsRaw(expected, received);
    const common = diffs.some((d) => d[0] === 0);
    const side = (op) => diffs.reduce((r, d) => r + (d[0] === 0 ? d[1] : d[0] === op ? (common ? __dcol.inverse(d[1]) : d[1]) : ''), '');
    const shown = (v) => `"${v}"`.replace(/\s+$/gm, (sp) => '·'.repeat(sp.length));
    return `Expected: ${__dcol.green(shown(side(-1)))}\nReceived: ${__dcol.red(shown(side(1)))}`;
  }
  return __vdiff(expected, received, true) || '';
}
function __testRecord(t, chain, status, extra) {
  return {
    name: chain.map((s) => s.name).filter(Boolean).concat(t.name).join(' > '),
    title: t.name,
    ancestors: chain.slice(1).map((s) => s.name),
    status,
    duration_ms: 0,
    location: t.location,
    // its ctx.annotate() calls: [type, message, line, column] (turbo-test#96)
    annotations: t.task ? t.task.annotations.map((a) => [String(a.type), String(a.message), String(a.location ? a.location.line : 0), String(a.location ? a.location.column : 0)]) : [],
    ...extra,
  };
}

// ---- the test context (turbo-test#96): what vitest 4.1.12 passes every test function, and its
// beforeEach / afterEach hooks (@vitest/runner `createTestContext` + the runner's `expect`).
// What `ctx.skip()` throws: the test is reported skipped (with the note) at run time.
class __PendingError extends Error {
  constructor(note) {
    super('test is skipped; abort execution');
    this.code = 'VITEST_PENDING';
    this.note = note;
  }
}
// @vitest/runner `generateHash`: the file task's id (child ids append `_<index>`)
function __taskHash(s) {
  let h = 0;
  for (let i = 0; i < s.length; i++) { h = (h << 5) - h + s.charCodeAt(i); h &= h; }
  return `${h}`;
}
// The file's task (`ctx.task.file`): named relative to the run's root (native `__ttFileName`).
let __fileTask = null;
function __fileTaskOf() {
  if (__fileTask && __fileTask.filepath === globalThis.__ttFile) return __fileTask;
  const name = globalThis.__ttFileName || '';
  const file = { type: 'suite', name, fullName: name, filepath: globalThis.__ttFile, id: __taskHash(name), mode: 'run', meta: Object.create(null), projectName: '', tags: [], result: undefined };
  file.file = file;
  Object.defineProperty(file, 'tasks', { enumerable: true, get: () => root.tasks.map((c) => (c.suite ? __suiteTaskOf(c, [root]) : __testTaskOf(c, [root]))) });
  __fileTask = file;
  return file;
}
// A describe block's task (`ctx.task.suite`); `chain` is the suites enclosing it, root first.
function __suiteTaskOf(s, chain) {
  if (s.task) return s.task;
  const file = __fileTaskOf();
  const parent = chain[chain.length - 1];
  const names = chain.slice(1).map((x) => x.name).concat(s.name);
  const up = chain.length > 1 ? __suiteTaskOf(parent, chain.slice(0, -1)) : undefined;
  const task = { type: 'suite', name: s.name, id: `${(up || file).id}_${parent.tasks.indexOf(s)}`, mode: s.mode || 'run', file, suite: up, fullName: [file.name, ...names].join(' > '), fullTestName: names.join(' > '), meta: Object.create(null), each: undefined, concurrent: undefined, shuffle: undefined, tags: [], result: undefined };
  Object.defineProperty(task, 'tasks', { enumerable: true, get: () => s.tasks.map((c) => (c.suite ? __suiteTaskOf(c, [...chain, s]) : __testTaskOf(c, [...chain, s]))) });
  s.task = task;
  return task;
}
// A test's task (`ctx.task`), created once (it outlives retries).
function __testTaskOf(t, chain) {
  if (t.task) return t.task;
  const file = __fileTaskOf();
  const parent = chain[chain.length - 1];
  const up = chain.length > 1 ? __suiteTaskOf(parent, chain.slice(0, -1)) : undefined;
  const names = chain.slice(1).map((x) => x.name).concat(t.name);
  t.task = {
    type: 'test', name: t.name, id: `${(up || file).id}_${parent.tasks.indexOf(t)}`, mode: 'run', file, suite: up,
    fullName: [file.name, ...names].join(' > '), fullTestName: names.join(' > '), meta: Object.create(null),
    each: t.each || undefined, fails: t.fails || undefined, retry: t.retry || undefined, repeats: undefined,
    timeout: (typeof t.timeout === 'number' && t.timeout > 0) ? t.timeout : __defaultTimeout(), shuffle: undefined,
    tags: [], annotations: [], artifacts: [], result: undefined,
  };
  return t.task;
}
// The context itself: a function (calling it is the deprecated `done` callback), created with
// the task; `expect` is a lazily made expect of its own (non-enumerable, as vitest's).
function __contextOf(t, chain) {
  if (t.context) return t.context;
  const task = __testTaskOf(t, chain);
  const context = function () { throw new Error('done() callback is deprecated, use promise instead'); };
  const abort = new AbortController();
  context.signal = abort.signal;
  context.task = task;
  context.skip = (condition, note) => {
    if (condition === false) return undefined;
    task.result.pending = true;
    throw new __PendingError(typeof condition === 'string' ? condition : note);
  };
  context.annotate = (message, type, attachment) => {
    if (task.result && task.result.state !== 'run') {
      throw new Error(`Cannot annotate tests outside of the test run. The test "${task.name}" finished running with the "${task.result.state}" state already.`);
    }
    const annotation = { message, type: typeof type === 'object' || type === undefined ? 'notice' : type };
    const att = typeof type === 'object' ? type : attachment;
    if (att) annotation.attachment = att;
    const site = globalThis.__ttCallSite();
    if (site) annotation.location = { file: globalThis.__ttFile, line: site[0], column: site[1] };
    task.annotations.push(annotation);
    return Promise.resolve(annotation);
  };
  context.onTestFailed = (fn) => { t.onFailed.push(fn); };
  context.onTestFinished = (fn) => { t.onFinished.push(fn); };
  let ownExpect;
  Object.defineProperty(context, 'expect', { get: () => (ownExpect ||= new Proxy(globalThis.expect, {})) });
  Object.defineProperty(task, 'context', { value: context, configurable: true });
  t.abort = abort;
  t.context = context;
  return context;
}
// onTestFinished (last registered first) / onTestFailed handlers: each gets the context and a
// throw fails the test; registering another inside one throws, as vitest's `callTestHooks`.
async function __callTestHooks(t, hooks, fail) {
  const ctx = t.context;
  const keep = [ctx.onTestFailed, ctx.onTestFinished];
  ctx.onTestFailed = () => { throw new Error('Cannot call "onTestFailed" inside a test hook.'); };
  ctx.onTestFinished = () => { throw new Error('Cannot call "onTestFinished" inside a test hook.'); };
  for (const fn of hooks.slice().reverse()) {
    __loop.ticks++;
    try { await fn(ctx); } catch (e) { fail(e); }
  }
  [ctx.onTestFailed, ctx.onTestFinished] = keep;
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
  // a describe without tasks (failing, or a describe.todo / .skip one) is a TAP task of its own
  // (turbo-test#170): where it sits among the tests, its mode and its describe path
  if (ancestors.length && !suite.tasks.length) {
    summary.empty_suites.push([String(summary.tests.length), skipAll || 'run', ...ancestors.slice(1).map((x) => x.name), suite.name].join('\u001f'));
  }
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
  // console output from here on belongs to this suite (the file for the root suite)
  const outerTask = __ttLogTask;
  const suiteTask = ancestors.length ? [...ancestors.slice(1), suite].map((x) => x.name).join(' > ') : '';
  __ttLogTask = suiteTask;
  // a suite's hook errors (vitest's failed suite, a junit testcase of its own: turbo-test#81)
  const suiteStart = __loop.elapsed();
  const suiteErrors = [];
  // in vitest's getSuites order (a suite before the suites inside it)
  const suiteSlot = summary.suite_errors.push(null) - 1;
  // the functions its beforeAll hooks return: run after its afterAll hooks, last returned first
  // (vitest's `sequence.hooks: 'stack'`); a throwing beforeAll drops them all (turbo-test#101)
  let baCleanups = [];
  if (!skipAll && !idle && !forbidden) {
    const returned = [];
    for (const h of suite.hooks.ba) {
      try { returned.push(await h()); } catch (e) {
        summary.failures.push(`${suite.name || '<root>'} beforeAll: ${(e && e.message) || String(e)}`);
        suiteErrors.push(__errorInfo(e));
        hookFailed = true;
        failed = true;
        break;
      }
    }
    if (!hookFailed) baCleanups = returned;
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
    // vitest keeps every failed attempt's error and counts the retries (`(retry x1)`)
    const attemptErrors = [];
    let tries = 0;
    // Real wall time for the per-test start, and for the duration the reporters show plus the
    // drive loop's clock jumps (turbo-test#95). NOT performance.now() (fake timers replace it) nor Date.now() (fake timers /
    // vi.setSystemTime replace it).
    const __t0 = __loop.elapsed();
    const __start = __loop.realNow();
    __ttTestLogs = [];
    __ttLogTask = chain.slice(1).map((s) => s.name).concat(t.name).join(' > ');
    // its context (turbo-test#96): `task.result` while it runs, as vitest's
    const ctx = __contextOf(t, chain);
    const task = t.task;
    task.result = { state: 'run', startTime: __start, retryCount: 0, repeatCount: 0 };
    // a failing onTestFinished / onTestFailed handler fails the test (not a skipped one)
    const failWith = (e) => {
      if (task.result.pending) return;
      ok = false;
      lastErr = e;
      attemptErrors.push(e);
      task.result.state = 'fail';
      task.result.errors = attemptErrors;
    };
    for (let a = 0; a < attempts && !ok; a++) {
      tries++;
      task.result.retryCount = a;
      task.result.state = 'run';
      t.onFinished = [];
      t.onFailed = [];
      lastErr = undefined;
      // Reset per-test snapshot + assertion-count state (snapshot counter is keyed by test name,
      // so distinct multi-snapshot tests don't collide; assertion enforcement resets each attempt).
      __snap.testName = label;
      __snap.currentTest = t;
      for (const k of Object.keys(__snap.counters)) if (k === label || k.startsWith(`${label} > `)) __snap.counters[k] = 0;
      __snap.assertCount = 0;
      __snap.expectedAssertions = null;
      __snap.hasAssertions = false;
      // this attempt's errors start here; the functions its beforeEach hooks return run after
      // its afterEach hooks (a throwing beforeEach drops them all, as vitest: turbo-test#101)
      const mark = attemptErrors.length;
      let beCleanups = [];
      try {
        __clearModuleMocks();
        const returned = [];
        for (const s of chain) for (const h of s.hooks.be) { __loop.ticks++; returned.push(await h(ctx)); }
        beCleanups = returned;
        // Race t.fn() against a timeout. The timeout is an INTERNAL one-shot timer (separate
        // from the user/fake-timer queue), so the Rust drive loop advances the virtual clock to
        // it and rejects even a test that never resolves (e.g. `await new Promise(() => {})`) —
        // otherwise that hangs the whole worker — while staying invisible to vi.runAllTimers etc.
        let timer;
        const timed = new Promise((_, rej) => {
          timer = __scheduleInternal(() => {
            // vitest's message; its stack is the test's registration (`it(`), as vitest's
            const err = new Error(`Test timed out in ${timeoutMs}ms.\nIf this is a long-running test, pass a timeout value as the last argument or configure it globally with "testTimeout".`);
            // up to its frame in the test file (vitest collects a describe's tests after the
            // describe( call returned: no enclosing describe( frames)
            const frames = String(t.stackTrace.stack).split('\n');
            const own = frames.findIndex((l, i) => i > 0 && !!globalThis.__ttFile && l.includes(globalThis.__ttFile));
            err.stack = (own < 0 ? frames : frames.slice(0, own + 1)).join('\n');
            // the context's signal aborts with the timeout error (vitest's `withTimeout`)
            t.abort.abort(err);
            rej(err);
          }, timeoutMs);
        });
        try {
          // called as a plain function, as vitest calls it (its stack frame has no method name)
          const fn = t.fn;
          __loop.ticks++;
          await Promise.race([Promise.resolve().then(() => fn(ctx)), timed]);
        } finally {
          __clearInternal(timer);
        }
        // expect.assertions(n) / hasAssertions() enforcement (vitest: checked after the test body).
        if (__snap.expectedAssertions != null && __snap.assertCount !== __snap.expectedAssertions) {
          throw new Error(`expected number of assertions to be ${__snap.expectedAssertions}, but got ${__snap.assertCount}`);
        }
        if (__snap.hasAssertions && __snap.assertCount === 0) throw __snap.hasAssertions;
        ok = true;
      } catch (e) {
        // ctx.skip(): the test is skipped, with its note (a caught one skips it, without a note)
        if (e instanceof __PendingError) {
          task.result.pending = true;
          task.result.note = e.note;
          task.result.state = 'skip';
        } else lastErr = e;
      }
      if (task.result.state === 'run') task.result.state = ok ? 'pass' : 'fail';
      if (!ok && !task.result.pending) {
        attemptErrors.push(lastErr);
        task.result.errors = attemptErrors;
      }
      // afterEach (incl. @testing-library cleanup) runs even when the test failed — otherwise a
      // failed test's mounted DOM leaks into the next test — innermost suite first, each suite's
      // last registered first, then the beforeEach cleanups, last returned first (vitest's
      // `sequence.hooks: 'stack'`). A throw fails the test, next to its own error, and stops
      // the hooks after it (turbo-test#101).
      try {
        for (const s of [...chain].reverse()) for (const h of s.hooks.ae.slice().reverse()) { __loop.ticks++; await h(ctx); }
        for (const c of beCleanups.slice().reverse()) if (typeof c === 'function') { __loop.ticks++; await c(); }
      } catch (e) { failWith(e); }
      // then its onTestFinished handlers, and onTestFailed ones when it failed (vitest's order)
      await __callTestHooks(t, t.onFinished, failWith);
      if (task.result.state === 'fail') await __callTestHooks(t, t.onFailed, failWith);
      __snap.currentTest = null;
      // it.fails: invert the outcome once its hooks ran (vitest flips the final state) — an
      // error is the success condition, a clean pass fails.
      if (t.fails && !task.result.pending) {
        if (attemptErrors.length > mark) { ok = true; lastErr = undefined; attemptErrors.length = mark; }
        else { ok = false; lastErr = new Error('expected test to fail, but it passed'); attemptErrors.push(lastErr); }
      }
      if (task.result.pending) break;
    }
    const __dur = __loop.elapsed() - __t0;
    const timing = { duration_ms: __dur, start_ms: __start, retry_count: tries - 1, logs: __ttTestLogs.map((l) => [l[0], l[1]]) };
    __ttTestLogs = null;
    __ttLogTask = suiteTask;
    // skipped at run time (ctx.skip): its duration and note (turbo-test#96); its snapshots
    // count as checked, as a skipped test's
    if (task.result.pending) {
      task.mode = 'skip';
      task.result = { state: 'skip', note: task.result.note, pending: true, duration: __dur };
      summary.skipped++;
      summary.skippedNames.push(label);
      summary.tests.push(__testRecord(t, chain, 'skipped', { ...timing, note: task.result.note === undefined ? '' : String(task.result.note) }));
      continue;
    }
    task.result.duration = __dur;
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
      summary.tests.push(__testRecord(t, chain, 'failed', { ...timing, message: msg, failure_messages: attemptErrors.map(__failureMessage), errors: attemptErrors.map(__errorInfo) }));
    }
  }
  // afterAll last registered first, then the beforeAll cleanups (last returned first); a throw
  // fails the suite and stops the hooks after it (vitest, turbo-test#101)
  if (!skipAll && !idle && !forbidden) {
    try {
      for (const h of suite.hooks.aa.slice().reverse()) await h();
      for (const c of baCleanups.slice().reverse()) if (typeof c === 'function') await c();
    } catch (e) {
      summary.failures.push(`${suite.name || '<root>'} afterAll: ${(e && e.message) || String(e)}`);
      suiteErrors.push(__errorInfo(e));
      failed = true;
    }
  }
  if (suiteErrors.length && ancestors.length) summary.suite_errors[suiteSlot] = { name: suite.name, duration_ms: __loop.elapsed() - suiteStart, errors: suiteErrors };
  __ttLogTask = outerTask;
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
    const summary = { passed: 0, failed: 0, skipped: 0, failures: [], tests: [], suites: [], skippedNames: [], suite_errors: [], empty_suites: [] };
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
    const waited = __loop.waited;
    await runSuite(root, [], summary);
    summary.waited_ms = __loop.waited - waited;
    summary.snapshot = __snapFinish(summary.skippedNames);
    summary.logs = __ttFileLogs.map((l) => [l[0], l[3], l[1]]);
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
};

// Per-file reset, modeled on vitest's isolate:false loop (vitest/dist .../base*.js): with
// isolation OFF, vitest does NOT reset modules, the DOM, spies, timers or globals between files
// — it only calls `vi.resetConfig()` + `vi.restoreAllMocks()`, and its runner scopes test
// collection per file. We mirror that minimally. Over-resetting (clearing DOM, truncating the
// spy registry, unstubbing globals, poking React internals) was what broke interaction tests
// ~100 files into a shared isolate — vitest proves none of that is needed.
globalThis.__ttResetForNextFile = () => {
  __ttFileLogs = [];
  __ttLogTask = '';
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
  // each file starts on real timers (vitest's fresh environment per file): fake timers a file
  // left on would otherwise hold every timer of the next one (turbo-test#168)
  if (__loop.fake) __uninstallFake();
  try { globalThis.vi && globalThis.vi.resetConfig && globalThis.vi.resetConfig(); } catch (e) {}
  // the next file's config is set afresh (its project's test.fakeTimers, turbo-test#180)
  __ttConfig = __ttConfigDefaults();
  __ttConfigSaved = null;
  // NOTE: we do NOT call restoreAllMocks() here. A spyOn spy's mockRestore() redefines a
  // (possibly shared, cached) module export back to the `orig` captured at spy time. This
  // suite's own setup.ts afterEach already restores per test; calling it again between files
  // re-applies a now-stale orig onto the shared analytics module — corrupting trackEvent for
  // every later file (asserts then see the wrong/old spy). Per-file restore is the test's job.
};
