# turbo-test perf spike — HANDOFF (resume on another machine)

Status snapshot for picking up the perf work elsewhere. NOT a committed doc (yet) — it's the
"where we are / what to do next" brain dump. Pair it with the committed `docs/perf-spike.md`
(experiment log), `docs/reuse-spike.md` (reuse verdict), `docs/TODO-cache-poisoning.md`, and
`scripts/perf/README.md` (harness guide).

## TL;DR
- Shipped **v0.2.12** = V8 bytecode code-cache (E2), ~1.5–1.8%, identical pass/fail. Live on npm.
- E1 (V8 flags), E3 (mem-cache), E4 (worker-count) = KILLED. E5 (snapshot-keep) = no-ship (opt-in).
- Isolate-reuse spike = **NO** (breaks accuracy; fundamental). The big lever is closed by the
  100%-accuracy constraint.
- Realistic ceiling for safe generic per-file wins ≈ **10–20% cumulative**, not 50%. ~1.8% banked.
- Work paused because THIS machine had persistent external load (30–46 on 8 cores) making
  full-suite A/B unreliable. Resume on a quiet machine.

## Repo state (git, branch main)
Latest commits (newest first), all pushed to origin/main:
```
<hash> docs: add CHANGELOG covering v0.2.0 → v0.2.12 + unreleased perf-spike work
78d5efd perf(spike): snapshot bytecode-bake gate (TURBO_SNAP_KEEP) + E5 verdict
48b137f docs(perf): isolate-reuse spike verdict — NO (breaks accuracy)
cb803ac docs(perf): record E4 worker-count kill + full-suite measurement rule
65ec006 perf: v0.2.12 — V8 bytecode code-cache for CJS module compiles   <-- shipped/tagged
bd02d4a feat: v0.2.11 — experimental jest drop-in
```
Working tree clean. `package.json`/`Cargo.toml` = 0.2.12. npm latest = 0.2.12.
Binary: `bin/turbo-test-darwin-arm64` is the committed-source build (default behavior = 0.2.12 +
the harmless opt-in env gates below). On a new machine: `cargo build --release` then
`cp target/release/turbo-test bin/turbo-test-<plat>-<arch>` (see CLAUDE.md).

## Env gates currently in the code (all default = OFF/unchanged behavior)
- `TURBO_CODE_CACHE` removed as a gate — code-cache is ON by default now; disable via
  `TURBO_NO_CODE_CACHE`. (E2, shipped.)
- `TURBO_V8_FLAGS="<flags>"` — pass arbitrary V8 flags (E1 sweep hook). Default empty.
- `TURBO_JOBS=N` — override worker count (E4 sweep hook). Default = `available_parallelism()`.
- `TURBO_SNAP_KEEP=1` — bake framework bytecode into the snapshot (E5). Default = Clear.
- `TURBO_REUSE_ISOLATE=1` — isolate reuse (pre-existing; opt-in; see reuse spike). Also
  `TURBO_NO_REUSE`. `TURBO_NO_MR` disables module-runner.

## Baselines (the accuracy reference — MUST match on any change)
Full-suite pass/fail (fresh, default mode):
- payroll-app: **10580 passed / 1 failed** (the 1 is a pre-existing flaky DOM test; present with
  code-cache off too). ~1071 files. NOTE flaky variance seen 10580–10591 passed across runs.
- ui-design-components: **7006 passed / 0 failed**. 431 files.
Microbench median (40 stride files, was measured under varying load — treat as rough):
payroll ~4042ms, ui ~4859ms at first measure; absolute wall drifts hugely with load — only trust
in-sweep paired deltas.

## Measurement methodology (LEARNED THE HARD WAY — read before benchmarking)
1. **Absolute wall is meaningless across runs.** Saw meanA swing 2010↔3947↔3983ms for the same
   workload minutes apart. Only `harness.sh ab` (paired interleaved A/B) deltas are valid.
2. **Validated noise floor = ±0.4%** at `ab --sub 40 --jobs 8 --pairs 20 --trim 4` (control vs
   control) WHEN THE MACHINE IS QUIET. Under load it widens a lot.
3. **Microbenchmarks LIE about concurrency.** E4 (fewer workers) showed −14% on a warm 40-file
   micro but +40–75% SLOWER on the full suite. Rule: parallelism/scheduling/thread-pool/memory
   changes MUST be measured `ab --sub 0` (full suite). Microbench is only valid for PER-FILE work
   (transform, compile, allocation).
4. **NEVER kill a run mid-flight** — a killed esbuild leaves a truncated bundle in the shared
   cache (`$TMPDIR/turbo-test-cache`) and poisons every later run (phantom failures). Let runs
   drain; only then wipe the cache. (docs/TODO-cache-poisoning.md — also a real bug to fix.)
