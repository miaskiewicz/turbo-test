# turbo-test perf spike — 2x goal

Architecture (default path): fresh V8 isolate per test file, booted from a framework snapshot.
MR (module-runner) on by default: entry + require-graph loaded per-module as CJS. esbuild
transforms/bundles are disk-cached by content/mtime hash. Neither project uses isolate reuse →
fresh-isolate-per-file is THE hot path.

## Redundancy observed (cold-read of source, pre-profiling hypotheses)
1. **V8 compile not cached** — each module's transformed source is `v8::Script::compile`'d fresh
   in every isolate. node_modules barrels (react/MUI/emotion) recompiled ~1105× (payroll). Parse
   →bytecode is pure redundant CPU. → **V8 code cache (CachedData)**, keyed by content hash.
2. **Transform disk re-read per load** — `read_transformed`/`esbuild_*_cjs` re-read the cache file
   + re-hash on every module load in every isolate. → **in-memory per-worker cache** path→(mtime,code).
3. **Snapshot bytes cloned per isolate** — `blob.clone()` of the framework snapshot per file.
4. **install_natives per file** — re-binds native callbacks each isolate; cost TBD.
5. **Retry machinery** — transient + fresh-isolate retries (main.rs) can re-run files; only fires
   on failure, so 0 cost on green suites, but verify.
6. **esbuild service mode** — only cold-cache; deprioritized (warm = 0 spawns).

## Profile (macOS `sample`, ui 54 files --jobs 1, warm). BUSY=6996 samples (ex-wait):
- String/RegExp 26% · GC 23% · megamorphic ICs 20% · FS syscalls 7% · misc ~24%.
- Parse/compile TINY (Scanner::Next 52) → code-cache (old E1) is LOW value warm. Deprioritized.
- GC bucket = Scavenger + ConcurrentMarking::RunMajor (helper threads) + Sweeper + RecordWrite.
  In the real 8-job run, 8 isolates GC at once → GC tuning should help MORE than the 1-thread profile.
- ICs megamorphic because every file is a fresh isolate (ICs never warm) — only reuse fixes that (risky).
- Both Rust and JS (runtime.js framework layer) are fair game for experiments.

## Measurement protocol (validated)
- Absolute wall drifts hugely with machine load (saw meanA 3947ms→2010ms for the SAME run minutes
  apart). NEVER compare across separate invocations — only in-sweep paired deltas.
- `harness.sh ab` jobs=8, 40 files, 20 pairs, trim 4 → control-vs-control noise floor = **±0.4%**.
  So this config RESOLVES a 2% effect. jobs=1 is even lower-noise but ~30× slower (impractical).
- Gate every experiment behind an env flag (TURBO_V8_FLAGS, TURBO_CODE_CACHE, …) → A/B is one
  binary toggled by env = perfect pairing, no rebuild churn.

## Experiment log
- E1 V8 GC/heap flags: `--no-concurrent-marking/sweeping` = 17–25% SLOWER (concurrent GC offloads
  to helper threads; disabling forces sync GC on the worker). `--max-semi-space-size=64` alone
  ~3.5% at jobs=8 block-median but NOISE (paired -3.5% with ±37% pairs, weak 6/10). KILL the nocc
  flags; semi-space inconclusive — revisit with the validated paired protocol if revisited.
  Default reverted to no flags (gate kept).
  RE-BENCHMARK 2026-06-14 (quiet machine, noise floor verified -0.2% / ±0.4%): semi-space=64
  is a CLEAN KILL, not noise — paired jobs=1 micro (ui, 120 files, 16 pairs, trim 3) = +10.2%
  trimmed / +5.2% median SLOWER, B faster only 6/16. Bigger young-gen ⇒ larger per-scavenge cost
  dominates the fewer-scavenge savings (consistent with the GC bucket being scavenge-heavy). The
  earlier "promising ~3.5%" read was external-load artifact. E1 = dead. Do not revisit semi-space.
