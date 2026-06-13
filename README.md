# turbo-test

**A blazing-fast native test runner — a drop-in replacement for [vitest](https://vitest.dev).**

Written in Rust on V8: per-file transforms via [oxc](https://oxc.rs)/esbuild, a native
[turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) DOM, work-stealing parallelism,
and an optional isolate-reuse mode. Runs your existing `*.test.ts(x)` files — same `describe`/`it`/
`expect`/`vi`, same `@testing-library/react` + jest-dom — typically **~6× faster than vitest+jsdom**.
Also runs **jest** suites (`jest` global + jest config + `emitDecoratorMetadata`) — see
[Jest compatibility](#jest-compatibility-experimental) (experimental).

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

## Jest compatibility (experimental)

turbo-test also runs **jest** suites — no rewrite to `vi`. A `jest` global maps onto the same
machinery as `vi` (`jest.fn`, `jest.mock`/`doMock`, `jest.spyOn`, `jest.mocked`,
`clearAllMocks`/`resetAllMocks`/`restoreAllMocks`, fake timers, `jest.requireActual`,
`jest.isolateModules`, …); the type-only members (`jest.Mock`, `jest.Mocked`, `jest.SpyInstance`)
are erased by the transform. When there's **no vitest config**, the nearest **jest config**
(`jest.config.{js,cjs,mjs,ts,json}` or a `package.json` `"jest"` block) is read for `setupFiles` /
`setupFilesAfterEnv` (with `<rootDir>` resolved), so your existing setup runs unchanged.

**`emitDecoratorMetadata`.** esbuild can't emit decorator metadata, which NestJS / Mongoose /
Sequelize need at runtime (`@Injectable` constructor injection, `@Prop`/`@Column` reading
`design:type`). turbo-test handles it with **retry-on-load**: files transform on fast esbuild by
default, and only a decorator file that actually *throws* at load (a missing-metadata error) is
re-transformed through the project's own TypeScript (`ts.transpileModule`, exact ts-jest parity —
local type aliases like `type Percentage = number` resolve to `Number`, not `Object`), falling back
to oxc's metadata transform when the project ships no `typescript`. The common path never pays the
cost and never regresses.

```bash
npx turbo-test            # discovers *.spec.* too; reads jest config when no vitest config exists
```

> **Status — experimental.** Validated against a NestJS + ts-jest backend (~7,150 tests). On that
> suite turbo-test runs in **~60s vs jest's ~117s (~1.9×)** on a clean box. Pass coverage is
> partial today (**~4,500 / 7,147 green, ~63%**): the jest *shim*, jest-config reading, decorator
> metadata, and CommonJS-first resolution (sequelize-typescript, tslib, …) all work. The rest leans
> on node-native modules that bare V8 can't run yet — the `mongodb` driver, real OpenSSL crypto
> (`createCipheriv`/RSA), full ICU `Intl` time zones — which need a Node-compat native layer.
> React/vitest suites (payroll-app 10006/0, ui-design 6189/0) are **unaffected** — CommonJS-first
> resolution is scoped to jest projects with a node test environment.

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
npx turbo-test --coverage                       # line+func+branch+stmt → coverage/lcov.info, coverage-summary.json + a summary
npx turbo-test --coverage-dir build/cov         # custom output dir (implies --coverage)

# gate CI on thresholds (non-zero exit when unmet); --coverage-per-file applies them to each file
npx turbo-test --coverage --coverage-per-file \
  --coverage-thresholds lines=90,functions=80,branches=80,statements=90

# pick reporters (default: lcov,json-summary,text). html is opt-in.
npx turbo-test --coverage --coverage-reporter lcov,json-summary,text,html
```

Coverage uses **V8's native precise coverage** (the engine's own per-function/block counters via
the Inspector `Profiler` domain — the same source c8 uses) — not Istanbul-style source
instrumentation. Byte ranges are mapped back to your original `.ts`/`.tsx` lines through esbuild
source maps, emitted only under `--coverage`. Reports **line + function + branch + statement**
coverage as standard **lcov** (`coverage/lcov.info` — `DA`/`LF`/`LH`, `FN`/`FNDA`/`FNF`/`FNH`,
`BRDA`/`BRF`/`BRH`; consumable by Codecov, `genhtml`, VS Code Coverage Gutters, etc.) plus a
terminal summary:

```
 Coverage — 11 files (lines | funcs | branches | stmts)
  100.00% ln  100.00% fn  100.00% br  100.00% st   src/analytics/useAnalytics.ts
   97.31% ln   85.00% fn   78.00% br   96.40% st   src/theme/components.ts
  ------
   99.13% lines (796/803)   82.93% fns (34/41)   80.00% branches (...)   98.7% stmts (...)   → .../coverage/lcov.info
```

Branch coverage parses each source file with [oxc](https://oxc.rs) to find decision points
(`if`/`else`, `?:`, `&&`/`||`/`??`, `switch`) and correlates each arm with V8's block counts mapped
back through the source map — so it's real per-arm branch data (not block-as-branch). **Statement**
coverage reuses that same oxc pass (one parse, no extra cost) to locate each **executable**
statement and correlate it with V8's covered ranges. Following Istanbul, declarations are not
statements — `import`/`export`, `function`/`class` declarations (those are function coverage), and
TS type-only decls are excluded; the statements inside them still count. lcov has no statement
field, so statements appear in the json-summary / text / html reporters only.

node_modules and test/spec files are excluded. Coverage runs the fresh isolation path.

**Thresholds & gating.** `--coverage-thresholds lines=90,functions=80,branches=80` fails the run
(non-zero exit) when any metric is unmet; add `--coverage-per-file` to enforce them on *every*
reported file (offending files + the failing metric are printed). When a `vitest.config.*` defines
`test.coverage.thresholds`, those numbers are honored automatically — flags are optional and win
when both are present. Gateable metrics are **lines / functions / branches / statements**.
Under `--coverage`, **0 instrumented files is a hard failure** (non-zero exit), never a vacuous
`0/0` pass — so a misconfigured `include` that covers nothing can't show up as green.

**Reporters.** `--coverage-reporter` takes a comma list — `lcov`, `json-summary`, `text`, `html`
(default `lcov,json-summary,text`). `json-summary` writes a vitest/c8-shaped
`coverage-summary.json` (`total` + per-absolute-path `{lines,statements,functions,branches}` with
`{total,covered,pct}`); `html` writes a browsable `coverage/index.html`.

**Scoping the report.** `test.coverage.include` / `test.coverage.exclude` globs from the vitest
config are applied to the report set automatically (or pass `--coverage-include` /
`--coverage-exclude` directly). Globs support `**`, `*`, `?`, and `{a,b}` brace alternation
(`src/**/*.{ts,tsx}`). To exempt a single file, add a
`/* turbo-test-coverage-ignore-file */` comment near its top.

**Speed.** Collection is opt-in and runs ~2–3× slower than a plain run (V8 block coverage keeps
code un-optimized to count blocks) — in line with vitest's own coverage. Normal runs are completely
unaffected (no `--coverage` → no inspector, no source maps, identical transform cache). Branch
coverage and a per-worker speedup for `isolate: false` projects are tracked in
[`docs/COVERAGE-FUNC-BRANCH-BACKLOG.md`](docs/COVERAGE-FUNC-BRANCH-BACKLOG.md).

**Precision vs Istanbul.** This is V8 coverage, so execution *counts* are exact and collection is
near-free — but it measures compiled bytecode mapped back through source maps, where Istanbul
instruments the source AST directly. At the **line** level the two are comparable (our esbuild maps
are clean). For **branches**, turbo-test pairs oxc decision points with V8 block counts for real
per-arm data — including braceless early-return `if (c) return x;` (the implicit-else arm is
derived from the block/continuation counts, not stuck at 0). Istanbul still resolves a few exotic
shapes more finely; if you need that exhaustive gate, keep an Istanbul/vitest job alongside.

## CLI

```
turbo-test [files...] [--jobs N] [--shard i/n] [--reporter json]
           [--coverage] [--coverage-dir DIR]
           [--coverage-thresholds lines=,functions=,branches=] [--coverage-per-file]
           [--coverage-reporter lcov,json-summary,text,html]
           [--coverage-include GLOB] [--coverage-exclude GLOB]
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