5. Prefer env-gated experiments → A/B is one binary toggled by env (perfect pairing, no rebuild).
6. Gate every winner: identical pass/fail on BOTH suites + repeatable ≥1% (user relaxed from 2%).

## Harness (committed, reusable)
- `scripts/perf/harness.sh <micro|ab|full|profile> <proj> [opts]` — see scripts/perf/README.md.
  - micro: warm + R runs, median. ab: paired interleaved, trimmed-mean delta + win rate + PF-drift
    guard. full: whole suite. profile: macOS `sample` → GC/IC/String/FS buckets.
  - opts: `--sub N` (0 = all), `--jobs J` (or `env` to let TURBO_JOBS decide), `--pairs P`,
    `--trim T`. `SUB_ROOT=dir` overrides the `src` search root.
- `scripts/perf/accuracy-diff.sh <proj> "<ENV_A>" "<ENV_B>"` — per-file pass/fail diff between two
  configs (the rigorous accuracy gate; counts alone can hide a false-pass+false-fail that cancel).

## Experiment findings so far
- **E1 V8 GC/heap flags** (TURBO_V8_FLAGS) — KILL. `--no-concurrent-marking/sweeping` = 17–25%
  SLOWER (concurrent GC offloads to helper threads; disabling forces sync GC on the worker).
  `--max-semi-space-size=64` alone was noisy (~−3.5% block-median but paired was inconclusive,
  ±37% pairs). If revisited: test semi-space sizing with the validated paired protocol on a quiet
  machine; do NOT disable concurrent GC.
- **E2 V8 bytecode code-cache** — SHIPPED v0.2.12. consume/produce per-module compiled bytecode on
  disk keyed by wrapped source; safe fallback. ui −1.3% / payroll −1.8% paired; pass/fail identical.
- **E3 in-memory dep-bundle memo** — KILL. E2+E3 combined +5.2% slower (cloning big bundle strings
  out of a HashMap beats the OS-page-cached file read; growing map adds GC pressure). Removed.
- **E4 worker-count** — KILL (the big lesson). Host was 8 logical = 4 P + 4 E core. Warm micro:
  jobs 7→−1.3%, 6→−13.9%, 5→−10.2%, 4→−14.4% vs 8 (looked huge). Full-suite paired A/B: jobs=6 was
  +40–75% SLOWER. payroll was neutral at 6 and +10% at 4. Reverted to ncpu default. Why micro lied:
  small warm subset is pure CPU and finishes fast so GC helper threads visibly contend; the full
  cold suite keeps all cores productively busy, so cutting workers just idles cores.
- **E5 snapshot bytecode bake** (TURBO_SNAP_KEEP) — NO-SHIP. `FunctionCodeHandling::Keep` vs Clear:
  ui −3.6% (faster) but payroll neutral/+slower (bigger Keep blob costs more per-isolate
  deserialization, offsetting the recompile savings; balance flips by suite). Correctness clean
  (ui 7006/0, payroll 10591/1). Kept opt-in.
- **Isolate-reuse spike** — verdict NO (docs/reuse-spike.md). ui = 7006/0 + faster, but payroll =
  10368/221: deterministic whole-file flips (e.g. ImssWorkRiskCard 15/0→0/15), reproducible
  STANDALONE (`useApiContext must be used within an ApiProvider`). Root cause is fundamental, not a
  bug: payroll per-file `vi.mock`s node_modules pkgs (next-intl, @mui/material,
  @flux-payroll/flux-payroll-ui); reuse caches node_modules across files (that's the speedup) so a
  per-file mock of an already-cached package can't take effect → real module runs → file fails. The
  fresh-retry net doesn't rescue it. ui passes only because it mocks app modules (which reuse does
  reset). Can't be a default. Stays opt-in.

## Hotspot map (DETAILED — the map for picking the next experiment)
Source: macOS `sample` of the release binary, ui-design-components 54 files, warm cache.
(Captured `/tmp/sample_ui2.txt`, ~13MB — regenerate with `harness.sh profile <proj> --sub 60`.)
Caveat: this particular capture happened to be a reuse run (`run_test_file_reused` in the tree),
so the per-file isolate-setup slice is understated vs the default fresh path; the EXECUTION /
require / GC / string picture below is representative of both paths.