- E2 V8 bytecode code-cache: consume/produce per-module compiled bytecode on disk, keyed by the
  exact wrapped source; safe fallback on reject/miss. Paired A/B (jobs=8,40f,20pairs,trim4) vs
  ±0.4% noise floor: ui -1.3% / payroll -1.8% trimmed (median -2.1% / -2.6%), pass/fail identical.
  SHIPPED in 0.2.12 — default ON, disable with TURBO_NO_CODE_CACHE. This is the NEW baseline.
- E3 in-memory dep-bundle memo: E2+E3 combined = +5.2% SLOWER than control (cloning big bundle
  strings out of a HashMap each require beats the OS-page-cached file read, and the growing map
  adds GC pressure). KILL — code removed.

- E5 snapshot bytecode bake (FunctionCodeHandling::Keep vs Clear, gated TURBO_SNAP_KEEP):
  bake the framework's compiled bytecode into the snapshot so each per-file isolate skips
  recompiling RUNTIME_JS. Paired A/B: ui -3.6% (faster), payroll +4.8% trimmed / -0.5% median
  (neutral-to-slower — the bigger Keep blob costs more per-isolate deserialization, offsetting
  the recompile savings, and the balance flips by suite). Correctness clean (ui keep 7006/0).
  NOT a clean win on both → NOT shipped as default; gate kept opt-in (default Clear = unchanged).
