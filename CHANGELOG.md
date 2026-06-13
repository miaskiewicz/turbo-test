# Changelog

All notable changes to `@miaskiewicz/turbo-test`. Format based on
[Keep a Changelog](https://keepachangelog.com/); this project uses semver-ish `0.2.x`.

## [Unreleased]

Perf-spike work (no runtime default changes — all opt-in env gates, behavior identical to 0.2.12):

### Added
- `scripts/perf/` ultra harness: `harness.sh` (micro / paired-A/B / full / profile modes),
  `accuracy-diff.sh` (per-file pass/fail diff between two runner configs), and a README. Validated
  paired-A/B noise floor of ±0.4%.
- Env gates for rebuild-free A/B sweeps: `TURBO_V8_FLAGS` (V8 flag string), `TURBO_JOBS` (worker
  count), `TURBO_SNAP_KEEP` (bake framework bytecode into the snapshot), `TURBO_NO_CODE_CACHE`.
- `docs/`: `perf-spike.md` (experiment log), `reuse-spike.md` (isolate-reuse verdict),
  `TODO-cache-poisoning.md` (interrupted-bundle-write cache poisoning bug + fix plan).

### Investigated (not shipped)
- Isolate-reuse as default: **rejected** — accurate + faster on some suites (ui 7006/0) but breaks
  others (payroll: per-file `vi.mock` of node_modules is fundamentally incompatible with caching
  node_modules across files). Stays opt-in (`TURBO_REUSE_ISOLATE` / vitest `isolate: false`).
- Worker count `ncpu→0.75·ncpu`: **rejected** — looked like −14% on a warm microbench but +40–75%
  SLOWER on the full suite. Default stays `ncpu`. (Lesson: measure concurrency changes full-suite.)
- Snapshot bytecode bake (`Keep`): mixed (ui −3.6%, payroll neutral/slower); kept opt-in.
- V8 GC/heap flags, in-memory dep-bundle memo: rejected (slower).

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