### Thread model (this is the oversubscription story, concretely)
Sample saw, per process: 1 `com.apple.main-thread` (mostly `__psynch_cvwait` — idle, waits on the
workers), **7 `V8 DefaultWorker` threads** (the platform's GC/compile helper pool — spawned ~ncpu
by `new_default_platform(0,…)`), plus the job worker threads. So on an 8-core box, job threads +
7 helpers ≈ 2× cores of runnable threads. The helpers are *mostly idle* but fire hard during GC.
This is why E4 (cutting job count) helped a tiny warm microbench but HURT the full suite, and why
E10 (shrink the V8 platform pool via `new_default_platform(N,…)`) is the more surgical lever —
fewer helper threads instead of fewer workers. MUST be full-suite measured.

### Busy-sample buckets (ex-wait ≈ 6996 samples; top-of-stack/leaf attribution)
| bucket | ~% | what it is | dominant leaf symbols (sample counts) |
|---|---|---|---|
| String + RegExp | 26% | string build/scan/search, regex exec on test+lib code | `RegExpPrototypeExec` 179, `StringIndexOf` 98, `SlowEqualsNonThinSameLength` 84, `StringSubstring` 81, `RegExpReplace` 68, `StringAdd_CheckNone` 54, `StringPrototypeTrim` 48, `StringHasher::HashSequentialString` 45, `StringToLowerCaseIntl` 43, `WriteToFlat2` 34, `Runtime_StringSplit` 30, `NewProperSubString` 28, `StringSearch::InitialSearch` 28, `FastAsciiConvert` 27, `ComputeAndSetRawHash` 27, `StringPrototypeReplace` 21 |
| GC | 23% | young-gen scavenge + concurrent marking/sweeping (the 7 helper threads) | `Scavenger::ScavengeObject` 327, `ConcurrentMarking::RunMajor` 264, `EvacuateInPlaceInternalizableString` 165, `Scavenger::Process` 158, `MarkingVisitorBase::ProcessStrongHeapObject` 149, `RecordWriteIgnoreFP` 117, `IteratePointers` 67, `RecordSlot` 45, `ScavengePage` 37, `FreeListManyCached::Allocate` 35, `EvacuateThinString` 34, `Sweeper::RawSweep` 34, `RecordWriteSaveFP` 49 |
| Megamorphic ICs | 20% | property loads/stores with cold/megamorphic inline caches (fresh isolate ⇒ ICs never warm) | `LoadIC` 516, `LoadIC_Megamorphic` 250, `LoadICGenericBaseline` 128, `FindOrderedHashMapEntry` 105, `MapPrototypeSet` 105, `KeyedLoadIC_Megamorphic` 82, `KeyedStoreIC_Megamorphic` 57, `KeyedLoadIC` 17, `LoadIC_NoFeedback` 18, `LookupIterator::LookupInRegularHolder` 28, `TransitionArray::BinarySearchName` 26, `MigrateToMap` 17 |
| FS syscalls | 7% | per-module resolution + reading the on-disk transform cache | `__getattrlist` 145, `read` 144, `stat` 106, `__open` 81 |
| String interning / hashing | (subset, ~5%) | internalizing module/identifier strings, hash maps | `StringTable::TryLookupKey` 87, `murmur2_or_cityhash` 34, `Runtime_InternalizeString` 17, `MakeThin` 17, `TryStringToIndexOrLookupExisting` 17, `LookupString` 16 |
| Parse / compile | TINY (~1%) | actual JS parsing | `Scanner::Next` 52, `ScanIdentifierOrKeywordInner` 25, `AstValueFactory::GetString` 15 (E2 code-cache already removed most of this) |
| Alloc / object-literal | (subset) | object/array creation | `CreateObjectLiteral` 15, `AllocateRaw` 19, `Factory::New*` 16+16, `CreateShallowObjectLiteral` 14, `MemoryChunk` ctor 18, `PretenuringHandler::UpdateAllocationSite` 18 |

### Call-tree (inclusive samples — which OF OUR functions drive the cost; worker total ≈ 7429)
```
run_test_file (7424)
└─ run_test_file_reused (7402)        # fresh path = run_test_file body; same downstream
   ├─ drive_tests (6336)             # TEST EXECUTION dominates — running describe/it bodies
   │  └─ call_global_bool (1512)     # event-loop draining: __drainNextTicks / __hasNextTicks
   ├─ load_cjs_inner (4314)          # per-module compile + execute (overlaps drive via dyn import)
   │  └─ native_require (3627)       # require() resolution + module load — BIG, second hottest path
   │     ├─ resolve_spec_as (317)    # specifier resolution
   │     ├─ read_transformed (187)   # read on-disk transform cache (drives the FS syscalls)
   │     │  └─ esbuild_transform_cjs (155)
   │     ├─ load_graph (108)
   │     └─ nearest_tsconfig (100)   # tsconfig lookup per resolution
   └─ run_setup_file (56)
```
Takeaways for picking experiments:
- **EXECUTION (drive_tests 6336) is the true bulk** — most of String/GC/IC is the test + library
  code running. Hard to cut without reuse (closed off) or warming ICs (per-app, hard).
