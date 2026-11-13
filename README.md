# turbo-test

**A native test runner for vitest suites — a drop-in replacement for [vitest](https://vitest.dev)
(and, experimentally, jest).** Release candidate **0.8.4** (unreleased; the published release is **0.8.3**).

turbo-test is written in Rust on V8: per-file transforms through [oxc](https://oxc.rs) / esbuild,
work-stealing parallelism, native V8 coverage, an optional isolate-reuse mode, and an all-Rust DOM
bound directly to V8 (no jsdom, no Node). It runs your existing `*.test.ts(x)` files with the same
`describe` / `it` / `expect` / `vi`, `@testing-library/react` and jest-dom; the `vi`, mock and
`jest` surface follows vitest 4.1.12 and jest 30.5.2.

> **For AI agents and humans running the common workflows:** see [`docs/AGENTS.md`](docs/AGENTS.md) and the one-shot scripts under [`scripts/agent/`](scripts/agent).

## Where it sits: turbo-dom is the shared base

Since 0.6.0 the browser engine lives in one place,
[miaskiewicz/turbo-dom](https://github.com/miaskiewicz/turbo-dom), and three products consume it:

| turbo-dom crate | what it is | used by |
|---|---|---|
| `turbo-dom` | the Rust DOM (`rtdom`, html5ever), selector engine, CSSOM | turbo-test, turbo-html2pdf, turbo-surf |
| `turbo-dom-browser-env` | `window` / `document` / CSSOM / forms / events bound to V8 | **turbo-test**, turbo-surf |
| `turbo-dom-v8` (150.4.x) | turbo-dom's rusty_v8 fork (interceptor `ShouldThrowOnError` fix) | **turbo-test**, turbo-surf |
| `turbo-dom-layout` | HTML/CSS layout | turbo-html2pdf, turbo-surf |
| `turbo-dom-raster` | synthetic screenshots | turbo-surf |

turbo-test links `turbo-dom-browser-env` 0.8.4 and `turbo-dom-v8 ~150.4` (under the local alias
`v8`; `scripts/check-single-v8.sh` keeps exactly one V8 in the graph). This repository carries no
copy of the binding or of V8: the binding, its probes and the Chrome-control collectors that
record their oracles live in turbo-dom's
[`crates/turbo-dom-browser-env`](https://github.com/miaskiewicz/turbo-dom/tree/main/crates/turbo-dom-browser-env)
— fix DOM behaviour and recollect controls there. turbo-dom-v8 150.4.1 and
turbo-dom / -layout / -browser-env 0.8.4 use an exact pre-publication git pin on this
release branch (`25f59eac79993c5851f39ed75c3c233d0df96a6b`). The V8 fork remains the published crates.io version. Drop the DOM
pin after the 0.8.4 crates are published and before tagging a runner release.

## Install and run

```bash
npm i -D @miaskiewicz/turbo-test
npx turbo-test                       # discover + run every *.test.* / *.spec.* under cwd
npx turbo-test src/foo.test.ts       # run specific files
npx turbo-test --changed             # only files affected by your git diff
npx turbo-test --jobs 8              # parallelism (default = cores)
npx turbo-test --coverage            # native V8 coverage (honors the config's thresholds)
npx turbo-test --reporter json       # default | verbose | json | junit | tap | dot
```

No config changes: turbo-test reads your existing `vitest.config.*` (include/exclude,
environment, setup files, isolate, aliases, coverage thresholds). Replace `vitest run` with
`turbo-test` in your scripts:

```jsonc
// package.json
"scripts": {
  "test": "turbo-test",              // was: "vitest run"
  "test:cov": "turbo-test --coverage"
}
```

The default and verbose reporters print vitest 4.1.12's output (under an AI agent, its agent
reporter); `TURBO_LEGACY_REPORTER=1` keeps turbo-test's compact `PASS` / `FAIL` lines.

The DOM is on for `jsdom` / `happy-dom` / `turbo-dom` environments (and any file with a
`// @vitest-environment jsdom` pragma); `environment: 'node'` files get clean globals, as in
vitest. There is no environment package to install — the DOM is linked into the binary.

### Switches

turbo-test has no Cargo feature flags; behaviour switches are environment variables:

| variable | effect |
|---|---|
| `TURBO_REUSE_ISOLATE=1` / `TURBO_NO_REUSE=1` | force isolate reuse on / off (see below) |
| `TURBO_LEGACY_REPORTER=1` | compact `PASS` / `FAIL` reporter instead of vitest's |
| `TURBO_ZERO_DELAY_TIMERS=1` / `=0` | zero-delay timers on (default) / off (see below) |
| `TURBO_CHROME_MEDIA=1` / `=0` | Chrome's media / jsdom's (default) (see below) |
| `TURBO_CSS_TRANSITIONS=1` / `=0` | Chrome's CSS transitions and animations / jsdom's none (default) (see below) |
| `TURBO_SNAPSHOT=0` | each DOM file runs turbo-dom's engine install instead of opening the window snapshot (see below) |

**Window snapshot** (on by default for runs with at least 8 DOM-environment files). The worker
isolates' startup blob holds the framework layer twice: alone, for `node`-environment files, and
with turbo-dom's window bootstrapped over it (turbo-dom#249). A DOM file opens the second context,
so its install rehydrates the window instead of running the engine scripts (about 20-30 ms per
file). Building the blob costs about 170 ms once per process, so smaller runs skip it.
`TURBO_SNAPSHOT=0` turns it off, and so does any `TURBO_V8_FLAGS` (code deserialized from the blob
can conflict with custom V8 flags).

**Single-threaded V8 with a larger young generation** (the default V8 flags). Each worker's
isolate compiles and collects garbage on its own thread (`--single-threaded`): with an isolate per
core, V8's background compile and GC helpers only contend with the other workers (payroll's full
run: about 9% less wall and CPU time). Its young generation starts at 32 MB
(`--min-semi-space-size=32`), so a test file's short-lived objects take fewer scavenges (3-8% less
wall time across the reference projects). `TURBO_V8_FLAGS` replaces the defaults (set it empty to
run V8 with none).

The startup blob (window or framework-only) is saved in the build's cache directory and reused by
later runs: a rebuilt blob is not byte-identical, and V8 rejects module bytecode caches made
against another blob, so without it every run recompiled its modules.

**Zero-delay timers** (on by default). A real `setTimeout(fn, 0)` (or any delay below 1 ms) takes
0 ms of the clock, as in turbo-test 0.4.2, which is faster for timer-heavy suites. It keeps the
order of Node's 1 ms timer (#237): a setImmediate or MessageChannel message queued alongside it
runs first, as on vitest, so a React render posted through the scheduler is committed when RTL's
`setTimeout(0)` drain resumes the test. Off, the delay is clamped to 1 ms as on Node and vitest
(#212), so `Date.now()` and `performance.now()` also move 1 ms per such timer. Promise jobs run
first in both modes, and fake timers are not affected.

Prefer the flag or the env var. `test.zeroDelayTimers` is not in vitest's `InlineConfig` type, so a
typed config needs `// @ts-expect-error` above it.

| source | on | off |
|---|---|---|
| CLI flag (wins) | `--zero-delay-timers` | `--no-zero-delay-timers` |
| env var | `TURBO_ZERO_DELAY_TIMERS=1` | `TURBO_ZERO_DELAY_TIMERS=0` |
| vitest config | `test: { zeroDelayTimers: true }` | `test: { zeroDelayTimers: false }` |

**Media: jsdom's by default, Chrome's on opt-in.** By default the DOM's media behaviour is
jsdom 27's, as on vitest with `environment: 'jsdom'`: `window.matchMedia`, `MediaQueryList` and
`MediaQueryListEvent` are undefined, and `getComputedStyle` applies an `@media` rule only when one
of its media list items is exactly `screen` (an `@import` also with no media list). So
breakpoint rules such as MUI's `sx={{ display: { xs: 'flex', md: 'none' } }}` never apply and
both a mobile and a desktop layout stay in the accessibility tree, which is what tests written
against vitest expect. Tests that need `matchMedia` mock it, as on jsdom.

Chrome's media (turbo-dom's `MediaMode::Chrome`, the behaviour turbo-dom 0.8.0 has for every
other embedder): `matchMedia` returns a live `MediaQueryList`, and `@media` / `@import` media
lists are evaluated against the 1024x768 viewport, as in Chrome 156. MUI's `md` rules apply at
that width, so mobile-only elements are `display: none`. In both modes a test's replacement
`window.matchMedia` changes what `useMediaQuery` sees, never which CSS rules apply.

| source | Chrome | jsdom (default) |
|---|---|---|
| CLI flag (wins) | `--chrome-media` | `--no-chrome-media` |
| env var | `TURBO_CHROME_MEDIA=1` | `TURBO_CHROME_MEDIA=0` |
| vitest config | `test: { chromeMedia: true }` | `test: { chromeMedia: false }` |

`test.chromeMedia` is not in vitest's `InlineConfig` type either (`// @ts-expect-error`).

**CSS transitions: none by default (jsdom's), Chrome's on opt-in.** jsdom 27 runs no CSS
transitions or `@keyframes` animations, so by default turbo-test runs none either: after a class
or style change `getComputedStyle` reports the end value at once, and no `transitionrun` /
`transitionstart` / `transitionend` or `animation*` event fires. jsdom has no Web Animations, so
`element.animate`, `getAnimations()`, `document.timeline`, `Animation`, `CSSTransition`,
`CSSAnimation` and `AnimationEvent` are undefined too (`TransitionEvent` exists, as in jsdom).
Libraries that feature-detect `animate` take their jsdom path, and suites skip the per-element
transition work. On, turbo-dom runs them as Chrome 156 does (a transition starts at its start
value and `getAnimations()` lists the CSSTransition / CSSAnimation objects). This switch and the
media switch are independent: either can be on without the other.

| source | Chrome | jsdom (default) |
|---|---|---|
| CLI flag (wins) | `--css-transitions` | `--no-css-transitions` |
| env var | `TURBO_CSS_TRANSITIONS=1` | `TURBO_CSS_TRANSITIONS=0` |
| vitest config | `test: { cssTransitions: true }` | `test: { cssTransitions: false }` |

`test.cssTransitions` is not in vitest's `InlineConfig` type either (`// @ts-expect-error`).

**Touch events and computed `text-decoration`: as jsdom / happy-dom report them.** Under
`environment: 'jsdom'` or `'happy-dom'` (and with no environment set, turbo-test's DOM default), a
`TouchEvent`'s `touches` / `targetTouches` / `changedTouches` keep plain objects, so
`fireEvent.touchStart(el, { touches: [{ clientX: 10, clientY: 20 }] })` works (jsdom has no `Touch`
class), and `getComputedStyle(el).textDecoration` is the declared shorthand (`line-through`, not
Chrome's `line-through rgb(0, 0, 0)`), which is what jest-dom's `toHaveStyle` compares against
under vitest. A file declaring `// @vitest-environment turbo-dom` gets Chrome's behaviour instead
(`Touch` objects only, Chrome's serialization). Both are set per file, also on a reused isolate.

## Isolate-reuse (extra speed, zero config)

By default turbo-test isolates every file (like vitest's `isolate: true`). If your vitest config
sets `test.isolate: false`, turbo-test honors it and **reuses one V8 isolate per worker** — node_modules
barrels evaluate once instead of per file. (An `isolate` inside `test.poolOptions` is ignored, as
vitest 4 removed `poolOptions`.) As in vitest, files share the worker's state, each file's own
top-level hooks run only for that file, and each file runs once: an assertion failure is reported as it happened. A file whose run may be an
artifact of the reused isolate (a test throwing something other than an assertion error, a load
error, a crash, running out of heap) is re-run once on a clean isolate, whose result is reported. How much it buys
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
`clearAllMocks`/`resetAllMocks`/`restoreAllMocks`, fake timers, `jest.requireActual` /
`jest.requireMock`, `jest.isolateModules` / `isolateModulesAsync`, `jest.replaceProperty`, …); the
type-only members (`jest.Mock`, `jest.Mocked`, `jest.SpyInstance`) are erased by the transform.
When there's **no vitest config**, the nearest **jest config** (`jest.config.{js,cjs,mjs,ts,json}`
or a `package.json` `"jest"` block) is read, `<rootDir>` resolved:

- `setupFiles` (run before the test framework is installed, as in jest) and `setupFilesAfterEnv`;
- `globalSetup` / `globalTeardown` (0.8.3+): run once before / after all files in one shared
  context and require cache, called with `(globalConfig, projectConfig)`; the `process.env`
  changes globalSetup makes reach every test file, and a failing teardown fails the run;
- `moduleNameMapper` (0.8.3+): regex keys in config order, `$n` captures, a list of targets or
  `require.resolve(...)`; it is read from the config text, so mappings built in code (a spread, a
  helper's result) are reported on stderr and should be written as literal entries;
- `testMatch`, `injectGlobals: false`, `moduleDirectories`, `customExportConditions`,
  `testTimeout`, `clearMocks` / `resetMocks` / `restoreMocks` (0.8.3+);
- `fakeTimers` (0.8.0+): `enableGlobally: true` fakes the timers before each test file loads, and
  `now` / `doNotFake` / `timerLimit` / `advanceTimers` become the defaults of `jest.useFakeTimers()`
  (the call's own argument wins);
- `automock: true` (0.8.2+; also `jest.enableAutomock()`), with colocated `__mocks__/` manual mocks
  and chained `jest.onGenerateMock` callbacks.

Mocks from `jest.fn()` / `jest.spyOn()` have jest-mock's `whenCalledWith(...matchers)` (0.8.0+;
`vi.fn()` has none, as in vitest). Under a jest project, `mockReset` / `jest.resetAllMocks` /
`mockRestore` drop a mock's implementation so it returns `undefined` (jest 30 semantics, 0.8.0+);
`vi.fn()` / `vi.spyOn()` keep vitest's behaviour (the impl stays and a spy keeps calling through).
Assigning a namespace import's member (`import * as api from './api'; api.fetch = jest.fn()`) takes
effect for the namespace and its other importers, as under babel-jest / ts-jest (0.8.3+); vitest's
namespaces stay read-only. Test files get `process.env.JEST_WORKER_ID`. When a vitest config
applies, the jest config's `fakeTimers` is ignored.

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
cost and never regresses. A plain `import { SomeInterface }` used as a decorated member's type
needs no `import type`: imports used only as types are read through a namespace import (TypeScript's
own fix for TS1272), so their `design:type` is `Object` and ES-module linking never looks for the
missing export. (The oxc fallback has no type information and still needs `import type` there.)

```bash
npx turbo-test            # discovers *.spec.* too; reads jest config when no vitest config exists
```

> **Status — experimental.** The jest shim, jest-config reading, decorator metadata and
> CommonJS-first resolution work. The native compatibility surface below is bounded;
> complete MongoDB driver execution and full ICU `Intl` time-zone support remain unavailable. CommonJS-first resolution is scoped to jest projects
> with a node test environment, so React/vitest suites are unaffected. The last measured jest
> numbers (0.2.11) are in [docs/BENCHMARKS-history.md](docs/BENCHMARKS-history.md).


Node-mode support in the 0.8.4 candidate includes native SHA-1/SHA-256 hashing,
P-256 ECDSA and RSA PKCS#1 v1.5 SHA-256 signing with PKCS#8/SPKI PEM/DER KeyObjects,
and AES-256-GCM with 12-byte IVs, 12–16-byte authentication tags and AAD. Key generation
and `randomBytes` use OS entropy; unsupported algorithms and encrypted key formats
report errors. It also includes gzip, native V8 serialization, transferable MessagePorts,
and AsyncLocalStorage across native await and scheduled resources.

CommonJS-first decorator metadata preserves partial exports in cyclic TypeScript
imports. `require('stream')` is a callable Stream constructor with matching named/default
identity and legacy inheritance support. These additions do not establish full Jest or
Flux APIs compatibility; consumer validation remains in progress.

The host I/O surface supports synchronous `spawnSync`/`execFileSync`/`execSync` and
loopback HTTP/1.0–1.1 with content-length or chunked bodies. HTTP uses one request per
connection, shared message/socket identity, socket close events and native TCP_NODELAY.
HTTP socket state reflects the real connection; it does not expose general-purpose raw TCP
stream I/O. `tls.Server` supports type discrimination, while construction and
`tls.createServer` throw `ERR_TLS_UNSUPPORTED_SERVER`. TLS transport, custom agents,
persistent connections and asynchronous child-process APIs remain unsupported. This does not yet provide a complete MongoDB driver
execution environment.

Fresh `vm` contexts support injected bindings, isolated globals, real values/errors
and timeouts. They currently require the framework-only snapshot: use
`TURBO_SNAPSHOT=0` in mixed DOM projects. Unsupported immutable-global shadows report
`ERR_VM_UNSUPPORTED_GLOBAL` rather than silently changing the sandbox semantics.

## How it works

- **Transform** — per-module oxc/esbuild → CJS; node_modules bundled once per package (Vite-style),
  cached on disk (99% hit on warm runs).
- **DOM** — [turbo-dom](https://crates.io/crates/turbo-dom)'s native `rtdom` (html5ever parser),
  bound directly to V8 by the turbo-dom-browser-env crate. Per-file env setup ~1–2 ms.
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

# also report source files no test loaded, at zero (the include globs define the surface)
npx turbo-test --coverage --coverage-all --coverage-include 'src/**/*.{ts,tsx}'
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

**Unexecuted files (`--coverage-all`, 0.8.3+).** By default the report lists the files the tests
loaded. `--coverage-all` (or `test.coverage.all: true`) adds every source file the include globs
match that no test loaded, with its statements, functions and branches at zero, so totals and
thresholds reflect the whole surface. Like vitest, it leaves out test / spec / bench files, `.d.ts`,
tools' config files (`vite.config.ts`, …), `test/` / `tests/` at the root, `__tests__` /
`__mocks__`, non-JS/TS files and files with the ignore comment; symlinked directories are not
followed. Without an include glob it adds nothing.

**Speed.** Collection is opt-in and runs ~2–3× slower than a plain run (V8 block coverage keeps
code un-optimized to count blocks) — in line with vitest's own coverage. Normal runs are completely
unaffected (no `--coverage` → no inspector, no source maps, identical transform cache). Branch
coverage and a per-worker speedup for `isolate: false` projects are tracked in
[`docs/COVERAGE-FUNC-BRANCH-BACKLOG.md`](docs/COVERAGE-FUNC-BRANCH-BACKLOG.md).

**Precision vs Istanbul.** This is V8 coverage, so execution *counts* are exact and collection is
near-free — but it measures compiled bytecode mapped back through source maps, where Istanbul
instruments the source AST directly. At the **line** level the two are comparable (our esbuild maps
are clean). For **branches**, turbo-test pairs oxc decision points with V8 block counts for real
per-arm data — including braceless early-return `if (c) return x;` and an `if` with no `else`
(the then-count is sampled at the consequent like vitest's v8 provider does, where V8 drops the
range of a consequent that always ran; the implicit-else arm is the `if`'s count minus it, not
stuck at 0). Top-level module code is counted (statements and branches), esbuild's ESM export
getters are not reported as functions, and lines of switch cases that never ran read 0. Istanbul
still resolves a few exotic shapes more finely; if you need that exhaustive gate, keep an
Istanbul/vitest job alongside.

## CLI

```
turbo-test [files...] [--jobs N] [--shard i/n] [--reporter json]
           [--coverage] [--coverage-dir DIR]
           [--coverage-thresholds lines=,functions=,branches=] [--coverage-per-file]
           [--coverage-reporter lcov,json-summary,text,html]
           [--coverage-include GLOB] [--coverage-exclude GLOB] [--coverage-all]
```

No file args → discovers test files. If a `vitest.config.*` is found, turbo-test honors its
**`test.include` / `test.exclude`** globs (so e.g. Playwright `*.spec.ts` under `e2e/` are skipped
exactly like vitest); otherwise it uses vitest's defaults, include `**/*.{test,spec}.?(c|m)[jt]s?(x)`
and exclude `**/node_modules/**, **/.git/**` (a config `exclude` replaces that default, as in
vitest). With `--root`, files are named relative to the root. Flags pass through to the native binary. Every
value flag takes either form, `--reporter json` or `--reporter=json`.

## Programmatic

```js
const { run } = require('@miaskiewicz/turbo-test');
const { status } = run(['src/a.test.ts'], { jobs: 8, env: { TURBO_REUSE_ISOLATE: '1' } });
process.exit(status);
```

## Vue 3 Single File Components

Vue SFC loading is off by default, including projects declaring `@vitejs/plugin-vue`.
Enable it explicitly for a Vue test run:

```sh
TURBO_VUE=1 npx turbo-test
```

Use the `jsdom` test environment when mounting components into the DOM. The loader uses the
project's real `vue/compiler-sfc` (or `@vue/compiler-sfc`), then oxc for TypeScript lowering.
It supports `<script setup lang="ts">`, ordinary TypeScript scripts, templates and scoped CSS
through both import and require graphs. Local extensionless `.vue` resolution is enabled only
for opted-in projects. `TURBO_VUE=0` disables it explicitly. With the flag unset, Vue
resolution and compiler hooks stay disabled; no plugin-manifest or Vue-file discovery runs.

This first loader supports inline JavaScript/TypeScript and CSS blocks. External blocks,
preprocessors and CSS modules produce explicit unsupported errors. Vue coverage source-map
composition is not implemented yet, so coverage runs that load an SFC fail explicitly rather
than attributing generated render code to incorrect source positions. The standalone compiler
oracle can be checked with `npm run test:vue-oracle` when developing this repository.

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

## Plugins & aliases

turbo-test is a native runner — it does **not** execute your Vite/vitest `plugins` array. Instead it
reproduces what the most common plugins do, natively and out of the box, so most suites need no plugin
setup at all:

| Plugin | Status | Notes |
| --- | --- | --- |
| **`vite-tsconfig-paths`** | ✅ built in | `tsconfig.json` `compilerOptions.paths` aliases (e.g. `@/*` → `src/*`) resolve natively from the nearest tsconfig. No plugin needed. |
| **`@vitejs/plugin-react` / `-react-swc`** | ✅ built in | JSX/TSX (automatic runtime), TS, and modern JS are transformed natively (oxc). |
| **`vite-plugin-svgr`** | ✅ built in | `import Icon from './x.svg?react'` resolves to a render-safe `<svg>` React component (default **and** legacy `ReactComponent` export). Plain `import url from './x.svg'` still yields the file contents. |
| **`@vitest/coverage-v8`** | ✅ built in | V8 coverage with lcov / json-summary / text / html reporters and gateable thresholds — see [Coverage](#coverage). |
| **`@testing-library/jest-dom`** | ✅ built in | jest-dom matchers (`toBeInTheDocument`, …) are available on the vitest `expect` surface. |
| **`vitest-canvas-mock`** | ✅ built in | `<canvas>.getContext('2d')` returns a no-op 2D context stub. |

**`resolve.alias` / `test.alias`** — user-defined aliases from your `vitest.config.ts` are honored
natively, in both the object form and the array (`find`/`replacement`) form:

```ts
export default defineConfig({
  resolve: { alias: { '@': path.resolve(__dirname, './src') } },
  test:    { alias: [{ find: '@math', replacement: path.resolve(__dirname, './lib/math.ts') }] },
});
```

`@/foo` resolves to `src/foo` (and never captures `@scope/pkg` node_modules imports). A regex-literal
`find` is skipped; a quoted `find` is a prefix match whatever its first character (`'/@z'` included).
The alias target is derived from the value's string literals, which covers
`path.resolve(__dirname, '…')` / `path.join(…)` (all segments joined), `fileURLToPath(new URL('…',
import.meta.url))`, string concatenation (`__dirname + '/z'`, `path.resolve(__dirname, 'src') +
'/sub'`) and bare literals.

## Compatibility notes

- A handful of vitest features that depend on a full Node runtime are stubbed/approximated; e2e
  helper files that import heavy Node-only packages (e.g. `@playwright/test`) get those deps
  stubbed so the file still runs.
- `Date.now()` reflects a real base + the virtual event-loop clock, so `setTimeout`-driven elapsed
  assertions pass deterministically.
- Node built-in modules (`node:buffer`, `fs/promises`, `assert/strict`, `timers/promises`,
  `querystring`, `string_decoder`, `vm`, `worker_threads`, …) are runtime shims; what each one
  provides is listed in [vitest.compat.md](vitest.compat.md#node-built-in-modules).
- Under a DOM environment the window is a 1024 x 768 viewport at `devicePixelRatio` 1, the size
  vitest's jsdom and happy-dom report; assigning `window.innerWidth` (a simulated resize) replaces
  it, as in Chrome.
- pnpm's isolated layout resolves as Node does: a package's bare imports from its real path in
  `node_modules/.pnpm/<name>@<version>/node_modules/<name>`. On Windows, canonical paths are
  spelled as Node spells them (`C:\...`, `\\server\share\...`), never with the `\\?\` prefix.
  `TURBO_PNPM_ANCESTOR=1` additionally permits an unresolved bare import to use the
  nearest ancestor's explicit `node_modules/.pnpm/node_modules` hoist link. It is off
  by default, never scans arbitrary version directories, and does not override a
  normal installed package or bypass its exports. Install real `tslib` and
  `hey-listen` dependencies when needed; turbo-test provides no shadow substitutes.

## Benchmarks (0.8.2, 2026-11-10, macOS arm64)

Wall-clock on four real codebases: each app's own `vitest.config.ts`, the turbo-test column
running this repo's 0.8.2 build (`node cli.js --reporter=dot`), the vitest column running
`vitest@latest run --environment jsdom` with the plumbing (`vitest`, `vite`, `jsdom`,
`@vitejs/plugin-react-swc`) freshly installed into each app. Interleaved, best of three on
ui-design-components; single clean run on the others. Machine: Apple Silicon, quiet.

| App                   | Files |  Tests   | vitest (collected / wall / t/s)  | turbo-test 0.8.2 (collected / wall / t/s) | Throughput Δ |
|-----------------------|------:|---------:|---------------------------------:|------------------------------------------:|-------------:|
| standalone-chat-app   |    68 |      770 |      0 / 2.75 s / —          *(a)* |                   770 / 1.61 s / 478 t/s |      —       |
| website-global        |   109 |   1 551  |    515 / 7.73 s /  67 t/s    *(a)* |                 1 551 / 1.75 s / 886 t/s |      13×     |
| ui-design-components  |   459 |   7 334  |  7 334 / 36.6 s / 200 t/s          |                 7 334 / 15.5 s / 473 t/s |     2.4×     |
| payroll-app           | 1 942 |  24 494  |  7 657 / 68.5 s / 112 t/s    *(a)* |                24 494 / 25.7 s / 953 t/s |      8.5×    |

The last column is a per-test-throughput ratio (`turbo-test t/s ÷ vitest t/s`), *not* a
wall-time ratio — a wall-time ratio on rows where vitest ran a subset of the suite and
turbo-test ran all of it would be comparing different workloads. Throughput divides out the
test-count difference. On ui-design-components, where both runners collected the full 7 334
tests, the wall-time ratio and the throughput ratio are the same thing.

*(a)* Under vitest a share of each app's files fail to import (Next.js layout aliases,
generated types, framework path rewrites that vitest's default vite setup doesn't reproduce),
so vitest's wall and tests-collected cover only the files it *could* import. turbo-test
resolves and runs every file in the same suite. ui-design-components is the clean,
apples-to-apples row; the other three carry the throughput ratio so the absolute wall times
aren't misread as same-work speedups.

Historical numbers from the 0.2.x generation (June 2026) are kept, labelled with version, date
and machine, in [docs/BENCHMARKS-history.md](docs/BENCHMARKS-history.md).

## Requirements

`node >= 18`. Prebuilt binaries ship for macOS arm64, Linux x64 and arm64 (glibc) and Windows
x64. musl (Alpine) is not built — the prebuilt V8 is glibc-only; `cli.js` prints build-from-source
instructions (`cargo build --release` in this repo, with a Rust toolchain) on platforms without a
binary.

## Developing

`npm ci` works with npm 10 and npm 11 (CI installs the committed `package-lock.json` with both and
fails if either rewrites it). Run it before the Rust tests: the CLI integration tests in `tests/`
link the repo's esbuild/typescript installs into throwaway fixture projects and spawn the real
`turbo-test` binary against them.

```sh
npm ci
cargo test --locked          # unit + CLI integration tests
npm test                     # node --test test/*.mjs (CLI compat + tsc type checks)
npm run test:vue-oracle       # real @vitejs/plugin-vue / Vitest component controls
cargo tarpaulin              # line-coverage gate (tarpaulin.toml, fail-under = 99.9)
cargo fmt --check            # CI job `lint`, with:
cargo clippy --all-targets -- -D warnings
npm run complexity:rust      # CI job `complexity / rust (cc <= 7)` (needs cc-check, below)
npm run complexity:js        # CI job `complexity / js (cc <= 7)`
```

### Gates and CI

- **Pull requests** (`.github/workflows/ci.yml`): the `build` job builds and runs the Rust tests on
  ubuntu (x64, arm64) and Windows x64, plus the smoke test, the CLI compatibility/type tests and
  the native DOM compatibility fixtures on linux x64. The aarch64-apple-darwin build runs only on
  `main` and `v*` tag pushes, i.e. in the release.
- **Windows experimental — nonblocking.** The separate `windows-experimental.yml`
  workflow installs dependencies, builds the runner and runs locked Rust tests on
  Windows, including platform path fixtures. Its actual results are informational;
  it does not weaken the existing release binary matrix.
- **Coverage — blocking.** `coverage` (ubuntu) runs `cargo tarpaulin` (Llvm engine) and fails
  below `fail-under = 99.9` line coverage. Subprocess runs of the instrumented binaries are
  attributed through `tarpaulin.toml`'s `objects`; the `#[path]` unit-test modules are excluded.
- **Lint — blocking.** `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`;
  `npm-lockfile` checks the lock under npm 10 and 11.
- **Complexity — blocking (since 0.8.0).** Every function has cyclomatic complexity <= 7 (one
  limit, no allow-list, no baseline). A failing job lists the functions over the limit in its step
  summary. Rust uses turbo-html2pdf's `cc-check` over every
  tracked `.rs` file, installed from its home at the rev CI pins:

  ```sh
  cargo install --locked --git https://github.com/miaskiewicz/turbo-html2pdf \
    --rev 923c89f791f3a89cb089a59626a25cef0a9cbd5f cc-check
  ```

  JS uses oxlint's `complexity` rule (`oxlint@1.87.0` via npx; `.oxlintrc.complexity.json`) over
  `cli.js`, `index.js` and `src/runtime.js`.
- **Release** (`v*` tag): `release-guard` (no `[patch]`, no git sources, versions equal the tag),
  then the npm and crates.io publish jobs ([RELEASING.md](RELEASING.md)).

## Status (0.8.3)

- vitest suites: drop-in; the `vi` / mock / `expect` surface is vitest 4.1.12's
  ([vitest.compat.md](vitest.compat.md) lists what is covered and what is approximated).
- jest suites: experimental (see above).
- DOM: turbo-dom-browser-env 0.8.3, Chrome 156-controlled in turbo-dom.
- Open work is tracked as GitHub issues in this repository and in turbo-dom.

## License

MIT
