# turbo-test perf harness (`scripts/perf/`)

One tool — `harness.sh` — for benchmarking and A/B-testing perf experiments on
real consumer suites (e.g. `../payroll-app`, `../ui-design-components`). Built
during the v0.2.x perf spike; kept in-repo so future agents can reuse it.

## Quick start
```sh
# representative microbench (median wall + pass/fail), serial for low noise
scripts/perf/harness.sh micro ../ui-design-components --sub 40 --jobs 1 --runs 3

# decide a small (>=2%) change: paired interleaved A/B, trimmed-mean delta
scripts/perf/harness.sh ab ../ui-design-components --sub 120 --jobs 1 --pairs 16 \
    "TURBO_V8_FLAGS=" "TURBO_V8_FLAGS=--max-semi-space-size=64" control semi64

# find the hot spots (buckets GC / IC / String+RegExp / FS)
scripts/perf/harness.sh profile ../ui-design-components --sub 60 --jobs 1

# validate the real-world full suite before publishing (default jobs, pass/fail)
scripts/perf/harness.sh full ../payroll-app --runs 2
```

## The golden rules (learned the hard way)
0. **Parallelism/scheduling changes MUST be measured on the FULL suite (`--sub 0`),
   never a stride subset.** A warm 40-file microbench said cutting workers from 8→6
   was -14%; the full 431-file suite was +40–75% SLOWER. A small warm subset is pure
   CPU and finishes fast, so GC/compile helper threads visibly contend → fewer workers
   look better. The full cold suite keeps all cores busy with real work, so cutting
   workers just idles cores. The microbench is reliable for PER-FILE work (transform,
   compile, allocation) but LIES about anything touching concurrency, thread pools,
   scheduling, or memory pressure. For those: `harness.sh ab <proj> --sub 0 --jobs env`.
1. **NEVER kill a run mid-flight.** A killed esbuild leaves a truncated bundle in
   the shared cache (`$TMPDIR/turbo-test-cache`) and poisons EVERY later run with
   phantom `Illegal return statement` / render failures. See
   `docs/TODO-cache-poisoning.md`. Let runs drain; only then wipe the cache.
2. **8-job wall variance (~±30%/run) swamps a 2% effect.** For small effects use
   `ab --jobs 1` (serial → low variance) with many `--pairs`. The per-pair delta
   cancels slow machine drift; the trimmed mean drops outlier pairs.
3. **Confirm winners with `full` at default jobs** before publishing — some
   effects (e.g. GC/heap, thread contention) only show up under real parallelism.
4. **Accuracy gate:** every mode compares pass/fail. `ab` prints `PF DRIFT` and
   marks the result INVALID if A and B disagree. A speed win that changes results
   is a loss.
5. **Prefer env-gated experiments** (like `TURBO_V8_FLAGS`) so A/B is a single
   build toggled by env — perfect pairing, no rebuild churn. For code changes
   that can't be env-gated, build two binaries and A/B via `TT_BIN=` (wire in the
   binary path) or swap `bin/turbo-test-<plat>` between runs.

## Modes
| mode | what | use for |
|---|---|---|
| `micro` | N stride-sampled files, R warm runs, median wall | quick read of a change |
| `ab` | paired interleaved A/B, trimmed-mean delta + win rate | the **decision** for >=2% |
| `full` | whole suite, real config | accuracy + real-world validation |
| `profile` | macOS `sample` → hot-symbol buckets | finding the next hot spot |

Options: `--sub N` (file sample size; `0`/large ⇒ all), `--jobs J` (default 1 for
measurement; omit-as-default uses cores), `--runs R`, `--pairs P`, `--trim T`
(pairs dropped each end before the trimmed mean). `SUB_ROOT=dir` overrides the
`src` search root. Args after `--` pass through to the turbo-test CLI.

## Profiling the all-Rust DOM (`TURBO_RUST_DOM=1`)
Set `TURBO_RUST_DOM=1` in the env before any mode to profile/benchmark the native-DOM path:
```sh
TURBO_RUST_DOM=1 scripts/perf/harness.sh profile ../ui-design-components --sub 200 --jobs 1
```
`profile` buckets are tuned for this path and print samples + % of BUSY (non-wait):
`V8 parse/compile`, `JS execution`, `GC`, `malloc/free`, `IC/props`, `native DOM (rtdom)`.

### Hotspot map (warm suite, measured 2026-06)
| bucket | %busy | note |
|---|---|---|
| **V8 parse/compile** | **~36–45%** | the hotspot. Each fresh per-file isolate deserializes the (shared, warm) bytecode cache AND lazy-compiles inner React/MUI closures on first call. |
| JS execution | ~15% | baseline/builtins running the test |
| IC / GC / malloc | ~6–7% each | property access, scavenge, node-wrapper allocs |
| native DOM (rtdom) | ~1% | the Rust DOM itself is cheap |

Findings that shape the next perf push:
- The on-disk **bytecode cache already shares dep modules across files** (key = source hash; a dep's
  `cc-<hash>.bin` is reused by every test file — verified: 6 distinct files added 0 new cc entries).
  So compilation is cached; the residual cost is V8 **re-deserializing + lazy-compiling into each
  fresh per-file isolate**.
- **Isolate-reuse** (`TURBO_REUSE_ISOLATE=1`) now buys ~2% (not the old ~1.5×) — the warm cc cache
  already captures most of what reuse used to save.
- **Eager code-cache** (`EagerCompile`) only shaved ~36% vs ~45% on the parse/compile bucket while
  bloating the cache; reverted. Not the lever.
- The remaining big lever is eliminating per-file deps cost entirely — a **V8 startup snapshot**
  (deps pre-compiled AND pre-instantiated, each file forks a context from it). Large + V8-snapshot
  constraints with React/MUI host state; not yet attempted.

> Measurement caveat: this box runs a VM (`com.apple.Virtualization` ~20% CPU) under long uptime,
> so jobs=1 walls show ~2× outliers mid-run. Validate any candidate win on a quiet box (golden rule
> #2) before believing a sub-20% delta.

## Files
- `harness.sh` — the tool (canonical).
- `bench.sh`, `bench-ab.sh` — thin back-compat shims forwarding to `harness.sh`.
