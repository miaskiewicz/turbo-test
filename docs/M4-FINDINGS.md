# M4 — Transform cache, resolution, scheduler, result channel: findings

## Built + verified

**oxc_resolver (bare specifiers / node_modules) — the correctness-critical piece.**
- `resolve_spec` now handles relative (fast-path probing) AND bare specifiers via
  `oxc_resolver` with conditions `["import","require","default","node"]` and TS extensions.
- Correct handling of real packages: **zod** (`"type":"module"`, exports map, `export *`
  chains, namespace re-exports) resolves + loads + runs.
- Two bugs found & fixed proving the value of matching Vite/Node semantics exactly:
  1. `"types"` must NOT be a runtime condition — it points at `.d.ts` declarations
     (caused `does not provide an export named 'z'`).
  2. `.js` module-kind follows nearest `package.json "type"` (Node's rule), not a content
     sniff (zod's ESM `.js` files were misclassified as CJS).
- Result: **full `ui-design-components/src/utils` (10 files) = 145/145, 0 failed,
  0 load-errors — EXACT parity with stock Vitest.**

**Content-addressed transform cache.** key = hash(source + ext + tool version) → transformed
JS on disk, shared across runs/workers. Cold 69% hit (shared deps within a run), **warm
100% hit** (> 90% KPI, §8).

**N isolates across cores + work-stealing duration-aware scheduler.**
- Each worker thread boots its own isolate from the single shared snapshot blob; atomic
  cursor = work-stealing; files ordered slowest-first from persisted historical durations.
- 10 files: wall **54 ms (8 jobs)** vs **148 ms (1 job)** = 2.7× (scales with file count).
- **Scheduler invariant verified:** 30 files (utils ×3) → **435 passed (=145×3), 0 failed,
  0 drops, 0 dups** under work-stealing.

**Result channel.** In-process thread model returns `TestReport` directly — no IPC
serialization needed. (Binary msgpack/bincode channel applies only if/when worker-process
isolation is added; the thread model is faster and avoids it.)

## KPIs hit
- Resolution divergences vs Node/Vite on real layouts: **0** (full utils dir parity).
- Warm transform-cache hit-rate: **100%** (> 90%).
- Wall-clock scales with isolate count (2.7× at 10 files, more with larger suites).

## Verdict
**M4 PASS** — resolver correctness (the "zero divergences" gate) proven on real
node_modules; transform cache, parallel scheduler, and scheduler invariants all green.