- **`native_require` (3627) is the second hottest path** and is OUR code → most actionable. It
  drives FS syscalls (`read_transformed`), `resolve_spec_as`, and repeated `nearest_tsconfig`
  lookups. Targets: memoize tsconfig lookups per dir; cache resolution results; cut `read`/`stat`
  churn. (E6/E7/E8 live here.)
- **`call_global_bool` (1512)** = the event-loop drain calling into JS twice per macrotask
  (`__drainNextTicks` + `__hasNextTicks`). A tighter drain protocol (batch the checks, or a single
  native-side queue-empty flag instead of two JS round-trips per loop) could shave real time. New
  candidate experiment (per-file, microbench-valid).
- **GC 23% + 7 helper threads** → E10 (shrink platform pool) is the surgical concurrency lever;
  measure full-suite.
- Parse/compile is already tiny (E2 did its job) — do NOT chase more compile-cache wins.

### How to refresh on the new machine
`scripts/perf/harness.sh profile ../ui-design-components --sub 60 --jobs 1` (jobs 1 = clean
single-thread attribution). For the call-tree, the harness only prints buckets; for the inclusive
turbo_test:: frame table run `sample` yourself and:
`grep -oE "[0-9]+ turbo_test::[A-Za-z_:]+" <samplefile> | awk '{c=$1;$1="";a[$0]+=c}END{for(k in a)print a[k],k}' | sort -rn`.

## Backlog / next experiments to try (all per-file = microbench-valid unless noted)
- E6: avoid the double `esbuild_transform_cjs` existence-check at entry load (runner.rs ~line 3690
  calls it just to test, then load_cjs calls it again — cache the existence result).
- E7: faster hashing in cache-key paths (DefaultHasher → fxhash/ahash) — many per-module hashes.
- E8: reduce per-module wrapper string alloc (`format!("(function(exports,...){...})")` every load)
  — reuse a buffer / avoid the realloc.
- E9: reduce per-file `v8::String::new` churn in install_natives / per-file setup.
- E11 (NEW, from call-tree): tighten the event-loop drain. `drive_tests` → `call_global_bool`
  (1512 incl. samples) crosses into JS twice per macrotask (`__drainNextTicks` + `__hasNextTicks`).
  Try a single native-readable "queues empty" flag or batch the two checks → fewer JS round-trips
  per loop iteration. Per-file, microbench-valid.
- E12 (NEW, from call-tree): memoize `nearest_tsconfig` (100 incl.) per directory and cache
  `resolve_spec_as` (317) results per (specifier, importer-dir) — `native_require` repeats these
  for the same dirs across every module/file.
- E10: V8 platform thread-pool size `new_default_platform(N,...)` — CONCURRENCY change, must be
  full-suite measured (E4 lesson). Could cut GC-helper oversubscription without cutting workers.
- Bigger swings (need design): bake common node_modules (react/MUI pure-JS) into the snapshot;
  smarter scheduling; per-test (not per-file) parallelism. All require full-suite validation.
- Robustness (not perf): fix the atomic-bundle-write cache poisoning (docs/TODO-cache-poisoning.md).
- If reuse is ever revisited: route per-file-node_modules-mocking files to the fresh path (use
  `extract_mocks` to detect), and fix why the fresh-retry doesn't rescue. Large; uncertain.

## How to resume (clean machine, quiet)
1. `git pull`; `cargo build --release`; refresh `bin/turbo-test-<plat>-<arch>`.
2. Confirm baselines: `harness.sh full ../payroll-app` and `... ../ui-design-components` →
   expect 10580/1 (±flaky) and 7006/0. If different, the suites or deps drifted — re-baseline.
3. Warm cache once (any full run). Verify quiet: `uptime` load should be < ~ncpu.
4. Sanity-check the protocol: `harness.sh ab <ui> --sub 40 --jobs 8 --pairs 20 --trim 4
   "TURBO_NO_CODE_CACHE=1" "TURBO_NO_CODE_CACHE=1"` (control vs control) → expect ≈0% / ±0.4%.
5. Pick E6/E7/E8 from backlog, env-gate it, `ab` microbench both suites, then `full` correctness +
   (if concurrency-touching) `ab --sub 0`. Ship on identical pass/fail + repeatable ≥1%:
   bump BOTH package.json + Cargo.toml, refresh bin, commit, `git tag vX.Y.Z`, `git push origin
   main && git push origin vX.Y.Z` (CI builds all platforms + publishes; needs NPM_TOKEN secret).
```
```