- E10 V8 platform helper-pool size (gate TURBO_V8_POOL=N, default 0 = ncpu, 2026-06-14):
  hypothesis was that new_default_platform(0,…) spawns ~ncpu (~7 on 8-core) GC/compile helper
  threads on TOP of the N job-threads ⇒ ~2× cores oversubscription, so capping the pool would
  cut GC-helper contention WITHOUT idling job cores (the surgical lever E4 wasn't). DISPROVEN.
  Full-suite paired A/B (ui, --sub 0, --jobs env), quiet machine: pool 0 vs 2 = +11.3% trimmed
  SLOWER (0/5); pool 0 vs 4 = +5.3% trimmed (1/8; the 6 genuine-low-load pairs ALL slower
  +0.4–16%). Monotonic with E4: fewer helper threads = slower full suite. The helpers earn their
  keep — concurrent scavenge/marking parallelism during GC bursts outweighs the oversubscription,
  because the full cold suite keeps job cores busy with real work so the "extra" helpers aren't
  actually stealing CPU from useful work; they only fire during GC, where more of them = faster GC.
  KILL. Gate kept opt-in (default 0 = unchanged) for future sweeps, same as TURBO_JOBS.
  ===> Both concurrency levers (E4 worker-count, E10 helper-pool) are now closed: on the full
  cold suite, ncpu workers + ncpu helpers is the throughput optimum; cutting either idles capacity.
- REUSE SPIKE: see docs/reuse-spike.md. Verdict NO (breaks payroll accuracy — per-file vi.mock of
  node_modules is fundamentally incompatible with caching node_modules across files). Stays opt-in.
- E12 SHIPPED v0.2.14 (see CHANGELOG). Memoize nearest_tsconfig + resolve_spec_as per worker.
  ui full −7.4%, payroll full −13.9%, identical pass/fail. Default ON (TURBO_NO_E12 to disable).

## Post-E12 micro-opt experiments (N1–N5, 2026-06-14) — ALL KILLED
After E12 banked the resolution-cache win, profiled the remaining per-file hot spots and tried 5
micro-opts (gated TURBO_N1..N5, jobs=8 --alt validated protocol). Accuracy IDENTICAL both suites
(ui 431, payroll 1072). All NEUTRAL — none clears the ≥1% both-suites ship gate:
| exp | what | ui micro | payroll micro |
|---|---|---|---|
| N1 | fxhash (FxHasher) for transform cache keys instead of SipHash DefaultHasher | +1.7% | −0.2% |
| N2 | memoize is_esm_module per path (package.json "type" walk) | +0.9% | −0.3% |
| N3 | memoize project_root per path (node_modules/.bin/esbuild walk) | +1.3% | −0.4% |
| N4 | memoize cache_dir (skip create_dir_all syscall per transform) | −0.7% | +0.6% |
| N5 | skip source read+hash on warm transform (mtime-keyed cache-path memo) | +0.3% | +0.0% |
| ALL | all five together | +0.8% | −1.0% |
Reverted (binary stays clean v0.2.14). WHY neutral: E12 already cached the expensive resolution;
the leftover FS/hash spots are individually tiny and OS-page-cached on a warm run, and the source
hash isn't the bottleneck. The dominant buckets (String+RegExp 26%, GC 23%, megamorphic ICs 20%)
are TEST EXECUTION, not per-file plumbing — unreachable by resolution/transform micro-opts. Future
per-file wins look played out; the remaining levers are execution-side (IC warming, reuse) and are
either closed (reuse breaks accuracy) or app-specific/hard.

## Experiment backlog (ranked by profile, generic-only)
- E1: V8 bytecode code-cache for compiled CJS modules (biggest expected)
- E2: in-memory per-worker transform cache (kill repeated disk read + hash)
- E3: snapshot blob share (avoid per-isolate clone if v8 allows borrowed StartupData)
- E4: install_natives cost reduction / fold into snapshot
- E5: V8 flags tuning (--no-opt for short-lived? lite mode? jitless? single-threaded GC)
- E6: parallelism/scheduling (jobs count, work-steal granularity)
- E7: faster transform for app files (oxc in-proc) IF it doesn't change output shape — risky
- E8: avoid double-compile in mock prepass (esbuild_transform_cjs called twice: existence check
  at 3690 then load) — cache the existence result
- E9: string alloc reduction in hot loops (wrapped fn template per module)
- E10: lazy/skip unused native installs; reduce per-file v8 String::new churn

## Baselines (local HEAD == published 0.2.11, clean cache, microbench 40 stride-sampled files)
| suite | median warm wall | pass/fail |
|---|---|---|
| payroll-app | 4042 ms | 406 / 0 |
| ui-design-components | 4859 ms | 687 / 0 |

NEW BASELINE = 0.2.12 (code-cache ON). Full-suite reference pass/fail: payroll 10580/1
(the 1 failure is PRE-EXISTING, present with cache off too — flaky), ui 7006/0. Future A/B
controls run with code-cache ON (default).

## Cumulative gain tracker (target: 50%; ship gate: repeatable >=1%)
- v0.2.12: E2 code-cache ~1.5-1.8%  (SHIPPED). Running total ≈ 1.8%.
- E4 worker-count: KILLED. The 40-file WARM microbench said jobs=6 was -14% vs jobs=8 (clean,
  consistent across 8v6/6v4/etc.). But the FULL 431-file ui suite, paired A/B, showed jobs=6
  +75% / +40% SLOWER. Opposite result. Why: a small warm subset finishes fast and is pure CPU,
  so the GC/compile helper threads visibly contend → fewer workers win. The full cold suite has
  enough work in flight that all cores stay productively busy and the helpers are mostly idle, so
  cutting workers just leaves cores empty. ===> LESSON (added to harness README): parallelism /
  scheduling / thread-count changes MUST be measured on the FULL suite, never a stride subset.
  The microbench is fine for per-file work (transform/compile/alloc) but lies about concurrency.
  Reverted to ncpu default. TURBO_JOBS env hook kept for sweeps.
  RE-BENCHMARK 2026-06-14 (quiet machine): full-suite paired A/B (ui, --sub 0, --jobs env,
  5 pairs, trim 1) reconfirms the KILL — jobs=8 vs 7 = +15.9% trimmed (7 faster 2/5);
  jobs=8 vs 6 = +59.8% trimmed SLOWER (6 faster 0/5, every pair +49–90%). Monotonic: ncpu(8)
  is best, fewer workers strictly worse. The earlier "jobs=6 promising" read was the warm-micro
  lie + external load. E4 = dead. The surgical concurrency lever is E10 (shrink the V8 platform
  helper pool via new_default_platform(N,…)), NOT worker count — must also be full-suite measured.

Microbench harness: `scripts/bench.sh <proj> 3 --sub 40`. Full-suite validation before any
publish. NEVER kill runs (poisons bundle cache — see docs/TODO-cache-poisoning.md).

## Guardrails
- pass/fail counts MUST match baseline on BOTH suites (accuracy).
- median warm wall MUST not regress (speed).
- no codebase-specific hacks — every change must help generic TS/JS suites.
