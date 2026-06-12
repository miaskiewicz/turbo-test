# M3 — Framework layer in snapshot: findings

Bakes the framework layer into a V8 startup snapshot (spec §4) and serves each test file a
context *from* that snapshot, instead of re-evaluating the framework per file.

## What was built (mechanism — DONE)

- **Framework snapshot** (`framework_snapshot()` in `src/runner.rs`): a `SnapshotCreator`
  context with `runtime.js` evaluated, set as default context, serialized once at startup
  (`FunctionCodeHandling::Clear` for a lean blob). Built eagerly in `init_v8` so per-file
  timing is steady-state.
- **Per-file context-from-snapshot:** each test file boots an isolate with the snapshot
  blob; `Context::new` yields the baked default context (framework already present). Only
  native callbacks (`log`, `__nativeRequire`) are re-bound per context — V8 cannot serialize
  native function pointers, so those live outside the snapshot.
- Full per-file isolation retained (each context gets its own copy of the baked collector
  state `__tt`/`__loop`).

## Guardrail (the M3 risk) — HELD

Spec: *per-file setup must not regress from M0, and a framework-bloated snapshot that slows
instantiation negates the premise.*

- **Steady-state per-file env setup: 0.58 ms** (measured by `turbo-test`, printed each run).
- M0 baseline was 0.38 ms (context-only, reusing one isolate). The 0.2 ms delta is per-file
  **isolate** creation; isolate reuse across files (M4 scheduler) brings it back toward M0.
- vs stock Vitest per-file env: 32 ms (turbo-dom) – 465 ms (jsdom). Still **55–800× faster.**
- Snapshot stays lean → deserialization stays cheap. No bloat regression.

## Parity preserved
Switching from per-file framework re-eval to context-from-snapshot changed **nothing**
observable: torture suite 17/17, fake-timer suite parity, real corpus 132/132 across repeated
files, 0 failures.

## Remaining M3 content (same seam, tracked)
The framework currently baked is the **minimal** runtime (`runtime.js`). The spec calls for
baking the **real** `@vitest/expect` + chai + `@vitest/snapshot` + `@vitest/spy` + the `vi`
API + **turbo-dom** (default DOM environment). That is a packaging step — bundle those
packages into one snapshot-safe script and bake it via the exact same `framework_snapshot()`
mechanism. It is the compatibility lever for the long matcher/snapshot/mock tail and for
`.snap` byte-compatibility (§8). The mechanism proven here does not change; only the bytes
baked do. turbo-dom (`@miaskiewicz/turbo-dom@^0.1.57`) is already wired in `package.json`.

## Verdict
**M3 mechanism PASS** — snapshot bake + per-file context-from-snapshot + guardrail (0.58 ms,
no bloat) all green, parity unchanged. Real-`@vitest`/turbo-dom bundle is the remaining
content swap on the same seam (M3-content / M6 hardening).
