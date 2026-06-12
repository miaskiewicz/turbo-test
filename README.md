# turbo-test

**A blazing-fast native test runner — a drop-in replacement for [vitest](https://vitest.dev).**

Written in Rust on V8: per-file transforms via [oxc](https://oxc.rs)/esbuild, a native
[turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) DOM, work-stealing parallelism,
and an optional isolate-reuse mode. Runs your existing `*.test.ts(x)` files — same `describe`/`it`/
`expect`/`vi`, same `@testing-library/react` + jest-dom — typically **~6× faster than vitest+jsdom**.

```bash
npm i -D @miaskiewicz/turbo-test
npx turbo-test            # discovers + runs every *.test.* / *.spec.* under cwd
npx turbo-test src/foo.test.ts --jobs 8 --reporter json
```

## Benchmarks

Two real production app suites, **same machine, same session, identical pass counts** (Apple
M-series, 10 workers). To keep it apples-to-apples we measured vitest two ways — with stock
**jsdom** (the default most projects run) and with **turbo-dom** swapped in as the environment —
so you can see the DOM's contribution separately from the runner's:

| Suite | Tests | vitest + jsdom | vitest + turbo-dom | **turbo-test** | vs jsdom | vs turbo-dom |
|---|---|---|---|---|---|---|
| **payroll-app** | 10,006 | 296s | 130s | **51s** | **5.8×** | 2.6× |
| **ui-design-components** | 6,189 | 428s | 358s | **76s** | **5.6×** | 4.7× |

`ui-design-components` also runs under turbo-test's optional **isolate-reuse** mode (see below).
On a less-loaded box it lands **46.8s — 9.1× vs vitest+jsdom**, ~1.6× over fresh isolation, still
6189/0:

| ui-design-components | jsdom | turbo-dom | turbo-test fresh | **turbo-test reuse** |
|---|---|---|---|---|
| 6,189 tests | 428s | 358s | 76s | **46.8s (9.1× vs jsdom)** |

All configs pass 100% (10006/0 and 6189/0). Two takeaways:

- **The DOM matters.** Just swapping jsdom → turbo-dom under plain vitest already cuts wall time
  ~1.2–2.3× (jsdom's `environment` setup alone was **1228s cumulative** across workers on payroll
  vs turbo-dom's **96s**). If you can't switch runners yet, switching the environment is free speed.
- **The runner matters more.** turbo-test's native transform + per-package dep-bundling + V8
  worker pool collapses vitest's `setup + import` cost (hundreds of seconds cumulative) and lands
  **~6× faster than the jsdom baseline** — with zero config changes.

> Numbers are from a busy long-uptime workstation, so absolute seconds run high; the **ratios**
> are what travel. Reuse mode's win over fresh scales with how loaded the machine is — under heavy
> contention the two converge (~80s); on a quieter box reuse pulls ahead (46.8s above).

## Isolate-reuse (extra speed, zero config)

By default turbo-test isolates every file (like vitest's `isolate: true`). If your vitest config
sets `isolate: false`, turbo-test honors it and **reuses one V8 isolate per worker** — node_modules
barrels evaluate once instead of per file. Any file that fails under reuse is automatically re-run
on a clean isolate, so correctness always matches isolated mode (still 6189/0). How much it buys
depends on how barrel-heavy your imports are and how loaded the machine is — on a quiet box it can
beat fresh isolation handily; under heavy contention it converges with it.

Force it on/off regardless of config:

```bash
TURBO_REUSE_ISOLATE=1 npx turbo-test   # force reuse on
TURBO_NO_REUSE=1       npx turbo-test   # force fresh isolation
```

## How it works

- **Transform** — per-module oxc/esbuild → CJS; node_modules bundled once per package (Vite-style),
  cached on disk (99% hit on warm runs).
- **DOM** — [turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) via a Node-API host
  (native html5ever parser + lazy copy-on-write DOM). Per-file env setup ~1–2 ms.
- **Mocks** — `vi.mock` (sync + async factories), hoisting, `vi.spyOn`, `vi.fn`, `vi.hoisted`,
  `importActual`, jest-dom matchers — all supported.
- **Parallelism** — work-stealing across `--jobs` workers; duration-aware slowest-first ordering.

## Coverage

```bash
npx turbo-test --coverage                       # line coverage → coverage/lcov.info + a summary
npx turbo-test --coverage-dir build/cov         # custom output dir (implies --coverage)
```

Coverage uses **V8's native precise coverage** (the engine's own per-function/block counters via
the Inspector `Profiler` domain — the same source c8 uses) — not Istanbul-style source
instrumentation. Byte ranges are mapped back to your original `.ts`/`.tsx` lines through esbuild
source maps, emitted only under `--coverage`. Reports **line + function** coverage as standard
**lcov** (`coverage/lcov.info` — `DA`/`LF`/`LH` + `FN`/`FNDA`/`FNF`/`FNH`; consumable by Codecov,
`genhtml`, VS Code Coverage Gutters, etc.) plus a terminal summary:

```
 Coverage — 11 files (lines | funcs)
  100.00% lines  100.00% fns   src/analytics/useAnalytics.ts
   97.31% lines   85.00% fns   src/theme/components.ts
  ------
   99.13% lines (796/803)   82.93% fns (34/41)   → .../coverage/lcov.info
```

node_modules and test/spec files are excluded. Coverage runs the fresh isolation path.

**Speed.** Collection is opt-in and runs ~2–3× slower than a plain run (V8 block coverage keeps
code un-optimized to count blocks) — in line with vitest's own coverage. Normal runs are completely
unaffected (no `--coverage` → no inspector, no source maps, identical transform cache). Branch
coverage and a per-worker speedup for `isolate: false` projects are tracked in
[`docs/COVERAGE-FUNC-BRANCH-BACKLOG.md`](docs/COVERAGE-FUNC-BRANCH-BACKLOG.md).

**Precision vs Istanbul.** This is V8 coverage, so execution *counts* are exact and collection is
near-free — but it measures compiled bytecode mapped back through source maps, where Istanbul
instruments the source AST directly. At the **line** level the two are comparable (our esbuild maps
are clean); for **branch**-level attribution Istanbul is finer. turbo-test currently reports
**line coverage only** — if you need exhaustive branch metrics, keep an Istanbul/vitest coverage
job for that gate and use turbo-test's coverage for fast everyday line feedback.

## CLI

```
turbo-test [files...] [--jobs N] [--shard i/n] [--reporter json] [--coverage] [--coverage-dir DIR]
```

No file args → discovers test files. If a `vitest.config.*` is found, turbo-test honors its
**`test.include` / `test.exclude`** globs (so e.g. Playwright `*.spec.ts` under `e2e/` are skipped
exactly like vitest); otherwise it falls back to `**/*.{test,spec}.{ts,tsx,js,jsx,mts,cts}`
(skipping `node_modules`, `dist`, `build`, etc.). Flags pass through to the native binary.

## Programmatic

```js
const { run } = require('@miaskiewicz/turbo-test');
const { status } = run(['src/a.test.ts'], { jobs: 8, env: { TURBO_REUSE_ISOLATE: '1' } });
process.exit(status);
```

## Config

turbo-test reads your project's `vitest.config.ts` for `setupFiles`, `environment`, `isolate`, and
`test.include` / `test.exclude`. No separate config needed for most suites.

## Compatibility notes

- A handful of vitest features that depend on a full Node runtime are stubbed/approximated; e2e
  helper files that import heavy Node-only packages (e.g. `@playwright/test`) get those deps
  stubbed so the file still runs.
- `Date.now()` reflects a real base + the virtual event-loop clock, so `setTimeout`-driven elapsed
  assertions pass deterministically.

## Requirements

`node >= 18`. Prebuilt binaries ship for macOS (arm64/x64), Linux (x64/arm64-gnu), and Windows x64.
Other platforms: build from source with a Rust toolchain (`npm run build`).

## License

MIT
