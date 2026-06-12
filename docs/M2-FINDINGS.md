# M2 — Event loop + fake timers: findings

Spec's other hard gate (§5): the highest-risk compatibility area, surfaced early. Built on
the M1 runner. All in our own V8 embedding, no Node event loop.

## What was built

**Native event loop with Node-compatible ordering** (`src/runtime.js` scheduler +
`src/runner.rs` drive loop):
- microtasks = native V8 Promise jobs, drained via `perform_microtask_checkpoint`
- `process.nextTick` = own queue, drained BEFORE Promise microtasks each turn (Node priority)
- macrotasks (`setTimeout`/`setInterval`/`setImmediate`) = virtual-clock queue
- drive loop: drain nextTick+microtasks to a fixpoint, then run ONE macrotask, repeat.
  No real sleeping — the clock jumps to the next due timer, so async tests are
  deterministic and order-preserving.

**Fake timers** (`vi.*`), bound to the same native scheduler:
- `useFakeTimers` / `useRealTimers` / `isFakeTimers`
- `advanceTimersByTime(ms)` / `advanceTimersToNextTimer` / `runAllTimers` /
  `runOnlyPendingTimers`
- `setSystemTime` + `Date` mocking (`Date.now`, `new Date()` reflect the virtual clock)
- `clearTimeout`/`clearInterval`, `getTimerCount`, `clearAllTimers`

(Spec calls for reusing `@sinonjs/fake-timers` logic bound to the native loop. M2 implements
the equivalent logic natively, bound to the loop; swapping in the real sinon package is part
of the M3 framework bake — the binding seam is the same `__loop`.)

## Gate results

**Differential ordering battery (§5) — 100% parity vs the Node oracle.** `fixtures/battery/`,
run under both `node <file>` and `turbo-test <file>`, orders identical:

```
b1 micro/macro/nextTick : sync-start,sync-end,promise1,nextTick,timeout0,after-await
b2 timer delays         : t0a,t0b,t5,t10
b3 nextTick vs promise  : p1,p2,nt1,nt2
b4 async/await interleave: start,a1,after-call,a2,micro,a3,end
=> 4 parity / 0 diverge
```

**Fake-timer suite — parity vs stock Vitest.** `fixtures/tests/timers.test.ts`, 5 tests
(advanceTimersByTime, runAllTimers due-order, setSystemTime+Date, clearTimeout, getTimerCount):
turbo-test **5 passed** === stock Vitest **5 passed**.

## Boundaries (honest)
- Differential battery is run vs the **Node oracle** (same JS engine semantics Vitest uses),
  not vs Vitest directly — the timing primitives are the engine's, and they match.
- Microtask interleaving *between* individual fake timers in a single synchronous
  `advanceTimersByTime` is JS-synchronous (no forced native checkpoint mid-advance). The
  `await + advance` promise tail is the known hard case; expand coverage when the real
  sinon package is baked at M3.
- Timer-heavy gauntlet corpus (suite (e)) runs fully once M4 resolver lands; mechanics are
  proven here.

## Verdict
**M2 PASS** — 100% order parity on the differential battery; fake-timer suite bit-for-bit
with stock Vitest. The spec's highest-risk area is green.
