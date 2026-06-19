# turbo-test

**A blazing-fast native test runner — a drop-in replacement for [vitest](https://vitest.dev).**

Written in Rust on V8: per-file transforms via [oxc](https://oxc.rs)/esbuild, an **all-Rust DOM**
([turbo-dom](https://crates.io/crates/turbo-dom)'s `rtdom`, bound natively to V8 — no jsdom, no
Node), work-stealing parallelism, and an optional isolate-reuse mode. Runs your existing
`*.test.ts(x)` files — same `describe`/`it`/`expect`/`vi`, same `@testing-library/react` + jest-dom —
typically **~5–12× faster than vitest+jsdom**. Also runs **jest** suites (`jest` global + jest config
+ `emitDecoratorMetadata`) — see [Jest compatibility](#jest-compatibility-experimental) (experimental).

## Running it (drop-in for vitest)

No config changes — turbo-test reads your existing `vitest.config.*` (include/exclude, environment,
coverage thresholds) and runs the same test files:

```bash
npm i -D @miaskiewicz/turbo-test
npx turbo-test                       # discover + run every *.test.* / *.spec.* under cwd
npx turbo-test src/foo.test.ts       # run specific files
npx turbo-test --changed             # only files affected by your git diff
npx turbo-test --jobs 8              # parallelism (default = cores)
npx turbo-test --coverage            # native V8 coverage (honors the config's thresholds)
npx turbo-test --reporter json       # json | tap | default
```

Swap it into CI/scripts by replacing `vitest run` with `turbo-test`:

```jsonc
// package.json
"scripts": {
  "test": "turbo-test",              // was: "vitest run"
  "test:cov": "turbo-test --coverage"
}
```

The DOM is on by default for `jsdom`/`happy-dom`/`turbo-dom` environments (and any file with a
`// @vitest-environment jsdom` pragma); `environment: 'node'` files get clean globals, just like
vitest. There is **no environment to install or configure** — the Rust DOM ships inside the binary.

## Benchmarks

Two real production app suites, **same machine, same session, identical pass counts** (Apple
M-series, 8 workers), each the **median of repeated full-suite runs**. To keep it apples-to-apples
we measured vitest two ways — with stock **jsdom** (the default most projects run) and with
**turbo-dom** swapped in as the environment — so you can see the DOM's contribution separately from
the runner's; turbo-test ran under its all-Rust DOM (`TURBO_RUST_DOM`):

| Suite | Tests | vitest + jsdom | vitest + turbo-dom | **turbo-test** | vs jsdom | vs turbo-dom |
|---|---|---|---|---|---|---|
| **payroll-app** | 10,471 | ~252s | ~88s | **~20s** | **~12×** | ~4.4× |
| **ui-design-components** | 7,062 | ~173s | ~142s | **~36s** | **~4.8×** | ~3.9× |

`ui-design-components` also runs under turbo-test's optional **isolate-reuse** mode (see below).
On a quieter box an earlier measurement landed it at **~1.5–1.6× over fresh isolation** vs the same
jsdom baseline — reuse stays opt-in because it changes per-file `vi.mock` semantics for projects
that mock node_modules (see [Isolate-reuse](#isolate-reuse-extra-speed-zero-config)).

All configs pass 100% (**10,471/0** and **7,062/0** — both suites run green under turbo-test's
all-Rust DOM). Two takeaways:

- **The DOM matters.** Just swapping jsdom → turbo-dom under plain vitest already cuts wall time
  ~2.1–2.7× (jsdom's `environment` setup alone was **718s cumulative** across workers on payroll
  vs turbo-dom's **33s**). If you can't switch runners yet, switching the environment is free speed.
- **The runner matters more.** turbo-test's native transform + per-package dep-bundling + V8
  worker pool collapses vitest's `setup + import` cost (hundreds of seconds cumulative) and lands
  **~5–12× faster than the jsdom baseline** — with zero config changes.

> ⚠️ **Tentative numbers** — re-measured on the all-Rust-DOM build (`TURBO_RUST_DOM`) on a busy
> long-uptime workstation (a background VM held ~20% CPU), so absolute seconds run high and variance
> is large (±30%+ per run): the medians here are placeholders pending a clean-box interleaved re-run.
> The **ratios** are what travel, and even compressed by contention turbo-test lands multiples ahead
> of both baselines (the ui-design-components jsdom-vs-turbo-dom split is the noisiest cell — it
> should be wider, as payroll shows). Each column is the median of the suite's runs.

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

Both the injected globals **and** the explicit-import form work — a suite with `injectGlobals: false`
that does `import { describe, it, expect, jest } from '@jest/globals'` resolves that specifier from
turbo-test's own runtime (never the real `@jest/globals` package), the same builtin that backs
`import … from 'vitest'`.

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

## TypeScript types (drop the `vitest` devDependency)

turbo-test injects `describe` / `it` / `test` / `expect` / `vi` / `jest` and the hook functions as
globals at run time, and resolves `import … from 'vitest'` (and `'@jest/globals'`) from its own
runtime. To make `tsc --noEmit` type-check that surface **without** keeping `vitest` (or
`@types/jest`) in `devDependencies` purely for types, turbo-test ships its own `.d.ts` bundle:

- **`@miaskiewicz/turbo-test/globals`** — ambient globals; the drop-in for `types: ["vitest/globals"]`.
- **`types/vitest.d.ts`** — the `vitest` module surface (for `import … from 'vitest'`).
- **`types/jest-globals.d.ts`** — the `@jest/globals` module surface (for `import … from '@jest/globals'`).

Wire it up in `tsconfig.json` — swap the `types` entry and add `paths` so the bare specifiers
resolve to the shipped shims instead of `node_modules`:

```jsonc
{
  "compilerOptions": {
    "types": ["@miaskiewicz/turbo-test/globals"],   // was: ["vitest/globals"]
    "paths": {
      "vitest": ["./node_modules/@miaskiewicz/turbo-test/types/vitest.d.ts"],
      "@jest/globals": ["./node_modules/@miaskiewicz/turbo-test/types/jest-globals.d.ts"]
    }
  }
}
```

> **Subset, by design.** These are a pragmatic vitest/jest-**compatible** subset — matcher and mock
> signatures widen arguments to `any`, so real test code type-checks and no *false* errors are
> introduced, at the cost of some of the precise matcher-argument inference upstream `vitest` gives
> you. If you need the exact upstream types, keep `vitest` as a **types-only** devDependency instead
> (it never has to be in the runtime path).

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
