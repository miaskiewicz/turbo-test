# Changelog

All notable changes to `@miaskiewicz/turbo-test`. Format based on
[Keep a Changelog](https://keepachangelog.com/); this project uses semver-ish `0.2.x`.

## [Unreleased]

## [0.2.15] — bump turbo-dom to 0.2.2

### Changed
- **Bump `@miaskiewicz/turbo-dom` `^0.2.0` → `^0.2.2`.** Pulls in the latest published DOM
  runtime so consumers installing fresh resolve the current line rather than the older 0.2.0
  floor. No turbo-test source changes; patch release to ship the updated dependency range.

## [0.2.14] — E12: memoize module resolution (tsconfig + specifier resolve)

### Performance
- **Memoize `nearest_tsconfig` and `resolve_spec_as` per worker** (thread-local). `native_require`
  was the second-hottest path in the profile and re-walked the directory tree (`is_file` syscalls)
  for the nearest `tsconfig.json` on *every* resolve, and re-ran the full oxc resolve + canonicalize
  for the same `(specifier, importer-dir)` across every file that imports it. Both are deterministic
  for a run, so caching them turns repeats into a hashmap hit and cuts the FS-syscall + resolution
  cost. **Default ON** (disable with `TURBO_NO_E12`).
  - Paired A/B vs the validated noise floor, identical pass/fail on both suites (accuracy-diff:
    ui 431/431 and payroll 1072/1072 files byte-identical pass/fail):
    micro ui −13.8% / payroll −14.1% (20/20); **full-suite ui −7.4% (7/8) / payroll −13.9% (4/4)**.

### Added
- `scripts/perf/harness.sh --alt`: alternate A/B run order per pair. Cancels a deterministic
  *within-pair* thermal bias (at `--jobs 1` the second run of each pair was ~46% slower as the
  pinned P-core throttled — a control-vs-control run showed +46%/0-of-12, pure artifact). The
  earlier "validated ±0.4% noise floor" was a **jobs=8** measurement; jobs=8 has only random
  variance (cancels over pairs), jobs=1 has this deterministic bias (does not). Use jobs=8 or
  `--alt` for per-file micro A/B.

### Investigated (not shipped)
- **E1 V8 `--max-semi-space-size=64`**: re-benchmarked on a quiet machine — neutral-to-slightly
  slower (+2.7% at jobs=8). The earlier "−10% promising" was jobs=1 thermal-bias inflation. Killed.
- **E4 worker count** (`ncpu→7/6`): full-suite +15.9% / +59.8% slower. Killed (reconfirmed).
- **E10 V8 platform helper-pool size**: capping `new_default_platform(N,…)` to 2/4 was +11.3% /
  +5.3% slower full-suite — fewer GC helpers idles concurrent-GC parallelism. Killed. (Both
  concurrency levers — worker count and helper pool — are closed: ncpu workers + ncpu helpers is
  the full-cold-suite optimum.)
- **E6 transform-existence memo**: neutral (+0.0%). **E11 drain-loop fn-lookup hoist**: +3.9%
  (the per-iteration `v8::String::new` churn is negligible vs total). Neither shipped.

### Added (earlier perf-spike, behavior identical to 0.2.12)
- `scripts/perf/` harness (`harness.sh` micro/ab/full/profile, `accuracy-diff.sh`) + README.
- Sweep env gates: `TURBO_V8_FLAGS`, `TURBO_JOBS`, `TURBO_SNAP_KEEP`, `TURBO_NO_CODE_CACHE`.
- `docs/`: `perf-spike.md`, `reuse-spike.md`, `TODO-cache-poisoning.md`.

### Investigated earlier (not shipped)
- Isolate-reuse as default: **rejected** — faster on some suites (ui 7006/0) but breaks payroll
  (per-file `vi.mock` of node_modules is incompatible with caching node_modules across files).
  Stays opt-in (`TURBO_REUSE_ISOLATE` / vitest `isolate: false`).
- Snapshot bytecode bake (`Keep`): mixed (ui −3.6%, payroll neutral/slower); kept opt-in.
- In-memory dep-bundle memo: rejected (slower — clone beats OS-page-cached read).

## [0.2.13] — `toEqual` undefined-own-property stripping
### Fixed
- `toEqual` / `toHaveBeenCalledWith` / `toStrictEqual`'s negation and all the matchers backed by
  the deep-equality path now follow jest/vitest semantics: an own property whose value is
  `undefined` is treated as **absent**, so `{ a: 1, b: undefined }` deep-equals `{ a: 1 }`.
  Previously `deepEqual` compared raw `Object.keys().length`, so any received object carrying an
  explicit-`undefined` key failed against an expectation that omitted it — and because the
  `undefined` value isn't rendered, the printed `expected` and `received` were byte-identical,
  making the failure look impossible. (`BUG-equality-state-leak-across-shard-files.md`.)
- The bug was reported as a shard/worker **state leak** (intermittent, order-dependent). It is not:
  the gap is deterministic and fires whenever the received object has an explicit-`undefined` own
  property. It only *looked* shard-dependent because whether the asserted object carried such a key
  depended on which code path the consumer's earlier files exercised.
- `toStrictEqual` keeps `undefined`-valued keys significant (now via an explicit `strict` flag),
  matching jest/vitest. Array elements are never stripped in either mode — `[1, undefined]` still
  differs from `[1]` because the length differs.
### Tests
- `fixtures/tests/equality-undefined-strip.test.mjs` — regression covering the reported
  `toHaveBeenCalledWith({...})` shape, nested objects, `toStrictEqual`, and array non-stripping.

## [0.2.12] — V8 bytecode code-cache
### Added
- Persistent V8 code (bytecode) cache for compiled CJS module wrappers, keyed by the exact wrapped
  source, consumed across isolates/workers/runs. A fresh isolate per test file otherwise re-parses
  + re-compiles every required module (incl. node_modules barrels) from scratch. On by default;
  disable with `TURBO_NO_CODE_CACHE`; skipped under coverage. Safe: V8 rejects a stale/mismatched
  cache and recompiles.
### Performance
- ~1.5–1.8% faster warm (paired A/B); identical pass/fail (payroll 10580/1, ui 7006/0).

## [0.2.11] — experimental jest drop-in
### Added
- Experimental jest compatibility: `jest`-global shim, jest config reading, `emitDecoratorMetadata`
  support, CJS-first resolution path (sequelize/tslib/lexical get their `require`-condition build).

## [0.2.10] — mock + dual-React fixes
### Fixed
- Dual-React instance in setup-file bundles (mock factories now use the test's React).
- `vi.mock` factories that close over outer `let`s (shared via a routed global).
- ESM deps resolved under the wrong node export condition.

## [0.2.9] — async self-importing mock factories
### Fixed
- An async `vi.mock` factory that `await import('<self>')` now rebinds named imports to the spies.

## [0.2.8] — statements coverage accuracy
### Fixed
- Statements coverage previously counted declarations; now Istanbul-accurate (executable
  statements only, correlated to V8 covered ranges).

## [0.2.7] — coverage globs + fail-loud
### Fixed
- Brace globs (`{ts,tsx}`) in `coverage.include` were torn into malformed globs; the multi-glob
  separator is now a top-level comma only.
### Changed
- Under `--coverage`, 0 instrumented files is a hard FAIL (non-zero exit) instead of a vacuous 0/0.

## [0.2.6] — statements coverage
### Added
- `statements` coverage metric, derived via oxc (shared parse with the branch pass) correlated to
  V8 covered ranges. Appears in json-summary / text / html and is gateable (omitted from lcov).

## [0.2.5] — platform reporting
### Fixed
- Report the real host platform/arch so turbo-dom loads the matching prebuilt `.node` (previously
  reported darwin/arm64 everywhere → wrong `.node` on other hosts → "document is not defined").

## [0.2.4] — linux-x64 startup
### Fixed
- linux-x64 startup segfault (`e_entry=0` from a misparsed linker flag).

## [0.2.3] — coverage gating + reporters
### Added
- Coverage thresholds / gating; `json-summary` and `html` reporters.
### Fixed
- A branch-coverage correlation bug.

## [0.2.2] — function + branch coverage
### Added
- Function and branch coverage (oxc AST decision points correlated with V8 block counts).
### Fixed
- Config parser matched `setupFiles` / `isolate` inside comments instead of the real keys.

## [0.2.1] — native V8 coverage
### Added
- Native V8 line + function coverage via the Inspector Profiler; honors vitest
  `coverage.include` / `exclude`.

## [0.2.0] — initial public release
First published `@miaskiewicz/turbo-test`: a native Rust + V8 vitest-compatible test runner.
### Added
- Native multi-worker runner: one V8 isolate per worker booted from a shared framework snapshot,
  work-stealing scheduler, duration-aware slowest-first ordering.
- vitest-compatible surface: `describe`/`it`/`expect`, `vi.mock`/`vi.fn`/`vi.spyOn`, hooks, timers,
  module-runner CJS loading with live bindings + shared React, vitest config honoring
  (include/exclude, setupFiles, environment).
- Isolate-reuse mode (vitest `isolate: false` / `TURBO_REUSE_ISOLATE`) with a fresh-isolate retry
  net; node builtin shims, virtual clock, e2e dep-stubbing.
- npm distribution: `cli.js` launcher over prebuilt per-platform binaries, CI matrix build +
  tag-triggered publish (macOS arm64, Linux x64 gnu/musl, Windows x64).
- Validated against real suites: payroll-app 100%, ui-design-components 6189/0; ~5.6–9× vs
  vitest+jsdom in apples-to-apples benchmarks.
