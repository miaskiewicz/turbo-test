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

## Files
- `harness.sh` — the tool (canonical).
- `bench.sh`, `bench-ab.sh` — thin back-compat shims forwarding to `harness.sh`.
