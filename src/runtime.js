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
function __schedule(cb, delay, interval, fake, immediate, clamped) {
  const id = __loop.seq++;
  // `immediate`: a setImmediate (jest's legacy runAllImmediates)
  if (fake) __loop.fmacro.push({ id, due: __loop.fnow + Math.max(0, delay | 0), cb, interval, gen: __loop.gen, immediate });
  else {
    const due = __loop.now + Math.max(0, delay | 0);
    // `ord`: where the timer runs. A zero-delay timer (`clamped`) is due now but runs where Node's
    // 1 ms timer does, after the immediates and messages queued alongside it (turbo-test#237)
    __loop.macro.push({ id, due, ord: clamped ? due + 1 : due, cb, interval, gen: __loop.gen });
  }
  return id;
}
const __byOrd = (a, b) => a.ord - b.ord || a.id - b.id;
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
function __fireInternalTimer(it) {
  __loop.now = Math.max(__loop.now, it.due);
  __clearInternal(it.id);
  try { it.cb(); } catch (e) { __reportUncaught(e); }
}
function __fireMacroTimer(m) {
  __loop.now = Math.max(__loop.now, m.due);
  if (m.interval == null) __clearTimer(m.id);
  else m.ord = m.due = __loop.now + m.interval;
  try { m.cb(); } catch (e) { __reportUncaught(e); }
}
function __stepMacroPick(m, it) {
  if (!m && !it) return null;
  // earliest-due wins; ties favor the user timer (matches scheduling order intent).
  const useInternal = it && (!m || it.due < m.due);
  return { useInternal, due: useInternal ? it.due : m.due };
}
__loop.stepMacro = function () {
  __loop.ticks++;
  __dropStaleTimers();
  __loop.macro.sort(__byOrd);
  __loop.internal.sort((a, b) => a.due - b.due || a.id - b.id);
  const m = __loop.macro[0];
  const it = __loop.internal[0];
  const pick = __stepMacroPick(m, it);
  if (!pick) return false;
  if (pick.due > __loop.now) __loop.waited += pick.due - __loop.now;
  if (pick.useInternal) __fireInternalTimer(it);
  else __fireMacroTimer(m);
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
// zeroDelayTimers (on by default; --no-zero-delay-timers, TURBO_ZERO_DELAY_TIMERS=0 or
// `test.zeroDelayTimers: false` turn it off): a real timer's delay below 1 ms stays 0 ms of the
// clock, as in turbo-test 0.4.2 (faster for timer-heavy suites), but keeps the order of Node's
// 1 ms timer (turbo-test#237). Off, it is Node's 1 ms clamp (turbo-test#212).
let __ttZeroDelay = true;
const __nodeDelay = (d) => { const n = Number(d); return n >= 1 && n <= 2147483647 ? n : __ttZeroDelay ? 0 : 1; };
// a real timer zeroDelayTimers took below Node's 1 ms clamp: it keeps Node's order (__schedule)
const __clamped = (d) => __ttZeroDelay && !(Number(d) >= 1 && Number(d) <= 2147483647);
function __timerFns(fake) {
  return {
    // Node makes a real timer's delay below 1 ms (0, NaN, negative) 1 ms, so a setImmediate or a
    // MessageChannel message queued after a setTimeout(fn, 0) runs first (turbo-test#212); that
    // clamp applies with zeroDelayTimers off (above). The fake clock (sinon) keeps 0
    setTimeout: (cb, d, ...a) => __schedule(() => cb(...a), fake ? d : __nodeDelay(d), null, fake, false, !fake && __clamped(d)),
    setInterval: (cb, d, ...a) => __schedule(() => cb(...a), fake ? d : __nodeDelay(d), Math.max(1, d | 0), fake, false, !fake && __clamped(d)),
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
function __collectPerfMethodNames(real) {
  const names = new Set();
  for (let o = real; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) {
    for (const k of Object.getOwnPropertyNames(o)) names.add(k);
  }
  names.delete('constructor');
  names.delete('now');
  return names;
}
function __stubPerfMethod(real, k) {
  let v;
  try { v = real[k]; } catch (e) { return null; }
  if (typeof v !== 'function') return null;
  return k.indexOf('getEntries') === 0 ? () => [] : () => {};
}
function __fakePerformance(real) {
  const perf = Object.create(null);
  for (const k of __collectPerfMethodNames(real)) {
    const stub = __stubPerfMethod(real, k);
    if (stub) perf[k] = stub;
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
// Under isolate reuse the worker's config once its setup files ran (__ttCaptureConfigBaseline):
// vitest `isolate: false` re-runs the setup files per file, so their vi.setConfig holds for each.
let __ttConfigBaseline = null;
const __ttCloneConfig = (c) => Object.assign({}, c, { fakeTimers: Object.assign({}, c.fakeTimers) });
globalThis.__ttCaptureConfigBaseline = () => {
  __ttConfigBaseline = { config: __ttCloneConfig(__ttConfig), saved: __ttConfigSaved && __ttCloneConfig(__ttConfigSaved), zero: __ttZeroDelay };
};
function __ttApplyJestFields(cfg) {
  const { jestFakeTimers, jestAutomock, jestTestTimeout } = cfg;
  __jestApplyConfig(jestFakeTimers);
  if (jestAutomock === true) __ttAutomockOn = true;
  if (typeof jestTestTimeout === 'number' && globalThis.__TT_DEFAULT_TIMEOUT === undefined) globalThis.__TT_DEFAULT_TIMEOUT = jestTestTimeout;
}
globalThis.__ttApplyConfig = (cfg) => {
  if (!cfg || typeof cfg !== 'object') return;
  const { fakeTimers, zeroDelayTimers, jestFakeTimers, jestAutomock, jestTestTimeout, ...rest } = cfg;
  __ttApplyJestFields(cfg);
  if (typeof zeroDelayTimers === 'boolean') __ttZeroDelay = zeroDelayTimers;
  if (fakeTimers && typeof fakeTimers === 'object') __ttConfig.fakeTimers = Object.assign(__ttConfigDefaults().fakeTimers, fakeTimers);
  Object.assign(__ttConfig, rest);
};
// jest setupFiles pre-framework phase (turbo-test#344): temporarily hide test globals so
// typeof expect === 'undefined' during setupFiles, matching Jest's two-phase lifecycle.
let __hiddenFramework;
globalThis.__ttHideFramework = () => {
  __hiddenFramework = {};
  for (const k of ['expect', 'test', 'it', 'describe', 'beforeAll', 'afterAll', 'beforeEach', 'afterEach']) {
    if (k in globalThis) { __hiddenFramework[k] = globalThis[k]; delete globalThis[k]; }
  }
};
globalThis.__ttRestoreFramework = () => {
  if (__hiddenFramework) { Object.assign(globalThis, __hiddenFramework); __hiddenFramework = undefined; }
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
function __fakeToFake(c) {
  return Array.isArray(c.toFake) && c.toFake.length ? c.toFake : __TO_FAKE;
}
function __fakeStart(c) {
  return c.now !== undefined ? __epochOf(c.now) : globalThis.Date.now();
}
function __applyFakeDate() {
  if (__loop.faked.has('Date')) __installFakeDate();
  else __restoreDate();
}
function __installFake(cfg) {
  const c = Object.assign({}, cfg || {});
  const start = __fakeStart(c);
  if (__loop.fake) __uninstallFake();
  __loop.dateMocked = false;
  __loop.fake = true;
  __loop.faked = new Set(__fakeToFake(c));
  __loop.loopLimit = c.loopLimit || 1000;
  __loop.systemTime = start - __loop.fnow;
  __loop.fstart = __loop.fnow;
  __loop.epoch0 = start;
  __applyFakeDate();
  __swapFakeTimers();
  __installFakeClocks();
  // shouldAdvanceTime: a real-clock interval ticks the fake clock by advanceTimeDelta (20ms)
  if (c.shouldAdvanceTime === true) __setTickMode('interval', c.advanceTimeDelta);
}
function __swapFakeTimers() {
  for (const k of Object.keys(__fakeTimers)) globalThis[k] = __loop.faked.has(k) ? __fakeTimers[k] : __realTimers[k];
}
// the faked performance / process.hrtime of __installFake
function __installFakeClocks() {
  if (__loop.faked.has('performance') && globalThis.performance) {
    __loop.realPerf = Object.getOwnPropertyDescriptor(globalThis, 'performance');
    Object.defineProperty(globalThis, 'performance', { value: __fakePerformance(globalThis.performance), writable: true, configurable: true, enumerable: !!(__loop.realPerf && __loop.realPerf.enumerable) });
  }
  if (__loop.faked.has('hrtime') && globalThis.process && typeof globalThis.process.hrtime === 'function') {
    __loop.realHrtime = globalThis.process.hrtime;
    globalThis.process.hrtime = __fakeHrtime;
  }
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
// A module's own hoisted vi.mock whose factory reads a binding still in its temporal dead zone
// (`const m = {...}` below the call: vitest hoists the call, but runs the factory on the first
// import, turbo-test#205): the module is registered as this stand-in, which runs the factory on
// its first use, as vitest does; a factory that still throws then throws vitest's error.
const __ttLazyMocks = new WeakMap();
function __ttLazyFactoryMock(run, vitest) {
  let done = false;
  let value;
  const resolve = () => {
    if (!done) {
      try { value = run(); } catch (e) { throw __ttMockErrors.get(__ttFactoryErrorMock(e, vitest)); }
      done = true;
    }
    return value;
  };
  const target = () => { const v = resolve(); return v !== null && typeof v === 'object' ? v : {}; };
  const lazy = new Proxy({}, {
    get: (_, k) => (typeof k === 'symbol' || k === 'then') && !done ? undefined : Reflect.get(target(), k),
    has: (_, k) => k in target(),
    ownKeys: () => { try { return Reflect.ownKeys(target()); } catch (e) { return []; } },
    getOwnPropertyDescriptor: (_, k) => {
      const d = Reflect.getOwnPropertyDescriptor(target(), k);
      return d && { ...d, configurable: true };
    },
    set: (_, k, v) => Reflect.set(target(), k, v),
    defineProperty: (_, k, d) => Reflect.defineProperty(target(), k, d),
    deleteProperty: (_, k) => Reflect.deleteProperty(target(), k),
  });
  __ttLazyMocks.set(lazy, resolve);
  return lazy;
}
globalThis.__ttMockFactoryError = (m) => {
  const lazy = __ttLazyMocks.get(__ttStrictMocks.get(m)?.target ?? m);
  if (lazy) {
    try { lazy(); return undefined; } catch (e) { return e; }
  }
  return __ttMockErrors.get(m);
};

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
// a UTF-8 lead byte's [continuation count, its code point bits, first continuation's bounds]
// (null: not a lead byte)
function __utf8Lead3(c) { return [2, c & 0xf, c === 0xe0 ? 0xa0 : 0x80, c === 0xed ? 0x9f : 0xbf]; }
function __utf8Lead4(c) { return [3, c & 0x7, c === 0xf0 ? 0x90 : 0x80, c === 0xf4 ? 0x8f : 0xbf]; }
function __utf8Lead(c) {
  if (c >= 0xc2 && c <= 0xdf) return [1, c & 0x1f, 0x80, 0xbf];
  if (c >= 0xe0 && c <= 0xef) return __utf8Lead3(c);
  if (c >= 0xf0 && c <= 0xf4) return __utf8Lead4(c);
  return null;
}
// a code point as its UTF-16 code units
function __utf16Units(cp, unit) {
  if (cp < 0x10000) { unit(cp); return; }
  cp -= 0x10000; unit(0xd800 + (cp >> 10)); unit(0xdc00 + (cp & 0x3ff));
}
// The one WHATWG UTF-8 decoder (turbo-test#121): b[start..end) (any indexable bytes) to a string,
// one U+FFFD per maximal invalid subpart (or `onBad()`'s throw, for `fatal`). With `stream`, an
// incomplete-but-valid sequence at the end is left undecoded; returns [string, where it starts].
// Consume UTF-8 continuation bytes after the lead. Returns [j, cp]: j is the position past the
// last accepted byte (equals end if we ran out mid-sequence), cp is the accumulated code point.
function __utf8ContRun(b, i, end, need, cp, lo, hi) {
  let j = i + 1;
  for (; j <= i + need && j < end; j++) {
    if (b[j] < lo || b[j] > hi) break;
    cp = (cp << 6) | (b[j] & 0x3f); lo = 0x80; hi = 0xbf;
  }
  return [j, cp];
}
function __utf8Run(b, start, end, stream, onBad) {
  const units = []; let out = '';
  const unit = (u) => { units.push(u); if (units.length >= 8192) { out += String.fromCharCode.apply(null, units); units.length = 0; } };
  const bad = () => { if (onBad) onBad(); unit(0xfffd); };
  let i = start;
  while (i < end) {
    const c = b[i];
    if (c < 0x80) { unit(c); i++; continue; }
    const lead = __utf8Lead(c);
    if (!lead) { bad(); i++; continue; }
    const [need, cp0, lo, hi] = lead;
    const [j, cp] = __utf8ContRun(b, i, end, need, cp0, lo, hi);
    if (j > i + need) __utf16Units(cp, unit);
    else if (j === end && stream) break; // valid so far: wait for more
    else bad(); // one U+FFFD for the maximal subpart b[i..j)
    i = j;
  }
  return [out + String.fromCharCode.apply(null, units), i];
}
// UTF-8 decode without BOM handling (Buffer#toString, querystring, URLSearchParams, StringDecoder)
function __utf8Decode(b, start = 0, end = b.length) { return __utf8Run(b, start, end, false, null)[0]; }
if (typeof globalThis.TextEncoder === 'undefined') {
  // Write one code point as `len` UTF-8 bytes into out starting at j; returns new j.
  const writeCp = (out, j, c, len) => {
    if (len === 1) { out[j++] = c; return j; }
    if (len === 2) { out[j++] = 0xc0 | (c >> 6); out[j++] = 0x80 | (c & 0x3f); return j; }
    if (len === 3) { out[j++] = 0xe0 | (c >> 12); out[j++] = 0x80 | ((c >> 6) & 0x3f); out[j++] = 0x80 | (c & 0x3f); return j; }
    out[j++] = 0xf0 | (c >> 18); out[j++] = 0x80 | ((c >> 12) & 0x3f); out[j++] = 0x80 | ((c >> 6) & 0x3f); out[j++] = 0x80 | (c & 0x3f); return j;
  };
  // Resolve a surrogate at s[i] to [codePoint, codeUnitsConsumed]: a lone surrogate becomes U+FFFD.
  const surrogateCp = (c, s, i, n) => {
    const d = c < 0xdc00 && i + 1 < n ? s.charCodeAt(i + 1) : 0;
    if (d >= 0xdc00 && d <= 0xdfff) return [0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00), 2];
    return [0xfffd, 1];
  };
  const cpLen = (c) => c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
  // UTF-8 bytes of s[i..] into out[j..] while they fit; returns [code units read, bytes written]
  const encodeTo = (s, out) => {
    const n = s.length; const max = out.length; let i = 0; let j = 0;
    while (i < n) {
      let c = s.charCodeAt(i); let units = 1;
      if (c >= 0xd800 && c <= 0xdfff) [c, units] = surrogateCp(c, s, i, n);
      const len = cpLen(c);
      if (j + len > max) break;
      j = writeCp(out, j, c, len);
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
  // Apply the per-stream BOM strip (once at the start of a stream unless `ignoreBOM`) and reset
  // `bomSeen` at end of stream — a partial sequence already became U+FFFD above.
  const stripBom = (st, out, stream) => {
    if (!st.ignoreBOM && !st.bomSeen && out.length > 0) {
      st.bomSeen = true;
      if (out.charCodeAt(0) === 0xfeff) out = out.slice(1);
    }
    if (!stream) st.bomSeen = false;
    return out;
  };
  // the UTF-8 decode of TextDecoder#decode: a pending partial sequence first, BOM per stream
  const decodeUtf8 = (st, b, stream) => {
    if (st.pending) { const m = new Uint8Array(st.pending.length + b.length); m.set(st.pending); m.set(b, st.pending.length); b = m; st.pending = null; }
    const fail = () => { st.pending = null; st.bomSeen = false; throw __codecError(TypeError, 'ERR_ENCODING_INVALID_ENCODED_DATA', 'The encoded data was not valid for encoding utf-8'); };
    let decoded, rest;
    try { [decoded, rest] = __utf8Run(b, 0, b.length, stream, st.fatal ? fail : null); } catch (e) { throw e instanceof RangeError ? tooLong() : e; }
    if (rest < b.length) st.pending = b.slice(rest);
    return stripBom(st, decoded, stream);
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
      const b = bytesOf(input);
      const stream = !!options(opts).stream;
      if (st.id) {
        const text = globalThis.__ttTextDecode(st.id, st.encoding, st.ignoreBOM, st.fatal, stream, b);
        if (text === null) throw __codecError(TypeError, 'ERR_ENCODING_INVALID_ENCODED_DATA', `The encoded data was not valid for encoding ${st.encoding}`);
        if (text === undefined) throw tooLong();
        return text;
      }
      return decodeUtf8(st, b, stream);
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
  // sequence<sequence<USVString>>: every pair is an iterable of exactly two
  const fromPairs = (list, init, m) => {
    if (typeof m !== 'function') throw new TypeError('Query pairs must be iterable');
    for (const pair of { [Symbol.iterator]: () => m.call(init) }) {
      if ((typeof pair !== 'object' || pair === null) && typeof pair !== 'function') {
        throw new TypeError('Each query pair must be an iterable [name, value] tuple');
      }
      const kv = [...pair];
      if (kv.length !== 2) throw new TypeError('Each query pair must be an iterable [name, value] tuple');
      list.push([usv(kv[0]), usv(kv[1])]);
    }
  };
  // record<USVString, USVString>: own enumerable keys; a key repeated after conversion keeps its
  // first position and takes the last value
  const fromRecord = (list, init) => {
    const rec = new Map();
    for (const key of Reflect.ownKeys(init)) {
      const d = Reflect.getOwnPropertyDescriptor(init, key);
      if (d && d.enumerable) rec.set(usv(key), usv(init[key]));
    }
    for (const kv of rec) list.push(kv);
  };
  const initObjectLike = (init) => (typeof init === 'object' && init !== null) || typeof init === 'function';
  const fillFromObjectLike = (list, init) => {
    const m = init[Symbol.iterator];
    if (m !== undefined && m !== null) fromPairs(list, init, m);
    else fromRecord(list, init);
  };
  globalThis.URLSearchParams = class URLSearchParams {
    constructor(init = undefined) {
      // list of [name, value] pairs; not enumerable, as a real URLSearchParams has no own keys
      // (pretty-format prints `URLSearchParams {}`)
      Object.defineProperty(this, '_', { value: [], writable: true, enumerable: false, configurable: true });
      if (init === undefined) return;
      if (initObjectLike(init)) { fillFromObjectLike(this._, init); return; }
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
      if (typeof depth === 'number') {
        if (depth < 0) return __sty(o, '[Object]', 'special');
        o.depth = depth - 1;
      }
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
  const __urlNormalizeInput = (url, base) => {
    url = String(url).replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, '').replace(/[\t\n\r]/g, '');
    if (base !== undefined && !/^[a-z][a-z0-9+.-]*:/i.test(url)) {
      url = globalThis.URL._resolve(url, new globalThis.URL(String(base)));
    }
    if (!/^[a-z][a-z0-9+.-]*:/i.test(url)) throw new TypeError('Invalid URL: ' + url);
    return url;
  };
  globalThis.URL = class URL {
    constructor(url, base) {
      // leading/trailing C0 controls and spaces go, tabs and newlines anywhere (WHATWG);
      // for base-relative inputs, resolve against the parsed base; reject inputs without a
      // valid scheme (matches real URL for validators like zod's .url()).
      url = __urlNormalizeInput(url, base);
      // protocol is the scheme up to the first ':' (works for opaque schemes like
      // javascript:/data:/mailto: that have no '//' authority).
      const scheme = /^([a-z][a-z0-9+.-]*:)/i.exec(url);
      this._protocol = scheme ? scheme[1].toLowerCase() : 'http:';
      const m = /^([a-z][a-z0-9+.-]*:)\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(url);
      const searchStr = m ? this._parseAuthority(url, m) : this._parseOpaque(url);
      // the query as parsed, percent-encoded with the query set (null: none); searchParams is
      // parsed from it and writes back to it when changed (turbo-test#115)
      this._query = searchStr ? URL._encode(searchStr.slice(1), URL._querySet(this.protocol)) : null;
      this._sp = URL._params(this._query || '', this);
      // the parts live in non-enumerable slots: a real URL has no own enumerable keys
      for (const k of Object.keys(this)) Object.defineProperty(this, k, { enumerable: false });
    }
    // the constructor's parse of a hierarchical URL (scheme://authority/path?query#hash); returns
    // the search string
    _rejectBadAuthority(url, hp) {
      if (this._special && !this._isFile && URL._badHostPort(hp)) {
        throw new TypeError('Invalid URL: ' + url);
      }
    }
    _parseAuthority(url, m) {
      this._hierarchical = true;
      // `user:pass@` before the last '@' is the userinfo, percent-encoded (WHATWG; turbo-test#114)
      const at = m[2].lastIndexOf('@');
      const info = at < 0 ? '' : m[2].slice(0, at);
      const hostport = at < 0 ? m[2] : m[2].slice(at + 1);
      this._parseUserInfo(info);
      // a special host takes no forbidden code point, and a port is digits up to 65535 (WHATWG)
      this._rejectBadAuthority(url, /^(\[[^\]]*\]|[^:]*)(?::(.*))?$/.exec(hostport));
      // a file URL's `localhost` host is the empty host (WHATWG)
      this._hostname = '';
      this._port = '';
      this._setHost(hostport, true);
      this.pathname = m[3] || '/';
      this.hash = m[5] || '';
      return m[4] || '';
    }
    _parseUserInfo(info) {
      const colon = info.indexOf(':');
      this._username = URL._encode(colon < 0 ? info : info.slice(0, colon), URL._USERINFO);
      this._password = colon < 0 ? '' : URL._encode(info.slice(colon + 1), URL._USERINFO);
    }
    // the constructor's parse of an opaque URL; returns the search string
    _parseOpaque(url) {
      // opaque URL (no authority): everything after the scheme is the path (sans query/hash).
      this._hierarchical = false;
      this._username = ''; this._password = '';
      this._hostname = ''; this._port = '';
      const rest = url.slice(this.protocol.length);
      const qi = rest.indexOf('?'); const hi = rest.indexOf('#');
      let cut = rest.length;
      if (qi >= 0) cut = Math.min(cut, qi);
      if (hi >= 0) cut = Math.min(cut, hi);
      this._path = URL._encode(rest.slice(0, cut), URL._C0);
      const after = rest.slice(cut);
      const am = /^(\?[^#]*)?(#.*)?$/.exec(after) || [];
      this.hash = am[2] || '';
      return am[1] || '';
    }
    // a special URL's host has no forbidden code point, and its port is digits up to 65535
    static _badHostPort(hp) {
      if (!hp || /[\x00-\x20#/:<>?@[\\\]^|]/.test(hp[1].replace(/^\[.*\]$/, ''))) return true;
      return !!hp[2] && (!/^\d+$/.test(hp[2]) || Number(hp[2]) > 65535);
    }
    // host, hostname and port are views of _hostname/_port, so a set reaches href and origin
    // (WHATWG host/hostname/port setters, as Node; turbo-test#201)
    get host() { return this._hostname + (this._port ? ':' + this._port : ''); }
    set host(v) { if (this._hierarchical) this._setHost(String(v), true); }
    get hostname() { return this._hostname; }
    set hostname(v) { if (this._hierarchical) this._setHost(String(v), false); }
    get port() { return this._port; }
    set port(v) {
      if (!this._hierarchical || this._isFile || this._hostname === '') return;
      v = String(v);
      if (v === '') { this._port = ''; return; }
      this._setPort(v);
    }
    // protocol/username/password are views too: a set is validated and encoded (WHATWG setters)
    get protocol() { return this._protocol; }
    _rejectProtocolChange(next) {
      const special = (p) => /^(https?|wss?|ftp|file):$/.test(p);
      if (special(next) !== special(this._protocol)) return true;
      if (next === 'file:' && (this._userinfo || this._port)) return true;
      if (this._isFile && this._hostname === '') return true;
      return false;
    }
    set protocol(v) {
      const m = /^([a-z][a-z0-9+.-]*)(?::|$)/i.exec(String(v));
      if (!m) return;
      const next = m[1].toLowerCase() + ':';
      if (this._rejectProtocolChange(next)) return;
      this._protocol = next;
      if (this._port) this._setPort(this._port);
    }
    get username() { return this._username; }
    set username(v) { if (this._hierarchical && this._hostname !== '' && !this._isFile) this._username = URL._encode(String(v), URL._USERINFO); }
    get password() { return this._password; }
    set password(v) { if (this._hierarchical && this._hostname !== '' && !this._isFile) this._password = URL._encode(String(v), URL._USERINFO); }
    get origin() { return !this._special || this._isFile ? 'null' : this.protocol + '//' + this.host; }
    get _isFile() { return this.protocol.toLowerCase() === 'file:'; }
    get _special() { return /^(https?|wss?|ftp|file):$/i.test(this.protocol); }
    // WHATWG host parsing for the setters: the value ends at / ? # (and \ for a special
    // scheme); a special non-file URL keeps its host when the value is empty; `withPort` (host)
    // also takes a `:port`, the hostname setter leaves the URL alone when the value has one.
    _setHost(v, withPort) {
      const end = v.search(this._special ? /[\/?#\\]/ : /[\/?#]/);
      if (end >= 0) v = v.slice(0, end);
      const m = /^(\[[^\]]*\]|[^:]*)(?::(.*))?$/.exec(v);
      if (!m) return;
      const port = m[2];
      const name = this._setHostName(m[1], port, withPort);
      if (name === null) return;
      this._hostname = name;
      if (port !== undefined && port !== '') this._setPort(port);
    }
    // Normalises the host part for _setHost. Returns null to reject the whole set (port present
    // when withPort=false or on a file URL; empty name on a special non-file URL).
    _rejectHostName(name, port, withPort) {
      if (port !== undefined && (!withPort || this._isFile)) return true;
      if (name === '' && this._special && !this._isFile) return true;
      return false;
    }
    _setHostName(name, port, withPort) {
      if (this._rejectHostName(name, port, withPort)) return null;
      if (this._special) name = URL._idnAscii(name);
      if (this._isFile && name === 'localhost') return '';
      return name;
    }
    // WHATWG `domain to ASCII` for a special URL's host (turbo-test#259): Node runs an IDN
    // hostname through UTS#46 ToASCII (`new URL('http://ü.com').host === 'xn--tda.com'`); the
    // lowercase that was here before did the ASCII half of that. IPv6 brackets skip the IDN
    // pass (the brackets' contents are not a domain); a pure-printable-ASCII name without an
    // `xn--`-anchored label is lower-cased directly (ToASCII is a no-op on it, and the native
    // call isn't free); otherwise ToASCII runs through `__ttIdnaToAscii`, with a fallback to
    // the lower-cased name if the native is missing or rejects the input — the upstream
    // `_parseHost` step (NOT the thin `_rejectHostName` helper, which only checks port /
    // empty-special-host) rejects forbidden code points, and the fast-path only trusts
    // printable ASCII so a control byte that ever slipped through still crosses the native
    // UTS#46 boundary.
    static _idnAscii(name) {
      const lower = name.toLowerCase();
      if (lower.startsWith('[')) return lower;
      if (/^[\x21-\x7e]*$/.test(lower) && !/(^|\.)xn--/.test(lower)) return lower;
      const ascii = globalThis.__ttIdnaToAscii ? globalThis.__ttIdnaToAscii(lower) : undefined;
      return ascii === undefined ? lower : ascii;
    }
    // the leading digits of `v`; a scheme's default port serializes as no port
    _setPort(v) {
      const d = /^\d*/.exec(v)[0];
      if (d === '') return;
      const n = Number(d);
      if (n > 65535) return;
      const dflt = { 'http:': 80, 'https:': 443, 'ws:': 80, 'wss:': 443, 'ftp:': 21 }[this.protocol.toLowerCase()];
      this._port = n === dflt ? '' : String(n);
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
    set href(v) { const u = new globalThis.URL(v); this._protocol = u._protocol; this._username = u._username; this._password = u._password; this._hostname = u._hostname; this._port = u._port; this._path = u._path; this._hash = u._hash; this._hierarchical = u._hierarchical; this._query = u._query; this._sp._ = u._sp._; }
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
    static _resolveOpaque(rel, noHash) {
      if (rel.startsWith('#')) return noHash + rel;
      throw new TypeError('Invalid URL: ' + rel);
    }
    static _resolve(rel, b) {
      const auth = b.protocol + '//' + b._userinfo + b.host;
      const noHash = b.href.replace(/#.*$/, '');
      if (!b._hierarchical) return URL._resolveOpaque(rel, noHash);
      if (rel.startsWith('//')) return b.protocol + rel;
      if (rel.startsWith('/')) return auth + rel;
      if (rel === '' || rel.startsWith('#')) return noHash + rel;
      if (rel.startsWith('?')) return auth + b.pathname + rel;
      return auth + b.pathname.replace(/[^/]*$/, '') + rel;
    }
    // RFC 3986 / WHATWG dot-segment removal on a path (`/a/./b/../c` -> `/a/c`).
    static _dotKind(seg) {
      const lower = seg.toLowerCase();
      if (seg === '..' || lower === '%2e%2e') return 'up';
      if (seg === '.' || lower === '%2e') return 'cur';
      return 'keep';
    }
    static _removeDots(path) {
      const out = [];
      const segs = path.split('/');
      for (let i = 1; i < segs.length; i++) {
        const seg = segs[i];
        const last = i === segs.length - 1;
        const kind = URL._dotKind(seg);
        if (kind === 'up') {
          if (out.length) out.pop();
          if (last) out.push('');
        } else if (kind === 'cur') {
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
  // `blobParts` must be undefined or an iterable (object-ish with Symbol.iterator)
  const isIterableParts = (bp) => {
    if (bp === undefined) return true;
    if (bp === null) return false;
    if (typeof bp !== 'object' && typeof bp !== 'function') return false;
    return typeof bp[Symbol.iterator] === 'function';
  };
  const isDictLike = (v) => v == null || typeof v === 'object' || typeof v === 'function';
  const blobEndings = (o) => {
    const endings = o.endings === undefined ? 'transparent' : String(o.endings);
    if (endings !== 'transparent' && endings !== 'native')
      throw __codecError(TypeError, 'ERR_INVALID_ARG_VALUE', `The property 'options.endings' is invalid. Received '${endings}'`);
    return endings;
  };
  // the Blob constructor's argument checks; returns [options, endings]
  const blobOptions = (blobParts, options) => {
    if (!isIterableParts(blobParts))
      throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'Value cannot be converted to sequence.');
    if (options != null && !isDictLike(options))
      throw __codecError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be a dictionary. Received ' + received(options));
    const o = options == null ? {} : options;
    return [o, blobEndings(o)];
  };
  globalThis.Blob = class Blob {
    #chunks = [];
    #size = 0;
    #type = '';
    constructor(blobParts = undefined, options = undefined) {
      const [o, endings] = blobOptions(blobParts, options);
      this.#type = o.type === undefined ? '' : normType(o.type);
      const add = (c) => { if (c.length) { this.#chunks.push(c); this.#size += c.length; } };
      for (const p of blobParts === undefined ? [] : blobParts) Blob.#addPart(p, endings, add);
    }
    static #isArrayBufferLike(p) {
      return p instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && p instanceof SharedArrayBuffer);
    }
    static #addStringPart(p, endings, add) {
      let s = String(p);
      if (endings === 'native') s = s.replace(/\r\n?/g, '\n');
      add(new globalThis.TextEncoder().encode(s));
    }
    // one blob part's bytes, to `add`
    static #addPart(p, endings, add) {
      if (Blob.#isArrayBufferLike(p)) add(new Uint8Array(p.slice(0)));
      else if (ArrayBuffer.isView(p)) add(new Uint8Array(p.buffer, p.byteOffset, p.byteLength).slice());
      else if (p !== null && typeof p === 'object' && #chunks in p) p.#chunks.forEach(add);
      else Blob.#addStringPart(p, endings, add);
    }
    get type() { if (!(#chunks in this)) throw thisErr('Blob'); return this.#type; }
    get size() { if (!(#chunks in this)) throw thisErr('Blob'); return this.#size; }
    #sliceRel(v, dflt) {
      if (v === undefined) return dflt;
      const n = clamp(v);
      return n < 0 ? Math.max(this.#size + n, 0) : Math.min(n, this.#size);
    }
    #appendSliceChunk(out, c, lo, hi) {
      out.#chunks.push(lo === 0 && hi === c.length ? c : c.subarray(lo, hi));
      out.#size += hi - lo;
    }
    slice(start, end, contentType) {
      if (!(#chunks in this)) throw thisErr('Blob');
      const from = this.#sliceRel(start, 0), to = this.#sliceRel(end, this.#size);
      const out = new Blob([], contentType === undefined ? undefined : { type: contentType });
      let at = 0;
      for (const c of this.#chunks) {
        const lo = Math.max(from - at, 0), hi = Math.min(to - at, c.length);
        if (lo < hi) this.#appendSliceChunk(out, c, lo, hi);
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
  const BOMS = [
    [[0xef, 0xbb, 0xbf], 'utf-8'],
    [[0xff, 0xfe], 'utf-16le'],
    [[0xfe, 0xff], 'utf-16be'],
  ];
  const bomMatches = (b, bytes) => bytes.every((v, i) => b[i] === v);
  const bomEncoding = (b) => {
    for (const [bytes, enc] of BOMS) if (bomMatches(b, bytes)) return enc;
    return null;
  };
  const labelEncoding = (label) => {
    if (label === undefined) return 'utf-8';
    try { return new globalThis.TextDecoder(String(label)).encoding; } catch { return 'utf-8'; }
  };
  // the encoding readAsText decodes with: a UTF-8 / UTF-16 BOM wins, then the label, then UTF-8
  const textOf = (b, label) => {
    const enc = bomEncoding(b) || labelEncoding(label);
    return new globalThis.TextDecoder(enc).decode(b);
  };
  // bytes[] -> FileReader#result for the four read kinds
  const binaryString = (bytes) => {
    let s = '';
    for (let i = 0; i < bytes.length; i += 8192) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
    return s;
  };
  const decodeReadResult = (bytes, blob, kind, label) => {
    if (kind === 'readAsDataURL') return 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + globalThis.__b64encodeBytes(bytes);
    if (kind === 'readAsArrayBuffer') return bytes.slice().buffer;
    if (kind === 'readAsBinaryString') return binaryString(bytes);
    return textOf(bytes, label);
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
    async #readBytes(blob) {
      // the Blob's bytes, not char codes of its decoded text (turbo-test#132)
      return typeof blob.bytes === 'function' ? await blob.bytes() : new Uint8Array(await blob.arrayBuffer());
    }
    #onReadError(gen, e) {
      if (gen !== this.#gen) return;
      this.readyState = 2; this.error = e;
      this.#fire('loadstart'); this.#fire('error'); this.#fire('loadend');
    }
    #onReadSuccess(gen, bytes, blob, kind, label) {
      if (gen !== this.#gen) return;
      this.#fire('loadstart');
      if (gen !== this.#gen) return;
      if (bytes.length) { this.#fire('progress', bytes.length, bytes.length, true); if (gen !== this.#gen) return; }
      this.result = decodeReadResult(bytes, blob, kind, label);
      this.readyState = 2;
      this.#fire('load');
      this.#fire('loadend');
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
        try { bytes = await this.#readBytes(blob); }
        catch (e) { this.#onReadError(gen, e); return; }
        this.#onReadSuccess(gen, bytes, blob, kind, label);
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
  const __requestUrl = (url) => {
    if (typeof url !== 'object' || !url) return String(url);
    return String(url.url || url.href || url);
  };
  const __requestHeaders = (h) => {
    if (h instanceof globalThis.Headers) return h;
    const out = new globalThis.Headers();
    if (h) for (const k of Object.keys(h)) out.set(k, h[k]);
    return out;
  };
  globalThis.Request = class Request {
    constructor(url, init) {
      init = init || {};
      // define url as an own, writable, configurable prop — a plain `this.url = x` would invoke
      // a getter-only `url` on a subclass prototype (e.g. Next's NextRequest) and throw.
      const u = __requestUrl(url);
      try { Object.defineProperty(this, 'url', { value: u, writable: true, configurable: true, enumerable: true }); }
      catch (e) {}
      this.method = init.method || 'GET';
      this.headers = __requestHeaders(init.headers);
      this._body = init.body;
    }
    json() { return Promise.resolve(typeof this._body === 'string' ? JSON.parse(this._body) : this._body); }
    text() { return Promise.resolve(this._body == null ? '' : String(this._body)); }
    clone() { return new globalThis.Request(this.url, { method: this.method, headers: this.headers, body: this._body }); }
  };
  const __responseStatus = (init) => (init && init.status) || 200;
  const __responseStatusText = (init, status) => (init && init.statusText) || globalThis.__STATUS_TEXT[status] || '';
  const __responseHeaders = (init) => {
    if (init && init.headers instanceof globalThis.Headers) return init.headers;
    return new globalThis.Headers(init && init.headers);
  };
  globalThis.Response = class Response {
    constructor(body, init) {
      this.body = body;
      this._body = body;
      this.status = __responseStatus(init);
      this.statusText = __responseStatusText(init, this.status);
      this.ok = this.status >= 200 && this.status < 300;
      this.headers = __responseHeaders(init);
    }
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
  const __isIterableValue = (t) => {
    if (t === null) return false;
    if (typeof t !== 'object' && typeof t !== 'function') return false;
    return typeof t[Symbol.iterator] === 'function';
  };
  globalThis.structuredClone = function structuredClone(value, options) {
    if (arguments.length === 0) throw new TypeError('The "The value argument must be specified" argument must be specified');
    const t = options == null ? undefined : options.transfer;
    let transfer = [];
    if (t !== undefined) {
      if (!__isIterableValue(t)) throw new TypeError("Failed to execute 'structuredClone': transfer in Options cannot be converted to sequence.");
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

  const __rsHwm = (strategy) => (strategy && typeof strategy.highWaterMark === 'number' ? strategy.highWaterMark : 1);
  const __rsRunStart = (stream) => {
    if (typeof stream._src.start !== 'function') return;
    try { stream._startP = stream._src.start(stream._ctrl); } catch (e) { stream._ctrl.error(e); }
  };
  class ReadableStream {
    constructor(underlyingSource, strategy) {
      this._src = underlyingSource || {};
      this._hwm = __rsHwm(strategy);
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
      __rsRunStart(this);
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
    static #fromHex(d) {
      const b = new Buffer(d.length / 2);
      for (let i = 0; i < b.length; i++) b[i] = parseInt(d.substr(i * 2, 2), 16);
      return b;
    }
    static #fromString(d, enc) {
      if (enc === 'base64') return new Buffer(globalThis.__b64decodeToBytes(d));
      // base64url: RFC 4648 §5 — `-`/`_` instead of `+`/`/`, optional padding. Node's base64
      // decoder tolerates either, so normalise to standard base64 and reuse the same path.
      if (enc === 'base64url') return new Buffer(globalThis.__b64decodeToBytes(d.replace(/-/g, '+').replace(/_/g, '/')));
      if (enc === 'hex') return Buffer.#fromHex(d);
      return new Buffer(new globalThis.TextEncoder().encode(d));
    }
    static from(d, enc) {
      if (typeof d === 'string') return Buffer.#fromString(d, enc);
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
      // base64url: RFC 4648 §5 — `+` → `-`, `/` → `_`, no `=` padding (Node WHATWG, turbo-test#260).
      if (enc === 'base64url') return globalThis.__b64encodeBytes(Array.from(this)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
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
          if (o) __realTimers.setImmediate(() => {
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
// A factory, so chai gets its own host classes (turbo-test#203): the DOM environment takes over
// the global EventTarget / Event prototypes, as jsdom replaces them in vitest while chai keeps
// Node's.
const __hostEventTarget = () => class EventTarget {
  constructor() { this.__listeners = Object.create(null); }
  addEventListener(type, fn) { if (!fn) return; (this.__listeners[type] || (this.__listeners[type] = [])).push(fn); }
  removeEventListener(type, fn) { const a = this.__listeners[type]; if (a) this.__listeners[type] = a.filter((x) => x !== fn); }
  dispatchEvent(ev) {
    ev.target = ev.target || this; ev.currentTarget = this;
    (this.__listeners[ev.type] || []).slice().forEach((fn) => { try { (fn.handleEvent || fn).call(this, ev); } catch {} });
    return !ev.defaultPrevented;
  }
};
if (typeof globalThis.EventTarget === 'undefined') globalThis.EventTarget = __hostEventTarget();
// FileReader is defined above EventTarget; make it one (turbo-test#139)
if (globalThis.FileReader && !(globalThis.FileReader.prototype instanceof globalThis.EventTarget)) {
  Object.setPrototypeOf(globalThis.FileReader.prototype, globalThis.EventTarget.prototype);
  Object.setPrototypeOf(globalThis.FileReader, globalThis.EventTarget);
}
const __hostEvent = () => class Event {
  constructor(type, init) { init = init || {}; this.type = type; this.bubbles = !!init.bubbles; this.cancelable = !!init.cancelable; this.composed = !!init.composed; this.defaultPrevented = false; this.target = null; this.currentTarget = null; this.timeStamp = 0; }
  preventDefault() { this.defaultPrevented = true; }
  stopPropagation() {}
  stopImmediatePropagation() {}
};
if (typeof globalThis.Event === 'undefined') globalThis.Event = __hostEvent();
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
const __fileURLToPathWin = (url) => {
  if (/%2f|%5c/i.test(url.pathname)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded \\ or / characters');
  const path = decodeURIComponent(url.pathname).replace(/\//g, '\\');
  if (url.hostname !== '') return `\\\\${url.hostname}${path}`;
  if (!/^\\[a-zA-Z]:/.test(path)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must be absolute');
  return path.slice(1);
};
const __fileURLToPathPosix = (url) => {
  if (url.hostname !== '') throw __urlError('ERR_INVALID_FILE_URL_HOST', `File URL host must be "localhost" or empty on ${globalThis.__ttPlatform || 'darwin'}`);
  if (/%2f/i.test(url.pathname)) throw __urlError('ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded / characters');
  return decodeURIComponent(url.pathname);
};
globalThis.__fileURLToPath = (u) => {
  const url = typeof u === 'string' ? new globalThis.URL(u) : u;
  if (url.protocol !== 'file:') throw __urlError('ERR_INVALID_URL_SCHEME', 'The URL must be of scheme file');
  return __ttWin() ? __fileURLToPathWin(url) : __fileURLToPathPosix(url);
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
const __ttNormSegs = (path) => {
  const out = [];
  for (const seg of path.split('/')) { if (seg === '..') out.pop(); else if (seg && seg !== '.') out.push(seg); }
  return out;
};
globalThis.__pathToFileURL = (p) => {
  let host = '';
  let root = '';
  let path = String(p);
  // Node keeps a trailing separator only when the given path ends in one ('' is the cwd)
  const trailing = /\/$/.test(path) || (__ttWin() && path.endsWith('\\')) ? '/' : '';
  if (__ttWin()) [host, root, path] = __ttWinSplit(path);
  else if (!path.startsWith('/')) path = globalThis.process.cwd() + '/' + path;
  const out = __ttNormSegs(path);
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
// Node's `Cannot find package 'X' imported from Y` for a package name we couldn't resolve.
const __ttMetaMissing = (spec, from) => {
  const pkg = spec.startsWith('@') ? spec.split('/').slice(0, 2).join('/') : spec.split('/')[0];
  const e = new Error(`Cannot find package '${pkg}' imported from ${from}`);
  e.code = 'ERR_MODULE_NOT_FOUND';
  return e;
};
// Relative or bare-URL specifiers resolve without going through the Node resolver.
const __ttMetaTrivial = (spec, parent) => {
  if (/^\.{0,2}\//.test(spec) || spec === '.' || spec === '..') return new globalThis.URL(spec, parent).href;
  if (/^[a-zA-Z][a-zA-Z\d+.-]*:/.test(spec) && !spec.startsWith('node:')) {
    try { return new globalThis.URL(spec).href; } catch (e) { /* not a URL: a package name */ }
  }
  return null;
};
const __ttMetaFrom = (parent) => {
  try { return globalThis.__fileURLToPath(parent); } catch (e) { return String(parent); }
};
const __ttMetaFormatResolved = (r, spec) => {
  if (typeof r !== 'string') return null;
  if (globalThis.__ttIsAbsolutePath(r)) return globalThis.__pathToFileURL(r).href;
  if (spec !== 'vitest' && spec !== '@jest/globals') return 'node:' + r.replace(/^node:/, '');
  return null;
};
const __ttMetaResolve = (specifier, parent) => {
  const spec = String(specifier);
  const trivial = __ttMetaTrivial(spec, parent);
  if (trivial !== null) return trivial;
  const from = __ttMetaFrom(parent);
  const r = globalThis.__ttRequireResolve(globalThis.__dirnameOf(from), spec, true);
  const resolved = __ttMetaFormatResolved(r, spec);
  if (resolved !== null) return resolved;
  throw __ttMetaMissing(spec, from);
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
// Node's posix path (lib/path.js): `..`/`.` collapse, resolve against the cwd from the right,
// relative by common segments, and Node's basename/extname/format edge cases.
const __normStrStep = (out, seg, absolute) => {
  if (seg === '' || seg === '.') return;
  if (seg !== '..') { out.push(seg); return; }
  if (out.length && out[out.length - 1] !== '..') out.pop();
  else if (!absolute) out.push('..');
};
const __normStr = (p, absolute) => {
  const out = [];
  for (const seg of p.split('/')) __normStrStep(out, seg, absolute);
  return out.join('/');
};
const __normalize = (p) => {
  p = String(p);
  if (p === '') return '.';
  const abs = p.startsWith('/');
  let r = __normStr(p, abs);
  if (r === '' && !abs) r = '.';
  if (r !== '' && p.endsWith('/')) r += '/';
  return abs ? '/' + r : r;
};
const __resolveCwd = () => String(globalThis.process && globalThis.process.cwd ? globalThis.process.cwd() : '/');
const __resolve = (...a) => {
  let r = '';
  for (let i = a.length - 1; i >= -1 && !r.startsWith('/'); i--) {
    const seg = i >= 0 ? String(a[i]) : __resolveCwd();
    if (seg === '') continue;
    r = seg + (r ? '/' + r : '');
  }
  return '/' + __normStr(r, true);
};
const __extname = (p) => { const b = __basename(p); const i = b.lastIndexOf('.'); return i <= 0 ? '' : b.slice(i); };
const __basename = (p, ext) => {
  let b = String(p).replace(/\/+$/, '');
  b = b.slice(b.lastIndexOf('/') + 1);
  if (ext !== undefined && b !== ext && b.endsWith(ext)) b = b.slice(0, -ext.length);
  return b;
};
const __pathFormatExt = (ext) => (ext ? (ext.startsWith('.') ? '' : '.') + ext : '');
const __pathFormatBase = (o) => o.base || ((o.name || '') + __pathFormatExt(o.ext));
const __pathFormat = (opt) => {
  const o = opt || {};
  const dir = o.dir || o.root;
  const base = __pathFormatBase(o);
  if (!dir) return base;
  return dir === o.root ? dir + base : dir + '/' + base;
};
const __path = {
  sep: '/',
  join: (...a) => { const j = a.map(String).filter((x) => x !== '').join('/'); return j === '' ? '.' : __normalize(j); },
  resolve: __resolve,
  dirname: (p) => globalThis.__dirnameOf(p),
  basename: __basename,
  extname: __extname,
  isAbsolute: (p) => String(p).startsWith('/'),
  relative: (f, t) => {
    f = __resolve(f); t = __resolve(t);
    if (f === t) return '';
    const fs = f === '/' ? [] : f.slice(1).split('/');
    const ts = t === '/' ? [] : t.slice(1).split('/');
    let i = 0;
    while (i < fs.length && i < ts.length && fs[i] === ts[i]) i++;
    return [...fs.slice(i).map(() => '..'), ...ts.slice(i)].join('/');
  },
  normalize: __normalize,
  parse: (p) => { const s = String(p); const dir = globalThis.__dirnameOf(s); const base = __basename(s); const ext = __extname(base); return { root: s.startsWith('/') ? '/' : '', dir, base, ext, name: ext ? base.slice(0, -ext.length) : base }; },
  format: (o) => __pathFormat(o),
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
// readdir recursive: a dirent is a directory when its mode is S_IFDIR directly, or it's a symlink
// whose target is a directory (statSync's throw on a broken link falls through as non-directory).
const __readdirIsDir = (full, type) => {
  if (type === 0o040000) return true;
  if (type === 0o120000) {
    try { return (globalThis.__fs_stat(full, false)[1] & __S_IFMT) === 0o040000; } catch (e) {}
  }
  return false;
};
const __readdirEntry = (name, type, dir, base, opts) => {
  const full = (dir.endsWith('/') ? dir + name : dir + '/' + name);
  const value = opts.withFileTypes ? new Dirent(name, type, dir) : full.slice(base.length + (base.endsWith('/') ? 0 : 1));
  return { full, value };
};
const __readdir = (p, options) => {
  const base = __fsPath(p);
  const opts = typeof options === 'object' && options !== null ? options : {};
  const read = (dir) => globalThis.__fs_readdir(dir);
  if (!opts.recursive) return read(base).map(([name, type]) => (opts.withFileTypes ? new Dirent(name, type, base) : name));
  const out = [], queue = [base];
  for (let i = 0; i < queue.length; i++) {
    for (const [name, type] of read(queue[i])) {
      const { full, value } = __readdirEntry(name, type, queue[i], base, opts);
      out.push(value);
      if (__readdirIsDir(full, type)) queue.push(full);
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
  const readDigits = () => {
    let digits = '';
    while (i < re.length && re[i] >= '0' && re[i] <= '9') digits += re[i++];
    return digits;
  };
  // `\u{..}`/`\p{..}`/`\P{..}` inside a class: a composite form rendered as a group. Returns
  // true if handled and the caller should return early.
  const classEscapeComposite = (next, seq) => {
    if (re[i] !== '{') return false;
    if (next === 'u' || next === 'p' || next === 'P') { writeGroup(`${seq}{`, '}', 0); return true; }
    return false;
  };
  // inside a character class
  const classEscape = () => {
    let seq = '\\';
    i++;
    if (i < re.length) {
      seq += re[i++];
      const next = seq[1];
      if (classEscapeComposite(next, seq)) return;
      if (next === 'x') { seq += re.slice(i, i + 2); i += 2; }
    }
    write(seq);
  };
  const isRangeDash = (ch) => ch === '-' && re[i - 1] !== '[' && i + 1 < re.length && re[i + 1] !== ']';
  const classChar = (ch) => {
    if (ch === '\\') { classEscape(); return; }
    if (ch === ']') { depth--; write(']'); i++; inClass = false; return; }
    if (isRangeDash(ch)) { writeDepth('-', 1, 1); return; }
    write(ch); i++;
  };
  const namedGroup = () => {
    i++;
    const start = i;
    while (i < re.length && re[i] !== '>') i++;
    const name = re.slice(start, i);
    if (i < re.length && re[i] === '>') {
      depth--; write('?<'); writeDepth(name, 1, 0); write('>'); depth++; i++;
    } else {
      writeDepth('?<', -1, 0); write(name);
    }
  };
  // after `(?`
  const GROUP_PREFIX = new Set([':', '=', '!']);
  const GROUP_LOOKBEHIND = new Set(['=', '!']);
  const groupModifier = () => {
    const a = i < re.length ? re[i] : '';
    if (GROUP_PREFIX.has(a)) { writeDepth(`?${a}`, -1, 1); return; }
    if (a !== '<') { write('?'); return; }
    const b = i + 1 < re.length ? re[i + 1] : '';
    if (GROUP_LOOKBEHIND.has(b)) writeDepth(`?<${b}`, -1, 2);
    else namedGroup();
  };
  const openGroup = () => {
    write('('); depth++; i++;
    if (i < re.length && re[i] === '?') {
      i++;
      groupModifier();
    }
  };
  // Follow-up for `\<next>` when `next` opens a composite form (\u{..}, \xNN, \1.., \k<..>,
  // \p{..}, \P{..}); returns the updated `seq` or null if nothing special, and writes directly
  // for the forms that emit a writeGroup (returning 'done' then).
  const BRACE_ESCAPE = new Set(['u', 'p', 'P']);
  const escapeTailBrace = (seq, next) => {
    if (BRACE_ESCAPE.has(next) && re[i] === '{') { writeGroup(`${seq}{`, '}', 0); return true; }
    if (next === 'k' && re[i] === '<') { writeGroup(`${seq}<`, '>'); return true; }
    return false;
  };
  const escapeTail = (seq, next) => {
    if (next === 'x') { seq += re.slice(i, i + 2); i += 2; return seq; }
    if (next >= '0' && next <= '9') return seq + readDigits();
    if (escapeTailBrace(seq, next)) return 'done';
    return seq;
  };
  const escape = () => {
    let seq = '\\';
    i++;
    if (i < re.length) {
      seq += re[i++];
      if (i < re.length) {
        const r = escapeTail(seq, seq[1]);
        if (r === 'done') return;
        seq = r;
      }
    }
    writeDepth(seq, 1, 0);
  };
  // Close out a `{...}` quantifier: emit the closing brace if present, then a lazy '?' modifier.
  const closeQuantifier = () => {
    if (i < re.length && re[i] === '}') { depth--; write('}'); i++; }
    if (i < re.length && re[i] === '?') writeDepth('?', 3, 1);
  };
  const quantifier = () => {
    i++;
    const digits = readDigits();
    if (digits) { write('{'); depth++; writeDepth(digits, 1, 0); }
    if (i < re.length) {
      if (re[i] === ',') {
        if (!digits) { write('{'); depth++; }
        write(','); i++;
      } else if (!digits) {
        depth += 1; write('{'); depth -= 1;
        return;
      }
    }
    const digits2 = readDigits();
    if (digits2) writeDepth(digits2, 1, 0);
    closeQuantifier();
  };
  // A one-char token that writes at a lifted depth (operators |+*?,^$ at 3,1; '.' at 2,1).
  const stepLifted = '|+*?,^$';
  const stepSimple = { '(': openGroup, ')': () => { depth--; write(')'); i++; }, '\\': escape, '{': quantifier };
  const step = (ch) => {
    if (ch === '[') { write('['); depth++; i++; inClass = true; return; }
    const fn = stepSimple[ch];
    if (fn) return fn();
    if (stepLifted.indexOf(ch) !== -1) return writeDepth(ch, 3, 1);
    if (ch === '.') return writeDepth(ch, 2, 1);
    writeDepth(ch, 1, 1);
  };
  write('/');
  depth++;
  i = 1;
  while (i < re.length) {
    const ch = re[i];
    if (inClass) classChar(ch);
    else if (ch === '/') break;
    else step(ch);
  }
  writeDepth('/', -1, 1);
  if (i < re.length) write(re.slice(i));
  return out;
}
const __inspectMeta = ['\\x00', '\\x01', '\\x02', '\\x03', '\\x04', '\\x05', '\\x06', '\\x07', '\\b', '\\t', '\\n', '\\x0B', '\\f', '\\r', '\\x0E', '\\x0F', '\\x10', '\\x11', '\\x12', '\\x13', '\\x14', '\\x15', '\\x16', '\\x17', '\\x18', '\\x19', '\\x1A', '\\x1B', '\\x1C', '\\x1D', '\\x1E', '\\x1F'];
function __strEscapeQuote(str) {
  if (!str.includes("'")) return "'";
  if (!str.includes('"')) return '"';
  if (!str.includes('`') && !str.includes('${')) return '`';
  return "'";
}
function __strEscapeChar(ch, c, quote) {
  if (ch === quote || ch === '\\') return '\\' + ch;
  if (c < 0x20) return __inspectMeta[c];
  if (c === 0x7f) return '\\x7F';
  return ch;
}
function __strEscape(str) {
  const quote = __strEscapeQuote(str);
  let out = '';
  for (const ch of str) out += __strEscapeChar(ch, ch.charCodeAt(0), quote);
  return quote + out + quote;
}
function __inspectKey(key) {
  if (typeof key === 'symbol') return key.toString();
  return /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : __strEscape(key);
}
function __isInstance(obj, fn) {
  try { return Function.prototype[Symbol.hasInstance].call(fn, obj); } catch { return false; }
}
function __ctorName(obj) {
  let o = obj;
  while (o !== null && o !== undefined) {
    const d = Object.getOwnPropertyDescriptor(o, 'constructor');
    // as Node's getConstructorName: only a constructor `obj` is an instance of (`foo.prototype` is `{}`)
    if (d && typeof d.value === 'function' && d.value.name !== '' && __isInstance(obj, d.value)) return d.value.name;
    o = Object.getPrototypeOf(o);
  }
  return null;
}
// `[class Name extends Super]` tag for a class. (The class/non-class branch of __fnBase.)
function __fnBaseClass(fn) {
  let base = `[class ${fn.name || '(anonymous)'}`;
  const sup = Object.getPrototypeOf(fn);
  if (sup && sup.name) base += ` extends ${sup.name}`;
  return base + ']';
}
const __FN_TYPE_TAGS = new Set(['AsyncFunction', 'GeneratorFunction', 'AsyncGeneratorFunction']);
function __fnBaseType(fn) {
  const tag = fn[Symbol.toStringTag];
  return __FN_TYPE_TAGS.has(tag) ? tag : 'Function';
}
function __fnBase(fn, ctor) {
  const src = Function.prototype.toString.call(fn);
  if (src.startsWith('class') && /^class\s*[\s{A-Za-z_$]/.test(src)) return __fnBaseClass(fn);
  let base = `[${__fnBaseType(fn)}`;
  base += fn.name ? `: ${fn.name}]` : ' (anonymous)]';
  if (ctor === null) base += ' (null prototype)';
  return base;
}
// A long string that spans newlines is rendered as a joined per-line sequence with the indent
// of the surrounding value; anything else stays as a single escaped literal.
function __inspectStrLines(v, ctx, sty) {
  if (v.length > 16 && v.length > 80 - ctx.indentationLvl - 4) {
    const parts = v.split(/(?<=\n)/);
    if (parts.length > 1) return parts.map((l) => sty(__strEscape(l), 'string')).join(` +\n${' '.repeat(ctx.indentationLvl + 2)}`);
  }
  return sty(__strEscape(v), 'string');
}
// formatPrimitive; `plain` is Node's stylizeNoColor (boxed primitives)
function __inspectPrimitiveSimple(v, sty) {
  switch (typeof v) {
    case 'number': return sty(Object.is(v, -0) ? '-0' : String(v), 'number');
    case 'bigint': return sty(`${v}n`, 'bigint');
    case 'boolean': return sty(String(v), 'boolean');
    case 'undefined': return sty('undefined', 'undefined');
    case 'symbol': return sty(v.toString(), 'symbol');
  }
  return null;
}
function __inspectPrimitive(v, ctx, plain) {
  const sty = (str, type) => (plain ? str : __sty(ctx, str, type));
  if (typeof v === 'string') return __inspectStrLines(v, ctx, sty);
  const simple = __inspectPrimitiveSimple(v, sty);
  return simple !== null ? simple : String(v);
}
function __formatCustomInspect(ctx, value, recurseTimes) {
  const custom = value[Symbol.for('nodejs.util.inspect.custom')];
  if (typeof custom !== 'function' || custom === __util.inspect) return null;
  const r = custom.call(value, ctx.depth - recurseTimes, { depth: ctx.depth - recurseTimes, breakLength: ctx.breakLength, compact: ctx.compact, colors: ctx.colors }, __util.inspect);
  if (r === value) return null;
  // a multi-line result is indented to where it is printed, as Node does
  return typeof r === 'string' ? r.replace(/\n/g, `\n${' '.repeat(ctx.indentationLvl)}`) : __formatValue(ctx, r, recurseTimes);
}
function __formatValue(ctx, value, recurseTimes, typedArray) {
  if (typeof value !== 'object' && typeof value !== 'function') return __inspectPrimitive(value, ctx);
  if (value === null) return __sty(ctx, 'null', 'null');
  // a [util.inspect.custom](depth, options, inspect) method formats its own value
  const custom = __formatCustomInspect(ctx, value, recurseTimes);
  if (custom !== null) return custom;
  if (ctx.seen.includes(value)) return __sty(ctx, `[Circular *${__trackCircular(ctx, value)}]`, 'special');
  return __formatRaw(ctx, value, recurseTimes, typedArray);
}
// On the first circular hit for `value`, assign it a fresh 1-based index; later hits reuse it.
function __trackCircular(ctx, value) {
  if (ctx.circular === undefined) { ctx.circular = new Map(); ctx.circular.set(value, 1); return 1; }
  let index = ctx.circular.get(value);
  if (index === undefined) { index = ctx.circular.size + 1; ctx.circular.set(value, index); }
  return index;
}
// The styled value side of __formatProperty: the data / accessor shape of desc.
function __formatPropertyValue(ctx, value, recurseTimes, type, desc) {
  if (desc.value !== undefined) {
    const diff = ctx.compact !== true || type !== 0 ? 2 : 3;
    ctx.indentationLvl += diff;
    const str = __formatValue(ctx, desc.value, recurseTimes);
    ctx.indentationLvl -= diff;
    return str;
  }
  if (desc.get !== undefined) return __sty(ctx, desc.set !== undefined ? '[Getter/Setter]' : '[Getter]', 'special');
  if (desc.set !== undefined) return __sty(ctx, '[Setter]', 'special');
  return __sty(ctx, 'undefined', 'undefined');
}
// The styled key side of __formatProperty (never called for array entries).
function __formatPropertyKey(ctx, key, desc) {
  if (typeof key === 'symbol') return desc.enumerable === false ? `[${__sty(ctx, key.toString(), 'symbol')}]` : __sty(ctx, key.toString(), 'symbol');
  if (desc.enumerable === false) return `[${key}]`;
  return /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : __sty(ctx, __strEscape(key), 'string');
}
function __formatProperty(ctx, value, recurseTimes, key, type, desc) {
  desc = desc || Object.getOwnPropertyDescriptor(value, key) || { value: value[key], enumerable: true };
  const str = __formatPropertyValue(ctx, value, recurseTimes, type, desc);
  if (type === 1) return str; // an array element (2: an array's extra key)
  return `${__formatPropertyKey(ctx, key, desc)}: ${str}`;
}
// groupArrayElements: each column's width (the widest entry in it, plus the separator)
function __groupColumnWidths(output, dataLen, columns, separatorSpace) {
  const maxLineLength = [];
  for (let i = 0; i < columns; i++) {
    let lineLength = 0;
    for (let j = i; j < output.length; j += columns) if (dataLen[j] > lineLength) lineLength = dataLen[j];
    maxLineLength.push(lineLength + separatorSpace);
  }
  return maxLineLength;
}
// groupArrayElements: numbers (and bigints) are right-aligned
function __groupPadStart(value, output) {
  if (value === undefined) return true;
  for (let i = 0; i < output.length; i++) {
    if (typeof value[i] !== 'number' && typeof value[i] !== 'bigint') return false;
  }
  return true;
}
// groupArrayElements: the rows of `columns` entries
function __groupRows(output, outputLength, dataLen, columns, separatorSpace, value) {
  const tmp = [];
  const maxLineLength = __groupColumnWidths(output, dataLen, columns, separatorSpace);
  const padStart = __groupPadStart(value, output);
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
  return tmp;
}
function __groupMeasure(ctx, output, outputLength, separatorSpace) {
  let totalLength = 0;
  let maxLength = 0;
  const dataLen = new Array(outputLength);
  for (let i = 0; i < outputLength; i++) {
    const len = ctx.colors ? __stripAnsi(output[i]).length : output[i].length;
    dataLen[i] = len;
    totalLength += len + separatorSpace;
    if (maxLength < len) maxLength = len;
  }
  return { totalLength, maxLength, dataLen };
}
function __groupColumns(ctx, totalLength, outputLength, actualMax) {
  const averageBias = Math.sqrt(actualMax - totalLength / outputLength);
  const biasedMax = Math.max(actualMax - 3 - averageBias, 1);
  return Math.min(
    Math.round(Math.sqrt(2.5 * biasedMax * outputLength) / biasedMax),
    Math.floor((ctx.breakLength - ctx.indentationLvl) / actualMax),
    ctx.compact * 4,
    15,
  );
}
function __groupArrayElements(ctx, output, value) {
  let outputLength = output.length;
  if (ctx.maxArrayLength < output.length) outputLength--;
  const separatorSpace = 2;
  const { totalLength, maxLength, dataLen } = __groupMeasure(ctx, output, outputLength, separatorSpace);
  const actualMax = maxLength + separatorSpace;
  if (actualMax * 3 + ctx.indentationLvl >= ctx.breakLength) return output;
  if (!(totalLength / actualMax > 5 || maxLength <= 6)) return output;
  const columns = __groupColumns(ctx, totalLength, output.length, actualMax);
  if (columns <= 1) return output;
  const tmp = __groupRows(output, outputLength, dataLen, columns, separatorSpace, value);
  if (ctx.maxArrayLength < output.length) tmp.push(output[outputLength]);
  return tmp;
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
// reduceToSingleString with a numeric `compact`: [the (grouped) output, its one-line form or
// undefined when it does not fit on one line]
function __reduceOneLine(ctx, output, base, braces, entries, recurseTimes) {
  if (ctx.currentDepth - recurseTimes >= ctx.compact || entries !== output.length) return undefined;
  const start = output.length + ctx.indentationLvl + braces[0].length + base.length + 10;
  if (!__isBelowBreakLength(ctx, output, start, base)) return undefined;
  const joined = output.join(', ');
  if (joined.includes('\n')) return undefined;
  return `${base ? `${base} ` : ''}${braces[0]} ${joined} ${braces[1]}`;
}
function __reduceCompactNumber(ctx, output, base, braces, extrasType, recurseTimes, value) {
  const entries = output.length;
  if (extrasType === 2 && entries > 6) output = __groupArrayElements(ctx, output, value);
  return [output, __reduceOneLine(ctx, output, base, braces, entries, recurseTimes)];
}
// The multi-line branch of __reduceToSingleString (ctx.compact !== true): entries one per line,
// with a numeric `compact` first trying the one-line grouped form from __reduceCompactNumber.
function __reduceExpanded(ctx, output, base, braces, extrasType, recurseTimes, value) {
  if (typeof ctx.compact === 'number' && ctx.compact >= 1) {
    const [grouped, line] = __reduceCompactNumber(ctx, output, base, braces, extrasType, recurseTimes, value);
    if (line !== undefined) return line;
    output = grouped;
  }
  const indentation = `\n${' '.repeat(ctx.indentationLvl)}`;
  return `${base ? `${base} ` : ''}${braces[0]}${indentation}  ${output.join(`,${indentation}  `)}${indentation}${braces[1]}`;
}
function __reduceToSingleString(ctx, output, base, braces, extrasType, recurseTimes, value) {
  if (ctx.compact !== true) return __reduceExpanded(ctx, output, base, braces, extrasType, recurseTimes, value);
  if (__isBelowBreakLength(ctx, output, 0, base)) return `${braces[0]}${base ? ` ${base}` : ''} ${output.join(', ')} ${braces[1]}`;
  const indentation = ' '.repeat(ctx.indentationLvl);
  const ln = base === '' && braces[0].length === 1 ? ' ' : `${base ? ` ${base}` : ''}\n${indentation}  `;
  return `${braces[0]}${ln}${output.join(`,\n${indentation}  `)} ${braces[1]}`;
}
// formatRaw's per-kind result: the keys still to print, the braces, the entries' formatter (given
// the entries' recurseTimes), the
// base (a function's, regexp's, date's ... text) and the extras type (2: array-like)
const __rawSpec = (keys, braces, formatter = () => [], base = '', extrasType = 0) => ({ keys, braces, formatter, base, extrasType });
const __moreItems = (n) => `... ${n} more item${n > 1 ? 's' : ''}`;
const __emptyItems = (ctx, holes) => __sty(ctx, `<${holes} empty item${holes > 1 ? 's' : ''}>`, 'undefined');
function __rawArrayItems(ctx, value, recurseTimes) {
  const out = [];
  const len = Math.min(ctx.maxArrayLength, value.length);
  let holes = 0;
  for (let i = 0; i < len; i++) {
    if (!Object.prototype.hasOwnProperty.call(value, i)) { holes++; continue; }
    if (holes) { out.push(__emptyItems(ctx, holes)); holes = 0; }
    out.push(__formatProperty(ctx, value, recurseTimes, i, 1));
  }
  if (holes) out.push(__emptyItems(ctx, holes));
  if (value.length > len) out.push(__moreItems(value.length - len));
  return out;
}
function __rawArray(ctx, value, keys, ctor, tag, prefix) {
  const p = ctor !== 'Array' || (typeof tag === 'string' && tag !== '') ? prefix('Array', `(${value.length})`) : '';
  keys = keys.filter((k) => typeof k === 'symbol' || !(/^(0|[1-9][0-9]*)$/.test(k) && Number(k) < value.length));
  const braces = [`${p}[`, ']'];
  if (value.length === 0 && keys.length === 0) return `${braces[0]}]`;
  return __rawSpec(keys, braces, (rt) => __rawArrayItems(ctx, value, rt), '', 2);
}
function __rawMapSet(ctx, value, keys, prefix) {
  const isMap = value instanceof Map;
  const braces = [`${prefix(isMap ? 'Map' : 'Set', `(${value.size})`)}{`, '}'];
  if (value.size === 0 && keys.length === 0) return `${braces[0]}}`;
  return __rawSpec(keys, braces, (rt) => {
    const out = [];
    ctx.indentationLvl += 2;
    if (isMap) for (const [k, v] of value) out.push(`${__formatValue(ctx, k, rt)} => ${__formatValue(ctx, v, rt)}`);
    else for (const v of value) out.push(__formatValue(ctx, v, rt));
    ctx.indentationLvl -= 2;
    return out;
  });
}
// Node's Buffer inspect: `<Buffer deadbeef ... N more bytes>`; the first 50 bytes in hex,
// with an ellipsis for anything past that. (Buffer branch of __rawTyped.)
function __rawBuffer(value, ctor) {
  const bytes = Array.from(value.subarray(0, 50), (b) => b.toString(16).padStart(2, '0')).join(' ');
  const more = value.length > 50 ? ` ... ${value.length - 50} more byte${value.length - 50 > 1 ? 's' : ''}` : '';
  return `<${ctor || 'Buffer'}${bytes ? ` ${bytes}` : ''}${more}>`;
}
function __rawTyped(ctx, value, keys, ctor, tag, prefix) {
  if (globalThis.Buffer && globalThis.Buffer.isBuffer && globalThis.Buffer.isBuffer(value)) return __rawBuffer(value, ctor);
  keys = keys.filter((k) => !/^(0|[1-9][0-9]*)$/.test(k));
  const braces = [`${prefix(tag || 'TypedArray', `(${value.length})`)}[`, ']'];
  if (value.length === 0 && keys.length === 0) return `${braces[0]}]`;
  return __rawSpec(keys, braces, () => {
    const out = [];
    const len = Math.min(ctx.maxArrayLength, value.length);
    for (let i = 0; i < len; i++) out.push(__inspectPrimitive(value[i], ctx));
    if (value.length > len) out.push(__moreItems(value.length - len));
    return out;
  }, '', 2);
}
function __rawBoxed(ctx, value, keys) {
  const type = value instanceof Number ? 'Number' : value instanceof String ? 'String' : 'Boolean';
  const base = `[${type}: ${__inspectPrimitive(value.valueOf(), ctx, true)}]`;
  if (value instanceof String) keys = keys.filter((k) => !/^(0|[1-9][0-9]*)$/.test(k));
  if (keys.length === 0) return __sty(ctx, base, type.toLowerCase());
  return __rawSpec(keys, ['{', '}'], undefined, base);
}
function __rawFn(ctx, value, keys, ctor) {
  const base = __fnBase(value, ctor);
  return keys.length === 0 ? __sty(ctx, base, 'special') : __rawSpec(keys, ['{', '}'], undefined, base);
}
function __rawRegExp(ctx, value, keys, recurseTimes) {
  const base = __sty(ctx, RegExp.prototype.toString.call(value), 'regexp');
  return keys.length === 0 || recurseTimes > ctx.depth ? base : __rawSpec(keys, ['{', '}'], undefined, base);
}
function __rawDate(ctx, value, keys) {
  const base = Number.isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
  return keys.length === 0 ? __sty(ctx, base, 'date') : __rawSpec(keys, ['{', '}'], undefined, base);
}
function __rawErr(value, keys) {
  const base = value.stack ? String(value.stack) : `[${Error.prototype.toString.call(value)}]`;
  keys = keys.filter((k) => k !== 'stack' && k !== 'message');
  return keys.length === 0 ? base : __rawSpec(keys, ['{', '}'], undefined, base);
}
// a function, regexp, date or error: its base text, alone when it has no keys to print
function __rawBased(ctx, value, recurseTimes, keys, ctor) {
  if (typeof value === 'function') return __rawFn(ctx, value, keys, ctor);
  if (value instanceof RegExp) return __rawRegExp(ctx, value, keys, recurseTimes);
  if (value instanceof Date) return __rawDate(ctx, value, keys);
  return __rawErr(value, keys);
}
function __rawIsBased(value) {
  return typeof value === 'function' || value instanceof RegExp || value instanceof Date || value instanceof Error;
}
function __rawIsBoxed(value) {
  return value instanceof Number || value instanceof String || value instanceof Boolean;
}
function __rawWeak(ctx, value, keys, prefix) {
  return __rawSpec(keys, [`${prefix(value instanceof WeakMap ? 'WeakMap' : 'WeakSet', '')}{`, '}'], () => [__sty(ctx, '<items unknown>', 'special')]);
}
function __rawOther(ctx, value, recurseTimes, keys, ctor, tag, prefix) {
  if (__rawIsBased(value)) return __rawBased(ctx, value, recurseTimes, keys, ctor);
  if (__rawIsBoxed(value)) return __rawBoxed(ctx, value, keys);
  if (value instanceof Promise) return __rawSpec(keys, [`${prefix('Promise', '')}{`, '}'], () => [__sty(ctx, '<pending>', 'special')]);
  if (value instanceof WeakMap || value instanceof WeakSet) return __rawWeak(ctx, value, keys, prefix);
  return __rawPlain(keys, ctor, tag, prefix);
}
function __rawPlain(keys, ctor, tag, prefix) {
  const p = ctor === 'Object' && !(typeof tag === 'string' && tag !== '') ? '' : prefix('Object', '');
  if (keys.length === 0) return `${p}{}`;
  return __rawSpec(keys, [`${p}{`, '}']);
}
// formatRaw's kind of value: a string when that alone is the result
function __rawKind(ctx, value, recurseTimes, keys, ctor, tag, prefix) {
  if (Array.isArray(value)) return __rawArray(ctx, value, keys, ctor, tag, prefix);
  if (value instanceof Map || value instanceof Set) return __rawMapSet(ctx, value, keys, prefix);
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) return __rawTyped(ctx, value, keys, ctor, tag, prefix);
  return __rawOther(ctx, value, recurseTimes, keys, ctor, tag, prefix);
}
function __rawPrefix(ctor, tag) {
  return (fallback, size) => {
    let p = ctor === null ? `[${fallback}: null prototype] ` : `${ctor}${size} `;
    if (typeof tag === 'string' && tag !== '' && tag !== (ctor || fallback)) p = `${p.slice(0, -1)} [${tag}] `;
    return p;
  };
}
function __rawAllKeys(value, showHidden) {
  if (showHidden) return Reflect.ownKeys(value);
  return Object.keys(value).concat(Object.getOwnPropertySymbols(value).filter((s) => Object.prototype.propertyIsEnumerable.call(value, s)));
}
function __rawCircularPrefix(ctx, value, res) {
  if (ctx.circular === undefined) return res;
  const index = ctx.circular.get(value);
  if (index === undefined) return res;
  return `${__sty(ctx, `<ref *${index}>`, 'special')} ${res}`;
}
function __formatRaw(ctx, value, recurseTimes, typedArray) {
  void typedArray;
  const ctor = __ctorName(value);
  const tag = value[Symbol.toStringTag];
  const prefix = __rawPrefix(ctor, tag);
  const sp = __rawKind(ctx, value, recurseTimes, __rawAllKeys(value, ctx.showHidden), ctor, tag, prefix);
  if (typeof sp === 'string') return sp;
  if (recurseTimes > ctx.depth) {
    const name = (ctor || tag || 'Object');
    return __sty(ctx, Array.isArray(value) ? '[Array]' : `[${name}]`, 'special');
  }
  recurseTimes++;
  ctx.seen.push(value);
  ctx.currentDepth = recurseTimes;
  // the entries are formatted one level down
  const output = sp.formatter(recurseTimes);
  for (const k of sp.keys) output.push(__formatProperty(ctx, value, recurseTimes, k, sp.extrasType));
  ctx.seen.pop();
  const res = __reduceToSingleString(ctx, output, sp.base, sp.braces, sp.extrasType, recurseTimes, value);
  return __rawCircularPrefix(ctx, value, res);
}
// The options a [util.inspect.custom] method passes on to its inner inspect calls (Node's
// `breakLength` 80 when the caller gives none).
function __inspectCustomOpts(opts) {
  const o = { breakLength: 80, colors: false };
  if (!opts) return o;
  if (typeof opts.breakLength === 'number') o.breakLength = opts.breakLength;
  if (opts.compact !== undefined) o.compact = opts.compact;
  if (opts.colors) o.colors = true;
  if (typeof opts.depth === 'number' || opts.depth === null) o.depth = opts.depth;
  return o;
}
function __nodeInspectDepth(opts) {
  if (opts.depth === null) return Infinity;
  if (typeof opts.depth === 'number') return opts.depth;
  return undefined;
}
function __nodeInspectCtx(opts) {
  const ctx = { seen: [], indentationLvl: 0, currentDepth: 0, depth: 2, breakLength: 80, compact: 3, maxArrayLength: 100, circular: undefined, colors: false };
  if (!opts) return ctx;
  const d = __nodeInspectDepth(opts);
  if (d !== undefined) ctx.depth = d;
  if (typeof opts.breakLength === 'number') ctx.breakLength = opts.breakLength;
  if (opts.compact !== undefined) ctx.compact = opts.compact;
  if (opts.colors) ctx.colors = true;
  if (opts.showHidden) ctx.showHidden = true;
  return ctx;
}
function __nodeInspect(value, opts) {
  return __formatValue(__nodeInspectCtx(opts), value, 0);
}
// util.format: %s %d %i %f %j %o %O %c %%, the rest inspected (strings as they are)
function __nodeFormat(...args) {
  return __nodeFormatWith({}, ...args);
}
// util.formatWithOptions: `o.colors` colours %o / %O and the inspected rest (not %s / %d / …)
// util.format's %s: strings as they are, objects inspected one level deep
function __formatSpecS(a) {
  if (typeof a === 'string') return a;
  if (typeof a === 'bigint') return `${a}n`;
  if (typeof a === 'symbol') return String(a);
  if (typeof a === 'object' && a !== null) return __nodeInspect(a, { depth: 0 });
  return Object.is(a, -0) ? '-0' : String(a);
}
// util.format's numeric specifiers %d, %i, %f
function __formatSpecNum(c, a) {
  if (typeof a === 'symbol') return 'NaN';
  if (c === 'f') return String(parseFloat(a));
  if (typeof a === 'bigint') return `${a}n`;
  if (c === 'i') return String(parseInt(a));
  return Object.is(Number(a), -0) ? '-0' : String(Number(a));
}
function __formatSpecJ(a) { try { return JSON.stringify(a); } catch (e) { return '[Circular]'; } }
const __NUM_SPECS = new Set(['d', 'i', 'f']);
// one util.format specifier (sdifjoOc) applied to its argument
function __formatSpec(c, a, colors) {
  if (c === 's') return __formatSpecS(a);
  if (__NUM_SPECS.has(c)) return __formatSpecNum(c, a);
  if (c === 'j') return __formatSpecJ(a);
  if (c === 'o') return __nodeInspect(a, { depth: 4, colors, showHidden: true });
  if (c === 'O') return __nodeInspect(a, { colors });
  return '';
}
// Returns { out, lastAdvance, consumed: true|false } for the %-handling at position p.
function __nodeFormatStep(first, p, args, i, colors, last) {
  const c = first[p + 1];
  if (c === '%') return { chunk: first.slice(last, p) + '%', nextLast: p + 2, iDelta: 0 };
  if (i >= args.length || !'sdifjoOc'.includes(c)) return null;
  const s = __formatSpec(c, args[i], colors);
  return { chunk: first.slice(last, p) + s, nextLast: p + 2, iDelta: 1 };
}
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
    const step = __nodeFormatStep(first, p, args, i, colors, last);
    if (!step) continue;
    out += step.chunk; last = step.nextLast; i += step.iDelta; p++;
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
// querystring.parse: a repeated key collects its values in an array
function __qsAdd(obj, k, v) {
  if (!(k in obj)) obj[k] = v;
  else if (Array.isArray(obj[k])) obj[k].push(v);
  else obj[k] = [obj[k], v];
}
// querystring.parse: split qs by sep, honoring options.maxKeys (default 1000; non-positive disables)
function __qsSplit(qs, sep, options) {
  const maxKeys = options && typeof options.maxKeys === 'number' ? options.maxKeys : 1000;
  const parts = qs.split(sep);
  return maxKeys > 0 && parts.length > maxKeys ? parts.slice(0, maxKeys) : parts;
}
// querystring.parse: options.decodeURIComponent or the default __querystring.unescape
function __qsDecoder(options) {
  return options && typeof options.decodeURIComponent === 'function' ? options.decodeURIComponent : __querystring.unescape;
}
function __qsEncoder(options) {
  return options && typeof options.encodeURIComponent === 'function' ? options.encodeURIComponent : __querystring.escape;
}
function __qsParsePart(obj, part, eq, dec) {
  if (part.length === 0) return;
  const i = part.indexOf(eq);
  const k = dec((i >= 0 ? part.slice(0, i) : part).replace(/\+/g, ' '));
  const v = i >= 0 ? dec(part.slice(i + eq.length).replace(/\+/g, ' ')) : '';
  __qsAdd(obj, k, v);
}
function __qsStringifyEntry(out, k, v, eq, enc) {
  const ks = enc(__qsPrim(k)) + eq;
  if (Array.isArray(v)) { for (const x of v) out.push(ks + enc(__qsPrim(x))); return; }
  out.push(ks + enc(__qsPrim(v)));
}
const __querystring = {
  unescape: __qsUnescape,
  escape: (s) => encodeURIComponent(typeof s === 'string' ? s : String(s)),
  stringify(obj, sep, eq, options) {
    if (obj === null || typeof obj !== 'object') return '';
    const s = sep || '&', e = eq || '=';
    const enc = __qsEncoder(options);
    const out = [];
    for (const k of Object.keys(obj)) __qsStringifyEntry(out, k, obj[k], e, enc);
    return out.join(s);
  },
  parse(qs, sep, eq, options) {
    const obj = Object.create(null);
    if (typeof qs !== 'string' || qs.length === 0) return obj;
    const s = sep || '&', e = eq || '=';
    const dec = __qsDecoder(options);
    for (const part of __qsSplit(qs, s, options)) __qsParsePart(obj, part, e, dec);
    return obj;
  },
};
__querystring.decode = __querystring.parse;
__querystring.encode = __querystring.stringify;
// Length of the incomplete-but-valid UTF-8 sequence at the end of b (0 if none).
// Continuation bytes a lead byte needs (0 for non-leads / disallowed bytes).
function __utf8LeadNeed(c) {
  if (c >= 0xc2 && c <= 0xdf) return 1;
  if (c >= 0xe0 && c <= 0xef) return 2;
  if (c >= 0xf0 && c <= 0xf4) return 3;
  return 0;
}
function __utf8Tail(b) {
  for (let back = 1; back <= 3 && back <= b.length; back++) {
    const c = b[b.length - back];
    if ((c & 0xc0) === 0x80) continue; // continuation byte: keep looking for the lead
    const need = __utf8LeadNeed(c);
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
  _decodeUtf8(b, final) {
    const keep = final ? 0 : __utf8Tail(b);
    this._rest = b.slice(b.length - keep);
    return __utf8Decode(b, 0, b.length - keep);
  }
  _decodeSingleByte(b) {
    this._rest = new Uint8Array(0);
    let s = '';
    for (const c of b) s += this.encoding === 'hex' ? c.toString(16).padStart(2, '0') : String.fromCharCode(this.encoding === 'ascii' ? c & 0x7f : c);
    return s;
  }
  _decode(b, final) {
    if (this.encoding === 'utf8') return this._decodeUtf8(b, final);
    if (this.encoding === 'utf16le') return this._utf16(b, final);
    if (this.encoding === 'base64' || this.encoding === 'base64url') return this._base64(b, final);
    return this._decodeSingleByte(b);
  }
  _utf16(b, final) {
    let n = b.length - (b.length % 2);
    if (!final && n >= 2) { const last = b[n - 2] | (b[n - 1] << 8); if (last >= 0xd800 && last <= 0xdbff) n -= 2; }
    this._rest = final ? new Uint8Array(0) : b.slice(n);
    let s = '';
    for (let i = 0; i < n; i += 2) s += String.fromCharCode(b[i] | (b[i + 1] << 8));
    return s;
  }
  _base64(b, final) {
    const n = final ? b.length : b.length - (b.length % 3);
    this._rest = b.slice(n);
    let bin = '';
    for (let i = 0; i < n; i++) bin += String.fromCharCode(b[i]);
    const s = n ? globalThis.btoa(bin) : '';
    return this.encoding === 'base64' ? s : s.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }
  write(buf) { return this._decode(this._bytes(buf), false); }
  end(buf) { const s = this._decode(this._bytes(buf === undefined ? new Uint8Array(0) : buf), true); this._rest = new Uint8Array(0); return s; }
}
// ---- assert / assert/strict ----
function __fmtv(v) { try { return typeof v === 'string' ? JSON.stringify(v) : typeof v === 'bigint' ? v + 'n' : (v && typeof v === 'object') ? JSON.stringify(v) : String(v); } catch (e) { return String(v); } }
const __isPrimOrNull = (x) => x === null || typeof x !== 'object';
// deepEqual of two values not both objects
function __deepEqPrim(a, b, strict) {
  if (__isPrimOrNull(a) && __isPrimOrNull(b)) return strict ? Object.is(a, b) : (a == b || (a !== a && b !== b));
  return false;
}
// deepEqual's shape check of two objects: their tag, or null when prototype (strict), tag or
// array-ness differ
function __deepEqTag(a, b, strict) {
  if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return null;
  const tag = Object.prototype.toString.call(a);
  if (tag !== Object.prototype.toString.call(b)) return null;
  if (Array.isArray(a) !== Array.isArray(b)) return null;
  return tag;
}
const __BOXED_TAGS = new Set(['[object Number]', '[object String]', '[object Boolean]', '[object BigInt]', '[object Symbol]']);
function __deepEqView(a, b, strict) {
  if (a.length !== b.length || a.byteLength !== b.byteLength) return false;
  for (let i = 0; i < a.length; i++) if (strict ? !Object.is(a[i], b[i]) : a[i] != b[i]) return false;
  return true;
}
function __deepEqRegExp(a, b) {
  return a.source === b.source && a.flags === b.flags && a.lastIndex === b.lastIndex;
}
// deepEqual of the internal value of a date, regexp, error, typed array or boxed primitive
function __deepEqValue(a, b, strict, tag) {
  if (a instanceof Date) return a.getTime() === b.getTime();
  if (a instanceof RegExp) return __deepEqRegExp(a, b);
  if (a instanceof Error) return a.message === b.message && a.name === b.name;
  if (ArrayBuffer.isView(a)) return __deepEqView(a, b, strict);
  if (__BOXED_TAGS.has(tag)) return Object.is(a.valueOf(), b.valueOf());
  return true;
}
// Loose-equal fallback for a Map key that b.has() misses: deep-equal k to some b-key with a
// deep-equal value. (Used by __deepEqMap.)
function __deepEqMapHit(k, v, b, strict, seen) {
  for (const [k2, v2] of b) if (__deepEq(k, k2, strict, seen) && __deepEq(v, v2, strict, seen)) return true;
  return false;
}
function __deepEqMapEntry(k, v, b, strict, seen) {
  if (b.has(k)) return __deepEq(v, b.get(k), strict, seen);
  // an object key (or, loosely, a primitive one) can still match a deep-equal key
  if (strict && (k === null || typeof k !== 'object')) return false;
  return __deepEqMapHit(k, v, b, strict, seen);
}
function __deepEqMap(a, b, strict, seen) {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) if (!__deepEqMapEntry(k, v, b, strict, seen)) return false;
  return true;
}
function __deepEqSetHit(v, b, strict, seen) {
  for (const v2 of b) if (__deepEq(v, v2, strict, seen)) return true;
  return false;
}
function __deepEqSetEntry(v, b, strict, seen) {
  if (b.has(v)) return true;
  if (strict && (v === null || typeof v !== 'object')) return false;
  return __deepEqSetHit(v, b, strict, seen);
}
function __deepEqSet(a, b, strict, seen) {
  if (a.size !== b.size) return false;
  for (const v of a) if (!__deepEqSetEntry(v, b, strict, seen)) return false;
  return true;
}
// deepEqual of the own enumerable keys (and, strictly, symbols)
function __deepEqKeys(a, b, strict, seen) {
  const own = (o) => { const ks = Object.keys(o); if (strict) for (const s of Object.getOwnPropertySymbols(o)) if (Object.prototype.propertyIsEnumerable.call(o, s)) ks.push(s); return ks; };
  const ka = own(a), kb = own(b);
  if (ka.length !== kb.length) return false;
  for (const k of ka) {
    if (!Object.prototype.propertyIsEnumerable.call(b, k)) return false;
    if (!__deepEq(a[k], b[k], strict, seen)) return false;
  }
  return true;
}
// deepEqual's cycle memo: true when a/b is already being compared (else it is recorded)
function __deepEqSeen(seen, a, b) {
  const memo = seen.get(a);
  if (memo && memo.has(b)) return true;
  if (memo) memo.add(b); else seen.set(a, new Set([b]));
  return false;
}
// A Map or Set of entries must deep-equal before __deepEq walks own-keys; other objects pass.
function __deepEqCollection(a, b, strict, seen) {
  if (a instanceof Map) return __deepEqMap(a, b, strict, seen);
  if (a instanceof Set) return __deepEqSet(a, b, strict, seen);
  return true;
}
// `a === b` ends the comparison except for -0 vs 0 under strict, which splits on Object.is.
function __deepEqSame(a, b, strict) { return a !== 0 || !strict || Object.is(a, b); }
function __deepEqObjects(a, b, strict, seen, tag) {
  if (__deepEqSeen(seen, a, b)) return true;
  if (!__deepEqValue(a, b, strict, tag)) return false;
  if (!__deepEqCollection(a, b, strict, seen)) return false;
  return __deepEqKeys(a, b, strict, seen);
}
function __deepEq(a, b, strict, seen) {
  if (a === b) return __deepEqSame(a, b, strict);
  if (__isPrimOrNull(a) || __isPrimOrNull(b)) return __deepEqPrim(a, b, strict);
  const tag = __deepEqTag(a, b, strict);
  if (tag === null) return false;
  return __deepEqObjects(a, b, strict, seen || new Map(), tag);
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
  const matchesFnErrorClass = (expected) => Error.isPrototypeOf(expected) || expected === Error;
  // error validation for throws/rejects: a class, a RegExp, a validation function or an object
  const matchesFn = (err, expected, message, operator) => {
    if (expected.prototype !== undefined && err instanceof expected) return;
    if (matchesFnErrorClass(expected)) fail(err, expected, message, operator, 'The error is expected to be an instance of "' + expected.name + '". Received "' + (err && err.constructor && err.constructor.name) + '"');
    const res = expected.call({}, err);
    if (res !== true) fail(err, expected, message, operator, 'The validation function is expected to return "true". Received ' + __fmtv(res));
  };
  const matchesObj = (err, expected, message, operator) => {
    for (const k of Object.keys(expected)) {
      const want = expected[k], got = err == null ? undefined : err[k];
      const ok = want instanceof RegExp && typeof got === 'string' ? want.test(got) : __deepEq(got, want, true);
      if (!ok) fail(err, expected, message, operator, 'Expected values to be strictly deep-equal: ' + k + ' ' + __fmtv(got) + ' !== ' + __fmtv(want));
    }
  };
  const matches = (err, expected, message, operator) => {
    if (expected === undefined) return;
    if (typeof expected === 'function') { matchesFn(err, expected, message, operator); return; }
    if (expected instanceof RegExp) { if (expected.test(String(err))) return; fail(err, expected, message, operator, 'The input did not match the regular expression ' + String(expected) + '. Input:\n\n' + __fmtv(String(err)) + '\n'); }
    if (expected && typeof expected === 'object') matchesObj(err, expected, message, operator);
  };
  // Does `e` match the doesNotThrow / doesNotReject `expected` — a class, a RegExp, or a predicate?
  const unwantedHit = (e, expected) => {
    if (expected instanceof RegExp) return expected.test(String(e));
    if (expected.prototype !== undefined && e instanceof expected) return true;
    return !Error.isPrototypeOf(expected) && expected !== Error && expected.call({}, e) === true;
  };
  // doesNotThrow / doesNotReject: an error that does not match `expected` propagates as is
  const unwanted = (e, expected, message, operator, what) => {
    if (expected !== undefined) {
      if (typeof expected !== 'function' && !(expected instanceof RegExp)) { const t = new TypeError('The "expected" argument must be of type function or an instance of RegExp'); t.code = 'ERR_INVALID_ARG_TYPE'; throw t; }
      if (!unwantedHit(e, expected)) throw e;
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
// a name the namespace does not list (a CommonJS source's `__esModule`) is not re-exported
const __ttNsLookupKey = (info, ns, k) => {
  const r = Object.prototype.hasOwnProperty.call(info.re, k) ? info.re[k] : undefined;
  if (r !== undefined) return { v: globalThis.__ttR(r[0], r[1]) };
  if (k !== 'default') for (const s of info.stars) if (k in s) return { v: globalThis.__ttR(s, k) };
  return undefined;
};
const __ttNsLookup = (ns, k) => {
  const info = __ttNsInfo.get(__ttNsTarget(ns));
  if (info === undefined || typeof k !== 'string') return undefined;
  if (info.over.has(k)) return { v: info.over.get(k) };
  if (!info.own.has(k) && k in ns) return __ttNsLookupKey(info, ns, k);
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
const __ttNsRawOf = (ns) => (ns !== null && typeof ns === 'object' && __ttNsRaw.get(ns)) || ns;
const __ttNsInfoOf = (raw) => (raw !== null && typeof raw === 'object' ? __ttNsInfo.get(__ttNsTarget(raw)) : undefined);
globalThis.__ttNsSet = (ns, k, v) => {
  const raw = __ttNsRawOf(ns);
  const info = __ttNsInfoOf(raw);
  if (info === undefined || typeof k !== 'string' || !(k in raw)) return false;
  info.over.delete(k);
  if (globalThis.__ttR(raw, k) !== v) info.over.set(k, v);
  return true;
};
// require() of an ES module from CommonJS (`isEsm` false) gives Node's require(esm) object: one
// with a default export and no `__esModule` of its own reads an enumerable `__esModule: true`
// (turbo-test#133).
const __ttRequireEsmTag = (m) => m && typeof m === 'object' && m[Symbol.for('turbo-test.esm')] && 'default' in m;
const __ttRequireEsm = (m) => {
  if (!__ttRequireEsmTag(m)) return m;
  const d = Object.getOwnPropertyDescriptor(m, '__esModule');
  if (d && d.configurable && !d.enumerable) Object.defineProperty(m, '__esModule', { value: true, enumerable: true, configurable: true });
  return m;
};
// jest automock (turbo-test#184): when enabled, a `require('./x')` of a local module whose
// spec was not explicitly mocked or unmocked returns a generated mock of the real exports.
// State is file-scoped (reset in __ttResetForNextFile, like __TT_JEST_TIMEOUT), per jest.
let __ttAutomockOn = false;
// The generator (`jest.onGenerateMock(factory)`): `(exports) => mockedExports`. Default:
// `vi.mockObject(exports)` (same deep automocker vitest's factory-less vi.mock uses).
const __ttAutomockDefaultGen = (_path, exports) => __automock(exports, 0);
let __ttAutomockGenerator = __ttAutomockDefaultGen;
// Specs the file has explicitly mocked or unmocked (jest.mock/doMock/unmock/setMock/vi.mock
// /vi.unmock), by their resolved key: these skip automock interception.
const __ttAutomockExplicit = new Set();
// Per-file (dir, spec) → resolved absolute path cache so __ttAutomockShould doesn't cross the
// JS→Rust boundary for every require. Cleared in __ttResetForNextFile.
const __ttAutomockResolveCache = new Map();
function __ttAutomockResolve(dir, spec) {
  const k = dir + '\0' + spec;
  let v = __ttAutomockResolveCache.get(k);
  if (v === undefined) {
    v = globalThis.__ttRequireResolve(dir, spec, true) || '';
    __ttAutomockResolveCache.set(k, v);
  }
  return v;
}
function __ttAutomockMark(dir, spec) {
  const s = String(spec);
  const key = (dir && __ttAutomockResolve(dir, s)) || '\0' + s;
  __ttAutomockExplicit.add(key);
}
globalThis.__ttAutomockMark = __ttAutomockMark;
// A specifier that automock may touch: relative ('./x', '../x') or absolute ('/x') only —
// never a bare package ('react') or node: builtin. Bare specifiers always fall through.
function __ttAutomockLocalSpec(s) {
  const first = s.charCodeAt(0);
  return (first === 46 /* . */ || first === 47 /* / */) && !s.startsWith('node:');
}
// Whether a `require(spec)` from `dir` should be auto-mocked: automock is on, the spec is
// local, the resolved module is not under node_modules, and it has not been explicitly
// mocked / unmocked.
function __ttAutomockShould(dir, spec) {
  if (!__ttAutomockOn) return false;
  const s = String(spec);
  if (!__ttAutomockLocalSpec(s)) return false;
  const resolved = dir ? __ttAutomockResolve(dir, s) : '';
  if (!resolved || resolved.includes('/node_modules/')) return false;
  return !__ttAutomockExplicit.has(resolved);
}
// Check for a colocated `__mocks__/<basename>` manual mock next to the resolved module path.
// Jest loads the manual mock when present, even for a factory-less `jest.mock('./x')`.
function __ttManualMockPath(dir, spec) {
  const resolved = dir ? __ttAutomockResolve(dir, String(spec)) : '';
  if (!resolved) return '';
  const d = globalThis.__dirnameOf(resolved);
  const b = __basename(resolved);
  const candidate = d + '/__mocks__/' + b;
  return globalThis.__fs_existsSync(candidate) ? candidate : '';
}
// Do NOT swallow onGenerateMock errors: jest surfaces them to the test (a broken generator
// must fail loudly, not silently return the real module and hide the bug).
function __ttAutomockApply(exports, resolvedPath) {
  return __ttAutomockGenerator(resolvedPath, exports);
}
function __ttAutomockResetForNextFile() {
  __ttAutomockOn = false;
  __ttAutomockGenerator = __ttAutomockDefaultGen;
  __ttAutomockExplicit.clear();
  __ttAutomockResolveCache.clear();
}
globalThis.__mkRequire = (dir, isEsm, importer) => {
  const req = (spec) => {
    __ttJestLazyRun && __ttJestLazyRun(dir, spec);
    const raw = globalThis.__nativeRequire(dir, spec, isEsm, importer);
    const real = isEsm === false ? __ttRequireEsm(raw) : raw;
    if (!__ttAutomockShould(dir, spec)) return real;
    const mp = __ttManualMockPath(dir, spec);
    if (mp) return globalThis.__nativeRequire(globalThis.__dirnameOf(mp), './' + __basename(mp), isEsm, importer);
    return __ttAutomockApply(real, __ttAutomockResolve(dir, String(spec)));
  };
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
// Builtin constructors handled specially by __Any#asymmetricMatch: constructor -> [typeof tag,
// wrapper constructor or null]. Function/Object have no wrapper form (typeof check only).
const __anyMatchTable = new Map([
  [String, ['string', String]],
  [Number, ['number', Number]],
  [Function, ['function', null]],
  [Boolean, ['boolean', Boolean]],
  [BigInt, ['bigint', BigInt]],
  [Symbol, ['symbol', Symbol]],
  [Object, ['object', null]],
]);
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
    // sample -> [typeof tag, boxed constructor or null]; boxed is null for Function/Object (no wrapper form)
    const row = __anyMatchTable.get(s);
    if (row) return typeof other == row[0] || (row[1] !== null && other instanceof row[1]);
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
function __hasTypedKey(o, k, type) {
  return k in o && typeof o[k] === type;
}
function __isDomNode(o) {
  // `in` first, as vitest: a mocked module's namespace throws on a read of a missing export
  if (o === null || typeof o !== 'object') return false;
  return __hasTypedKey(o, 'nodeType', 'number') && __hasTypedKey(o, 'nodeName', 'string') && __hasTypedKey(o, 'isEqualNode', 'function');
}
function __equals(a, b, testers, strictCheck) {
  return __eq(a, b, [], [], testers || [], strictCheck ? __hasKey : __hasDefinedKey);
}
// __eq's "no answer yet" (an asymmetric matcher or tester may answer anything else)
const __EQ_NEXT = Symbol('eq-next');
// __eq custom-tester chain: the first non-undefined result wins; __EQ_NEXT if none answered.
function __eqTesters(a, b, testers) {
  const ctx = { equals: __equals };
  for (const t of testers) {
    const r = t.call(ctx, a, b, testers);
    if (r !== undefined) return r;
  }
  return __EQ_NEXT;
}
// A side-only asymmetric matcher runs its own equality.
function __eqAsym(a, b, asymA, testers) {
  if (asymA) return a.asymmetricMatch(b, testers);
  return b.asymmetricMatch(a, testers);
}
const __eqBothUrl = (a, b) => typeof URL === 'function' && a instanceof URL && b instanceof URL;
// __eq before the structural walk: asymmetric matchers, custom testers, URLs, identity, null
function __eqPre(a, b, testers) {
  const asymA = __isAsym(a), asymB = __isAsym(b);
  if (asymA !== asymB) return __eqAsym(a, b, asymA, testers);
  const t = __eqTesters(a, b, testers);
  if (t !== __EQ_NEXT) return t;
  if (__eqBothUrl(a, b)) return a.href === b.href;
  if (Object.is(a, b)) return true;
  if (a === null || b === null) return a === b;
  return __EQ_NEXT;
}
// Dates are equal when they hold the same timestamp (NaN === NaN is also equal).
function __eqDate(a, b) {
  const x = +a, y = +b;
  return x === y || (Number.isNaN(x) && Number.isNaN(y));
}
const __EQ_PRIMITIVE_TAGS = new Set(['[object Boolean]', '[object String]', '[object Number]']);
function __eqPrimitiveClass(a, b) {
  if (typeof a !== typeof b) return false;
  if (typeof a !== 'object') return Object.is(a, b);
  return Object.is(a.valueOf(), b.valueOf());
}
// __eq of booleans, strings, numbers (boxed or not), dates and regexps (undefined: other classes)
function __eqByClass(a, b, cls) {
  if (__EQ_PRIMITIVE_TAGS.has(cls)) return __eqPrimitiveClass(a, b);
  if (cls === '[object Date]') return __eqDate(a, b);
  if (cls === '[object RegExp]') return a.source === b.source && a.flags === b.flags;
  return undefined;
}
// __eq's cycle check: a pair already on the stacks (undefined: not there)
function __eqCycle(a, b, aStack, bStack) {
  for (let i = aStack.length; i--;) {
    if (aStack[i] === a) return bStack[i] === b;
    if (bStack[i] === b) return false;
  }
  return undefined;
}
function __eqErrorShape(a, b) {
  return Object.prototype.toString.call(a) === Object.prototype.toString.call(b) && a.name === b.name && a.message === b.message;
}
function __eqErrorExtras(a, b, aStack, bStack, testers, hasKey) {
  if (typeof b.cause !== 'undefined' && !__eq(a.cause, b.cause, aStack, bStack, testers, hasKey)) return false;
  if (a instanceof AggregateError && b instanceof AggregateError && !__eq(a.errors, b.errors, aStack, bStack, testers, hasKey)) return false;
  return __eq({ ...a }, { ...b }, aStack, bStack, testers, hasKey);
}
function __eqError(a, b, aStack, bStack, testers, hasKey) {
  try {
    return __eqErrorShape(a, b) && __eqErrorExtras(a, b, aStack, bStack, testers, hasKey);
  } finally {
    aStack.pop();
    bStack.pop();
  }
}
function __eqOwnKeys(a, b, aStack, bStack, testers, hasKey) {
  const aKeys = __eqKeys(a, hasKey);
  if (__eqKeys(b, hasKey).length !== aKeys.length) return false;
  for (let i = aKeys.length; i--;) {
    const k = aKeys[i];
    if (!(hasKey(b, k) && __eq(a[k], b[k], aStack, bStack, testers, hasKey))) return false;
  }
  return true;
}
// __eq fast-path before the stack push: class/byClass/DOM/cycle. Returns __EQ_NEXT when the
// caller must continue with the structural walk; any other value is the final answer.
function __eqFastObject(a, b, aStack, bStack) {
  if (__isDomNode(a) && __isDomNode(b)) return a.isEqualNode(b);
  const cycle = __eqCycle(a, b, aStack, bStack);
  return cycle !== undefined ? cycle : __EQ_NEXT;
}
function __eqFast(a, b, aStack, bStack, cls) {
  if (cls !== Object.prototype.toString.call(b)) return false;
  const byClass = __eqByClass(a, b, cls);
  if (byClass !== undefined) return byClass;
  if (typeof a !== 'object' || typeof b !== 'object') return false;
  return __eqFastObject(a, b, aStack, bStack);
}
function __eqWalk(a, b, aStack, bStack, testers, hasKey, cls) {
  aStack.push(a);
  bStack.push(b);
  if (cls === '[object Array]' && a.length !== b.length) return false;
  if (__isError(a) && __isError(b)) return __eqError(a, b, aStack, bStack, testers, hasKey);
  if (!__eqOwnKeys(a, b, aStack, bStack, testers, hasKey)) return false;
  aStack.pop();
  bStack.pop();
  return true;
}
function __eq(a, b, aStack, bStack, testers, hasKey) {
  const pre = __eqPre(a, b, testers);
  if (pre !== __EQ_NEXT) return pre;
  const cls = Object.prototype.toString.call(a);
  const fast = __eqFast(a, b, aStack, bStack, cls);
  if (fast !== __EQ_NEXT) return fast;
  return __eqWalk(a, b, aStack, bStack, testers, hasKey, cls);
}
const __isComparableIter = (v) => v != null && typeof v === 'object' && !Array.isArray(v) && !!v[Symbol.iterator];
const __bothIterable = (a, b) => __isComparableIter(a) && __isComparableIter(b);
// iterableEquality of two Sets: every member of `a` is in `b` or deep-equals one there
function __iterSetEq(a, b, inner) {
  for (const av of a) {
    if (!b.has(av)) {
      let has = false;
      for (const bv of b) if (__equals(av, bv, inner) === true) has = true;
      if (!has) return false;
    }
  }
  return true;
}
// iterableEquality of two Maps: every entry of `a` is in `b` or deep-equals one there
function __iterMapHit(ak, av, b, inner) {
  for (const [bk, bv] of b) if (__equals(ak, bk, inner) === true && __equals(av, bv, inner) === true) return true;
  return false;
}
function __iterMapEq(a, b, inner) {
  for (const [ak, av] of a) {
    if (b.has(ak) && __equals(av, b.get(ak), inner)) continue;
    if (!__iterMapHit(ak, av, b, inner)) return false;
  }
  return true;
}
// iterableEquality of other iterables: in iteration order, then their enumerable own entries
function __iterOrderedEq(a, b, inner) {
  const bi = b[Symbol.iterator]();
  for (const av of a) {
    const nb = bi.next();
    if (nb.done || !__equals(av, nb.value, inner)) return false;
  }
  if (!bi.next().done) return false;
  return !!__equals(Object.entries(a), Object.entries(b), inner);
}
// iterableEquality's sized branch (Set/Map). Returns a boolean answer, or __EQ_NEXT when the
// value is sized but needs the ordered fallthrough (same-constructor Sets without a toString
// of [object Set], etc.).
function __iterSizedEq(a, b, inner) {
  if (a.size !== b.size) return false;
  const tag = Object.prototype.toString.call(a);
  if (tag === '[object Set]') return __iterSetEq(a, b, inner);
  if (tag === '[object Map]') return __iterMapEq(a, b, inner);
  return __EQ_NEXT;
}
// The structural comparison inside __iterableEquality, once both values are on the stacks:
// tries the Set / Map sized path first, then the ordered iteration fallback, popping on success.
function __iterableEqualityCompare(a, b, testers, aStack, bStack) {
  const withStack = (x, y) => __iterableEquality(x, y, [...testers], [...aStack], [...bStack]);
  const inner = [...testers.filter((t) => t !== __iterableEquality), withStack];
  const sized = a.size !== undefined ? __iterSizedEq(a, b, inner) : __EQ_NEXT;
  if (sized !== __EQ_NEXT) { aStack.pop(); bStack.pop(); return sized; }
  if (!__iterOrderedEq(a, b, inner)) return false;
  aStack.pop();
  bStack.pop();
  return true;
}
// vitest iterableEquality: two non-array iterables of the same constructor — Sets by members and
// Maps by entries (in any order, deep-equal members match), others in iteration order plus their
// enumerable own entries.
function __iterStackHit(a, b, aStack, bStack) {
  for (let i = aStack.length; i--;) if (aStack[i] === a) return bStack[i] === b;
  return __EQ_NEXT;
}
function __iterableEquality(a, b, testers = [], aStack = [], bStack = []) {
  if (!__bothIterable(a, b)) return undefined;
  if (a.constructor !== b.constructor) return false;
  const seen = __iterStackHit(a, b, aStack, bStack);
  if (seen !== __EQ_NEXT) return seen;
  aStack.push(a);
  bStack.push(b);
  return __iterableEqualityCompare(a, b, testers, aStack, bStack);
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
function __arrayBufferViews(a, b) {
  if (a instanceof DataView && b instanceof DataView) return [a, b];
  if (!(a instanceof ArrayBuffer) || !(b instanceof ArrayBuffer)) return null;
  try { return [new DataView(a), new DataView(b)]; } catch { return null; }
}
// vitest arrayBufferEquality: ArrayBuffers / DataViews by their bytes.
function __arrayBufferEquality(a, b) {
  const views = __arrayBufferViews(a, b);
  if (views === null) return undefined;
  const [x, y] = views;
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
  const isPlainSubsetTarget = (sub) => sub !== null && typeof sub === 'object' && !__isError(sub) && !Array.isArray(sub) && !(sub instanceof Date) && !(sub instanceof Set) && !(sub instanceof Map);
  const withSeen = (seen = new WeakMap()) => (obj, sub) => {
    if (!isPlainSubsetTarget(sub)) return undefined;
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
// natural-compare-lite's char-code mapping: lifts digits/letters into one monotonic range so
// mixed alphanumeric strings compare in natural order (kept exactly as the upstream expression).
// Each entry: [upper bound, delta]. c < hi → c + delta. The final branch handles c ≥ 123 as c - 63.
const __NAT_RANGES = [[46, 20], [48, -1], [58, 18], [65, -11], [91, 11], [97, -37], [123, 5]];
function __natCharCode(c) {
  if (c < 45 || c > 127) return c;
  for (const [hi, delta] of __NAT_RANGES) if (c < hi) return c + delta;
  return c - 63;
}
const __isDigitCode = (c) => c < 76 && c > 65;
// natural-compare-lite, the order vitest writes snapshot keys in.
function __naturalCompare(a, b) {
  let i, codeA, codeB = 1, posA = 0, posB = 0;
  const code = (str, pos, num) => {
    if (num) {
      for (i = pos; (num = code(str, i)), __isDigitCode(num);) ++i;
      return +str.slice(pos - 1, i);
    }
    return __natCharCode(str.charCodeAt(pos) || 0);
  };
  const promoteDigits = () => {
    codeA = code(a, posA, posA);
    codeB = code(b, posB, (posA = i));
    posB = i;
  };
  if ((a += '') !== (b += '')) {
    for (; codeB;) {
      codeA = code(a, posA++);
      codeB = code(b, posB++);
      if (__isDigitCode(codeA) && __isDigitCode(codeB)) promoteDigits();
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
// toMatchSnapshot's first-arg overload: a string is a hint, an object is the property matcher.
function __snapPropArgs(propertiesOrHint, hint) {
  if (typeof propertiesOrHint === 'string') return [undefined, propertiesOrHint];
  if (propertiesOrHint !== null && typeof propertiesOrHint === 'object') return [propertiesOrHint, hint];
  return [undefined, hint];
}
// toMatchInlineSnapshot's arg shuffle: (inline?) or (props, inline?, hint?)
function __inlineSnapArgs(a, b, c) {
  if (typeof a === 'string') return { props: undefined, inline: __stripSnapIndent(a), hint: b };
  return { props: a, inline: b ? __stripSnapIndent(b) : undefined, hint: c };
}
// Validate + merge the property matcher object over `received`; identity-pass when there is none.
function __snapApplyProps(received, props) {
  if (!props) return received;
  if (typeof received !== 'object' || !received) throw new Error('Received value must be an object when the matcher has properties');
  if (!__subsetMatches(received, props)) throw new Error('Snapshot properties mismatched');
  return __snapMerge(received, props);
}
const __snapIsObj = (x) => x != null && typeof x === 'object' && !Array.isArray(x) && !__isAsym(x);
function __snapMergeEntry(target, source) {
  if (__snapIsObj(source) && __snapIsObj(target)) return __snapMerge(target, source);
  if (Array.isArray(source)) return __snapMerge(target, source);
  return source;
}
function __snapMergeArrays(target, source) {
  const out = Array.from(target);
  source.forEach((s, i) => { out[i] = __snapMergeEntry(out[i], s); });
  return out;
}
// vitest deepMergeSnapshot: the property matchers replace the received values they name
function __snapMerge(target, source) {
  if (Array.isArray(target) && Array.isArray(source)) return __snapMergeArrays(target, source);
  if (!__snapIsObj(target) || !__snapIsObj(source)) return source === undefined ? target : source;
  const out = { ...target };
  for (const k of Object.keys(source)) out[k] = __snapMergeEntry(target[k], source[k]);
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
function __snapReconcileWrite(st, pass, hasSnapshot, add, mode) {
  if (mode !== 'all') { add(); st.added++; return; }
  if (pass) { st.matched++; return; }
  if (hasSnapshot) st.updated++; else st.added++;
  add();
}
function __snapReconcile(st, pass, hasSnapshot, add) {
  const mode = __snapMode();
  if ((hasSnapshot && mode === 'all') || (!hasSnapshot && mode !== 'none')) {
    __snapReconcileWrite(st, pass, hasSnapshot, add, mode);
    return true;
  }
  if (!pass) { st.unmatched++; return false; }
  st.matched++;
  return true;
}
// @vitest/snapshot stripSnapshotIndentation — returns the stripped lines or null when the
// snapshot shouldn't be stripped (keeps its original indent instead).
function __stripSnapLines(lines, indent) {
  for (let i = 1; i < lines.length - 1; i++) {
    if (lines[i] === '') continue;
    if (lines[i].indexOf(indent) !== 0) return null;
    lines[i] = lines[i].substring(indent.length);
  }
  return lines;
}
function __stripSnapIndent(snap) {
  const m = snap.match(/^([^\S\n]*)\S/m);
  if (!m || !m[1]) return snap;
  const lines = snap.split(/\n/g);
  if (lines.length <= 2 || lines[0].trim() !== '' || lines[lines.length - 1].trim() !== '') return snap;
  const stripped = __stripSnapLines(lines, m[1]);
  if (stripped === null) return snap;
  stripped[stripped.length - 1] = '';
  return stripped.join('\n');
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
// End of a `{..}` or `(..)` span, counting nested pairs: returns the index just past the
// matching close, or -1 if it is not in `at`.
function __matchingClose(at, from, openCh, closeCh) {
  let depth = 0;
  for (let j = from; j < at.length; j++) {
    if (at[j] === openCh) depth++;
    else if (at[j] === closeCh && --depth === 0) return closeCh === '}' ? j + 1 : j;
  }
  return -1;
}
// __inlineSplice of a call whose first argument is a property matcher object: the snapshot
// goes after it
function __inlineSpliceProps(code, index, snap, name, at, ws) {
  // toMatchInlineSnapshot({ …properties }) [, `…`]
  const obj = new RegExp(`${name}${ws}\\{`).exec(at);
  if (!obj) return null;
  const end = __matchingClose(at, obj.index + obj[0].length - 1, '{', '}');
  const callEnd = __matchingClose(at, obj.index + name.length, '(', ')');
  if (end < 0 || callEnd < 0) return null;
  const lit = `, ${__snapLiteral(snap, code, index)}`;
  return end === callEnd ? [index + callEnd, index + callEnd, lit] : [index + end, index + callEnd, lit];
}
// @vitest/snapshot replaceInlineSnap / replaceObjectSnap on `code` at `index` (the method name):
// the splice [start, end, text] that writes `snap` as the call's snapshot argument.
function __inlineSplice(code, index, snap, name) {
  const at = code.slice(index);
  const ws = String.raw`\s*\(\s*(?:\/\*[\s\S]*\*\/\s*|\/\/.*(?:[\n\r  ]\s*|[\t\v\f \xA0  -   　﻿]))*`;
  const start = new RegExp(`${name}${ws}[\\w$]*(['"\`)])`).exec(at);
  const first = at.indexOf(name);
  if (!start || start.index !== first) {
    return __inlineSpliceProps(code, index, snap, name, at, ws);
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
// the snapshots of skipped tests count as checked
function __snapCheckSkipped(st, skipped) {
  for (const name of skipped) {
    for (const k of st.unchecked) if (k.startsWith(name) && / \d+$| > /.test(k.slice(name.length))) st.unchecked.delete(k);
  }
}
// __snapFinish's save of the snapshot file (or, under -u, delete of an emptied one); returns
// whether the file counts as deleted
function __snapWriteExternal(st) {
  globalThis.__fs_mkdir(st.sf.dir, true);
  globalThis.__fs_writeFile(st.sf.path, __serializeSnapFile(st.data), false);
}
function __snapHasPendingWrite(st, external) {
  return (st.dirty || st.unchecked.size) && (external || st.inline || st.raw);
}
function __snapSave(st, external, mode) {
  if (__snapHasPendingWrite(st, external)) {
    if (external) __snapWriteExternal(st);
    return false;
  }
  if (!external && st.exists) {
    if (mode === 'all') globalThis.__fs_remove(st.sf.path, 'unlink');
    return true;
  }
  return false;
}
// End of the file: mark the snapshots of skipped tests checked, drop the obsolete ones under -u,
// save (or, under -u, delete an emptied file) and report the counts.
function __snapDropObsolete(st, uncheckedKeys) {
  for (const k of uncheckedKeys) delete st.data[k];
  st.dirty = true;
  st.unchecked.clear();
}
function __snapFinish(skipped) {
  const st = __snapState;
  if (!st.loaded) {
    const sf = __snapFile();
    if (!sf || !globalThis.__fs_existsSync(sf.path)) return undefined;
    __snapLoad();
  }
  __snapCheckSkipped(st, skipped);
  const mode = __snapMode();
  const uncheckedKeys = [...st.unchecked];
  if (uncheckedKeys.length && mode === 'all') __snapDropObsolete(st, uncheckedKeys);
  __writeInlineSnapshots(st.inlineWrites);
  const external = Object.keys(st.data).length;
  const deleted = __snapSave(st, external, mode);
  return {
    added: st.added, matched: st.matched, unmatched: st.unmatched, updated: st.updated,
    unchecked: deleted ? 0 : uncheckedKeys.length, unchecked_keys: uncheckedKeys, file_deleted: deleted,
  };
}

// ---- expect ----
// toThrow's call of `actual`: [whether it threw (or is an Error), what]
function __thrownBy(actual) {
  if (typeof actual === 'function') { try { actual(); } catch (e) { return [true, e]; } return [false, undefined]; }
  if (actual instanceof Error) return [true, actual];
  return [false, undefined];
}
function __isDomContain(actual) { return typeof Node !== 'undefined' && actual instanceof Node; }
function __toContainDom(c, actual, x) {
  if (!(x instanceof Node)) throw new TypeError(`toContain() expected a DOM node as the argument, but got ${typeof x}`);
  return c(actual.contains(x), 'expected #{this} to contain element #{exp}', 'expected #{this} not to contain element #{exp}', x);
}
// toThrow() / toThrow(string) / toThrow(RegExp), as chai's `throws` with vitest's messages
function __throwShown(x) {
  if (x instanceof Error) return x.toString();
  if (typeof x === 'string') return x;
  return x && x.constructor && x.constructor.name;
}
function __throwMessage(err) {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return String(err);
}
function __toThrowChai(negated, expected, threw, err, fnShown) {
  if (expected === undefined) {
    return __chaiAssert(negated, threw, 'expected #{this} to throw an error', 'expected #{this} to not throw an error but #{act} was thrown', fnShown, undefined, __throwShown(err));
  }
  if (!threw) {
    if (negated) return;
    throw new __ChaiAssertionError(__chaiMsg('expected #{this} to throw an error', fnShown));
  }
  const message = __throwMessage(err);
  const re = expected === '' ? /^$/ : expected;
  const matches = re instanceof RegExp ? re.test(message) : message.includes(re);
  const verb = re instanceof RegExp ? 'matching' : 'including';
  return __chaiAssert(negated, matches, `expected #{this} to throw error ${verb} #{exp} but got #{act}`, `expected #{this} to throw error not ${verb} #{exp}`, fnShown, re, message);
}
// toThrow(class / Error / asymmetric matcher / value); `c` is the matcher's chai assert
function __toThrowFn(c, expected, threw, err) {
  const name = expected.name || expected.prototype.constructor.name;
  return c(threw && err instanceof expected, `expected error to be instance of ${name}`, `expected error not to be instance of ${name}`, expected, err);
}
function __toThrowValue(c, testers, expected, threw, err) {
  if (typeof expected === 'function') return __toThrowFn(c, expected, threw, err);
  // an Error or any other value: vitest's `equals(thrown, expected, [iterableEquality])`
  // (Errors by name / message / cause / own props; a plain object never equals an Error)
  if (__isError(expected)) {
    return c(threw && __equals(err, expected, [...testers, __iterableEquality]), 'expected a thrown error to be #{exp}', 'expected a thrown error not to be #{exp}', expected, err);
  }
  if (__isAsym(expected)) {
    return c(threw && expected.asymmetricMatch(err), 'expected error to match asymmetric matcher', 'expected error not to match asymmetric matcher', expected, err);
  }
  c(threw && __equals(err, expected, [...testers, __iterableEquality]), 'expected a thrown value to equal #{exp}', 'expected a thrown value not to equal #{exp}', expected, err);
}
// The jest-style matchers of one call on a chai Assertion (__attachMatcher): `actual`, `negated`,
// `message` and `promise` are the assertion's object / negate / message / promise flags.
function makeExpect(actual, negated, message, promise) {
  negated = negated === true;
  promise = promise || '';
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
      if (__isDomContain(actual)) return __toContainDom(c, actual, x);
      if (typeof actual === 'string' && typeof x === 'string') return c(actual.includes(x), 'expected #{this} to contain #{exp}', 'expected #{this} not to contain #{exp}', x);
      // chai's include on the value made indexable (vitest: Array.from)
      const list = actual != null && typeof actual !== 'string' ? Array.from(actual) : actual;
      __chaiAssert(negated, list != null && list.includes(x), 'expected #{this} to include #{exp}', 'expected #{this} to not include #{exp}', list, x, list);
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
      const [props, h] = __snapPropArgs(propertiesOrHint, hint);
      const key = __snapNextKey(h ? `${__snap.testName} > ${h}` : __snap.testName);
      const received = __snapApplyProps(actual, props);
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
        const parts = __ttNormSegs(String(file).startsWith('/') ? String(file) : `${testDir}/${file}`);
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
      const { props, inline, hint } = __inlineSnapArgs(a, b, c);
      const key = __snapNextKey(hint ? `${__snap.testName} > ${hint}` : __snap.testName);
      const received = __snapApplyProps(actual, typeof props === 'object' ? props : undefined);
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
      // A literal key (incl. one containing dots, e.g. a CSS selector) takes precedence over
      // dot-path traversal — matches jest/vitest behavior.
      const { cur, present } = __lookupProperty(actual, path);
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
      const [threw, err] = __thrownBy(actual);
      const fnShown = typeof actual === 'function' ? actual : () => {};
      // chai's `throws` (no argument, a string, a RegExp): vitest's messages
      if (expected === undefined || typeof expected === 'string' || expected instanceof RegExp) return __toThrowChai(negated, expected, threw, err, fnShown);
      if (!threw && !negated) throw new __ChaiAssertionError("expected function to throw an error, but it didn't", { showDiff: false });
      return __toThrowValue(c, testers, expected, threw, err);
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
// matcherHint's options, with their defaults
function __hintOptions(options) {
  const d = { comment: '', isDirectExpectCall: false, isNot: false, promise: '', secondArgument: '', expectedColor: __dcol.green, receivedColor: __dcol.red, secondArgumentColor: __dcol.green };
  const o = {};
  for (const k of Object.keys(d)) { const v = options[k]; o[k] = v === undefined ? d[k] : v; }
  return o;
}
// matcherHint up to the matcher name: [hint, the dim text still to come]
function __hintHead(matcherName, received, o, dim) {
  let hint = '';
  let dimString = 'expect';
  if (!o.isDirectExpectCall && received !== '') { hint += dim(`${dimString}(`) + o.receivedColor(received); dimString = ')'; }
  if (o.promise !== '') { hint += dim(`${dimString}.`) + o.promise; dimString = ''; }
  if (o.isNot) { hint += `${dim(`${dimString}.`)}not`; dimString = ''; }
  if (matcherName.includes('.')) dimString += matcherName;
  else { hint += dim(`${dimString}.`) + matcherName; dimString = ''; }
  return [hint, dimString];
}
function __hintExpectedPart(o, dim, expected, dimString) {
  if (expected === '') return { hint: '', dimString: dimString + '()' };
  let hint = dim(`${dimString}(`) + o.expectedColor(expected);
  if (o.secondArgument) hint += dim(', ') + o.secondArgumentColor(o.secondArgument);
  return { hint, dimString: ')' };
}
function __matcherHint(matcherName, received = 'received', expected = 'expected', options = {}) {
  const o = __hintOptions(options);
  const dim = __dcol.dim;
  let [hint, dimString] = __hintHead(matcherName, received, o, dim);
  const part = __hintExpectedPart(o, dim, expected, dimString);
  hint += part.hint;
  dimString = part.dimString;
  if (o.comment !== '') dimString += ` // ${o.comment}`;
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
const __chai = globalThis.__ttLoadChai && globalThis.__ttLoadChai(__hostEventTarget(), __hostEvent());
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
  const __pollRejectKey = (key) => {
    if (key === 'resolves' || key === 'rejects') throw new SyntaxError(`expect.poll() is not supported in combination with .${key}`);
    if (typeof key === 'string' && __pollUnsupported.includes(key)) throw new SyntaxError(`expect.poll() is not supported in combination with .${key}(). Use vi.waitFor() if your assertion condition is unstable.`);
  };
  const proxy = new Proxy(assertion, { get(target, key, receiver) {
    __pollRejectKey(key);
    const matcher = Reflect.get(target, key, receiver);
    if (typeof matcher !== 'function') return matcher instanceof __chai.Assertion ? proxy : matcher;
    if (key === 'assert') return matcher;
    return function __VITEST_POLL_CHAIN__(...args) {
      const STACK_TRACE_ERROR = new Error('STACK_TRACE_ERROR');
      const withStack = (err) => {
        if (err && typeof err === 'object' && STACK_TRACE_ERROR.stack !== undefined) err.stack = STACK_TRACE_ERROR.stack.replace(STACK_TRACE_ERROR.message, err.message);
        return err;
      };
      const onPollFail = (err, last) => {
        if (!last) return false;
        if (err && typeof err === 'object' && err.cause == null) err.cause = new Error('Matcher did not succeed in time.');
        throw withStack(err);
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
              onPollFail(err, last);
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
const __mockConfigKey = Symbol('turbo-test mock config');
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
// makeSpy: the original's (or implementation's) own and inherited statics onto the mock
function __copyMockStaticKey(fn, src, k) {
  if (__mockStatic.has(k) || __mockDescriptor(fn, k)) return;
  try { Object.defineProperty(fn, k, Object.getOwnPropertyDescriptor(src, k)); } catch (e) {}
}
function __copyMockStatics(fn, from) {
  for (let src = from; src && src !== Object.prototype && src !== Function.prototype; src = Object.getPrototypeOf(src)) {
    for (const k of [...Object.getOwnPropertyNames(src), ...Object.getOwnPropertySymbols(src)]) __copyMockStaticKey(fn, src, k);
  }
}
const __mockLength = (impl, original) => ((impl || original) ? (impl || original).length : 0);
// makeSpy's error path: record the throw/reject on the pending result and settled slot, and
// warn when the implementation was a plain object so `new spy()` surfaces that as the cause.
function __handleMockError(e, result, settled, fn) {
  result.type = 'throw';
  result.value = e;
  settled.type = 'rejected';
  settled.value = e;
  if (e instanceof TypeError && String(e.message).includes('is not a constructor')) {
    globalThis.console.warn(`[vitest] The ${fn.getMockName()} mock did not use 'function' or 'class' in its implementation, see https://vitest.dev/api/vi#vi-spyon for examples.`);
  }
}
function __mockMakeState() {
  const state = {
    calls: [], contexts: [], instances: [], invocationCallOrder: [], settledResults: [], results: [],
    get lastCall() { return state.calls[state.calls.length - 1]; },
  };
  return state;
}
function __mockSettleResult(settled, ret) {
  if (ret instanceof Promise) {
    ret.then((v) => { settled.type = 'fulfilled'; settled.value = v; }, (e) => { settled.type = 'rejected'; settled.value = e; });
    return;
  }
  settled.type = 'fulfilled';
  settled.value = ret;
}
function __mockRecordCall(state, args, ctx, newTarget) {
  state.calls.push(args);
  state.invocationCallOrder.push(++__mockCallOrder);
  const result = { type: 'incomplete', value: undefined };
  const settled = { type: 'incomplete', value: undefined };
  state.results.push(result);
  state.settledResults.push(settled);
  const context = newTarget ? undefined : ctx;
  const at = state.instances.push(context);
  state.contexts.push(context);
  return { result, settled, at };
}
function __mockApplyImpl(config, original, args, newTarget, ctx) {
  const implementation = config.once.shift() || config.impl || original || function () {};
  if (newTarget) return Reflect.construct(implementation, args, newTarget);
  return implementation.apply(ctx, args);
}
function __mockInvoke(fn, state, config, original, args, newTarget, ctx) {
  const { result, settled, at } = __mockRecordCall(state, args, ctx, newTarget);
  let ret;
  try { ret = __mockApplyImpl(config, original, args, newTarget, ctx); }
  catch (e) { __handleMockError(e, result, settled, fn); throw e; }
  result.type = 'return';
  result.value = ret;
  if (newTarget) { state.contexts[at - 1] = ret; state.instances[at - 1] = ret; }
  __mockSettleResult(settled, ret);
  return ret;
}
function __mockMakeConfig(original) {
  const config = { impl: undefined, once: [], name: 'vi.fn()' };
  // spyOn's original: jest.fn()'s whenCalledWith (__jestWhen) falls through to it
  Object.defineProperty(config, 'original', { value: original });
  return config;
}
function __mockSpyName(original) { return (original && original.name) || 'Mock'; }
function __mockInitOpts(opts, config, original, fn) {
  if (opts.restore) __mockRestores.add(opts.restore);
  if (opts.keepName) config.name = fn.name || 'vi.fn()';
}
function makeSpy(impl, opts) {
  opts = opts || {};
  const original = opts.original;
  const config = __mockMakeConfig(original);
  const state = __mockMakeState();
  const name = __mockSpyName(original);
  const fn = { [name]: function (...args) { return __mockInvoke(fn, state, config, original, args, new.target, this); } }[name];
  // the original's (or implementation's) own and inherited statics, as vitest copies them
  __copyMockStatics(fn, original || impl);
  Object.defineProperty(fn, 'length', { writable: true, enumerable: false, configurable: true, value: __mockLength(impl, original) });
  __mockInitOpts(opts, config, original, fn);
  fn._isMockFunction = true;
  // the closure's state, for jest.fn()'s whenCalledWith (__jestWhen); a symbol, so no member
  Object.defineProperty(fn, __mockConfigKey, { value: config });
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
function __automockProtoKey(target, p, k, spy) {
  if (k === 'constructor' || Object.prototype.hasOwnProperty.call(target, k)) return;
  const d = Object.getOwnPropertyDescriptor(p, k);
  if (typeof d.value === 'function') target[k] = __trackSpy(makeSpy(undefined, { original: spy ? d.value : undefined }));
}
function __automockProtoMethods(target, proto, spy) {
  for (let p = proto; p && !__isBuiltinProto(p); p = Object.getPrototypeOf(p)) {
    for (const k of Object.getOwnPropertyNames(p)) __automockProtoKey(target, p, k, spy);
  }
}
// A mock's names an ES import may name although the mock lacks them (the ESM graph's synthetic
// module exports them, undefined): an automock's of a package's CommonJS module.
const __ttMockNames = new WeakMap();
globalThis.__ttMockNames = (m) => (m && (typeof m === 'object' || typeof m === 'function') ? __ttMockNames.get(m) : undefined);
// `spy` (vi.mockObject's `{ spy: true }`, vitest's autospy): functions call their originals and
// arrays are kept.
// Own static keys that an automocked function must not copy (function built-ins and shape).
const __AUTOMOCK_FN_SKIP = new Set(['length', 'name', 'prototype', 'caller', 'arguments']);
// __automockFn's own-static pass: copy each non-skipped key through the recursive automocker.
function __automockFnStatics(m, mod, depth, spy) {
  for (const k of Object.getOwnPropertyNames(mod)) {
    if (__AUTOMOCK_FN_SKIP.has(k)) continue;
    try { m[k] = __automock(mod[k], depth + 1, spy); } catch (e) {}
  }
}
function __automockFn(mod, depth, spy) {
  const m = __trackSpy(makeSpy(undefined, { original: spy ? mod : undefined }));
  // vitest's automocker keeps the function's own name (`Comp.name`, not the spy's) (turbo-test#136)
  try { Object.defineProperty(m, 'name', { value: mod.name, configurable: true }); } catch (e) {}
  if (depth < 3) {
    __automockFnStatics(m, mod, depth, spy);
    // class / constructor function: `new m()` returns an instance of m.prototype, whose
    // methods (inherited ones included) are mocks
    if (mod.prototype && mod.prototype.constructor === mod) __automockProtoMethods(m.prototype, mod.prototype, spy);
  }
  return m;
}
function __automockObj(mod, depth, spy) {
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
function __automock(mod, depth, spy) {
  depth = depth || 0;
  if (typeof mod === 'function') return __automockFn(mod, depth, spy);
  if (Array.isArray(mod)) return spy ? mod.slice() : [];
  if (mod && typeof mod === 'object') return __automockObj(mod, depth, spy);
  return mod;
}
// vitest's autospy (vi.mockObject(value, { spy: true })): in place, each function (nested
// objects' too) becomes a spy on it; arrays and other values stay.
// Only these children are walked recursively by __autospy (functions and plain objects).
function __autospyWalkable(x) { return typeof x === 'function' || (x && typeof x === 'object'); }
// Values __autospy leaves alone: null, non-objects, arrays, cycles, and depths past 4.
function __autospyOpaque(v, seen, depth) {
  return v === null || typeof v !== 'object' || Array.isArray(v) || seen.has(v) || depth > 4;
}
function __autospy(v, depth, seen) {
  if (typeof v === 'function') return __automock(v, depth, true);
  if (__autospyOpaque(v, seen, depth)) return v;
  seen.add(v);
  for (const k of Object.keys(v)) {
    let x;
    try { x = v[k]; } catch (e) { continue; }
    if (__autospyWalkable(x)) {
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
  __ttStrictMocks.set(proxy, { key, vitest: vitest !== false, target: mod });
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
// __ttToESM: values returned unchanged (primitives, strict mocks, namespaces).
const __ttToESMAsIs = (m) => !m || (typeof m !== 'object' && typeof m !== 'function') || __ttStrictMocks.has(m) || __ttIsNs(m);
// __ttToESM's bundler interop of a CommonJS-marked module (__esModule / '__esModule' in m).
const __ttToESMBundler = (m) => ('default' in m || m.default !== undefined) ? m : __ttCjsView(m);
// Last-resort: try to add `default` to m in place; fall back to a view when the module is frozen.
function __ttToESMAddDefault(m) {
  try {
    Object.defineProperty(m, 'default', { value: m, configurable: true });
    return m;
  } catch (e) {
    return __ttCjsView(m);
  }
}
globalThis.__ttToESM = (m) => {
  if (__ttToESMAsIs(m)) return m;
  if (m[__ttEsmMark]) return __ttEsmNs(m);
  if (m.__esModule || '__esModule' in m) return __ttToESMBundler(m);
  if (__ttExternalCjs.has(m)) return __ttNodeNs(m);
  if ('default' in m) return m;
  return __ttToESMAddDefault(m);
};
// `__toESM` in a package's bundle: as Node imports in an ES module that Node runs as one
// (`node`: a `.mjs` file or a `"type": "module"` package's) — CommonJS is Node's namespace of it
// (its default the whole `module.exports`); else a bundler's interop, the exports object itself
// with a `default` (a package's CommonJS with its own `default` and no `__esModule` read through
// a view, its own `default` staying what CommonJS code reads).
const __ttNotObject = (m) => m === null || (typeof m !== 'object' && typeof m !== 'function');
// __ttToESMDep's values taken as they are: primitives, strict mocks and namespaces
const __ttDepAsIs = (m) => !m || __ttNotObject(m) || __ttStrictMocks.has(m) || __ttIsNs(m) || m[__ttEsmMark];
globalThis.__ttToESMDep = (m, node) => {
  // Node's namespace of a primitive module.exports: the value as default and 'module.exports'
  // (turbo-test#193)
  if (node && __ttNotObject(m)) return { default: m, 'module.exports': m };
  if (__ttDepAsIs(m)) return m;
  if (node) return __ttNodeNs(m);
  return __ttBundlerNs(m);
};
// __ttToESMDep's bundler interop of a package's CommonJS module
function __ttBundlerEsMod(m) { return ('default' in m || m.default !== undefined) ? m : __ttCjsView(m); }
function __ttBundlerDefault(m) { return m.default === m || !__ttExternalCjs.has(m) ? m : __ttCjsView(m); }
function __ttBundlerNs(m) {
  if (m.__esModule || '__esModule' in m) return __ttBundlerEsMod(m);
  if ('default' in m) return __ttBundlerDefault(m);
  try { Object.defineProperty(m, 'default', { value: m, configurable: true }); return m; }
  catch (e) { return __ttCjsView(m); }
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
const __ttQueueMock = (spec, exports, vitest, key) => {
  (globalThis.__pendingMocks || (globalThis.__pendingMocks = [])).push({ spec: String(spec), exports, vitest, key });
};
// __ttMock outside the prepass: the module's lenient-ness, and whether the prepass already
// registered it (true: nothing more to do); this mock supersedes a pending async doMock
function __ttMockPrepassed(spec, vitest, key, dir) {
  if (vitest) __ttLenientMockKeys.delete(key); else __ttLenientMockKeys.add(key);
  if (__ttPrepassMocks.size && __ttPrepassMocks.delete(__ttPrepassKey(spec))) return true;
  // this mock supersedes a still-pending async doMock of the module
  const pending = __ttDoMockKey(dir, spec);
  if (pending) __ttDoMockWaits.delete(pending);
  return false;
}
// AUTOMOCK: no factory -> auto-generate a mock from the real module (functions -> vi.fn()).
// A real module that fails to load is left unmocked: importing it reports its own error.
// vitest automocks the module's namespace: a package's CommonJS one is Node's (`default`,
// `module.exports` and the names cjs-module-lexer finds; turbo-test#136, #133).
// [false] when the module is left unmocked
function __ttAutomockOf(dir, spec) {
  try {
    // Jest: when a colocated __mocks__/<basename> exists, load it as the mock (turbo-test#341).
    const mp = __ttManualMockPath(dir, spec);
    if (mp) {
      const manual = globalThis.__ttImportActual(globalThis.__dirnameOf(mp), './' + __basename(mp));
      return [true, manual];
    }
    const actual = globalThis.__ttImportActual(dir, String(spec));
    const exports = __automock(globalThis.__ttToESM(actual));
    // its own names stay importable on the ESM graph (reading undefined, as vitest's)
    if (__ttExternalCjs.has(actual)) __ttMockNames.set(exports, Object.keys(actual));
    return [true, exports];
  }
  catch (e) { return [false]; }
}
// the factory's mock: [false] when nothing is registered
function __ttFactoryExports(spec, factory, vitest, isDoMock, dir) {
  // vitest passes `importOriginal` (a fn returning the real module, as a Promise) as the
  // factory's first arg — `vi.mock('x', async (importOriginal) => ({...await importOriginal(), y}))`.
  const importOriginal = () => Promise.resolve(globalThis.__ttToESM(globalThis.__ttImportActual(dir, String(spec))));
  try { return [true, typeof factory === 'function' ? factory(importOriginal) : factory]; }
  catch (e) {
    // The entry's mock prepass (__ttMockPrepass) runs factories outside the module's scope; a
    // throw there is not registered — the module's own vi.mock call re-runs it in scope.
    if (globalThis.__ttMockPrepass) return [false];
    // a hoisted call's factory reading a binding not initialised yet runs on first import
    // (turbo-test#205); any other throw registers the error stand-in now
    return [true, !isDoMock && e instanceof ReferenceError && typeof factory === 'function'
      ? __ttLazyFactoryMock(() => factory(importOriginal), vitest)
      : __ttFactoryErrorMock(e, vitest)];
  }
}
function __ttMockMake(spec, factory, vitest, isDoMock, dir) {
  return factory === undefined ? __ttAutomockOf(dir, spec) : __ttFactoryExports(spec, factory, vitest, isDoMock, dir);
}
// The calling module's dir: the module-runner's __ttDir, else (ESM graph, where modules have
// none) the entry file's dir, so a runtime vi.doMock registers immediately and a later import()
// of the module sees it.
function __ttMockDir() { return globalThis.__ttDir || globalThis.__ttEntryDir; }
function __ttMockKey(spec) { return (globalThis.__ttDir || '') + '\0' + String(spec); }
function __ttMock(spec, factory, vitest, isDoMock, options) {
  const key = __ttMockKey(spec);
  const dir = __ttMockDir();
  // jest.mock(spec, factory, { virtual: true }): skip resolution; keyed by the raw spec + dir
  // (turbo-test#308). Jest-only; vi.mock has no such option. Virtual registrations take their
  // own path and don't need the automock mark below.
  if (__ttIsVirtualMock(vitest, options)) return __ttVirtualMockEntry(spec, factory, isDoMock, dir);
  // jest automock (turbo-test#184): record this spec as explicitly handled so the automock
  // bypass does NOT re-mock it on require. Resolution may fail (non-existent spec from a
  // prepass): we still fall through to normal mock logic.
  __ttAutomockMark(dir, spec);
  if (!globalThis.__ttMockPrepass && __ttMockPrepassed(spec, vitest, key, dir)) return;
  // jest 30: `jest.mock(spec, factory)` defers the factory to the module's first require
  // (turbo-test#275). `vi.mock` keeps its eager hoist-time semantics.
  if (__ttJestLazyRegister(spec, factory, vitest, isDoMock, dir, key)) return;
  const [ok, made] = __ttMockMake(spec, factory, vitest, isDoMock, dir);
  if (ok) __ttMockRegister(spec, factory, vitest, isDoMock, dir, key, made);
}
// jest.mock(spec, factory, { virtual: true }): a mock keyed by the raw spec + caller's dir, so
// `require('non-existent')` returns the factory's exports without ever resolving the spec
// (turbo-test#308). Lazy for jest.mock (same as #275); eager for jest.doMock / jest.setMock.
function __ttVirtualRegister(dir, spec, exports) {
  if (globalThis.__ttRegisterVirtualMock && dir) {
    try { globalThis.__ttRegisterVirtualMock(String(dir), String(spec), exports); } catch (e) {}
  }
}
function __ttVirtualLazyEligible(factory, isDoMock) {
  if (isDoMock || typeof factory !== 'function') return false;
  return !(globalThis.__ttMockPrepass || globalThis.__ttNoLazyJestMock);
}
function __ttVirtualKey(dir, spec) {
  return (dir || '') + '\0virtual:' + String(spec);
}
function __ttIsVirtualMock(vitest, options) {
  return !vitest && !!options && options.virtual === true;
}
// Prepass can't resolve the raw spec to a path, so it's a no-op — the module's own call does
// the work.
function __ttVirtualMockEntry(spec, factory, isDoMock, dir) {
  if (globalThis.__ttMockPrepass) return;
  __ttVirtualMock(spec, factory, isDoMock, dir, __ttVirtualKey(dir, spec));
}
function __ttVirtualMock(spec, factory, isDoMock, dir, key) {
  __ttLenientMockKeys.add(key);
  if (__ttVirtualLazyEligible(factory, isDoMock)) {
    __ttJestLazy.set(key, { spec: String(spec), factory, dir, key, virtual: true });
    return;
  }
  const [ok, made] = __ttFactoryExports(spec, factory, false, isDoMock, dir);
  if (!ok) return;
  __ttVirtualRegister(dir, spec, __ttStrictMock(made, key, String(spec), false));
}
// jest 30's lazy `jest.mock(spec, factory)`: the factory runs on the module's first require,
// closing over bindings the test file has by then initialised (turbo-test#275). The prepass
// runs factories outside the module's scope, so the prepass form is a no-op — the module's own
// `jest.mock` call does the registration below. `vi.mock`, `jest.doMock` (runtime), and
// `jest.mock(spec)` without a factory (automock) keep the eager path.
const __ttJestLazy = new Map();
globalThis.__ttJestLazy = __ttJestLazy;
function __ttJestLazyKey(dir, spec) {
  return (dir && globalThis.__ttResolveSpec(dir, String(spec))) || '\0' + String(spec);
}
function __ttJestLazyEligible(factory, vitest, isDoMock) {
  if (vitest || isDoMock || typeof factory !== 'function') return false;
  // Both prepasses (the esbuild-MR one via __ttMockPrepass, and the untransformed ESM-graph
  // one via __ttNoLazyJestMock) run factories eagerly so the mock is in place before module
  // imports resolve: jest 30 lazy semantics only kick in from the module's own body call,
  // where the module's bindings exist.
  return !(globalThis.__ttMockPrepass || globalThis.__ttNoLazyJestMock);
}
function __ttJestLazyRegister(spec, factory, vitest, isDoMock, dir, key) {
  if (!__ttJestLazyEligible(factory, vitest, isDoMock)) return false;
  __ttLenientMockKeys.add(key);
  __ttJestLazy.set(__ttJestLazyKey(dir, spec), { spec: String(spec), factory, dir, key });
  if (globalThis.__ttRegisterMock && dir) {
    try { globalThis.__ttRegisterMock(dir, String(spec), {}); } catch (e) {}
  }
  return true;
}
function __ttJestLazyFind(dir, spec) {
  const regular = __ttJestLazyKey(dir, spec);
  const entry = __ttJestLazy.get(regular);
  if (entry) return [regular, entry];
  const virtual = __ttVirtualKey(dir, spec);
  return [virtual, __ttJestLazy.get(virtual)];
}
function __ttJestLazyFinalize(entry) {
  const [ok, made] = __ttFactoryExports(entry.spec, entry.factory, false, false, entry.dir);
  if (!ok) return;
  if (entry.virtual) {
    __ttVirtualRegister(entry.dir, entry.spec, __ttStrictMock(made, entry.key, entry.spec, false));
  } else {
    __ttMockRegister(entry.spec, entry.factory, false, false, entry.dir, entry.key, made);
  }
}
function __ttJestLazyRun(dir, spec) {
  if (!__ttJestLazy.size) return;
  const [k, entry] = __ttJestLazyFind(dir, spec);
  if (!entry) return;
  __ttJestLazy.delete(k);
  __ttJestLazyFinalize(entry);
}
globalThis.__ttJestLazyRun = __ttJestLazyRun;
// Async (Promise) factory branch of __ttMockRegister: a doMock (not hoisted: it applies to
// later imports) waits on its own; a hoisted vi.mock is awaited by the loader's resolve + drain.
function __ttRegisterAsyncMock(spec, vitest, isDoMock, dir, key, exports) {
  if (isDoMock && !globalThis.__ttMockPrepass && globalThis.__ttRegisterMock && __ttAwaitDoMock(dir, spec, exports, key, vitest)) return;
  __ttQueueMock(spec, exports, vitest, key);
}
// __ttMock's registration of the made mock
function __ttTryRegisterNow(dir, spec, exports, key) {
  return globalThis.__ttRegisterMock && dir && __ttRegisterMockNow(dir, spec, exports, key);
}
function __ttMockRegister(spec, factory, vitest, isDoMock, dir, key, exports) {
  // Sync factory: register IMMEDIATELY in the calling module's dir (vi.mock is hoisted above
  // the module's consumer requires, so the mock is in place before they load — and the factory
  // ran in THIS module's scope, so it shares the test's bindings). Async (Promise) factory:
  // queue for __resolvePendingMocks (run_entry_mocks prepass / drain).
  if (globalThis.__ttMockPrepass) __ttPrepassMocks.add(__ttPrepassKey(spec));
  if (exports && typeof exports.then === 'function') return __ttRegisterAsyncMock(spec, vitest, isDoMock, dir, key, exports);
  const final = factory !== undefined ? __ttStrictMock(exports, key, String(spec), vitest) : exports;
  if (__ttTryRegisterNow(dir, spec, final, key)) return;
  __ttQueueMock(spec, final, vitest, key);
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
function __spyCheckTarget(obj) {
  if (obj == null) throw new Error('The vi.spyOn() function could not find an object to spy upon. The first argument must be defined.');
  if (typeof obj !== 'object' && typeof obj !== 'function') throw new Error('Vitest cannot spy on a primitive value.');
}
// a registered ESM-graph namespace's own export (not an accessor spy)
function __spyIsNs(nsRaw, key, accessor) {
  try { return !accessor && typeof key === 'string' && __ttNsInfo.has(__ttNsTarget(nsRaw)) && key in nsRaw; } catch (e) { return false; }
}
function __spyCheckHas(obj, key, desc) {
  let has = !!desc;
  if (!has) { try { has = key in obj; } catch (e) { has = true; } }
  if (!has) throw new Error(`The property "${String(key)}" is not defined on the ${typeof obj}.`);
}
// [the access type spied on, whether a getter holding a function is spied through]
function __spyAccess(accessor, desc) {
  const accessType = accessor || 'value';
  if (accessType === 'value' && desc && desc.value == null && desc.get) return ['get', true];
  return [accessType, false];
}
// the spied-on value: the namespace's export, the descriptor's value / accessor, or the property
function __spyOriginal(isNs, nsRaw, obj, key, desc, accessType) {
  if (isNs) return globalThis.__ttR(nsRaw, key);
  if (desc) {
    const original = desc[accessType];
    return original == null && accessType === 'value' ? obj[key] : original;
  }
  if (accessType !== 'value') return () => obj[key];
  return obj[key];
}
// A spyable target: a function, or an accessor whose current value is nullish (so spying
// installs one). Anything else is a TypeError for the caller.
function __viSpyOnCheck(originalImpl, accessType, original) {
  if (typeof originalImpl === 'function') return;
  if (accessType !== 'value' && original == null) return;
  throw new Error(`vi.spyOn() can only spy on a function. Received ${typeof originalImpl}.`);
}
// @vitest/spy's spyOn: `accessor` 'get' / 'set' spies on an accessor; a getter holding a
// function (a live re-export) is spied through it; the original descriptor comes back on
// restore (a method found on a prototype is deleted from the object again)
function __viUnmockClear(spec) {
  if (globalThis.__pendingMocks) globalThis.__pendingMocks = globalThis.__pendingMocks.filter((m) => m.spec !== String(spec));
  const pending = __ttDoMockKey(globalThis.__ttDir || globalThis.__ttEntryDir, spec);
  if (pending) __ttDoMockWaits.delete(pending);
}
function __viUnmock(spec) {
  __viUnmockClear(spec);
  const dir = globalThis.__ttDir || globalThis.__cwd;
  if (globalThis.__ttUnmock && dir) { try { globalThis.__ttUnmock(dir, String(spec)); } catch (e) {} }
  // jest automock (turbo-test#184): a `jest.unmock(spec)` opts the module out of automock.
  if (globalThis.__ttAutomockMark) globalThis.__ttAutomockMark(dir, spec);
}
function __viSpyLookup(obj, key, accessor) {
  __spyCheckTarget(obj);
  const nsRaw = typeof obj === 'object' ? __ttNsRaw.get(obj) || obj : obj;
  const isNs = __spyIsNs(nsRaw, key, accessor);
  const found = isNs ? undefined : __mockDescriptor(obj, key);
  const [descObj, desc] = found || [];
  __spyCheckHas(obj, key, desc);
  const [accessType, ssr] = __spyAccess(accessor, desc);
  const original = __spyOriginal(isNs, nsRaw, obj, key, desc, accessType);
  const originalImpl = ssr && original ? original.call(obj) : original;
  return { nsRaw, isNs, descObj, desc, accessType, ssr, original, originalImpl };
}
function __viSpyReassign(obj, key, isNs, desc, accessType, cb) {
  if (isNs) { globalThis.__ttNsSet(obj, key, cb); return; }
  const d = Object.assign({}, desc || { configurable: true, writable: true });
  delete d.value;
  if (accessType !== 'value') delete d.writable;
  d[accessType] = cb;
  Object.defineProperty(obj, key, d);
}
// @vitest/spy's spyOn: `accessor` 'get' / 'set' spies on an accessor; a getter holding a
// function (a live re-export) is spied through it; the original descriptor comes back on
// restore (a method found on a prototype is deleted from the object again)
function __viSpyOn(obj, key, accessor) {
  const { isNs, descObj, desc, accessType, ssr, original, originalImpl } = __viSpyLookup(obj, key, accessor);
  __viSpyOnCheck(originalImpl, accessType, original);
  if (globalThis.vi.isMockFunction(originalImpl)) return originalImpl;
  const reassign = (cb) => __viSpyReassign(obj, key, isNs, desc, accessType, cb);
  const restore = () => {
    if (!isNs && descObj !== obj) Reflect.deleteProperty(obj, key);
    else if (!isNs && desc && !original) Object.defineProperty(obj, key, desc);
    else reassign(original);
  };
  const s = makeSpy(undefined, { original: originalImpl, restore, keepName: true });
  reassign(ssr ? () => s : s);
  return __trackSpy(s);
}
globalThis.vi = {
  // a mock given a mock is that mock (vitest)
  fn: (impl) => (impl != null && globalThis.vi.isMockFunction(impl) ? impl : __trackSpy(makeSpy(impl, { resetTo: impl }))),
  // @vitest/spy's spyOn: `accessor` 'get' / 'set' spies on an accessor; a getter holding a
  // function (a live re-export) is spied through it; the original descriptor comes back on
  // restore (a method found on a prototype is deleted from the object again)
  spyOn: (obj, key, accessor) => __viSpyOn(obj, key, accessor),
  // Record (specifier, evaluated-exports) into a queue the native loader drains after the
  // module runs, registering the mock keyed by the resolved absolute path. The factory runs
  // here, in the calling module's scope (so closures over setup-file vars work).
  mock: (spec, factory) => __ttMock(spec, factory, true),
  unmock: (spec) => __viUnmock(spec),
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
  importActual: (p) => { try { return Promise.resolve(globalThis.__ttToESM(globalThis.__ttImportActual(globalThis.__ttDir || globalThis.__ttEntryDir, String(p)))); } catch (e) { return Promise.resolve(undefined); } },
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
// Non-Reflect fallback for babelHelpers.decorate: apply each decorator in reverse, picking the
// member / class / parameter shape based on `c` (arguments.length).
function __babelDecorateFallback(decorators, target, key, r, c) {
  for (var i = decorators.length - 1; i >= 0; i--) {
    var d = decorators[i];
    if (d) r = (c < 3 ? d(r) : c > 3 ? d(target, key, r) : d(target, key)) || r;
  }
  return r;
}
globalThis.babelHelpers.decorate = function (decorators, target, key, desc) {
  var c = arguments.length,
    r = c < 3 ? target : desc === null ? (desc = Object.getOwnPropertyDescriptor(target, key)) : desc;
  if (typeof Reflect === 'object' && typeof Reflect.decorate === 'function') {
    r = Reflect.decorate(decorators, target, key, desc);
  } else {
    r = __babelDecorateFallback(decorators, target, key, r, c);
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
// a jest config's fakeTimers (turbo-test#186): the defaults of every jest.useFakeTimers (its
// argument over them), and with `enableGlobally: true` fake timers before the test file loads
let __jestTimerDefaults = {};
function __jestApplyConfig(ft) {
  __jestTimerDefaults = ft && typeof ft === 'object' ? ft : {};
  if (__jestTimerDefaults.enableGlobally === true) __jestInstallFake();
}
function __jestFakeSkip(legacy, c) {
  if (legacy) return new Set(['Date', 'performance', 'hrtime']);
  return new Set(Array.isArray(c.doNotFake) ? c.doNotFake : []);
}
function __jestInstallFake(cfg) {
  const c = Object.assign({}, __jestTimerDefaults, cfg);
  const legacy = c.legacyFakeTimers === true;
  const skip = __jestFakeSkip(legacy, c);
  const delta = typeof c.advanceTimers === 'number' ? c.advanceTimers : undefined;
  __installFake({
    now: legacy ? undefined : c.now,
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

// jest-mock's whenCalledWith (turbo-test#185) on a vi mock `fn`: the first call installs a
// dispatcher as the implementation (the one it replaces becomes the fall-through, and so does a
// later mockImplementation); each call whose arguments equal a branch's matchers
// ([iterableEquality], asymmetric matchers too) runs the earliest such branch with a queued once,
// else the last-registered one with an implementation, else the fall-through (a spy's original).
// mockReset drops every branch, resets each sub-mock and (as jest-mock does) the implementation.
function __jestWhenPick(branches, args) {
  const matching = branches.filter((b) => __equals(b.matchers, args, [__iterableEquality]));
  const once = matching.find((b) => b.sub[__mockConfigKey].once.length > 0);
  if (once) return once.sub;
  for (let i = matching.length - 1; i >= 0; i--) if (matching[i].sub[__mockConfigKey].impl !== undefined) return matching[i].sub;
  return undefined;
}
function __jestWhen(fn) {
  if (fn.whenCalledWith) return fn; // a spyOn of an already-spied method returns the same mock
  const config = fn[__mockConfigKey];
  const branches = [];
  let fallback;
  let reset = false; // jest-mock's reset drops the implementation: a reset spy no longer calls through
  const dispatcher = function (...args) {
    const target = __jestWhenPick(branches, args) || fallback || (reset ? undefined : config.original);
    return target ? target.apply(this, args) : undefined;
  };
  const { mockImplementation, getMockImplementation, mockReset } = fn;
  fn.mockImplementation = (f) => { if (config.impl === dispatcher) fallback = f; else mockImplementation(f); return fn; };
  fn.getMockImplementation = () => (config.impl === dispatcher ? fallback : getMockImplementation());
  fn.mockReset = () => {
    for (const b of branches.splice(0)) b.sub.mockReset();
    fallback = undefined;
    mockReset();
    // jest 30 (turbo-test#254), unlike vitest's back-to-vi.fn(impl): jest.fn(impl) and a spy
    // return undefined until a new implementation is set
    reset = true;
    config.impl = dispatcher;
    return fn;
  };
  fn.whenCalledWith = (...matchers) => {
    if (config.impl !== dispatcher) { fallback = config.impl; config.impl = dispatcher; }
    const sub = __jestWhen(globalThis.vi.fn());
    branches.push({ matchers, sub });
    return sub;
  };
  return fn;
}

// Jest's `jest` global maps onto the same machinery as `vi` (drop-in for jest projects, e.g.
// NestJS backends using ts-jest). Most methods are identical; the jest-only ones (sync
// requireActual/requireMock, isolateModules, setTimeout) are bridged here. Type-only members
// (jest.Mock, jest.Mocked, jest.SpyInstance, ...) are erased by the transform — no runtime needed.
// Defined unconditionally: it doesn't touch vitest behavior (vitest suites never reference `jest`).
globalThis.jest = {
  fn: (impl) => __jestWhen(globalThis.vi.fn(impl)),
  spyOn: (obj, key, accessType) => __jestWhen(globalThis.vi.spyOn(obj, key, accessType)),
  mock: (spec, factory, options) => __ttMock(spec, factory, false, false, options),
  unmock: (spec) => globalThis.vi.unmock(spec),
  doMock: (spec, factory, options) => __ttMock(spec, factory, false, true, options),
  dontMock: (spec) => globalThis.vi.unmock(spec),
  // jest's other module-registry members, on the same machinery (turbo-test#179): setMock is a
  // doMock of the given exports, unstable_mockModule / unstable_unmockModule the ESM forms,
  // deepUnmock an unmock, createMockFromModule the automock of the real module
  deepUnmock: (spec) => { globalThis.vi.unmock(spec); return globalThis.jest; },
  setMock: (spec, exports) => { __ttMock(spec, () => exports, false, true); return globalThis.jest; },
  unstable_mockModule: (spec, factory) => { __ttMock(spec, factory, false, true); return globalThis.jest; },
  unstable_unmockModule: (spec) => { globalThis.vi.unmock(spec); return globalThis.jest; },
  createMockFromModule: (spec) => __automock(globalThis.jest.requireActual(spec)),
  // jest automock (turbo-test#184): when enabled, `require('./x')` of a local module that
  // has not been explicitly mocked / unmocked returns a generated mock of its real exports.
  // Jest 30 adds the shorter aliases `autoMockOn` / `autoMockOff`. `onGenerateMock(factory)`
  // replaces the default generator (`vi.mockObject(exports)`).
  enableAutomock: () => { __ttAutomockOn = true; return globalThis.jest; },
  disableAutomock: () => { __ttAutomockOn = false; return globalThis.jest; },
  autoMockOn: () => { __ttAutomockOn = true; return globalThis.jest; },
  autoMockOff: () => { __ttAutomockOn = false; return globalThis.jest; },
  onGenerateMock: (factory) => {
    if (typeof factory === 'function') {
      const prev = __ttAutomockGenerator;
      __ttAutomockGenerator = (path, exports) => factory(path, prev(path, exports));
    }
    return globalThis.jest;
  },
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
  requireActual: (p) => globalThis.__ttImportActual(globalThis.__ttDir || globalThis.__ttEntryDir, String(p)),
  requireMock: (p) => __automock(globalThis.jest.requireActual(p)),
  // jest.isolateModules(fn): run `fn` with a fresh module registry. Approximate with a
  // resetModules around the (sync) callback — enough for the common "re-require with new env".
  isolateModules: (fn) => { globalThis.__ttSaveModules(); try { fn(); } finally { globalThis.__ttRestoreModules(); } },
  isolateModulesAsync: async (fn) => { globalThis.__ttSaveModules(); try { await fn(); } finally { globalThis.__ttRestoreModules(); } },
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
  runAllTicks: () => { __jestCheckFake(); __loop.drainNextTicks(); },
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
  // jest.setTimeout(ms) and jest.retryTimes(n): per-file overrides (turbo-test#273), cleared
  // in __ttResetForNextFile. jest 30 ignores a non-number argument.
  setTimeout: (ms) => { if (typeof ms === 'number' && ms >= 0) globalThis.__TT_JEST_TIMEOUT = ms; return globalThis.jest; },
  retryTimes: (n) => { if (typeof n === 'number' && n >= 0) globalThis.__TT_JEST_RETRY = n; return globalThis.jest; },
  // env stubbing parity (rarely used via jest, but harmless)
  replaceProperty: (obj, key, val) => {
    const orig = obj[key];
    obj[key] = val;
    const handle = {
      replaceValue: (v) => { obj[key] = v; return handle; },
      restore: () => { obj[key] = orig; },
    };
    __mockRestores.add(() => { obj[key] = orig; });
    return handle;
  },
};

// ---- test.each / describe.each titles (turbo-test#67) ----
// vitest's `formatTitle` (@vitest/runner): `%#` / `%$` (index / index + 1), printf placeholders
// (`%s %d %i %f %j %o %O %c %%`, each formatting ONE row item as @vitest/utils `format` does) and
// `$key` / `$a.b` / `$0` attributes shown as `objDisplay`. Values are rendered the way vitest's
// loupe `inspect` renders them (`'str'`, `[ 1, 2 ]`, `{ a: 1 }`, `[Function f]`, `+0`).
const __inspectEsc = /['\u0000-\u001f\u007f-\u009f\u00ad\u0600-\u0604\u070f\u17b4\u17b5\u200c-\u200f\u2028-\u202f\u2060-\u206f\ufeff\ufff0-\uffff]/g;
const __inspectEscMap = { '\b': '\\b', '\t': '\\t', '\n': '\\n', '\f': '\\f', '\r': '\\r', "'": "\\'" };
function __inspectTruncEnd(str, len) {
  let end = len - 1;
  if (end > 0 && str[end - 1] >= '\ud800' && str[end - 1] <= '\udbff') end--;
  return end;
}
function __inspectTrunc(str, len) {
  str = String(str);
  if (str.length <= 1) return str;
  if (1 > len) return '…';
  if (str.length <= len) return str;
  return `${str.slice(0, __inspectTruncEnd(str, len))}…`;
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
// inspectList: the entry does not fit (before the next one is looked at)
const __listCutBefore = (last, second, next, cutLen, outCut, orig) => (last && next > orig && outCut <= orig) || (!last && !second && cutLen > orig);
// inspectList: the last-but-one entry and the last do not fit
const __listCutPeek = (last, second, cutLen, withPeek, orig) => !last && second && cutLen > orig && withPeek > orig;
// inspectList: the entries after this one do not fit
const __listCutAfter = (last, second, withPeek, orig) => !last && !second && withPeek >= orig;
// __inspectList's rendered entry and the lookahead for the next one, with the given truncate.
function __inspectListItem(list, i, o, orig, outLen, item, sep, peek) {
  const last = i + 1 === list.length;
  const second = i + 2 === list.length;
  o.truncate = orig - outLen - (last ? 0 : sep.length);
  const str = peek || item(list[i], o) + (last ? '' : sep);
  const nextPeek = last ? '' : item(list[i + 1], o) + (second ? '' : sep);
  return { last, second, str, nextPeek };
}
function __inspectListStep(list, i, o, orig, out, peek, item, sep) {
  const cut = `…(${list.length - i})`;
  const { last, second, str, nextPeek } = __inspectListItem(list, i, o, orig, out.length, item, sep, peek);
  const next = out.length + str.length;
  const cutLen = next + cut.length;
  if (__listCutBefore(last, second, next, cutLen, out.length + cut.length, orig)) return { done: true, cut };
  if (__listCutPeek(last, second, cutLen, next + nextPeek.length, orig)) return { done: true, cut, peek: nextPeek };
  if (__listCutAfter(last, second, next + nextPeek.length, orig)) return { done: true, cut: `…(${list.length - i - 1})`, add: str, peek: nextPeek };
  return { done: false, add: str, peek: nextPeek };
}
function __inspectListApply(r, state) {
  if (r.add) state.out += r.add;
  if (r.peek !== undefined) state.peek = r.peek;
  state.cut = r.cut;
  if (!r.done) state.cut = '';
  return r.done;
}
function __inspectList(list, o, item, sep) {
  if (!list.length) return '';
  const orig = o.truncate;
  const useItem = item === undefined ? __inspect : item;
  const useSep = sep === undefined ? ', ' : sep;
  const state = { out: '', peek: '', cut: '' };
  for (let i = 0; i < list.length; i++) {
    const r = __inspectListStep(list, i, o, orig, state.out, state.peek, useItem, useSep);
    if (__inspectListApply(r, state)) break;
  }
  return `${state.out}${state.cut}`;
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
function __inspectBigInt(v, o) {
  const n = __inspectTrunc(v.toString(), o.truncate - 1);
  return n === '…' ? n : `${n}n`;
}
function __inspectFunction(v, o) {
  const tag = v[Symbol.toStringTag] || 'Function';
  return v.name ? `[${tag} ${__inspectTrunc(v.name, o.truncate - 11)}]` : `[${tag}]`;
}
const __INSPECT_PRIM = {
  undefined: () => 'undefined',
  boolean: (v) => String(v),
  number: __inspectNumber,
  bigint: __inspectBigInt,
  string: __inspectString,
  symbol: (v) => (v.description ? `Symbol(${v.description})` : 'Symbol()'),
  function: __inspectFunction,
};
// loupe's inspect of a primitive or function (undefined: an object)
function __inspectPrim(v, o) {
  const fn = __INSPECT_PRIM[typeof v];
  return fn ? fn(v, o) : undefined;
}
function __inspectArray(v, o) {
  const extra = Object.keys(v).slice(v.length);
  if (!v.length && !extra.length) return '[]';
  o.truncate -= 4;
  const items = __inspectList(Array.from({ length: v.length }, (_, i) => v[i]), o);
  o.truncate -= items.length;
  const props = extra.length ? __inspectList(extra.map((k) => [k, v[k]]), o, __inspectProp) : '';
  return `[ ${items}${props ? `, ${props}` : ''} ]`;
}
function __inspectDate(v, o) {
  const j = v.toJSON();
  if (j === null) return 'Invalid Date';
  const [d, t] = j.split('T');
  return `${d}T${__inspectTrunc(t, o.truncate - d.length - 1)}`;
}
function __inspectError(v, o) {
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
// loupe's inspect of a collection: `name{ entries }` (empty: `name{}`)
function __inspectCollection(v, o, name, size, entries, item) {
  if (!size) return `${name}{}`;
  o.truncate -= 7;
  return `${name}{ ${__inspectList(entries, o, item)} }`;
}
const __inspectMapEntry = ([k, x], oo) => { oo.truncate -= 4; const ks = __inspect(k, oo); oo.truncate -= ks.length; return `${ks} => ${__inspect(x, oo)}`; };
const __inspectEmpty = () => '';
const __inspectArguments = (v, o) => {
  if (!v.length) return 'Arguments[]';
  o.truncate -= 13;
  return `Arguments[ ${__inspectList([...v], o)} ]`;
};
// loupe's inspect by the object's class
const __inspectTypes = new Map([
  ['Boolean', (v) => String(v)],
  ['Number', (v, o) => __inspectNumber(Number(v), o)],
  ['String', (v, o) => __inspectString(v, o)],
  ['Array', __inspectArray],
  ['Date', __inspectDate],
  ['Map', (v, o) => __inspectCollection(v, o, 'Map', v.size, v.size ? [...v] : [], __inspectMapEntry)],
  ['Set', (v, o) => __inspectCollection(v, o, 'Set', v.size, v.size ? [...v] : [], undefined)],
  ['RegExp', (v, o) => `/${__inspectTrunc(v.source, o.truncate - (2 + v.flags.length))}/${v.flags}`],
  ['Promise', () => 'Promise{…}'],
  ['WeakSet', () => 'WeakSet{…}'],
  ['WeakMap', () => 'WeakMap{…}'],
  ['Arguments', __inspectArguments],
  ['Generator', __inspectEmpty], ['DataView', __inspectEmpty], ['ArrayBuffer', __inspectEmpty],
  ['Error', __inspectError],
]);
function __inspectTypedName(v) {
  if (typeof Buffer === 'function' && v instanceof Buffer) return 'Buffer';
  return v[Symbol.toStringTag] || v.constructor.name;
}
function __inspectTypedItem(v, i, o) {
  const sep = i === v.length - 1 ? '' : ', ';
  const str = `${__inspectTrunc(v[i], o.truncate)}${sep}`;
  o.truncate -= str.length;
  if (v[i] !== v.length && o.truncate <= 3) return { cut: `…(${v.length - v[i] + 1})` };
  return { str };
}
function __inspectTyped(v, o) {
  const name = __inspectTypedName(v);
  if (!v.length) return `${name}[]`;
  o.truncate -= name.length + 4;
  let out = '';
  for (let i = 0; i < v.length; i++) {
    const r = __inspectTypedItem(v, i, o);
    if (r.cut) { out += r.cut; break; }
    out += r.str;
  }
  return `${name}[ ${out} ]`;
}
// an object's own inspect: an asymmetric matcher, chai/inspect, util.inspect.custom or an
// `inspect` method (undefined: none)
function __inspectHooksCustom(v, o) {
  const custom = v[Symbol.for('nodejs.util.inspect.custom')];
  if (typeof custom !== 'function') return undefined;
  // loupe hands the method its normalised options (depth 2, breakLength Infinity, no colours)
  const r = custom.call(v, 2, Object.assign({ depth: 2, breakLength: Infinity, colors: false, showHidden: false }, o), __inspect);
  return typeof r === 'string' ? r : __inspect(r, o);
}
function __inspectHooks(v, o) {
  // vitest's asymmetric matchers inspect as their minimal pretty-format (`Any<Number>`, …)
  if (v instanceof Asymmetric) {
    const r = __pf(v, { min: true });
    return r.length <= o.truncate ? r : `${v.toString()}{…}`;
  }
  const chai = v[Symbol.for('chai/inspect')];
  if (typeof chai === 'function') return String(chai.call(v, o));
  const custom = __inspectHooksCustom(v, o);
  if (custom !== undefined) return custom;
  if (typeof v.inspect === 'function') { const r = v.inspect(2, o); return typeof r === 'string' ? r : __inspect(r, o); }
  return undefined;
}
function __inspectInstance(v, o) {
  const proto = Object.getPrototypeOf(v);
  if (proto === Object.prototype || proto === null) return __inspectObject(v, o);
  let name = v[Symbol.toStringTag] || (v.constructor && v.constructor.name);
  if (!name || name === '_class') name = '<Anonymous Class>';
  o.truncate -= name.length;
  return `${name}${__inspectObject(v, o)}`;
}
function __inspect(v, o = { truncate: Infinity, seen: [] }) {
  if (v === null) return 'null';
  const prim = __inspectPrim(v, o);
  if (prim !== undefined) return prim;
  const byType = __inspectTypes.get(Object.prototype.toString.call(v).slice(8, -1));
  if (byType) return byType(v, o);
  if (ArrayBuffer.isView(v)) return __inspectTyped(v, o);
  const hooked = __inspectHooks(v, o);
  if (hooked !== undefined) return hooked;
  return __inspectInstance(v, o);
}
// @vitest/utils `objDisplay` (truncateThreshold 40): loupe-truncated, and a rendering still that
// long collapses to a summary.
// toHaveProperty's path walk. Returns { cur, present }: a literal-key hit wins over the dot
// path, else the segments are walked; `present=false` if any segment is missing.
function __lookupPropertyLiteral(actual, path) {
  if (Array.isArray(path) || actual == null) return null;
  if (!(path in Object(actual))) return null;
  return { cur: actual[path], present: true };
}
function __lookupProperty(actual, path) {
  const literal = __lookupPropertyLiteral(actual, path);
  if (literal) return literal;
  const keys = Array.isArray(path) ? path : String(path).split('.');
  let cur = actual;
  for (const k of keys) {
    if (cur != null && (k in Object(cur))) cur = cur[k];
    else return { cur: undefined, present: false };
  }
  return { cur, present: true };
}
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
// __pfPrint's "not printed by this step"
const __PF_NONE = Symbol('pf-none');
// __pfAsym matcher-name tables: a toString() membership dispatches to the sample shape.
const __PF_ARR_MATCHERS = new Set(['ArrayContaining', 'ArrayNotContaining']);
const __PF_OBJ_MATCHERS = new Set(['ObjectContaining', 'ObjectNotContaining']);
const __PF_STR_MATCHERS = new Set(['StringMatching', 'StringNotMatching', 'StringContaining', 'StringNotContaining']);
// @vitest/pretty-format's AsymmetricMatcher plugin: anything with vitest's `$$typeof`, printed by
// its toString name (containing / matching samples) or its toAsymmetricMatcher()
function __pfAsym(v, cfg, ind, depth, refs) {
  const name = v.toString();
  const d = depth + 1;
  const pp = (x, i) => __pfPrint(x, cfg, i, d, refs, false);
  if (__PF_ARR_MATCHERS.has(name)) return d > cfg.maxDepth ? `[${name}]` : `${name} [${__pfItems(Array.from(v.sample), cfg, ind, d, refs, pp)}]`;
  if (__PF_OBJ_MATCHERS.has(name)) return d > cfg.maxDepth ? `[${name}]` : `${name} {${__pfProps(v.sample, cfg, ind, d, refs)}}`;
  if (__PF_STR_MATCHERS.has(name)) return `${name} ${__pfPrint(v.sample, cfg, ind, depth, refs, false)}`;
  if (typeof v.toAsymmetricMatcher !== 'function') throw new TypeError(`Asymmetric matcher ${v.constructor.name} does not implement toAsymmetricMatcher()`);
  return v.toAsymmetricMatcher();
}
// vitest's snapshot serializer for mock functions: `[MockFunction name] { calls, results }`
function __pfMockFn(v, cfg, ind, depth, refs) {
  const name = v.getMockName();
  let calls = '';
  if (v.mock.calls.length) {
    const next = ind + cfg.indent;
    calls = ` {${cfg.outer}${next}"calls": ${__pfPrint(v.mock.calls, cfg, next, depth, refs, false)}${cfg.min ? ', ' : ','}${cfg.outer}${next}"results": ${__pfPrint(v.mock.results, cfg, next, depth, refs, false)}${cfg.min ? '' : ','}${cfg.outer}${ind}}`;
  }
  return `[MockFunction${name === 'vi.fn()' ? '' : ` ${name}`}]${calls}`;
}
function __pfErrorEntries(v) {
  const entries = { message: v.message };
  if (v.cause !== undefined) entries.cause = v.cause;
  for (const k of Object.keys(v)) if (k !== 'message' && k !== 'cause') entries[k] = v[k];
  return entries;
}
function __pfError(v, cfg, ind, depth, refs) {
  if (refs.includes(v)) return '[Circular]';
  const r = [...refs, v];
  const d = depth + 1;
  const name = v.name !== 'Error' ? v.name : __pfCtorName(v);
  if (d > cfg.maxDepth) return `[${name}]`;
  return `${name} {${__pfEntries(Object.entries(__pfErrorEntries(v)), cfg, ind, d, r, ': ')}}`;
}
// Walk cfg.plugins, calling each one's test(v) and letting the first match print the value.
function __pfPluginFor(v, cfg, ind, depth, refs) {
  for (const plugin of cfg.plugins) {
    let hit;
    try { hit = plugin.test(v); } catch (error) { throw new __PrettyFormatPluginError(error.message, error.stack); }
    if (hit) return __pfPlugin(plugin, v, cfg, ind, depth, refs);
  }
  return __PF_NONE;
}
// plugins, asymmetric matchers, mock functions and (cfg.errors) errors
function __pfIsAsym(v) { return v && v.$$typeof === Symbol.for('jest.asymmetricMatcher'); }
function __pfIsMock(v, cfg) { return cfg.mocks && typeof v === 'function' && v._isMockFunction; }
function __pfHooks(v, cfg, ind, depth, refs) {
  const p = __pfPluginFor(v, cfg, ind, depth, refs);
  if (p !== __PF_NONE) return p;
  if (__pfIsAsym(v)) return __pfAsym(v, cfg, ind, depth, refs);
  if (__pfIsMock(v, cfg)) return __pfMockFn(v, cfg, ind, depth, refs);
  if (cfg.errors && v instanceof Error) return __pfError(v, cfg, ind, depth, refs);
  return __PF_NONE;
}
// __pfPrimitive's typeof-dispatch over number/bigint/string/function/symbol.
const __PF_TYPEOF = {
  number: (v) => Object.is(v, -0) ? '-0' : String(v),
  bigint: (v) => `${v}n`,
  string: (v, cfg) => cfg.escape ? `"${v.replace(/"|\\/g, '\\$&')}"` : `"${v}"`,
  function: (v, cfg) => cfg.fnName ? `[Function ${v.name || 'anonymous'}]` : '[Function]',
  symbol: (v) => String(v).replace(/^Symbol\((.*)\)(.*)$/, 'Symbol($1)'),
};
function __pfTypeof(v, cfg) {
  const fn = __PF_TYPEOF[typeof v];
  return fn ? fn(v, cfg) : undefined;
}
// a primitive or function (undefined: an object)
function __pfPrimitive(v, cfg) {
  if (v === true || v === false) return `${v}`;
  if (v === undefined) return 'undefined';
  if (v === null) return 'null';
  return __pfTypeof(v, cfg);
}
// weak collections, dates, errors and regexps (undefined: other objects)
const __PF_SIMPLE_CONST = { '[object WeakMap]': 'WeakMap {}', '[object WeakSet]': 'WeakSet {}' };
function __pfSimpleRegExp(v, cfg) {
  const r = RegExp.prototype.toString.call(v);
  return cfg.escapeRegex ? r.replace(/[$()*+.?[\\\]^{|}]/g, '\\$&') : r;
}
function __pfSimpleTag(v, cfg, tag) {
  if (__PF_SIMPLE_CONST[tag]) return __PF_SIMPLE_CONST[tag];
  if (tag === '[object Date]') return Number.isNaN(+v) ? 'Date { NaN }' : v.toISOString();
  if (tag === '[object Error]' || v instanceof Error) return `[${Error.prototype.toString.call(v)}]`;
  if (tag === '[object RegExp]') return __pfSimpleRegExp(v, cfg);
  return undefined;
}
function __pfArrayLike(v, cfg, ind, d, refs, hit, pp) {
  const name = v.constructor.name;
  if (hit) return `[${name}]`;
  const prefix = cfg.min ? '' : !cfg.basic && name === 'Array' ? '' : `${name} `;
  return `${prefix}[${__pfItems(Array.from({ length: v.length }, (_, i) => v[i]), cfg, ind, d, refs, pp)}]`;
}
function __pfObject(v, cfg, ind, d, refs, hit) {
  const name = __pfCtorName(v);
  if (hit) return `[${name}]`;
  const prefix = cfg.min ? '' : !cfg.basic && name === 'Object' ? '' : `${name} `;
  return `${prefix}{${__pfProps(v, cfg, ind, d, refs)}}`;
}
// arguments, arrays, typed arrays, maps, sets and other objects, one level down (`hit`: past maxDepth)
function __pfArgumentsBody(v, cfg, ind, d, refs, pp) {
  return `${cfg.min ? '' : 'Arguments '}[${__pfItems(Array.from(v), cfg, ind, d, refs, pp)}]`;
}
const __PF_TAGGED = {
  '[object Arguments]': { hit: '[Arguments]', body: __pfArgumentsBody },
  '[object Map]': { hit: '[Map]', body: (v, cfg, ind, d, refs) => `Map {${__pfEntries([...v.entries()], cfg, ind, d, refs, ' => ')}}` },
  '[object Set]': { hit: '[Set]', body: (v, cfg, ind, d, refs, pp) => `Set {${__pfItems([...v.values()], cfg, ind, d, refs, pp)}}` },
};
function __pfCompositeTagged(v, cfg, ind, d, refs, hit, tag, pp) {
  const entry = __PF_TAGGED[tag];
  if (!entry) return undefined;
  return hit ? entry.hit : entry.body(v, cfg, ind, d, refs, pp);
}
function __pfComposite(v, cfg, ind, d, refs, hit, tag) {
  const pp = (x, i) => __pfPrint(x, cfg, i, d, refs, false);
  const tagged = __pfCompositeTagged(v, cfg, ind, d, refs, hit, tag, pp);
  if (tagged !== undefined) return tagged;
  if (Array.isArray(v) || ArrayBuffer.isView(v)) return __pfArrayLike(v, cfg, ind, d, refs, hit, pp);
  return __pfObject(v, cfg, ind, d, refs, hit);
}
function __pfShouldJSON(cfg, v, hit, calledToJSON) {
  return cfg.toJSON && !hit && typeof v.toJSON === 'function' && !calledToJSON;
}
function __pfPrint(v, cfg, ind, depth, refs, calledToJSON) {
  const hooked = __pfHooks(v, cfg, ind, depth, refs);
  if (hooked !== __PF_NONE) return hooked;
  const prim = __pfPrimitive(v, cfg);
  if (prim !== undefined) return prim;
  const tag = Object.prototype.toString.call(v);
  const simple = __pfSimpleTag(v, cfg, tag);
  if (simple !== undefined) return simple;
  if (refs.includes(v)) return '[Circular]';
  const nextRefs = [...refs, v];
  const d = depth + 1;
  const hit = d > cfg.maxDepth;
  if (__pfShouldJSON(cfg, v, hit, calledToJSON)) return __pfPrint(v.toJSON(), cfg, ind, d, nextRefs, true);
  return __pfComposite(v, cfg, ind, d, nextRefs, hit, tag);
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
  // overlapF's division of the sequences at the middle snake
  const divisionF = (d, aStart, aEnd, bStart, bEnd, common, division, bF, aLastPrev, insert, kF, aFirst, bFirst, nCommonF, aLast) => {
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
  };
  // overlapR's division of the sequences at the middle snake
  const divisionR = (d, aStart, aEnd, bStart, bEnd, common, division, bR, aLastPrev, insert, kR, bFirst, nCommonR, aLast) => {
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
  };
  // Did the forward wavefront at (kF, aLast) reach a reverse-wavefront point already recorded?
  const overlapFReached = (kF, kMin, kMax, iMaxR, d, baDeltaLength, aIndexesR, aLast) => {
    if (kMin > kF || kF > kMax) return false;
    const iR = (d - 1 - (kF + baDeltaLength)) / 2;
    return iR <= iMaxR && aIndexesR[iR] - 1 <= aLast;
  };
  const overlapFStep = (iF, d, aIndexPrev1, aIndexesF, aEnd, bEnd, bF, kF, common) => {
    const insert = iF === 0 || (iF !== d && aIndexPrev1 < aIndexesF[iF]);
    const aLastPrev = insert ? aIndexesF[iF] : aIndexPrev1;
    const aFirst = insert ? aLastPrev : aLastPrev + 1;
    const bFirst = bF + aFirst - kF;
    const nCommonF = countF(aFirst + 1, aEnd, bFirst + 1, bEnd, common);
    return { insert, aLastPrev, aFirst, bFirst, nCommonF, aLast: aFirst + nCommonF };
  };
  const overlapF = (d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division) => {
    const bF = bStart - aStart;
    const baDeltaLength = (bEnd - bStart) - (aEnd - aStart);
    const kMin = -baDeltaLength - (d - 1);
    const kMax = -baDeltaLength + (d - 1);
    let aIndexPrev1 = 0;
    const nF = d < iMaxF ? d : iMaxF;
    for (let iF = 0, kF = -d; iF <= nF; iF += 1, kF += 2) {
      const s = overlapFStep(iF, d, aIndexPrev1, aIndexesF, aEnd, bEnd, bF, kF, common);
      aIndexPrev1 = aIndexesF[iF];
      aIndexesF[iF] = s.aLast;
      if (overlapFReached(kF, kMin, kMax, iMaxR, d, baDeltaLength, aIndexesR, s.aLast)) {
        divisionF(d, aStart, aEnd, bStart, bEnd, common, division, bF, s.aLastPrev, s.insert, kF, s.aFirst, s.bFirst, s.nCommonF, s.aLast);
        return true;
      }
    }
    return false;
  };
  // Did the reverse wavefront at (kR, aLast) reach a forward-wavefront point already recorded?
  const overlapRReached = (kR, kMin, kMax, iMaxF, d, baDeltaLength, aIndexesF, aLast) => {
    if (kMin > kR || kR > kMax) return false;
    const iF = (d + (kR - baDeltaLength)) / 2;
    return iF <= iMaxF && aLast - 1 <= aIndexesF[iF];
  };
  const overlapRStep = (iR, d, aIndexPrev1, aIndexesR, aStart, bStart, bR, kR, common) => {
    const insert = iR === 0 || (iR !== d && aIndexesR[iR] < aIndexPrev1);
    const aLastPrev = insert ? aIndexesR[iR] : aIndexPrev1;
    const aFirst = insert ? aLastPrev : aLastPrev - 1;
    const bFirst = bR + aFirst - kR;
    const nCommonR = countR(aStart, aFirst - 1, bStart, bFirst - 1, common);
    return { insert, aLastPrev, aFirst, bFirst, nCommonR, aLast: aFirst - nCommonR };
  };
  const overlapR = (d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division) => {
    const bR = bEnd - aEnd;
    const baDeltaLength = (bEnd - bStart) - (aEnd - aStart);
    const kMin = baDeltaLength - d;
    const kMax = baDeltaLength + d;
    let aIndexPrev1 = 0;
    const nR = d < iMaxR ? d : iMaxR;
    for (let iR = 0, kR = d; iR <= nR; iR += 1, kR -= 2) {
      const s = overlapRStep(iR, d, aIndexPrev1, aIndexesR, aStart, bStart, bR, kR, common);
      aIndexPrev1 = aIndexesR[iR];
      aIndexesR[iR] = s.aLast;
      if (overlapRReached(kR, kMin, kMax, iMaxF, d, baDeltaLength, aIndexesF, s.aLast)) {
        divisionR(d, aStart, aEnd, bStart, bEnd, common, division, bR, s.aLastPrev, s.insert, kR, s.bFirst, s.nCommonR, s.aLast);
        return true;
      }
    }
    return false;
  };
  const divideEven = (ctx, division) => {
    const { nChange, aStart, aEnd, bStart, bEnd, aLength, bLength, bR, bF, common, aIndexesF, aIndexesR, baDeltaLength } = ctx;
    const dMin = (nChange || baDeltaLength) / 2;
    const dMax = (aLength + bLength) / 2;
    let iMaxF = aLength, iMaxR = aLength;
    for (let d = 1; d <= dMax; d += 1) {
      iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
      if (d < dMin) iMaxR = extendR(d, aStart, bStart, bR, common, aIndexesR, iMaxR);
      else if (overlapR(d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division)) return true;
    }
    return false;
  };
  const divideOdd = (ctx, division) => {
    const { nChange, aStart, aEnd, bStart, bEnd, aLength, bLength, bR, bF, common, aIndexesF, aIndexesR, baDeltaLength } = ctx;
    const dMin = ((nChange || baDeltaLength) + 1) / 2;
    const dMax = (aLength + bLength + 1) / 2;
    let iMaxF = aLength, iMaxR = aLength;
    let d = 1;
    iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
    for (d += 1; d <= dMax; d += 1) {
      iMaxR = extendR(d - 1, aStart, bStart, bR, common, aIndexesR, iMaxR);
      if (d < dMin) iMaxF = extendF(d, aEnd, bEnd, bF, common, aIndexesF, iMaxF);
      else if (overlapF(d, aStart, aEnd, bStart, bEnd, common, aIndexesF, iMaxF, aIndexesR, iMaxR, division)) return true;
    }
    return false;
  };
  const divide = (nChange, aStart, aEnd, bStart, bEnd, common, aIndexesF, aIndexesR, division) => {
    const bF = bStart - aStart;
    const bR = bEnd - aEnd;
    const aLength = aEnd - aStart;
    const bLength = bEnd - bStart;
    const baDeltaLength = bLength - aLength;
    aIndexesF[0] = aStart - 1;
    aIndexesR[0] = aEnd;
    const ctx = { nChange, aStart, aEnd, bStart, bEnd, aLength, bLength, bR, bF, common, aIndexesF, aIndexesR, baDeltaLength };
    if (baDeltaLength % 2 === 0 ? divideEven(ctx, division) : divideOdd(ctx, division)) return;
    throw new Error(`diff-sequences: no overlap aStart=${aStart} aEnd=${aEnd} bStart=${bStart} bEnd=${bEnd}`);
  };
  // Transpose the halves so that `a` is the shorter sequence, lazily installing the mirrored
  // callback pair the first time this happens. Returns the new [aStart, aEnd, bStart, bEnd, transposed].
  const transposeHalves = (aStart, aEnd, bStart, bEnd, transposed, callbacks) => {
    const t = !transposed;
    if (t && callbacks.length === 1) {
      const { found, common } = callbacks[0];
      callbacks[1] = { found: (n, bc, ac) => found(n, ac, bc), common: (bi, ai) => common(ai, bi) };
    }
    return [bStart, bEnd, aStart, aEnd, t];
  };
  const findEmit = (division, found) => {
    const { nCommonPreceding, aCommonPreceding, bCommonPreceding, nCommonFollowing, aCommonFollowing, bCommonFollowing } = division;
    if (nCommonPreceding !== 0) found(nCommonPreceding, aCommonPreceding, bCommonPreceding);
    if (nCommonFollowing !== 0) found(nCommonFollowing, aCommonFollowing, bCommonFollowing);
  };
  const find = (nChange, aStart, aEnd, bStart, bEnd, transposed, callbacks, aIndexesF, aIndexesR, division) => {
    if (bEnd - bStart < aEnd - aStart) {
      [aStart, aEnd, bStart, bEnd, transposed] = transposeHalves(aStart, aEnd, bStart, bEnd, transposed, callbacks);
    }
    const { found, common } = callbacks[transposed ? 1 : 0];
    divide(nChange, aStart, aEnd, bStart, bEnd, common, aIndexesF, aIndexesR, division);
    const { nChangePreceding, aEndPreceding, bEndPreceding, nChangeFollowing, aStartFollowing, bStartFollowing } = division;
    if (aStart < aEndPreceding && bStart < bEndPreceding) find(nChangePreceding, aStart, aEndPreceding, bStart, bEndPreceding, transposed, callbacks, aIndexesF, aIndexesR, division);
    findEmit(division, found);
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
// joinAlignedDiffsNoExpand: whether common runs longer than the context are cut (`@@` marks)
function __diffsPatchHit(start, i, len, ctx, ctx2) {
  if (start === i) return false;
  const k = i - start;
  return k > (start === 0 || i === len ? ctx : ctx2);
}
function __diffsNeedPatch(diffs, ctx, ctx2) {
  const len = diffs.length;
  let i = 0;
  while (i !== len) {
    const start = i;
    while (i !== len && diffs[i][0] === 0) i++;
    if (__diffsPatchHit(start, i, len, ctx, ctx2)) return true;
    while (i !== len && diffs[i][0] !== 0) i++;
  }
  return false;
}
// __joinDiffs's main pass over the (op, line) sequence: a run of common lines first (deferred to
// commonRun), then every immediately-adjacent `-` and `+` as consecutive del/ins blocks.
function __joinDiffsOpRun(diffs, i, len, op, emit) {
  while (i !== len && diffs[i][0] === op) { emit(diffs[i][1]); i++; }
  return i;
}
function __joinDiffsLoop(diffs, len, commonRun, del, ins) {
  let i = 0;
  while (i !== len) {
    const start = i;
    while (i !== len && diffs[i][0] === 0) i++;
    if (start !== i) commonRun(start, i);
    i = __joinDiffsOpRun(diffs, i, len, -1, del);
    i = __joinDiffsOpRun(diffs, i, len, 1, ins);
  }
}
function __joinDiffs(diffs) {
  const ctx = 5;
  const ctx2 = ctx + ctx;
  const color = { '-': __dcol.green, '+': __dcol.red, ' ': __dcol.dim };
  const line = (ind, l) => (l.length ? color[ind](`${ind} ${l}`) : ind !== ' ' ? color[ind](ind) : '');
  const len = diffs.length;
  const patch = __diffsNeedPatch(diffs, ctx, ctx2);
  const lines = [];
  let mark = 0;
  if (patch) lines.push('');
  let aStart = 0, bStart = 0, aEnd = 0, bEnd = 0;
  const common = (l) => { lines.push(line(' ', l)); aEnd++; bEnd++; };
  const del = (l) => { lines.push(line('-', l)); aEnd++; };
  const ins = (l) => { lines.push(line('+', l)); bEnd++; };
  const patchMark = () => __dcol.yellow(`@@ -${aStart + 1},${aEnd - aStart} +${bStart + 1},${bEnd - bStart} @@`);
  // An inner common run (both ends are change lines): emit `ctx` lines on each side with a patch
  // mark between them when the gap is wider than `ctx2`; otherwise emit the whole run as-is.
  const innerCommonRun = (start, i) => {
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
  };
  // the common run diffs[start..i): leading / trailing ones keep `ctx` lines, inner ones `ctx`
  // on each side of a patch mark
  const commonRun = (start, i) => {
    if (start === 0) {
      if (i > ctx) { start = i - ctx; aStart = start; bStart = start; aEnd = aStart; bEnd = bStart; }
      for (let c = start; c !== i; c++) common(diffs[c][1]);
    } else if (i === len) {
      const end = i - start > ctx ? start + ctx : i;
      for (let c = start; c !== end; c++) common(diffs[c][1]);
    } else {
      innerCommonRun(start, i);
    }
  };
  __joinDiffsLoop(diffs, len, commonRun, del, ins);
  if (patch) lines[mark] = patchMark();
  return lines.join('\n');
}
// A single line of "" and no compare lines becomes an empty array, matching jest-diff's own
// collapse. Returns the normalized [aDisp, bDisp, aCmp, bCmp] for __diffLines to consume.
function __diffLinesPrep(aDisp, bDisp, aCmp, bCmp) {
  const empty = (l) => l.length === 1 && l[0].length === 0;
  if (empty(aDisp) && empty(aCmp)) { aDisp = []; aCmp = []; }
  if (empty(bDisp) && empty(bCmp)) { bDisp = []; bCmp = []; }
  if (aDisp.length !== aCmp.length || bDisp.length !== bCmp.length) { aCmp = aDisp; bCmp = bDisp; }
  return [aDisp, bDisp, aCmp, bCmp];
}
// jest-diff `diffLinesUnified2`: align by the compare lines, show the display lines; with
// `annotate` the `- Expected` / `+ Received` lines first (printDiffLines).
function __diffLines(aDisp, bDisp, aCmp = aDisp, bCmp = bDisp, annotate = false) {
  [aDisp, bDisp, aCmp, bCmp] = __diffLinesPrep(aDisp, bDisp, aCmp, bCmp);
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
function __commonOverlapTrim(t1, t2, l1, l2) {
  if (l1 > l2) return [t1.substring(l1 - l2), t2];
  if (l1 < l2) return [t1, t2.substring(0, l1)];
  return [t1, t2];
}
function __commonOverlapLoop(t1, t2, len) {
  let best = 0;
  let length = 1;
  for (;;) {
    const found = t2.indexOf(t1.substring(len - length));
    if (found === -1) return best;
    length += found;
    if (found === 0 || t1.substring(len - length) === t2.substring(0, length)) { best = length; length++; }
  }
}
function __commonOverlap(t1, t2) {
  const l1 = t1.length;
  const l2 = t2.length;
  if (l1 === 0 || l2 === 0) return 0;
  [t1, t2] = __commonOverlapTrim(t1, t2, l1, l2);
  const len = Math.min(l1, l2);
  if (t1 === t2) return len;
  return __commonOverlapLoop(t1, t2, len);
}
// semanticScore's classes of the character at a boundary: non-alphanumeric, whitespace, line
// break, blank line (`blank`: the text's blank-line test at that end)
function __scoreSide(c, text, blank) {
  const nonAlnum = /[^a-z0-9]/i.test(c);
  const ws = nonAlnum && /\s/.test(c);
  const lb = ws && /[\r\n]/.test(c);
  return { nonAlnum, ws, lb, blank: lb && blank.test(text) };
}
// True when either edge has the flag — the symmetric half of __semanticRank's table.
function __semanticEither(x, y, key) { return x[key] || y[key]; }
// __semanticScore's rank table over the two edges' character classes.
const __semanticSplit = (x, y) => x.nonAlnum && !x.ws && y.ws;
function __semanticRank(x, y) {
  if (__semanticEither(x, y, 'blank')) return 5;
  if (__semanticEither(x, y, 'lb')) return 4;
  if (__semanticSplit(x, y)) return 3;
  if (__semanticEither(x, y, 'ws')) return 2;
  return __semanticEither(x, y, 'nonAlnum') ? 1 : 0;
}
function __semanticScore(one, two) {
  if (!one || !two) return 6;
  const x = __scoreSide(one.charAt(one.length - 1), one, /\n\r?\n$/);
  const y = __scoreSide(two.charAt(0), two, /^\r?\n\r?\n/);
  return __semanticRank(x, y);
}
function __losslessShiftSuffix(eq1, edit, eq2) {
  const off = __commonSuffix(eq1, edit);
  if (!off) return { eq1, edit, eq2 };
  const common = edit.substring(edit.length - off);
  return { eq1: eq1.substring(0, eq1.length - off), edit: common + edit.substring(0, edit.length - off), eq2: common + eq2 };
}
function __losslessBestShift(eq1, edit, eq2) {
  let bestEq1 = eq1, bestEdit = edit, bestEq2 = eq2;
  let bestScore = __semanticScore(eq1, edit) + __semanticScore(edit, eq2);
  while (edit.charAt(0) === eq2.charAt(0)) {
    eq1 += edit.charAt(0);
    edit = edit.substring(1) + eq2.charAt(0);
    eq2 = eq2.substring(1);
    const score = __semanticScore(eq1, edit) + __semanticScore(edit, eq2);
    if (score >= bestScore) { bestScore = score; bestEq1 = eq1; bestEdit = edit; bestEq2 = eq2; }
  }
  return { bestEq1, bestEdit, bestEq2 };
}
function __losslessApply(diffs, pointer, best) {
  if (diffs[pointer - 1][1] === best.bestEq1) return pointer;
  if (best.bestEq1) diffs[pointer - 1][1] = best.bestEq1;
  else { diffs.splice(pointer - 1, 1); pointer--; }
  diffs[pointer][1] = best.bestEdit;
  if (best.bestEq2) diffs[pointer + 1][1] = best.bestEq2;
  else { diffs.splice(pointer + 1, 1); pointer--; }
  return pointer;
}
function __cleanupSemanticLossless(diffs) {
  let pointer = 1;
  while (pointer < diffs.length - 1) {
    if (diffs[pointer - 1][0] === 0 && diffs[pointer + 1][0] === 0) {
      const shifted = __losslessShiftSuffix(diffs[pointer - 1][1], diffs[pointer][1], diffs[pointer + 1][1]);
      const best = __losslessBestShift(shifted.eq1, shifted.edit, shifted.eq2);
      pointer = __losslessApply(diffs, pointer, best);
    }
    pointer++;
  }
}
// cleanupMerge: factor a common prefix / suffix of the run's insertions and deletions out into
// the equalities around it; returns [pointer, tIns, tDel]
function __mergeFactor(diffs, pointer, nDel, nIns, tIns, tDel) {
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
  return [pointer, tIns, tDel];
}
// cleanupMerge at an equality ending a run of edits: the run as one deletion and one insertion
// (or the equality merged into the previous one); returns the new pointer
function __mergeRunMulti(diffs, pointer, nDel, nIns, tDel, tIns) {
  if (nDel !== 0 && nIns !== 0) [pointer, tIns, tDel] = __mergeFactor(diffs, pointer, nDel, nIns, tIns, tDel);
  pointer -= nDel + nIns;
  diffs.splice(pointer, nDel + nIns);
  if (tDel.length) { diffs.splice(pointer, 0, [-1, tDel]); pointer++; }
  if (tIns.length) { diffs.splice(pointer, 0, [1, tIns]); pointer++; }
  return pointer + 1;
}
function __mergeRun(diffs, pointer, nDel, nIns, tDel, tIns) {
  if (nDel + nIns > 1) return __mergeRunMulti(diffs, pointer, nDel, nIns, tDel, tIns);
  if (pointer !== 0 && diffs[pointer - 1][0] === 0) {
    diffs[pointer - 1][1] += diffs[pointer][1];
    diffs.splice(pointer, 1);
    return pointer;
  }
  return pointer + 1;
}
// cleanupMerge's second pass: single edits surrounded by equalities shift to eliminate one;
// returns whether any did
function __mergeShifts(diffs) {
  let changes = false;
  let pointer = 1;
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
  return changes;
}
function __cleanupMergeStep(state, diffs) {
  const op = diffs[state.pointer][0];
  if (op === 1) { state.nIns++; state.tIns += diffs[state.pointer][1]; state.pointer++; return; }
  if (op === -1) { state.nDel++; state.tDel += diffs[state.pointer][1]; state.pointer++; return; }
  state.pointer = __mergeRun(diffs, state.pointer, state.nDel, state.nIns, state.tDel, state.tIns);
  state.nIns = 0; state.nDel = 0; state.tDel = ''; state.tIns = '';
}
function __cleanupMerge(diffs) {
  diffs.push([0, '']);
  const state = { pointer: 0, nDel: 0, nIns: 0, tDel: '', tIns: '' };
  while (state.pointer < diffs.length) __cleanupMergeStep(state, diffs);
  if (diffs.at(-1)?.[1] === '') diffs.pop();
  if (__mergeShifts(diffs)) __cleanupMerge(diffs);
}
// cleanupSemantic's last step: a deletion and insertion overlapping by at least half of either
// become deletion, equality, insertion
function __overlapsForward(diffs, pointer, deletion, insertion, o1) {
  diffs.splice(pointer, 0, [0, insertion.substring(0, o1)]);
  diffs[pointer - 1][1] = deletion.substring(0, deletion.length - o1);
  diffs[pointer + 1][1] = insertion.substring(o1);
}
function __overlapsReverse(diffs, pointer, deletion, insertion, o2) {
  diffs.splice(pointer, 0, [0, deletion.substring(0, o2)]);
  diffs[pointer - 1][0] = 1;
  diffs[pointer - 1][1] = insertion.substring(0, insertion.length - o2);
  diffs[pointer + 1][0] = -1;
  diffs[pointer + 1][1] = deletion.substring(o2);
}
function __overlapsAt(diffs, pointer) {
  const deletion = diffs[pointer - 1][1];
  const insertion = diffs[pointer][1];
  const o1 = __commonOverlap(deletion, insertion);
  const o2 = __commonOverlap(insertion, deletion);
  if (o1 >= o2) {
    if (o1 >= deletion.length / 2 || o1 >= insertion.length / 2) { __overlapsForward(diffs, pointer, deletion, insertion, o1); return true; }
    return false;
  }
  if (o2 >= deletion.length / 2 || o2 >= insertion.length / 2) { __overlapsReverse(diffs, pointer, deletion, insertion, o2); return true; }
  return false;
}
function __cleanupOverlaps(diffs) {
  let pointer = 1;
  while (pointer < diffs.length) {
    if (diffs[pointer - 1][0] === -1 && diffs[pointer][0] === 1) {
      if (__overlapsAt(diffs, pointer)) pointer++;
      pointer++;
    }
    pointer++;
  }
}
function __cleanupSemEqualityHit(lastEquality, ins1, del1, ins2, del2) {
  return lastEquality && lastEquality.length <= Math.max(ins1, del1) && lastEquality.length <= Math.max(ins2, del2);
}
function __cleanupSemStep(state, diffs) {
  const d = diffs[state.pointer];
  if (d[0] === 0) {
    state.equalities[state.nEq++] = state.pointer;
    state.ins1 = state.ins2; state.del1 = state.del2; state.ins2 = 0; state.del2 = 0;
    state.lastEquality = d[1];
    return;
  }
  if (d[0] === 1) state.ins2 += d[1].length;
  else state.del2 += d[1].length;
  if (!__cleanupSemEqualityHit(state.lastEquality, state.ins1, state.del1, state.ins2, state.del2)) return;
  diffs.splice(state.equalities[state.nEq - 1], 0, [-1, state.lastEquality]);
  diffs[state.equalities[state.nEq - 1] + 1][0] = 1;
  state.nEq -= 2;
  state.pointer = state.nEq > 0 ? state.equalities[state.nEq - 1] : -1;
  state.ins1 = 0; state.del1 = 0; state.ins2 = 0; state.del2 = 0;
  state.lastEquality = null;
  state.changes = true;
}
function __cleanupSemantic(diffs) {
  const state = { changes: false, equalities: [], nEq: 0, lastEquality: null, pointer: 0, ins1: 0, del1: 0, ins2: 0, del2: 0 };
  while (state.pointer < diffs.length) { __cleanupSemStep(state, diffs); state.pointer++; }
  if (state.changes) __cleanupMerge(diffs);
  __cleanupSemanticLossless(diffs);
  __cleanupOverlaps(diffs);
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
const __DIFF_CTOR_TYPE = new Map([[RegExp, 'regexp'], [Map, 'map'], [Set, 'set'], [Date, 'date']]);
function __diffType(v) {
  if (v === undefined) return 'undefined';
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'array';
  if (typeof v !== 'object') return typeof v;
  return __DIFF_CTOR_TYPE.get(v.constructor) || 'object';
}
const __NO_DIFF = 'Compared values have no visual difference.';
// @vitest/utils `diff(expected, received)`; `annotate` false is its `omitAnnotationLines: true`.
// __vdiff's diff type of `a`: [type, omit the difference]; null when there is no diff (a
// user-defined asymmetric matcher, or one without getExpectedType like expect.anything)
function __vdiffType(a) {
  const type = __diffType(a);
  // an asymmetric matcher diffs as its expected type (vitest)
  if (type === 'object' && typeof a.asymmetricMatch === 'function') {
    if (a.$$typeof !== Symbol.for('jest.asymmetricMatcher')) return null;
    if (typeof a.getExpectedType !== 'function') return null;
    const expected = a.getExpectedType();
    return [expected, expected === 'string'];
  }
  return [type, false];
}
// __vdiff of objects (maps and sets sorted): by their pretty-format, falling back to the
// structure without toJSON when they serialize the same
function __vdiffSort(type, a, b) {
  if (type === 'map') return [new Map([...a.entries()].sort()), new Map([...b.entries()].sort())];
  if (type === 'set') return [new Set([...a.values()].sort()), new Set([...b.values()].sort())];
  return [a, b];
}
function __vdiffObjects(a, b, type, fmt, fallback, annotate) {
  const noDiff = __dcol.dim(__NO_DIFF);
  const objects = (opts) => {
    const ac = __pf(a, { ...opts, indent: 0 });
    const bc = __pf(b, { ...opts, indent: 0 });
    if (ac === bc) return noDiff;
    return __diffLines(__pf(a, opts).split('\n'), __pf(b, opts).split('\n'), ac.split('\n'), bc.split('\n'), annotate);
  };
  [a, b] = __vdiffSort(type, a, b);
  let d;
  let threw = false;
  try { d = objects(fmt); } catch (e) { threw = true; }
  if (d === undefined || d === noDiff) {
    d = objects(fallback);
    if (d !== noDiff && !threw) d = `${__dcol.dim('Compared values serialize to the same structure.\nPrinting internal object structure without calling `toJSON` instead.')}\n\n${d}`;
  }
  return d;
}
function __vdiffMismatch(a, b, fallback) {
  return `${__dcol.green(`- ${__diffAnno[0]}:`)}\n${__pf(a, fallback)}\n\n${__dcol.red(`+ ${__diffAnno[1]}:`)}\n${__pf(b, fallback)}`;
}
function __vdiffScalar(a, b, type, fmt, annotate) {
  if (type === 'string') return __diffLines(a.split('\n'), b.split('\n'), undefined, undefined, annotate);
  if (type === 'boolean' || type === 'number') {
    const af = __pf(a, fmt), bf = __pf(b, fmt);
    return af === bf ? '' : __diffLines(af.split('\n'), bf.split('\n'), undefined, undefined, annotate);
  }
  return undefined;
}
function __vdiff(a, b, annotate = false) {
  if (Object.is(a, b)) return '';
  const typed = __vdiffType(a);
  if (typed === null) return undefined;
  const [type, omitDifference] = typed;
  const fmt = { indent: 2, basic: false, maxDepth: 20, errors: true };
  const fallback = { ...fmt, maxDepth: 8, toJSON: false };
  if (type !== __diffType(b)) return __vdiffMismatch(a, b, fallback);
  if (omitDifference) return undefined;
  const scalar = __vdiffScalar(a, b, type, fmt, annotate);
  if (scalar !== undefined) return scalar;
  return __vdiffObjects(a, b, type, fmt, fallback, annotate);
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
      return trimObject(seen, object, subset);
    }
    return object;
  };
  const setTrimmedCtor = (trimmed, object) => {
    if (typeof object.constructor === 'function' && typeof object.constructor.name === 'string') {
      Object.defineProperty(trimmed, 'constructor', { enumerable: false, value: object.constructor });
    }
  };
  const trimKey = (seen, trimmed, object, subset, key) => {
    if (inObj(subset, key)) {
      trimmed[key] = seen.has(object[key]) ? seen.get(object[key]) : withContext(seen)(object[key], subset[key]);
      return;
    }
    if (seen.has(object[key])) return;
    stripped += 1;
    if (isObj(object[key])) stripped += keys(object[key]).length;
    withContext(seen)(object[key], subset[key]);
  };
  // an object cut down to the subset's keys (the subset itself when it matches)
  const trimObject = (seen, object, subset) => {
    if (__subsetMatches(object, subset)) return subset;
    const trimmed = {};
    seen.set(object, trimmed);
    setTrimmedCtor(trimmed, object);
    for (const key of keys(object)) trimKey(seen, trimmed, object, subset, key);
    return keys(trimmed).length > 0 ? trimmed : object;
  };
  return { subset: withContext()(object, subset), stripped };
}
// vitest's toMatchObject comparison: equals with [iterableEquality, subsetEquality].
function __subsetMatches(a, e) {
  return __equals(a, e, [...__customTesters, __iterableEquality, __subsetEquality]);
}

// @vitest/utils format's extra argument
const __formatExtra = (v) => (v === null || typeof v !== 'object' ? (typeof v === 'symbol' ? v.toString() : v) : __inspect(v));
// @vitest/utils format's %s
function __formatOneSObject(v) {
  return (typeof v.toString === 'function' && v.toString !== Object.prototype.toString) ? v.toString() : __inspect(v);
}
function __formatOneS(v) {
  if (typeof v === 'bigint') return `${v}n`;
  if (typeof v === 'number' && v === 0 && 1 / v < 0) return '-0';
  if (typeof v === 'object' && v !== null) return __formatOneSObject(v);
  return String(v);
}
// @vitest/utils format's %j: a circular value is `[Circular]`
function __formatOneJ(v) {
  try { return JSON.stringify(v); } catch (e) {
    const m = String(e && e.message);
    if (/circular structure|cyclic structures|cyclic object/.test(m)) return '[Circular]';
    throw e;
  }
}
// @vitest/utils format %d: BigInt -> "123n", Symbol -> "NaN", else Number(v).
function __formatOneD(v) {
  if (typeof v === 'bigint') return `${v}n`;
  if (typeof v === 'symbol') return 'NaN';
  return Number(v).toString();
}
// @vitest/utils format %i: BigInt -> "123n", else parseInt(String(v)).
function __formatOneI(v) { return typeof v === 'bigint' ? `${v}n` : Number.parseInt(String(v)).toString(); }
// @vitest/utils `format(spec, item)` for one placeholder.
const __FORMAT_ONE = {
  '%%': (v) => `% ${__formatExtra(v)}`,
  '%s': __formatOneS,
  '%d': __formatOneD,
  '%i': __formatOneI,
  '%f': (v) => Number.parseFloat(String(v)).toString(),
  '%o': (v) => __inspect(v),
  '%O': (v) => __inspect(v),
  '%c': () => '',
  '%j': __formatOneJ,
};
function __formatOne(spec, item) {
  const fn = __FORMAT_ONE[spec];
  return fn ? fn(item) : undefined;
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
// vitest's %# / %$ substitutions: round-trip %% through a sentinel so a literal %% is preserved.
function __formatTitleIndices(template, idx) {
  return template.replace(/%%/g, '__tt_escaped_%__').replace(/%#/g, `${idx}`).replace(/%\$/g, `${idx + 1}`).replace(/__tt_escaped_%__/g, '%%');
}
// For each %f whose corresponding item is -0 or NaN, swap in `-%f` so util.format keeps the sign.
function __formatTitleHandleF(template, items) {
  (template.match(/%f/g) || []).forEach((_, i) => {
    if (__negZeroOrNaN(items[i])) {
      let n = 0;
      template = template.replace(/%f/g, (m) => (++n === i + 1 ? '-%f' : m));
    }
  });
  return template;
}
// Weave __formatOne specifiers (`%s`, `%d`, …) with `attrs` substitutions around them.
function __formatTitleJoin(template, items, attrs, count) {
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
function __formatTitleAttrs(items, isObjectItem) {
  return (str) => str.replace(/\$([$\w.]+)/g, (_, key) => {
    const isArrayKey = /^\d+$/.test(key);
    if (!isObjectItem && !isArrayKey) return `$${key}`;
    const el = isArrayKey ? __objectAttr(items, key) : undefined;
    return __objDisplay(isObjectItem ? __objectAttr(items[0], key, el) : el);
  });
}
function __formatTitle(template, items, idx) {
  if (template.includes('%#') || template.includes('%$')) template = __formatTitleIndices(template, idx);
  const count = template.split('%').length - 1;
  if (template.includes('%f')) template = __formatTitleHandleF(template, items);
  const isObjectItem = items[0] != null && typeof items[0] === 'object' && !Array.isArray(items[0]);
  return __formatTitleJoin(template, items, __formatTitleAttrs(items, isObjectItem), count);
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
// The prepass runs the factories inside `with (__ttScopeOf(file))`: the file's top-level names the
// factories mention (__ttScopeNames) resolve through getters / setters the file itself registers
// when it runs (__ttScopeBind); read before that, a name is in its temporal dead zone.
// One scope per file (the test file, each setup file: turbo-test#205), keyed by its path.
const __tdz = (k) => new ReferenceError(`Cannot access '${k}' before initialization`);
const __scopes = new Map();
const __mkScope = () => {
  const sc = { names: new Set(), get: new Map(), set: new Map() };
  sc.proxy = new Proxy(Object.create(null), {
    has: (_, k) => sc.names.has(k),
    get: (_, k) => {
      if (typeof k !== 'string') return undefined; // `with` asks for Symbol.unscopables
      const g = sc.get.get(k);
      if (!g) throw __tdz(k);
      return g();
    },
    set: (_, k, v) => {
      const set = sc.set.get(k);
      if (set) { set(v); return true; }
      throw sc.get.has(k) ? new TypeError('Assignment to constant variable.') : __tdz(k);
    },
  });
  return sc;
};
const __scopeFor = (file) => {
  let sc = __scopes.get(file);
  if (!sc) { sc = __mkScope(); __scopes.set(file, sc); }
  return sc;
};
globalThis.__ttScopeNames = (file, names) => { const sc = __scopeFor(file); sc.names = new Set(names); sc.get.clear(); sc.set.clear(); };
globalThis.__ttScopeOf = (file) => __scopeFor(file).proxy;
globalThis.__ttScopeBind = (file, get, set) => {
  const sc = __scopeFor(file);
  for (const k of Object.keys(get)) sc.get.set(k, get[k]);
  for (const k of Object.keys(set)) sc.set.set(k, set[k]);
};

// ---- collector: describe / it / hooks ----
// A suite's `tasks` hold its tests and child suites (`suite: true`) in declaration order: vitest
// runs and reports them in that order (turbo-test#64).
const root = { name: '', tasks: [], suite: true, hooks: { be: [], ae: [], ba: [], aa: [] }, parent: null };
let current = root;

// A describe's options object (`describe(name, { repeats }, fn)`) is merged over its enclosing
// suite's, and its tests inherit them (vitest's createSuite / createTest).
const __suiteOptions = (a, b) => (a && typeof a === 'object' ? a : b && typeof b === 'object' ? b : {});
// `.concurrent` / `.sequential` (turbo-test#256): the modifier of the call being made (vitest's
// chainable context), read by describe / the test collector (not inside a describe's body)
let __chain = '';
// it.concurrent / describe.sequential …: `target` called with the modifier set; its members
// (`.each`, `.skip`, `.fails` …) and the functions they return carry it too
function __withChain(target, mode) {
  return new Proxy(target, {
    apply(f, self, args) {
      const prev = __chain;
      __chain = mode;
      try {
        const r = Reflect.apply(f, self, args);
        return typeof r === 'function' ? __withChain(r, mode) : r;
      } finally { __chain = prev; }
    },
    get(f, k) {
      const v = Reflect.get(f, k);
      return typeof v === 'function' && k !== 'concurrent' && k !== 'sequential' ? __withChain(v, mode) : v;
    },
  });
}
// the options a suite's tests and nested suites inherit: the enclosing suite's (the root's:
// `sequence.concurrent`, host → __TT_SEQUENCE_CONCURRENT), then its own, with vitest's
// createSuite precedence of concurrent / sequential
const __parentOpts = (s) => s.opts || (globalThis.__TT_SEQUENCE_CONCURRENT ? { concurrent: true } : {});
// Suite is concurrent per own/chain/sequential=false; dual for __suiteSelfSeq.
function __suiteSelfCon(own, chain) { return !!own.concurrent || chain === 'concurrent' || own.sequential === false; }
function __suiteSelfSeq(own, chain) { return !!own.sequential || chain === 'sequential' || own.concurrent === false; }
function __suiteOpts(parent, own, chain) {
  const cs = __suiteSelfCon(own, chain);
  const ss = __suiteSelfSeq(own, chain);
  const o = { ...__parentOpts(parent), ...own };
  const isC = !!(cs || (o.concurrent && !ss));
  const isS = !!(ss || (o.sequential && !cs));
  o.concurrent = isC && !isS;
  o.sequential = isS && !isC;
  return o;
}
function describe(name, a, b) {
  const chain = __chain;
  const fn = typeof a === 'function' ? a : (typeof b === 'function' ? b : () => {});
  const opts = __suiteOpts(current, __suiteOptions(a, b), chain);
  const suite = { name, tasks: [], suite: true, hooks: { be: [], ae: [], ba: [], aa: [] }, parent: current, opts, concurrent: opts.concurrent };
  current.tasks.push(suite);
  const prev = current;
  current = suite;
  __chain = '';
  try { fn(); } finally { current = prev; __chain = chain; }
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
describe.each = (cases, ...targs) => __each(describe, cases, targs);
describe.for = (cases, ...targs) => __for(describe, cases, targs, true);
// Supports both vitest signatures: it(name, fn, opts) and it(name, opts, fn).
function __normTestOptsForFn(b) {
  if (typeof b === 'object' && b) return b;
  if (typeof b === 'number') return { timeout: b };
  return {};
}
function __normTest(a, b) {
  if (typeof a === 'function') return { fn: a, o: __normTestOptsForFn(b) };
  if (typeof a === 'object' && a) return { fn: typeof b === 'function' ? b : () => {}, o: a };
  return { fn: typeof b === 'function' ? b : () => {}, o: {} };
}
// Default per-test timeout (ms): `--testTimeout` (host → globalThis.__TT_DEFAULT_TIMEOUT) else
// vitest's 5000ms default. A test's own `{ timeout }` overrides this in runSuite.
function __defaultTimeout() {
  // jest.setTimeout(ms) sets a per-file override (cleared in __ttResetForNextFile).
  if (typeof globalThis.__TT_JEST_TIMEOUT === 'number') return globalThis.__TT_JEST_TIMEOUT;
  // vitest's withTimeout treats `<= 0 || Infinity` as "no timeout" (turbo-test#273); honour an
  // explicit 0 or Infinity from __TT_DEFAULT_TIMEOUT and fall back to 5 s otherwise.
  const raw = globalThis.__TT_DEFAULT_TIMEOUT;
  if (raw !== undefined && raw !== null && raw !== '') {
    const n = Number(raw);
    if (!Number.isNaN(n)) return n;
  }
  return 5000;
}
// Collect a test into the current suite. Under `includeTaskLocation` it records where it was
// registered from (`[line, column]` in the test file, native `__ttCallSite`, #63).
let __inEach = false;
// whether a test runs concurrently (vitest's createTest): `.concurrent` / `.sequential`, else
// its own / its suite's `concurrent`, else not `sequential` under `sequence.concurrent`
function __testConcurrent(own, chain) {
  if (chain) return chain === 'concurrent';
  const o = { ...__parentOpts(current), ...own };
  return o.concurrent != null ? !!o.concurrent : !o.sequential && !!globalThis.__TT_SEQUENCE_CONCURRENT;
}
let __testSeq = 0;
function __addTest(t) {
  t.seq = __testSeq++;
  if (globalThis.__TT_TASK_LOCATION) t.location = globalThis.__ttCallSite();
  // where it was registered: a timeout's stack (vitest's `STACK_TRACE_ERROR`, turbo-test#102)
  t.stackTrace = new Error('STACK_TRACE_ERROR');
  if (__inEach) t.each = true;
  // `repeats` from the enclosing describe's options (turbo-test#257)
  if (t.repeats === undefined && current.opts) t.repeats = current.opts.repeats;
  t.concurrent = __testConcurrent(t.own || {}, __chain);
  current.tasks.push(t);
}
function it(name, a, b) {
  const { fn, o } = __normTest(a, b);
  __addTest({ name, fn, skip: false, only: false, retry: o.retry || 0, repeats: o.repeats, own: o, timeout: o.timeout });
}
it.skip = (name, a, b) => __addTest({ name, fn: __normTest(a, b).fn, skip: true });
it.todo = (name) => __addTest({ name, fn: () => {}, skip: true, todo: true });
it.only = (name, a, b) => { __tt.hasOnly = true; const { fn, o } = __normTest(a, b); __addTest({ name, fn, skip: false, only: true, retry: o.retry || 0, repeats: o.repeats, own: o, timeout: o.timeout }); };
it.each = (cases, ...targs) => __each(it, cases, targs);
it.for = (cases, ...targs) => __for(it, cases, targs);
it.skipIf = (cond) => (name, fn) => __addTest({ name, fn, skip: !!cond, only: false, retry: 0 });
it.runIf = (cond) => (name, fn) => __addTest({ name, fn, skip: !cond, only: false, retry: 0 });
// it.fails: the test PASSES iff its body throws (runSuite inverts the outcome via `fails`).
it.fails = (name, a, b) => { const { fn, o } = __normTest(a, b); __addTest({ name, fn, skip: false, only: false, retry: o.retry || 0, repeats: o.repeats, own: o, fails: true }); };
// it.extend({...}) — test-context fixtures (best-effort). vitest passes a merged context object as
// the test fn's first arg; fixtures defined as plain values or `async ({}, use) => use(value)`
// functions are resolved here and provided. Returns a new test fn with these fixtures baked in.
// Per-suite fixture overrides from test.scoped / test.override (turbo-test#276), keyed by the
// extended `test` function. Walking up a test's suite chain picks the closest ancestor's map.
const __fixtureOverrides = new WeakMap();
function __closestFixtureOverride(suite, key) {
  let s = suite;
  while (s) {
    const m = __fixtureOverrides.get(s);
    if (m && m.has(key)) return m.get(key);
    s = s.parent;
  }
  return null;
}
async function __applyFixture(ctx, k, f) {
  if (typeof f !== 'function') { ctx[k] = f; return; }
  // vitest fixture fn: ({...deps}, use) => { ... await use(value) }. Capture the used value.
  let captured;
  const use = (v) => { captured = v; return Promise.resolve(); };
  const r = f(ctx, use);
  if (r && typeof r.then === 'function') await r;
  ctx[k] = captured;
}
it.extend = (fixtures) => {
  const base = { ...(fixtures || {}) };
  // the fixtures are set on the test's context (turbo-test#96), which the fixture functions get.
  // suite is the describe block the test was registered in (null for root); scoped overrides
  // registered on that suite or any ancestor take precedence (turbo-test#276).
  const resolveCtx = async (ctx, suite) => {
    const fx = __closestFixtureOverride(suite, ext) || base;
    for (const k of Object.keys(fx)) await __applyFixture(ctx, k, fx[k]);
    return ctx;
  };
  const wrap = (userFn, suite) => async (ctx) => userFn(await resolveCtx(ctx, suite));
  const ext = (name, a, b) => { const { fn, o } = __normTest(a, b); __addTest({ name, fn: wrap(fn, current), skip: false, only: false, retry: o.retry || 0, repeats: o.repeats, own: o }); };
  ext.skip = (name, a, b) => __addTest({ name, fn: wrap(__normTest(a, b).fn, current), skip: true });
  ext.only = (name, a, b) => { __tt.hasOnly = true; const { fn, o } = __normTest(a, b); __addTest({ name, fn: wrap(fn, current), skip: false, only: true, retry: o.retry || 0, repeats: o.repeats, own: o }); };
  ext.each = (cases, ...targs) => __each(ext, cases, targs);
  ext.for = (cases, ...targs) => __for(ext, cases, targs);
  ext.extend = (more) => it.extend({ ...base, ...more });
  // test.scoped({...}) / test.override({...}) — override fixture values for the enclosing suite
  // (turbo-test#276). Merged onto the closest ancestor override (or the base) and stored on the
  // current suite; nested describes without their own scoped call inherit it via resolveCtx's walk.
  // `scoped` is vitest's deprecated name for `override` — both set the same overrides.
  const applyScoped = (overrides) => {
    // Multiple scoped calls in the same describe layer onto each other, not onto the parent:
    // the baseline is the current suite's own override if one is already set here, else the
    // closest ancestor's override, else the base extend.
    let m = __fixtureOverrides.get(current);
    const own = m && m.get(ext);
    const parentFx = own || __closestFixtureOverride(current.parent, ext) || base;
    const merged = { ...parentFx, ...(overrides || {}) };
    if (!m) { m = new Map(); __fixtureOverrides.set(current, m); }
    m.set(ext, merged);
    return ext;
  };
  ext.scoped = applyScoped;
  ext.override = applyScoped;
  return ext;
};
for (const api of [describe, it]) {
  api.concurrent = __withChain(api, 'concurrent');
  api.sequential = __withChain(api, 'sequential');
}
globalThis.describe = describe;
globalThis.it = it;
globalThis.test = it;
// beforeEach / afterEach hooks get the test's context (turbo-test#96)
// Under isolate reuse a hook the test file itself (or a project module it imports) registers on
// the root suite belongs to that file, as vitest scopes it to the file's suite: it is recorded
// here and dropped before the next file (turbo-test#214). Hooks from the setup files (registered
// before the baseline is captured) and from node_modules (bundled, so their frames carry no file
// name) stay on the root, as before.
let __ttFileRootHooks = [];
const __ttIsProjectCaller = () => {
  const frames = String(new Error().stack).split('\n').slice(1);
  const named = frames.find((l) => l.includes('/') && !l.includes('<anonymous>'));
  return !!named && !named.includes('/node_modules/');
};
// A hook, an onTestFinished / onTestFailed handler: its function, its own timeout (vitest's
// last argument: turbo-test#255) and where it was registered (its timeout error's stack)
const __ttHook = (fn, timeout) => ({ fn, timeout, site: new Error('STACK_TRACE_ERROR') });
const __ttAddHook = (kind, fn, timeout) => {
  const h = __ttHook(fn, timeout);
  current.hooks[kind].push(h);
  if (current === root && globalThis.__ttHookBaseline && __ttIsProjectCaller()) {
    __ttFileRootHooks.push([kind, h]);
  }
};
globalThis.beforeEach = (fn, timeout) => __ttAddHook('be', fn, timeout);
globalThis.afterEach = (fn, timeout) => __ttAddHook('ae', fn, timeout);
globalThis.beforeAll = (fn, timeout) => __ttAddHook('ba', fn, timeout);
globalThis.afterAll = (fn, timeout) => __ttAddHook('aa', fn, timeout);
// onTestFinished / onTestFailed imported from 'vitest' (turbo-test#258): vitest's
// createTestHook — the callback is type-checked, then registered on the test running now (its
// body, a beforeEach or an afterEach), the same list as the context's own
const __ttTestHook = (name, key) => (fn, timeout) => {
  if (typeof fn !== 'function') throw new TypeError(`"${name}" callback value must be function, received "${typeof fn}"`);
  const t = __snap.currentTest;
  if (!t) throw new Error(`Hook ${name}() can only be called inside a test`);
  t[key].push(__ttHook(fn, timeout));
};
globalThis.onTestFinished = __ttTestHook('onTestFinished', 'onFinished');
globalThis.onTestFailed = __ttTestHook('onTestFailed', 'onFailed');

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
// an error whose actual / expected are diffed (chai's showDiff, else both present)
const __errorDiffable = (e) => e.showDiff || (e.showDiff === undefined && e.expected !== undefined && e.actual !== undefined);
function __errorDiff(e) {
  try { return __printDiffOrStringify(e.actual, e.expected); } catch (err) { return ''; }
}
const __errorHasCause = (e) => e.cause && typeof e.cause === 'object' && 'name' in e.cause;
// vite's details of a module's parse / transform error (printErrorInner's
// `__vitest_rollup_error__` lines, turbo-test#113)
function __errorRollup(re) {
  if (!re || typeof re !== 'object') return null;
  return { plugin: re.plugin ? String(re.plugin) : '', id: re.id ? String(re.id) : '', loc: re.loc ? `${re.loc.line}:${re.loc.column}` : '', frame: re.frame ? String(re.frame) : '' };
}
// the serialized error's name and message (and whether it has none)
function __errorNames(ser) {
  return {
    name: ser.name ? String(ser.name) : 'Unknown Error',
    message: ser.message === undefined ? '' : String(ser.message),
    no_name: ser.name === undefined,
    no_message: ser.message === undefined,
  };
}
// __errorInfo's diff block (diff string + actual / expected / show_diff flags for vitest's TAP).
function __errorInfoDiff(e, obj, diffable) {
  return {
    diff: (diffable ? __errorDiff(e) : '') || '',
    show_diff: !!(obj && e.showDiff),
    actual: diffable ? __prettifyValue(e.actual) : null,
    expected: diffable ? __prettifyValue(e.expected) : null,
  };
}
// __errorInfo's inspected-form, when the error has no message and is not an Error instance.
function __errorInfoInspected(e, ser) {
  if (ser.message) return null;
  if (e instanceof Error) return null;
  return __nodeInspect(__plainOf({ ...ser, stacks: [] }));
}
function __errorInfoCause(e, obj) {
  return obj && __errorHasCause(e) ? __errorInfo(e.cause) : null;
}
function __errorInfoStackField(ser) {
  return typeof ser.stack === 'string' ? ser.stack : '';
}
function __errorInfo(e) {
  const ser = __serializeError(e);
  const obj = e !== null && typeof e === 'object';
  const diffable = obj && __errorDiffable(e);
  const props = __errorProperties(ser);
  return {
    ...__errorNames(ser),
    ...__errorInfoDiff(e, obj, diffable),
    stack: __errorInfoStackField(ser),
    cause: __errorInfoCause(e, obj),
    inspected: __errorInfoInspected(e, ser),
    properties: props === null ? null : __inspect(props),
    rollup: __errorRollup(obj ? e.__vitest_rollup_error__ : undefined),
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
// an Error's toJSON(), completed with its message / stack / name / cause
function __serializeErrorJSON(val, seen) {
  const j = val.toJSON();
  if (j && j !== val && typeof j === 'object') __serializeErrorFill(j, val, seen);
  return __serializeValue(j, seen);
}
// Keys __serializeErrorFill tries to copy from `val` into the toJSON() result. Each entry
// reads through a getter that may throw; the picked value is copied only when it is a string
// (or, for `cause`, any non-nullish), and never overwrites one already in the toJSON output.
const __ERROR_FILL_STR = ['message', 'stack', 'name'];
function __fillStringKey(j, val, k) {
  try { if (typeof val[k] === 'string') j[k] ??= val[k]; } catch {}
}
function __fillCauseKey(j, val, seen) {
  try { if (val.cause != null) j.cause ??= __serializeValue(val.cause, seen); } catch {}
}
function __serializeErrorFill(j, val, seen) {
  for (const k of __ERROR_FILL_STR) __fillStringKey(j, val, k);
  __fillCauseKey(j, val, seen);
}
// __serializeValue's "not a leaf"
const __SER_NONE = Symbol('ser-none');
// Binary-like leaves: Node's Buffer and plain Uint8Array (contents elided).
function __serializeLeafBuf(val) {
  if (typeof Buffer !== 'undefined' && val instanceof Buffer) return `<Buffer(${val.length}) ...>`;
  if (val instanceof Uint8Array) return `<Uint8Array(${val.length}) ...>`;
  return __SER_NONE;
}
// the values serialized as themselves or a short text (functions, symbols, primitives, buffers,
// promises, elements)
function __serializeLeafObject(val) {
  const buf = __serializeLeafBuf(val);
  if (buf !== __SER_NONE) return buf;
  if (val instanceof Promise) return 'Promise';
  if (typeof Element !== 'undefined' && val instanceof Element) return val.tagName;
  return __SER_NONE;
}
function __serializeLeaf(val) {
  if (typeof val === 'function') return `Function<${val.name || 'anonymous'}>`;
  if (typeof val === 'symbol') return val.toString();
  if (typeof val !== 'object') return val;
  return __serializeLeafObject(val);
}
function __serializeArray(val, seen) {
  const clone = new Array(val.length);
  seen.set(val, clone);
  val.forEach((x, i) => { try { clone[i] = __serializeValue(x, seen); } catch (err) { clone[i] = __unserializable(err); } });
  return clone;
}
// an object's own and inherited properties (short of Object.prototype)
function __serializeObjectKey(clone, val, key, seen) {
  if (key in clone) return;
  try { clone[key] = __serializeValue(val[key], seen); } catch (err) { delete clone[key]; clone[key] = __unserializable(err); }
}
function __serializeObject(val, seen) {
  const clone = Object.create(null);
  seen.set(val, clone);
  for (let obj = val; obj && obj !== Object.prototype; obj = Object.getPrototypeOf(obj)) {
    for (const key of Object.getOwnPropertyNames(obj)) __serializeObjectKey(clone, val, key, seen);
  }
  if (val instanceof Error) { try { clone.message = val.message; } catch {} }
  return clone;
}
// An Error with a toJSON() method is serialized through it (so Node's full error payload stays).
function __serializeErrorIfJSON(val, seen) {
  if (val instanceof Error && 'toJSON' in val && typeof val.toJSON === 'function') return __serializeErrorJSON(val, seen);
  return __SER_NONE;
}
function __serializeShortcut(val, seen) {
  if (typeof val.toJSON === 'function') return __serializeValue(val.toJSON(), seen);
  if (seen.has(val)) return seen.get(val);
  return __SER_NONE;
}
function __serializeFast(val, seen) {
  const json = __serializeErrorIfJSON(val, seen);
  if (json !== __SER_NONE) return json;
  const leaf = __serializeLeaf(val);
  if (leaf !== __SER_NONE) return leaf;
  return __serializeShortcut(val, seen);
}
function __serializeValue(val, seen = new WeakMap()) {
  if (!val || typeof val === 'string') return val;
  const fast = __serializeFast(val, seen);
  if (fast !== __SER_NONE) return fast;
  if (Array.isArray(val)) return __serializeArray(val, seen);
  return __serializeObject(val, seen);
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
// __printDiffOrStringify's fast-path guard: both sides are non-empty strings short enough to
// char-diff. vitest's diffStringsRaw is quadratic, so the 20 000-char cap matches jest-diff's.
function __bothShortStrings(a, b) {
  return typeof a === 'string' && typeof b === 'string' && a.length > 0 && b.length > 0
    && a.length <= 20000 && b.length <= 20000;
}
// @vitest/utils printDiffOrStringify(received, expected)
function __printDiffOrStringify(received, expected) {
  if (__bothShortStrings(expected, received) && expected !== received) {
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
    seq: t.seq,
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
function __taskTimeout(t) {
  return typeof t.timeout === 'number' ? t.timeout : __defaultTimeout();
}
function __testTaskOf(t, chain) {
  if (t.task) return t.task;
  const file = __fileTaskOf();
  const parent = chain[chain.length - 1];
  const up = chain.length > 1 ? __suiteTaskOf(parent, chain.slice(0, -1)) : undefined;
  const names = chain.slice(1).map((x) => x.name).concat(t.name);
  t.task = {
    type: 'test', name: t.name, id: `${(up || file).id}_${parent.tasks.indexOf(t)}`, mode: 'run', file, suite: up,
    fullName: [file.name, ...names].join(' > '), fullTestName: names.join(' > '), meta: Object.create(null),
    each: t.each || undefined, fails: t.fails || undefined, retry: t.retry || undefined, repeats: t.repeats,
    timeout: __taskTimeout(t), shuffle: undefined,
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
  const __makeAnnotation = (message, type, attachment) => {
    const annotation = { message, type: typeof type === 'object' || type === undefined ? 'notice' : type };
    const att = typeof type === 'object' ? type : attachment;
    if (att) annotation.attachment = att;
    const site = globalThis.__ttCallSite();
    if (site) annotation.location = { file: globalThis.__ttFile, line: site[0], column: site[1] };
    return annotation;
  };
  context.annotate = (message, type, attachment) => {
    if (task.result && task.result.state !== 'run') {
      throw new Error(`Cannot annotate tests outside of the test run. The test "${task.name}" finished running with the "${task.result.state}" state already.`);
    }
    const annotation = __makeAnnotation(message, type, attachment);
    task.annotations.push(annotation);
    return Promise.resolve(annotation);
  };
  context.onTestFailed = (fn, timeout) => { t.onFailed.push(__ttHook(fn, timeout)); };
  context.onTestFinished = (fn, timeout) => { t.onFinished.push(__ttHook(fn, timeout)); };
  let ownExpect;
  Object.defineProperty(context, 'expect', { get: () => (ownExpect ||= new Proxy(globalThis.expect, {})) });
  Object.defineProperty(task, 'context', { value: context, configurable: true });
  t.abort = abort;
  t.context = context;
  __ctxAborts.set(context, abort);
  return context;
}
// onTestFinished (last registered first) / onTestFailed handlers: each gets the context and a
// throw fails the test; registering another inside one throws, as vitest's `callTestHooks`.
async function __callTestHooks(t, hooks, fail) {
  const ctx = t.context;
  const keep = [ctx.onTestFailed, ctx.onTestFinished];
  ctx.onTestFailed = () => { throw new Error('Cannot call "onTestFailed" inside a test hook.'); };
  ctx.onTestFinished = () => { throw new Error('Cannot call "onTestFinished" inside a test hook.'); };
  for (const h of hooks.slice().reverse()) {
    // an internal handler (expect.poll's) is a bare function, without a timeout
    const fn = typeof h === 'function' ? h : h.fn;
    const arm = typeof h === 'function' ? undefined : __hookArm(h, ctx);
    try {
      await __vtLimit(() => { __loop.ticks++; return __vtSettle(() => fn(ctx), arm); });
    } catch (e) { fail(e); }
  }
  [ctx.onTestFailed, ctx.onTestFinished] = keep;
}

// vitest 4.1.12's runner, call for call (turbo-test#251): where it awaits, and through which
// wrappers it calls each hook, test and handler, sets the microtask turns between them — a
// promise chain queued in one phase settles at the same point of the next ones as under vitest.
// Each `await null` stands for a runner step that settles at once (onBeforeRunTask,
// onBeforeTryTask, onAfterTryTask, onTaskFinished, onAfterRetryTask, onAfterRunTask,
// onBeforeRunSuite, onAfterRunSuite, the fixture cleanups).
const __vtNoop = () => {};
// onAfterRunTask: vitest's worker wraps the runner's (sync) one in an async function
async function __vtAfterRunTask() {
  await null;
}
// @vitest/runner's limitConcurrency (turbo-test#256): at most `concurrency` calls run at once,
// the others wait in order; a promise result settles through `.finally(release)`
function __limitConcurrency(concurrency) {
  let count = 0;
  let head;
  let tail;
  const finish = () => {
    count--;
    if (head) {
      head[0]();
      head = head[1];
      tail = head && tail;
    }
  };
  const acquire = () => {
    let released = false;
    const release = () => { if (!released) { released = true; finish(); } };
    if (count++ < concurrency) return release;
    return new Promise((resolve) => {
      const node = [() => resolve(release)];
      if (tail) tail = tail[1] = node;
      else head = tail = node;
    });
  };
  return (func) => {
    const run = (release) => {
      try {
        const r = func();
        if (r instanceof Promise) return r.finally(release);
        release();
        return Promise.resolve(r);
      } catch (e) {
        release();
        return Promise.reject(e);
      }
    };
    const release = acquire();
    return release instanceof Promise ? release.then(run) : run(release);
  };
}
// `maxConcurrency` (host → __TT_MAX_CONCURRENCY; vitest's default 5)
function __maxConcurrency() {
  const n = Number(globalThis.__TT_MAX_CONCURRENCY);
  return Number.isFinite(n) && n > 0 ? n : 5;
}
// vitest's limitMaxConcurrency (each hook, test function and handler) and limitTestConcurrency
// (each test of a suite), made on first use
let __maxLimiter;
let __testLimiter;
const __vtLimit = (func) => (__maxLimiter ||= __limitConcurrency(__maxConcurrency()))(func);
const __vtTestLimit = (func) => (__testLimiter ||= __limitConcurrency(__maxConcurrency()))(func);
// withTimeout / withCancel: a new promise, settled through a thenable result's `then`; `arm`
// (the timeout timer) gets its reject and returns what disarms it
function __vtSettle(fn, arm) {
  return new Promise((resolve, reject) => {
    const disarm = arm ? arm(reject) : __vtNoop;
    const ok = (v) => { disarm(); resolve(v); };
    const ko = (e) => { disarm(); reject(e); };
    try {
      const r = fn();
      if (typeof r === 'object' && r != null && typeof r.then === 'function') r.then(ok, ko);
      else ok(r);
    } catch (e) { ko(e); }
  });
}
// vitest's hookTimeout (turbo-test#255): the hook's own timeout, else `test.hookTimeout` /
// --hookTimeout (host → __TT_HOOK_TIMEOUT), else vitest's 10 s
function __hookTimeout(own) {
  if (typeof own === 'number') return own;
  // vitest's withTimeout treats `<= 0 || Infinity` as "no timeout" (turbo-test#268), so
  // honour an explicit 0 or Infinity from __TT_HOOK_TIMEOUT; only fall back to the 10 s
  // default when nothing was set (or the value does not parse as a number).
  const raw = globalThis.__TT_HOOK_TIMEOUT;
  if (raw !== undefined && raw !== null && raw !== '') {
    const n = Number(raw);
    if (!Number.isNaN(n)) return n;
  }
  return 10000;
}
// the contexts' AbortControllers: a beforeEach / afterEach / handler timing out aborts its test's
// signal with the error (vitest's abortIfTimeout)
const __ctxAborts = new WeakMap();
// withTimeout's timer for hook `h` (none for a timeout of 0 or Infinity, as vitest's): an
// internal timer, so a hook that never settles fails on the virtual clock with vitest's
// makeTimeoutError — its stack where the hook was registered
function __hookArm(h, ctx) {
  const ms = __hookTimeout(h.timeout);
  if (!(ms > 0) || ms === Infinity) return undefined;
  return (reject) => {
    const timer = __scheduleInternal(() => {
      const err = new Error(`Hook timed out in ${ms}ms.\nIf this is a long-running hook, pass a timeout value as the last argument or configure it globally with "hookTimeout".`);
      err.stack = String(h.site.stack).replace('STACK_TRACE_ERROR', err.message);
      const abort = ctx && __ctxAborts.get(ctx);
      if (abort) abort.abort(err);
      reject(err);
    }, ms);
    return () => __clearInternal(timer);
  };
}
// callAroundEachHooks / callAroundAllHooks with no aroundEach / aroundAll hook
async function __vtAround(inner) {
  await (async () => { await inner(); })();
}
// a hook (`be` / `ae` / `ba` / `aa`): withTimeout over withFixtures' async wrapper, through
// limitConcurrency; what it returns (a function: its cleanup) or undefined
async function __vtRunHook(h, args, name) {
  return __vtLimit(async () => {
    if (name === 'be' || name === 'ae') __loop.ticks++;
    // called as a plain function, as vitest calls it (its frame has no method name)
    const fn = h.fn;
    const r = await __vtSettle(async () => fn(...args), __hookArm(h, args[0]));
    // a cleanup runs under its hook's timeout (vitest's getBeforeHookCleanupCallback)
    return typeof r === 'function' ? { fn: r, hook: h, ctx: args[0] } : undefined;
  });
}
// callSuiteHook: a suite's hooks (afterEach / afterAll last registered first); beforeEach runs
// the enclosing suites' first, afterEach after. `chain[i]` is the suite. A throw stops them.
function __vtOrderedHooks(hooks, name) {
  return name === 'ae' || name === 'aa' ? hooks.slice().reverse() : hooks;
}
async function __vtSuiteHook(chain, i, name, args) {
  const callbacks = [];
  if (name === 'be' && i > 0) callbacks.push(...(await __vtSuiteHook(chain, i - 1, name, args)));
  for (const h of __vtOrderedHooks(chain[i].hooks[name], name)) callbacks.push(await __vtRunHook(h, args, name));
  if (name === 'ae' && i > 0) callbacks.push(...(await __vtSuiteHook(chain, i - 1, name, args)));
  return callbacks;
}
// callCleanupHooks: the functions beforeEach / beforeAll hooks returned, last first
async function __vtCleanups(cleanups, bump) {
  for (const c of cleanups.slice().reverse()) {
    if (!c) continue;
    await __vtLimit(() => { if (bump) __loop.ticks++; return __vtSettle(c.fn, __hookArm(c.hook, c.ctx)); });
  }
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

// runSuite: a describe without tasks (failing, or a describe.todo / .skip one) is a TAP task of
// its own (turbo-test#170): where it sits among the tests, its mode and its describe path; one
// that is not skipped fails with "No test found in suite <name>" (returns true)
function __emptySuite(suite, ancestors, summary, skipAll, record) {
  if (!ancestors.length || suite.tasks.length) return false;
  summary.empty_suites.push([String(summary.tests.length), skipAll || 'run', ...ancestors.slice(1).map((x) => x.name), suite.name].join('\u001f'));
  if (skipAll) return false;
  summary.failures.push(`No test found in suite ${suite.name}`);
  record('fail');
  return true;
}
// a suite's hook error: its failure line and its error
function __suiteHookError(suite, summary, suiteErrors, hook, e) {
  summary.failures.push(`${suite.name || '<root>'} ${hook}: ${(e && e.message) || String(e)}`);
  suiteErrors.push(__errorInfo(e));
}
// A test that does not run is reported `skipped` (`todo` for it.todo / inside describe.todo),
// as vitest's JSON reporter does — -t-filtered ones included (#63).
// Precedence as vitest's `interpretTaskModes` + runner: a failed beforeAll skips everything;
// a describe.todo block makes its tests todo and a describe.skip block skips them (an
// it.todo stays todo); otherwise -t, then the test's own todo / skip / `.only`.
// Returns whether the test is not run.
// True when `t` is not run at all for one of the vitest reasons (todo / skip / filtered / etc.).
function __skipTestWhy(t, todo, skipAll, hookFailed, filtered) {
  return todo || hookFailed || skipAll || filtered || t.skip || t.notOnly;
}
function __computeTodo(t, hookFailed, skipAll, filtered) {
  return !hookFailed && (skipAll === 'todo' || (!filtered && !!t.todo));
}
function __skipTest(t, chain, summary, label, skipAll, hookFailed, filtered) {
  const todo = __computeTodo(t, hookFailed, skipAll, filtered);
  if (!__skipTestWhy(t, todo, skipAll, hookFailed, filtered)) return false;
  summary.skipped++;
  // a forbidden `.only` test that does not run keeps its error (vitest)
  const extra = t.forbiddenOnly ? { failure_messages: [__onlyForbiddenError()] } : undefined;
  summary.tests.push(__testRecord(t, chain, todo ? 'todo' : 'skipped', extra));
  // vitest marks a skipped test's snapshots checked (not obsolete)
  if (!todo) summary.skippedNames.push(label);
  return true;
}
// retry: per-test `{ retry }` wins; else the global default (--retry → __TT_DEFAULT_RETRY).
function __testRetry(t) {
  if (t.retry != null && t.retry > 0) return t.retry;
  // jest.retryTimes(n) sets a per-file override; cleared in __ttResetForNextFile.
  if (typeof globalThis.__TT_JEST_RETRY === 'number' && globalThis.__TT_JEST_RETRY > 0) return globalThis.__TT_JEST_RETRY;
  return Number(globalThis.__TT_DEFAULT_RETRY) > 0 ? Number(globalThis.__TT_DEFAULT_RETRY) : 0;
}
// Reset per-test snapshot + assertion-count state (snapshot counter is keyed by test name,
// so distinct multi-snapshot tests don't collide; assertion enforcement resets each attempt).
// The label is also stashed on `t` so the body wrapper can re-install it when the microtask
// that runs `fn(ctx)` actually runs — otherwise a concurrent sibling's `__beginAttempt` (the
// `await null` inside __runAttemptInner yields between them) would leave the global holding
// its name when this test's body throws (turbo-test#277).
function __resetTestSnap(t, label) {
  t.snapLabel = label;
  __snap.testName = label;
  __snap.currentTest = t;
  for (const k of Object.keys(__snap.counters)) if (k === label || k.startsWith(`${label} > `)) __snap.counters[k] = 0;
  __snap.assertCount = 0;
  __snap.expectedAssertions = null;
  __snap.hasAssertions = false;
}
// The test function, as vitest wraps it (withTimeout(withCancel(withAwaitAsyncAssertions(
// withFixtures(fn))))) and calls it through limitConcurrency. The timeout is an INTERNAL one-shot
// timer (separate from the user/fake-timer queue), so the Rust drive loop advances the virtual
// clock to it and rejects even a test that never resolves (e.g. `await new Promise(() => {})`) —
// otherwise that hangs the whole worker — while staying invisible to vi.runAllTimers etc.
function __runTestBody(t, ctx, timeoutMs) {
  // vitest's withTimeout: no timeout when timeoutMs <= 0 or Infinity (turbo-test#273).
  const arm = (!(timeoutMs > 0) || timeoutMs === Infinity) ? undefined : (reject) => {
    const timer = __scheduleInternal(() => {
      // vitest's message; its stack is the test's registration (`it(`), as vitest's
      const err = new Error(`Test timed out in ${timeoutMs}ms.\nIf this is a long-running test, pass a timeout value as the last argument or configure it globally with "testTimeout".`);
      // up to its frame in the test file (vitest collects a describe's tests after the
      // describe( call returned: no enclosing describe( frames)
      const frames = String(t.stackTrace.stack).split('\n');
      const own = frames.findIndex((l, i) => i > 0 && !!globalThis.__ttFile && l.includes(globalThis.__ttFile));
      err.stack = (own < 0 ? frames : frames.slice(0, own + 1)).join('\n');
      // the context's signal aborts with the timeout error (vitest's `withTimeout`)
      t.abort.abort(err);
      reject(err);
    }, timeoutMs);
    return () => __clearInternal(timer);
  };
  // called as a plain function, as vitest calls it (its stack frame has no method name)
  const fn = t.fn;
  const body = async () => {
    __loop.ticks++;
    // Concurrent siblings share the global `__snap` and __beginAttempt's write races with
    // __runAttemptInner's awaits; re-install this test's own snap state synchronously so
    // `expect(...).toMatchInlineSnapshot(...)` inside fn(ctx) reads its own name
    // (turbo-test#277). An async body that awaits inside still races and should use the
    // context's `expect` for snapshots.
    __snap.testName = t.snapLabel;
    __snap.currentTest = t;
    return fn(ctx);
  };
  return __vtLimit(() => __vtSettle(() => __vtSettle(async () => await body()), arm));
}
// expect.assertions(n) / hasAssertions() enforcement (vitest: checked after the test body).
function __checkAssertions() {
  if (__snap.expectedAssertions != null && __snap.assertCount !== __snap.expectedAssertions) {
    throw new Error(`expected number of assertions to be ${__snap.expectedAssertions}, but got ${__snap.assertCount}`);
  }
  if (__snap.hasAssertions && __snap.assertCount === 0) throw __snap.hasAssertions;
}
// a thrown attempt error: ctx.skip() marks the test pending, with its note (a caught one skips
// it, without a note)
function __attemptError(task, run, e) {
  if (e instanceof __PendingError) {
    task.result.pending = true;
    task.result.note = e.note;
    task.result.state = 'skip';
  } else run.lastErr = e;
}
function __attemptState(task, run) {
  // a failed attempt fails the test even after a passed repeat; a passed one keeps a failed
  // repeat's state (vitest's failTask / `state !== 'fail'`)
  if (task.result.state === 'run' || (!run.ok && !task.result.pending)) task.result.state = run.ok ? 'pass' : 'fail';
  if (!run.ok && !task.result.pending) {
    run.attemptErrors.push(run.lastErr);
    task.result.errors = run.attemptErrors;
  }
}
// One attempt of a test: its beforeEach hooks (the enclosing suites' first) and body, then its
// afterEach hooks (even when it failed, so a failed test's mounted DOM does not leak into the
// next one: innermost suite first, each suite's last registered first), the functions its
// beforeEach hooks returned (last first; a throwing beforeEach drops them all: turbo-test#101),
// its onTestFinished handlers, and onTestFailed ones when it failed (vitest's order). A throwing
// afterEach fails the test next to its own error and stops the hooks after it.
async function __runAttemptInner(t, chain, ctx, timeoutMs, run) {
  const task = t.task;
  let beCleanups = [];
  try {
    __clearModuleMocks();
    await null;
    beCleanups = await __vtSuiteHook(chain, chain.length - 1, 'be', [ctx]);
    await __runTestBody(t, ctx, timeoutMs);
    __checkAssertions();
    await null;
    run.ok = true;
  } catch (e) { __attemptError(task, run, e); }
  __attemptState(task, run);
  await null;
  try {
    await __vtSuiteHook(chain, chain.length - 1, 'ae', [ctx]);
    if (beCleanups.length) await __vtCleanups(beCleanups, true);
    await null;
  } catch (e) { run.failWith(e); }
  if (t.onFinished.length) await __callTestHooks(t, t.onFinished, run.failWith);
  if (task.result.state === 'fail' && t.onFailed.length) await __callTestHooks(t, t.onFailed, run.failWith);
  await null;
}
// it.fails: invert the outcome once its hooks ran (vitest flips the final state) — an
// error (past this attempt's `mark`) is the success condition, a clean pass fails.
function __invertFails(run, mark) {
  if (run.attemptErrors.length > mark) { run.ok = true; run.lastErr = undefined; run.attemptErrors.length = mark; }
  else { run.ok = false; run.lastErr = new Error('expected test to fail, but it passed'); run.attemptErrors.push(run.lastErr); }
}
// an attempt's start: its state, handlers and snapshot counters; returns where its errors start
function __beginAttempt(t, label, run, repeat) {
  const task = t.task;
  task.result.retryCount = run.retries;
  task.result.repeatCount = repeat;
  run.ok = false;
  t.onFinished = [];
  t.onFailed = [];
  run.lastErr = undefined;
  __resetTestSnap(t, label);
  return run.attemptErrors.length;
}
// __failMessage: when an error's own `.message` is empty or a bare class name, fall back to
// the first three stack frames joined with ` | `.
function __failMessageFallback(msg, lastErr) {
  const bare = !msg || msg === 'AggregateError' || msg === 'Error';
  if (bare && lastErr.stack) return String(lastErr.stack).split('\n').slice(0, 3).join(' | ');
  return msg;
}
// __failMessage's optional stack appendix under __TT_STACK: the first inner error's first five
// frames, each prefixed with the summary marker.
function __failMessageStack(lastErr) {
  const e0 = lastErr.errors && lastErr.errors[0] ? lastErr.errors[0] : lastErr;
  if (e0 && e0.stack) return '\n          @ ' + String(e0.stack).split('\n').slice(0, 5).join('\n          @ ');
  return '';
}
// a failed test's summary message: the error's (with an AggregateError's errors), its stack's
// first lines when it says nothing more, and under __TT_STACK its stack
function __failMessage(lastErr) {
  if (!lastErr) return String(lastErr);
  let msg = lastErr.message || String(lastErr);
  if (lastErr.errors && lastErr.errors.length) {
    msg += ' [' + lastErr.errors.map((x) => (x && x.message) || String(x)).join('; ') + ']';
  }
  msg = __failMessageFallback(msg, lastErr);
  if (globalThis.__TT_STACK) msg += __failMessageStack(lastErr);
  return msg;
}
// a run test's record; returns whether it failed
function __recordTest(t, chain, summary, label, run, timing, dur) {
  const task = t.task;
  // skipped at run time (ctx.skip): its duration and note (turbo-test#96); its snapshots
  // count as checked, as a skipped test's
  if (task.result.pending) {
    task.mode = 'skip';
    task.result = { state: 'skip', note: task.result.note, pending: true, duration: dur };
    summary.skipped++;
    summary.skippedNames.push(label);
    summary.tests.push(__testRecord(t, chain, 'skipped', { ...timing, note: task.result.note === undefined ? '' : String(task.result.note) }));
    return false;
  }
  task.result.duration = dur;
  // a passing it.fails is vitest's `expected fail` in the summary footer (#78)
  if (run.ok) {
    summary.passed++;
    summary.tests.push(__testRecord(t, chain, 'passed', t.fails ? { ...timing, expected_fail: true } : timing));
    return false;
  }
  summary.failed++;
  const msg = __failMessage(run.lastErr);
  summary.failures.push(`${label}: ${msg}`);
  summary.tests.push(__testRecord(t, chain, 'failed', { ...timing, message: msg, failure_messages: run.attemptErrors.map(__failureMessage), errors: run.attemptErrors.map(__errorInfo) }));
  return true;
}
// An attempt's end: it.fails inverted; whether the repeat stands failed — a repeat after a
// failed one starts failed and only a retry clears that (vitest keeps `state: 'fail'` until it
// sets `run` for the retry). A retry (`retry`) clears it and counts.
function __endAttempt(t, run, mark, failed) {
  if (t.fails && !t.task.result.pending) __invertFails(run, mark);
  return !t.task.result.pending && (failed || !run.ok);
}
function __retryAttempt(task, run) {
  task.result.state = 'run';
  run.retries++;
  return false;
}
// Pre-await bookkeeping for __runTest: sets up `task.result`, the retry state and the global
// log context. Kept sync so __runTest's first suspension is the same `await null` as before.
function __runTestSetup(t, chain) {
  // timeout (ms): per-test `{ timeout }` wins; else the --testTimeout/5000 default.
  const timeoutMs = typeof t.timeout === 'number' ? t.timeout : __defaultTimeout();
  // vitest keeps every failed attempt's error and counts the retries (`(retry x1)`)
  const run = { ok: false, lastErr: undefined, attemptErrors: [], retries: 0, failWith: null };
  // Real wall time for the per-test start, and for the duration the reporters show plus the
  // drive loop's clock jumps (turbo-test#95). NOT performance.now() (fake timers replace it) nor Date.now() (fake timers /
  // vi.setSystemTime replace it).
  const t0 = __loop.elapsed();
  const start = __loop.realNow();
  // the array that will capture this test's console output (concurrent tests: what is logged
  // while it is the last started). NOT installed as __ttTestLogs here — __runTest does that
  // immediately before its try block so the finally's __exitTestLogs is paired with it.
  const logs = [];
  const logTask = chain.slice(1).map((s) => s.name).concat(t.name).join(' > ');
  // its context (turbo-test#96): `task.result` while it runs, as vitest's
  const ctx = __contextOf(t, chain);
  const task = t.task;
  task.result = { state: 'run', startTime: start, retryCount: 0, repeatCount: 0 };
  // a failing onTestFinished / onTestFailed handler fails the test (not a skipped one)
  run.failWith = (e) => {
    if (task.result.pending) return;
    run.ok = false;
    run.lastErr = e;
    run.attemptErrors.push(e);
    task.result.state = 'fail';
    task.result.errors = run.attemptErrors;
  };
  // `{ repeats: n }` (turbo-test#257): 1 + n repeats, each retried up to `retry` times
  const repeats = t.repeats > 0 ? t.repeats : 0;
  return { timeoutMs, run, t0, start, logs, logTask, ctx, task, repeats };
}
// post-loop bookkeeping on run (sync; keeps __runTest's microtask shape identical to vitest's)
function __finalizeAttempts(run, failed) {
  run.ok = !failed;
  if (failed && !run.lastErr) run.lastErr = run.attemptErrors[run.attemptErrors.length - 1];
}
// clears the per-test log capture (sync; same reason)
function __exitTestLogs(logs, suiteTask) {
  if (__ttTestLogs === logs) __ttTestLogs = null;
  __ttLogTask = suiteTask;
}
// a test that runs: its repeats and attempts (retries), then its record; returns whether it failed.
// The repeat/attempt loop is inlined (not extracted into an async helper): wrapping it in an async
// function and `await`-ing it adds a microtask tick per test, which breaks vitest's microtask order
// (turbo-test#251 probes).
async function __runTest(t, chain, summary, label, suiteTask) {
  const attempts = __testRetry(t) + 1;
  const { timeoutMs, run, t0, start, logs, logTask, ctx, task, repeats } = __runTestSetup(t, chain);
  __ttTestLogs = logs;
  __ttLogTask = logTask;
  await null;
  let failed = false;
  for (let r = 0; r <= repeats && !task.result.pending; r++) {
    for (let a = 0; a < attempts; a++) {
      const mark = __beginAttempt(t, label, run, r);
      await __vtAround(() => __runAttemptInner(t, chain, ctx, timeoutMs, run)).catch(__vtNoop);
      await null;
      __snap.currentTest = null;
      failed = __endAttempt(t, run, mark, failed);
      if (!failed) break;
      if (a + 1 < attempts) failed = __retryAttempt(task, run);
    }
  }
  __finalizeAttempts(run, failed);
  // `__vtAfterRunTask()` is `async () => { await null }` — a single microtask tick that
  // cannot reject. The review-round-1 try/finally around __runTest's body was guarding a
  // rejection path that doesn't exist in this code, and a try/finally in an async function
  // body makes V8 compile the whole function with a bigger state machine (extra resume
  // state on every await), which multiplied across tens of thousands of tests measured
  // 1.5-1.7x wall-time on payroll-app's 25481-test full suite. Keep the plain await here
  // and call __exitTestLogs inline below; the microtask-order probe still matches vitest.
  if (!task.result.pending) await __vtAfterRunTask();
  const dur = __loop.elapsed() - t0;
  const timing = { duration_ms: dur, start_ms: start, retry_count: run.retries, repeat_count: task.result.repeatCount, logs: logs.map((l) => [l[0], l[1]]) };
  __exitTestLogs(logs, suiteTask);
  return __recordTest(t, chain, summary, label, run, timing, dur);
}
// one test of a suite (not a nested suite): whether it failed, or the promise of that when it runs
function __runSuiteOnlyUnderSkip(t, chain, summary, skipAll, label) {
  if (!(t.only && (skipAll === 'skip' || skipAll === 'todo'))) return false;
  summary.tests.push(__testRecord(t, chain, 'pending'));
  summary.skippedNames.push(label);
  return true;
}
function __runSuiteForbiddenOnly(t, chain, summary, label) {
  if (!t.forbiddenOnly) return false;
  summary.failed++;
  summary.failures.push(`${label}: ${__ONLY_FORBIDDEN}`);
  summary.tests.push(__testRecord(t, chain, 'failed', { message: __ONLY_FORBIDDEN, failure_messages: [__onlyForbiddenError()], not_run: true }));
  return true;
}
function __runSuiteTest(t, chain, summary, skipAll, hookFailed, suiteTask) {
  const label = chain.map((s) => s.name).filter(Boolean).concat(t.name).join(' > ');
  const fullName = chain.slice(1).map((s) => s.name).concat(t.name).join(' ');
  const filtered = !skipAll && __tt.namePattern && !__tt.namePattern.test(fullName);
  if (__runSuiteOnlyUnderSkip(t, chain, summary, skipAll, label)) return false;
  if (__skipTest(t, chain, summary, label, skipAll, hookFailed, filtered)) return false;
  if (__runSuiteForbiddenOnly(t, chain, summary, label)) return true;
  return __runTest(t, chain, summary, label, suiteTask);
}
// runSuiteChild: a test through limitTestConcurrency, a suite as it is
async function __vtChild(t, run) {
  return t.suite ? run() : __vtTestLimit(run);
}
// a test that does not run settles after its onBeforeRunTask step
async function __vtNotRun(failed) {
  await null;
  return failed;
}
const __vtTest = (r) => (r instanceof Promise ? r : __vtNotRun(r));
// Returns whether the suite failed (a failed test, or a hook / empty-suite error, in it or a
// nested suite). `mode` is the enclosing mode: '' (run), 'skip', 'todo' or 'hook' (an enclosing
// beforeAll failed).
async function runSuite(suite, ancestors, summary, mode = '') {
  await null;
  const st = __suiteOpen(suite, ancestors, summary, mode);
  if (st.empty) return true;
  if (!st.hooks) __idleTasks(st);
  else {
    await __vtAround(() => __runSuiteBody(st));
    await null;
  }
  return __suiteClose(st);
}
// A suite none of whose tasks runs (skipped, todo, left out, or under a failed beforeAll):
// vitest does not visit it, so it records its tasks without a microtask turn.
function __idleSuite(suite, ancestors, summary, mode) {
  const st = __suiteOpen(suite, ancestors, summary, mode);
  if (st.empty) return true;
  __idleTasks(st);
  return __suiteClose(st);
}
function __idleTasks(st) {
  const mode = st.hookFailed ? 'hook' : st.skipAll;
  for (const t of st.suite.tasks) {
    if (t.suite ? __idleSuite(t, st.chain, st.summary, mode) : __runSuiteTest(t, st.chain, st.summary, st.skipAll, st.hookFailed, st.suiteTask)) st.failed = true;
  }
}
// vitest (turbo-test#69): a describe with no task fails with "No test found in suite <name>";
// a throwing beforeAll fails the suite (a file-level failure, not a failed test) and skips its
// tests and nested suites, whose hooks do not run, while its afterAll hooks still run. A
// throwing afterAll fails the suite too. Neither rejects the run: the file still reports.
// A describe.skip / describe.todo block (or one inside it) runs nothing (#63).
function __suiteOpen(suite, ancestors, summary, mode) {
  const skipAll = mode || suite.mode || '';
  const record = (state) => { if (ancestors.length) summary.suites.push(skipAll === 'todo' ? 'todo' : state); };
  if (__emptySuite(suite, ancestors, summary, skipAll, record)) return { empty: true };
  const { idle, forbidden, hooks } = __suitePlan(suite, ancestors, summary, skipAll);
  const st = {
    suite, ancestors, chain: [...ancestors, suite], summary, skipAll, idle, hooks, record,
    failed: forbidden, hookFailed: mode === 'hook' || forbidden,
    // console output from here on belongs to this suite (the file for the root suite)
    outerTask: __ttLogTask, suiteTask: __suiteTaskName(suite, ancestors),
    // a suite's hook errors (vitest's failed suite, a junit testcase of its own: turbo-test#81)
    suiteStart: __loop.elapsed(), suiteErrors: [],
    // in vitest's getSuites order (a suite before the suites inside it)
    suiteSlot: summary.suite_errors.push(null) - 1,
  };
  __ttLogTask = st.suiteTask;
  return st;
}
function __suiteClose(st) {
  if (st.suiteErrors.length && st.ancestors.length) __recordSuiteErrors(st.suite, st.summary, st.suiteSlot, st.suiteStart, st.suiteErrors);
  __ttLogTask = st.outerTask;
  st.record(__suiteState(st.failed, st.skipAll, st.idle));
  return st.failed;
}
// a running suite: its beforeAll hooks, its tasks (each as vitest's runSuiteChild runs it), its
// The conditional awaits after __runSuiteBody's afterAll hook: the beforeAll cleanups (when
// any ran) and a root-suite microtask tick. A plain lambda list keeps microtask ordering
// identical to the inline `if await` pair while taking those two branches out of __runSuiteBody.
function __suiteAfterAllSteps(baCleanups, st) {
  const steps = [];
  if (baCleanups.length) steps.push(() => __vtCleanups(baCleanups, false));
  if (!st.ancestors.length) steps.push(() => null);
  return steps;
}
// Per-child thunk for each task inside a suite body. A sync generator (not an async helper) is
// what keeps __runSuiteBody's microtask ticks identical to vitest's: each `await run()` awaits the
// direct promise of __runConcurrent/__vtChild without an extra async wrapper to resolve first.
function* __suiteItems(tasks, st) {
  for (const group of __partition(tasks)) {
    if (group[0].concurrent === true) yield () => __runConcurrent(group, st);
    else for (const t of group) yield () => __vtChild(t, () => __suiteChild(t, st));
  }
}
// sync bookkeeping helpers for the beforeAll/afterAll error paths (sync keeps microtask count)
function __mark(st, failed) { if (failed) st.failed = true; }
function __baError(st, suite, summary, suiteErrors, e) {
  __suiteHookError(suite, summary, suiteErrors, 'beforeAll', e);
  st.hookFailed = st.failed = true;
}
function __aaError(st, suite, summary, suiteErrors, e) {
  __suiteHookError(suite, summary, suiteErrors, 'afterAll', e);
  st.failed = true;
}
// afterAll hooks, then the functions its beforeAll hooks returned (last first: vitest's
// `sequence.hooks: 'stack'`; a throwing beforeAll drops them all, turbo-test#101). A throwing
// hook stops the hooks after it. Inlined on purpose: wrapping either half in an async helper and
// `await`-ing it adds a microtask tick per suite body (turbo-test#251 probes).
async function __runSuiteBody(st) {
  const { suite, summary, suiteErrors } = st;
  let baCleanups = [];
  try {
    baCleanups = await __vtSuiteHook([suite], 0, 'ba', []);
  } catch (e) {
    __baError(st, suite, summary, suiteErrors, e);
  }
  if (st.hookFailed) __idleTasks(st);
  else for (const run of __suiteItems(suite.tasks, st)) __mark(st, await run());
  const afterSteps = __suiteAfterAllSteps(baCleanups, st);
  try {
    await __vtSuiteHook([suite], 0, 'aa', []);
    for (const step of afterSteps) await step();
  } catch (e) {
    __aaError(st, suite, summary, suiteErrors, e);
  }
}
// a suite's child as vitest's runSuiteChild runs it
const __suiteChild = (t, st) => (t.suite ? runSuite(t, st.chain, st.summary) : __vtTest(__runSuiteTest(t, st.chain, st.summary, '', false, st.suiteTask)));
// consecutive children with the same `concurrent`
function __partition(tasks) {
  const groups = [];
  for (const t of tasks) {
    const last = groups[groups.length - 1];
    if (last && last[0].concurrent === t.concurrent) last.push(t);
    else groups.push([t]);
  }
  return groups;
}
// a group of concurrent children: started together through a group limiter of maxConcurrency
// (turbo-test#256); whether one failed. Their records are put back in declaration order at the
// end of the file.
async function __runConcurrent(group, st) {
  const limit = __limitConcurrency(__maxConcurrency());
  __tt.concurrent = true;
  const failed = await Promise.all(group.map((t) => limit(() => __vtChild(t, () => __suiteChild(t, st)))));
  return failed.some(Boolean);
}
// runSuite's plan: `idle` (nothing in it runs: no hooks either), `forbidden` (a forbidden
// `describe.only`, turbo-test#76: it fails the suite like a throwing beforeAll, but none of its
// hooks run) and whether its hooks run
function __suitePlan(suite, ancestors, summary, skipAll) {
  const idle = !skipAll && !__runnable(suite, ancestors);
  const forbidden = !skipAll && !!suite.forbiddenOnly;
  if (forbidden) summary.failures.push(`${suite.name}: ${__ONLY_FORBIDDEN}`);
  return { idle, forbidden, hooks: !skipAll && !idle && !forbidden };
}
const __suiteTaskName = (suite, ancestors) => (ancestors.length ? [...ancestors.slice(1), suite].map((x) => x.name).join(' > ') : '');
function __recordSuiteErrors(suite, summary, suiteSlot, suiteStart, suiteErrors) {
  summary.suite_errors[suiteSlot] = { name: suite.name, duration_ms: __loop.elapsed() - suiteStart, errors: suiteErrors };
}
const __suiteState = (failed, skipAll, idle) => (failed ? 'fail' : skipAll || idle ? 'skip' : 'pass');

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
// __interpretOnly's inclusion rule for `t` inside its suite: inside a suite that holds any
// `.only` task, only those run; otherwise an enclosing `.only` (via suiteIsOnly) runs every child.
function __interpretInclude(t, hasSomeOnly, suiteIsOnly) {
  if (hasSomeOnly) return !!t.only || (!!t.suite && __someOnly(t));
  return suiteIsOnly || !!t.only;
}
function __interpretOnlyTask(t, hasSomeOnly, suiteIsOnly, forbid) {
  const include = __interpretInclude(t, hasSomeOnly, suiteIsOnly);
  if (forbid && t.only) t.forbiddenOnly = true;
  if (t.suite) __interpretOnly(t, include, forbid && !t.mode);
  else if (!include) t.notOnly = true;
}
function __interpretOnly(suite, parentIsOnly, forbid) {
  const suiteIsOnly = parentIsOnly || !!suite.only;
  const hasSomeOnly = __someOnly(suite);
  for (const t of suite.tasks) __interpretOnlyTask(t, hasSomeOnly, suiteIsOnly, forbid);
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
    // the DOM's getComputedStyle exists by now (turbo-test#210)
    if (globalThis.__ttInstallJsdomComputedStyle) globalThis.__ttInstallJsdomComputedStyle();
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
    this.concurrent = false;
    await runSuite(root, [], summary);
    // concurrent tests finish out of order: the reports list them as declared (vitest's tree)
    if (this.concurrent) summary.tests.sort((a, b) => a.seq - b.seq);
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

// the next file starts from the worker's config after its setup files (vitest re-runs them per
// file), saved copy included (vitest's resetConfig copy outlives a file), or the defaults; the
// runner applies this file's config next (__ttApplyConfig, turbo-test#180)
function __ttApplyConfigBaseline() {
  const b = __ttConfigBaseline;
  __ttConfig = b ? __ttCloneConfig(b.config) : __ttConfigDefaults();
  __ttConfigSaved = b && b.saved ? __ttCloneConfig(b.saved) : null;
  __ttZeroDelay = b ? b.zero : true;
}
// Per-file reset, modeled on vitest's isolate:false loop (vitest/dist .../base*.js): with
// isolation OFF, vitest does NOT reset modules, the DOM, spies, timers or globals between files
// — it only calls `vi.resetConfig()` + `vi.restoreAllMocks()`, and its runner scopes test
// collection per file. We mirror that minimally. Over-resetting (clearing DOM, truncating the
// spy registry, unstubbing globals, poking React internals) was what broke interaction tests
// ~100 files into a shared isolate — vitest proves none of that is needed.
globalThis.__ttResetForNextFile = () => {
  // a file stopped mid-run holds no limiter slot into the next one (turbo-test#256)
  __maxLimiter = __testLimiter = undefined;
  __ttFileLogs = [];
  __ttLogTask = '';
  // turbo-test collects tests into a single global `root` (vitest scopes per file via
  // startTests([file])) — so we must drop the prior file's collected tests/suites. Setup files
  // run once per worker, so root.hooks only holds setup hooks; leave them in place.
  root.tasks = [];
  // the prior file's own root hooks (turbo-test#214); setup and node_modules hooks stay
  for (const [kind, fn] of __ttFileRootHooks) {
    const at = root.hooks[kind].lastIndexOf(fn);
    if (at >= 0) root.hooks[kind].splice(at, 1);
  }
  __ttFileRootHooks = [];
  // Reset snapshot counters between files (keyed by test name; cleared to avoid cross-file drift).
  __snap.counters = Object.create(null);
  __snapState.loaded = false;
  // NOTE: beyond the prior file's own hooks (above), root.hooks is LEFT in place. Restoring it to a post-global-setup baseline
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
  // jest.setTimeout / jest.retryTimes are per-file overrides (turbo-test#273)
  globalThis.__TT_JEST_TIMEOUT = undefined;
  globalThis.__TT_JEST_RETRY = undefined;
  // jest automock state is per-file (turbo-test#184): clear it so the next file starts fresh.
  __ttAutomockResetForNextFile();
  // each file starts on real timers (vitest's fresh environment per file): fake timers a file
  // left on would otherwise hold every timer of the next one (turbo-test#168)
  if (__loop.fake) __uninstallFake();
  try { globalThis.vi && globalThis.vi.resetConfig && globalThis.vi.resetConfig(); } catch (e) {}
  __ttApplyConfigBaseline();
  // NOTE: we do NOT call restoreAllMocks() here. A spyOn spy's mockRestore() redefines a
  // (possibly shared, cached) module export back to the `orig` captured at spy time. This
  // suite's own setup.ts afterEach already restores per test; calling it again between files
  // re-applies a now-stale orig onto the shared analytics module — corrupting trackEvent for
  // every later file (asserts then see the wrong/old spy). Per-file restore is the test's job.
};

// The embedder's window members (turbo-test#208): turbo-dom leaves dialogs, window management,
// printing and `open` to its embedder: Chrome's results without a user (jsdom has them too, as
// no-ops returning undefined); postMessage queues a `message` event. Own, non-enumerable
// members of the window, as Chrome's [Global] Window operations are; a member the DOM already
// has is kept.
globalThis.__ttInstallWindowMembers = () => {
  const w = globalThis.window;
  if (!w || typeof w !== 'object' && typeof w !== 'function') return;
  const def = (name, fn) => {
    if (typeof w[name] === 'function') return;
    Object.defineProperty(fn, 'name', { value: name });
    try { Object.defineProperty(w, name, { value: fn, writable: true, configurable: true, enumerable: false }); } catch (e) {}
  };
  for (const name of ['alert', 'blur', 'close', 'focus', 'moveBy', 'moveTo', 'print', 'resizeBy', 'resizeTo', 'stop', 'captureEvents', 'releaseEvents']) {
    def(name, function () { return undefined; });
  }
  // Chrome with no user to answer: a dialog is dismissed (confirm false, prompt null) and a
  // popup is blocked (open null); jsdom returns undefined for all three
  def('confirm', function () { return false; });
  def('prompt', function () { return null; });
  def('open', function () { return null; });
  def('scroll', function (...a) { return typeof w.scrollTo === 'function' ? w.scrollTo(...a) : undefined; });
  def('scrollBy', function () { return undefined; });
  def('postMessage', function (message) {
    const data = typeof structuredClone === 'function' ? structuredClone(message) : message;
    setTimeout(() => {
      const Ev = w.MessageEvent;
      const ev = typeof Ev === 'function' ? new Ev('message', { data, origin: '' }) : Object.assign(new w.Event('message'), { data });
      w.dispatchEvent(ev);
    }, 0);
  });
  // The viewport (turbo-test#230): 1024 x 768 at device pixel ratio 1, the size vitest's jsdom and
  // happy-dom report (Chrome opened at that window size gives the same inner size). Each is
  // Chrome's [Replaceable] accessor: assigning one (a test simulating a resize) replaces it with
  // a data property holding the new value.
  const metric = (name, value) => {
    if (Object.getOwnPropertyDescriptor(w, name)) return;
    const replace = (v) => Object.defineProperty(w, name, { value: v, writable: true, enumerable: true, configurable: true });
    const get = { [`get ${name}`]() { return value; } }[`get ${name}`];
    const set = { [`set ${name}`](v) { replace(v); } }[`set ${name}`];
    try { Object.defineProperty(w, name, { get, set, enumerable: true, configurable: true }); } catch (e) {}
  };
  for (const [name, value] of [['innerWidth', 1024], ['innerHeight', 768], ['outerWidth', 1024], ['outerHeight', 768], ['devicePixelRatio', 1]]) {
    metric(name, value);
  }
};

// OPT-IN jsdom quirks (TURBO_JSDOM_QUIRKS=1, turbo-test#210), off by default: under environment
// 'jsdom' / 'happy-dom', jsdom's getComputedStyle reports a
// property's cascaded declared value (the last matching rule in document order, then the inline
// style) and the shorthands built from those, where turbo-dom resolves it as Chrome does
// (`1.25rem` vs `20px`, `bold` vs `700`). A property nothing declares reads turbo-dom's computed
// value. `turbo-dom` keeps Chrome's.
const __jsdomCsEligible = (w) => w && typeof w.getComputedStyle === 'function' && !w.getComputedStyle.__ttJsdom;
globalThis.__ttInstallJsdomComputedStyle = () => {
  if (globalThis.__ttJsdomQuirks !== true) return;
  const w = globalThis.window;
  if (!__jsdomCsEligible(w)) return;
  const real = w.getComputedStyle;
  // opt-in only (TURBO_JSDOM_QUIRKS=1): by default getComputedStyle stays Chrome's
  const jsdomEnv = () => globalThis.__ttJsdomQuirks === true && (globalThis.__ttEnvName === 'jsdom' || globalThis.__ttEnvName === 'happy-dom');
  const kebab = (p) => p.startsWith('--') ? p : p.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()).replace(/^(webkit|moz|ms)-/, '-$1-');
  // the style rules that can apply, in cascade order (jsdom's @media / @supports handling);
  // re-flattened only when the sheets change
  const head = (rule) => String(rule.cssText || '').slice(0, String(rule.cssText || '').indexOf('{')).trim();
  // jsdom applies an @media block only for a media-type list naming screen / all, no
  // feature queries (its window has no layout to query)
  const mediaApplies = (rule) => {
    const m = String((rule.media && rule.media.mediaText) || rule.conditionText || head(rule).replace(/^@media/i, '')).trim().toLowerCase();
    return !m || (!m.includes('(') && m.split(',').some((t) => /^(only\s+)?(screen|all)$/.test(t.trim())));
  };
  const supportsApplies = (rule) => {
    const c = String(rule.conditionText || head(rule).replace(/^@supports/i, '')).trim();
    return typeof CSS !== 'undefined' && CSS.supports && CSS.supports(c);
  };
  const flattenGroup = (rule, out) => {
    if (rule.type === 4) { if (mediaApplies(rule)) flatten(rule.cssRules, out); return; }
    if (rule.type === 12) { if (supportsApplies(rule)) flatten(rule.cssRules, out); return; }
    flatten(rule.cssRules, out);
  };
  const flattenRule = (rule, out) => {
    if (rule.type === 1 && rule.style) { out.push(rule); return; }
    if (rule.cssRules) flattenGroup(rule, out);
  };
  const flatten = (rules, out) => {
    for (const rule of rules) {
      try { flattenRule(rule, out); } catch (e) {}
    }
    return out;
  };
  // jsdom keeps a document's sheets in the order they were created or last re-parsed (a
  // `<style>` whose text changes moves to the end), not in tree order: an observer of the
  // document's style elements, read synchronously on each call, keeps that order.
  let order = [];
  let version = 0;
  let observer;
  let observed;
  const isSheetOwner = (n) => n && n.nodeType === 1 && (n.localName === 'style' || (n.localName === 'link' && /stylesheet/i.test(n.rel || '')));
  const touch = (n) => { const i = order.lastIndexOf(n); if (i >= 0) order.splice(i, 1); order.push(n); version++; };
  // (re)start observing `doc`
  const observe = (doc) => {
    if (observer && observed !== doc) { observer.disconnect(); observer = undefined; order = []; }
    if (!observer && typeof w.MutationObserver === 'function') {
      observed = doc;
      for (const n of doc.querySelectorAll('style, link[rel~="stylesheet" i]')) order.push(n);
      observer = new w.MutationObserver(() => {});
      observer.observe(doc, { childList: true, subtree: true, characterData: true });
      version++;
    }
  };
  const childListNode = (n) => {
    if (isSheetOwner(n)) { touch(n); return; }
    if (n.nodeType !== 1 || !n.querySelectorAll) return;
    for (const m of n.querySelectorAll('style, link')) if (isSheetOwner(m)) touch(m);
  };
  const childList = (rec) => {
    if (isSheetOwner(rec.target)) touch(rec.target);
    for (const n of rec.addedNodes) childListNode(n);
    if (rec.removedNodes.length) version++;
  };
  const track = (doc) => {
    observe(doc);
    if (!observer) return;
    for (const rec of observer.takeRecords()) {
      if (rec.type === 'childList') {
        childList(rec);
      } else if (rec.type === 'characterData' && isSheetOwner(rec.target.parentNode)) {
        touch(rec.target.parentNode);
      }
    }
  };
  let cache = { key: '', rules: [] };
  const countRules = (tree, adopted) => {
    let count = 0;
    for (const sh of tree) { try { count += sh.cssRules.length; } catch (e) {} }
    for (const sh of adopted) { try { count += sh.cssRules.length; } catch (e) {} }
    return count;
  };
  // the document's sheets in jsdom's order: the observed ones last, by when they were created or
  // re-parsed
  const ranked = (tree) => {
    const inTree = new Set(tree);
    const out = [];
    const seen = new Set();
    for (const n of order) {
      const sh = n.isConnected && n.sheet;
      if (sh && inTree.has(sh) && !seen.has(sh)) { seen.add(sh); out.push(sh); }
    }
    return [...tree.filter((sh) => !seen.has(sh)), ...out];
  };
  const flattenSheets = (sheets, adopted) => {
    const rules = [];
    for (const sh of [...sheets, ...adopted]) {
      try { if (!sh.disabled) flatten(sh.cssRules, rules); } catch (e) {}
    }
    return rules;
  };
  const rulesOf = (doc) => {
    track(doc);
    const tree = Array.from(doc.styleSheets || []);
    const adopted = Array.from(doc.adoptedStyleSheets || []);
    const key = version + ':' + tree.length + ':' + adopted.length + ':' + countRules(tree, adopted);
    if (cache.key === key && cache.doc === doc) return cache.rules;
    const rules = flattenSheets(observer ? ranked(tree) : tree, adopted);
    cache = { key, rules, doc };
    return rules;
  };
  const declaredBlock = (decl, block) => {
    for (let i = 0; i < block.length; i++) {
      const name = block[i];
      // jsdom sets each declaration in turn, whatever its priority: the last one wins
      const value = block.getPropertyValue(name);
      if (value === 'unset') decl.delete(name);
      else decl.set(name, { value });
    }
  };
  const declared = (el) => {
    const blocks = [];
    const doc = el.ownerDocument || w.document;
    for (const rule of rulesOf(doc)) {
      try { if (el.matches(rule.selectorText)) blocks.push(rule.style); } catch (e) {}
    }
    if (el.style) blocks.push(el.style);
    const decl = new Map();
    for (const block of blocks) declaredBlock(decl, block);
    return decl;
  };
  let colorDoc;
  const valueColorProbe = (name, dValue) => {
    try {
      if (!colorDoc) colorDoc = w.document.implementation.createHTMLDocument('');
      const probe = colorDoc.createElement('span');
      probe.style.setProperty(name, dValue);
      colorDoc.body.appendChild(probe);
      const c = Reflect.apply(real, w, [probe]).getPropertyValue(name);
      probe.remove();
      return c || undefined;
    } catch (e) { return undefined; }
  };
  const valueWantsColor = (d, name) => d && /color$/.test(name) && !/var\(|currentcolor|inherit|initial|unset|revert/i.test(d.value);
  const value = (decl, name, scratch, cs) => {
    const d = decl.get(name);
    if (valueWantsColor(d, name)) {
      const c = valueColorProbe(name, d.value);
      if (c) return c;
    }
    if (d) return d.value;
    return scratch().getPropertyValue(name) || undefined;
  };
  const getComputedStyleSkip = (el, pseudo) => !jsdomEnv() || (pseudo !== undefined && pseudo !== null && pseudo !== '') || !el || el.nodeType !== 1;
  const getComputedStyle = function getComputedStyle(el, pseudo) {
    const cs = Reflect.apply(real, w, arguments);
    if (getComputedStyleSkip(el, pseudo)) return cs;
    const decl = declared(el);
    if (!decl.size) return cs;
    let block;
    const scratch = () => {
      if (!block) {
        block = w.document.createElement('div').style;
        for (const [name, d] of decl) block.setProperty(name, d.value);
      }
      return block;
    };
    // Does `key` look like a camelCase CSS property we should intercept? Skip numeric indices,
    // the `length` / `cssText` accessor members, and anything that isn't a plain identifier.
    const cssKey = (key) => (typeof key === 'string' && key !== 'length' && key !== 'cssText' && !/^\d+$/.test(key) && /^-?-?[a-zA-Z][\w-]*$/.test(key));
    // Read `key` as a CSS property (falling back to the proxy target for non-string values and
    // function members, which are bound to the target so getPropertyValue etc. keep working).
    const cssGet = (target, key) => {
      const orig = Reflect.get(target, key, target);
      if (typeof orig === 'string' || orig === undefined) {
        const v = value(decl, kebab(key), scratch, target);
        if (v !== undefined) return v;
      }
      return typeof orig === 'function' ? orig.bind(target) : orig;
    };
    return new Proxy(cs, {
      get(target, key) {
        if (key === 'getPropertyValue') {
          return (name) => {
            const v = typeof name === 'string' ? value(decl, kebab(String(name).trim()).toLowerCase(), scratch, target) : undefined;
            return v !== undefined ? v : target.getPropertyValue(name);
          };
        }
        if (cssKey(key)) return cssGet(target, key);
        const orig = Reflect.get(target, key, target);
        return typeof orig === 'function' ? orig.bind(target) : orig;
      },
    });
  };
  // watch the document's style elements from the start of the file
  try { if (jsdomEnv() && w.document) track(w.document); } catch (e) {}
  try {
    Object.defineProperty(getComputedStyle, '__ttJsdom', { value: true });
    Object.defineProperty(w, 'getComputedStyle', { value: getComputedStyle, writable: true, configurable: true, enumerable: true });
  } catch (e) {}
};
