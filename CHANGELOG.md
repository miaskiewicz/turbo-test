# Changelog

All notable changes to `@miaskiewicz/turbo-test`. Format based on
[Keep a Changelog](https://keepachangelog.com/); this project uses semver-ish `0.2.x`.

## [Unreleased]

## [0.3.2] — extra HTML*Element constructor globals

### Fixed
- Register the HTML element constructors the DOM bootstrap's base ctor list omitted:
  `HTMLDialogElement`, `HTMLDataListElement`, `HTMLFieldSetElement`, `HTMLLegendElement`,
  `HTMLOListElement`/`HTMLDListElement`, `HTMLPreElement`, `HTMLTableRowElement`/`-CellElement`/
  `-SectionElement`/`-ColElement`/`-CaptionElement`, `HTMLProgressElement`, `HTMLMeterElement`,
  `HTMLDetailsElement`, `HTMLPictureElement`, `HTMLSourceElement`, `HTMLMediaElement`/
  `HTMLVideoElement`/`HTMLAudioElement`, `HTMLTemplateElement`, `HTMLSlotElement`,
  `HTMLBodyElement`/`HTMLHtmlElement`/`HTMLHeadElement`, `HTMLMetaElement`, `HTMLLinkElement`,
  `HTMLTitleElement`, `HTMLBaseElement`, `HTMLBRElement`, `HTMLHRElement`, `HTMLOptGroupElement`,
  `HTMLMapElement`, `HTMLAreaElement`, `HTMLObjectElement`, `HTMLEmbedElement`,
  `HTMLOutputElement`, `HTMLQuoteElement`, `HTMLMenuElement`, `HTMLDataElement`, `HTMLTimeElement`,
  `HTMLUnknownElement`. App bundles reference these for feature-detect / `instanceof` /
  subclassing (e.g. MUI's Dialog touches `HTMLDialogElement`); an undefined reference threw mid-
  chunk during hydration and blanked the rendered tree. Single-tag elements get a tag-keyed
  `instanceof` (`x instanceof HTMLDialogElement` ⇔ `x.tagName === 'DIALOG'`); abstract/multi-tag
  interfaces fall back to a generic element check. The native `get_constructor` tagName→name map
  is extended in lockstep so `node.constructor.name` resolves to the specific interface, not a
  generic `HTMLElement`. (`browser_env.js` bootstrap + `browser_env.rs`.)

### Fixed — `.d.ts` shim widened to match real vitest API usage
The 0.3.1 type subset was too narrow to replace `vitest`'s types in a large suite (payroll-app:
525 `tsc` errors across ~170 spec files with `vitest` dropped). Widened `types/turbo-test-api.d.ts`:
- **`vi.mocked()`** now returns a `MockedFunction<T>` for functions (exposing `mockReturnValue`/
  `mockReturnValueOnce`/`mockResolved|RejectedValue(Once)`/`mockImplementation(Once)`/
  `mockClear|Reset|Restore` and `.mock.{calls,results,instances}`) and a `MockedObject<T>` for
  objects/`{deep:true}` — the dominant define-then-configure pattern. (~350 errors)
- **`vi.hoisted<T>(factory: () => T): T`** added to `ViAPI`. (92 errors)
- **Hook callbacks** (`beforeEach`/`afterEach`/…) widened to vitest's `() => unknown | Promise<…>`,
  so `beforeEach(() => vi.useFakeTimers())` (arrow returns the chainable `ViAPI`) type-checks. (81)
- **`Mock<T>` / `vi.fn<T>()`** adopt the vitest-4 function-signature generic
  (`Mock<(a: string) => number>`, `vi.fn<(a) => b>()`) instead of the old args-tuple generic. (54)
- **`it.each<T>()`** gains a single-value-row overload so an explicit non-array union type arg
  (`it.each<InviteType>(['a','b'])`) resolves; tuple rows still spread into positional args.
- **`expect.fail(message?)`** added to `ExpectStatic`. (1)

## [0.3.1] — turbo-dom 0.3.5 + jest `@jest/globals` imports + shipped TypeScript types

### Fixed
- Bump the `turbo-dom` crate (rtdom DOM runtime) `0.3.4` → `0.3.5`. The new release stops
  `rtdom::serialize` from escaping regular spaces to `&nbsp;` (U+00A0) — a quirk inherited from
  the old JS serializer. That corruption made a re-parse / accessible-name / text query see
  `"a b"` instead of `"a b"`, so whitespace-normalized matching
  (`getByRole`/`getByText`/`toHaveText` against a serialized DOM) failed. Spaces now serialize
  verbatim; only `&`/`<`/`>`/`"` are escaped.

### Added
- **`import { … } from '@jest/globals'`** now resolves from turbo-test's own runtime, the same
  builtin that backs `import … from 'vitest'`. jest projects with `injectGlobals: false` (or
  TS-strict setups that import the API explicitly) previously fell through to node-module
  resolution and broke. The builtin shim now also exports `jest` (alongside
  `describe`/`it`/`test`/`expect`/`vi`/hooks/`assert`), so the named `jest` import binds to the
  runtime controller. (`runner.rs`: `is_test_api_module()` matches both specifiers across the
  static-ESM, dynamic-`import()`, and CJS-`require` resolution paths.)
- **Shipped `.d.ts` bundle** so consumers can drop the `vitest` (or `@types/jest`) devDependency
  that was kept purely for types. `tsconfig.json` `types: ["vitest/globals"]` and `from 'vitest'`
  imports across a codebase otherwise force `vitest` into devDependencies even when turbo-test is
  the runtime. New `types/`: `globals.d.ts` (ambient globals → `@miaskiewicz/turbo-test/globals`),
  `vitest.d.ts` (the `vitest` module), `jest-globals.d.ts` (the `@jest/globals` module), over a
  shared `turbo-test-api.d.ts`. A pragmatic vitest/jest-compatible **subset** (matcher/mock args
  widen to `any` — no false type errors, less precise inference). Exposed via `package.json`
  `exports` (`./globals`, `./types/*`) and added to `files`. README → "TypeScript types".

### Tests
- `fixtures/jest/src/jest-globals-import.spec.ts` + `test/compat-jest.test.mjs` cover the
  `@jest/globals` named-import path alongside the existing global-`jest` shim fixture.

## [0.3.0] — all-Rust DOM is the default (JS turbo-dom removed)

The DOM environment is now turbo-dom's pure-Rust **rtdom**, bound natively to V8. The legacy JS
`installGlobals` bootstrap + the `.node` parser + the `@miaskiewicz/turbo-dom` npm dependency are
**gone**; `TURBO_RUST_DOM` is no longer consulted (rtdom is unconditional). All three production
oracles run 100% green with **zero env flags** (payroll 10,471/0, ui-design 7,062/0,
website-global 1,003/0).

### Changed
- **Flip:** `browser_env::enabled()` is always true; `setup_dom` binds rtdom directly. Removed
  `dom_bootstrap`/`turbodom_root` (imported `install.mjs` + shimmed CSSOM) and dropped the
  `@miaskiewicz/turbo-dom` runtime dep. esbuild stays (coverage / decorator-metadata / fallback).

### Added — all-Rust DOM coverage (rtdom + browser_env binding)
- DOM event-dispatch fix: the V8 NON_MASKING name interceptor defers to a real own property before
  returning undefined — V8's inline cache could otherwise mask React's `__reactFiber$`/`__reactProps$`
  expandos (added after a cached miss), breaking delegated onClick/onChange on portal'd content
  (MUI Autocomplete × userEvent). Cleared the whole cluster + the `--jobs 8` flakiness.
- Real Selection/Range (live caret, `getRangeAt`, `setBaseAndExtent`, `selectionchange`) + native
  CharacterData (`insertData`/`splitText`/…) so contenteditable editors (Lexical) + userEvent typing
  work; `contentEditable`/`isContentEditable`; visibility resolved via rtdom's inheritance-aware
  native cascade; `input.valueAsNumber`.
- Form-control reflection; doc state (`readyState`/`visibilityState`/`elementFromPoint`/
  `getClientRects`); native ChildNode/ParentNode (`before`/`after`/`replaceWith`/`replaceChildren`);
  `insertAdjacentHTML`/`Element`; `toggleAttribute`; `getAttributeNS`; `setAttributeNode`/
  `removeAttributeNode`; anchor URL decomposition; `link.rel`/`media`/`as`/`type`; `localName`.
  (rtdom DOM methods land in the published `turbo-dom` crate 0.3.4.)
- `testTimeout` honored from the vitest config (was a fixed 5000ms default).

### Fixed — runtime
- `URL`: `search`/`href`/`toString` derive live from `searchParams`, so post-construction
  `searchParams.set()` serializes (was frozen at construction). `URLSearchParams` form-urlencoded
  space ⇄ `+`. `MessageEvent` global added.
- Config scan: read test `include`/`exclude` only from the text before the `coverage` block — a
  `coverage.exclude` whose first glob is `**/*.test.{ts,tsx}` was wrongly taken as the test exclude
  → "no test files found".

### Added — Rust port (branch `rust-port`)
- **P1: launcher ported into the native binary (`src/launcher.rs`).** Default test discovery,
  vitest config include/exclude + coverage/environment scanning, `--changed [since]` git filter,
  isolate/environment env wiring, and `--passWithNoTests` now run in Rust — `cli.js` is a thin
  binary-resolving shim. The binary launches a run with no Node-side logic; npm stays a
  distribution wrapper. *Why:* removes the Node process from launching, the first step to a
  self-contained binary. All compat suites green; binary self-discovers standalone.
- **P2a: native oxc ESM→CJS emitter (`src/esm_cjs.rs`), on by default.** Replaces the per-app-file
  `esbuild --format=cjs` transform with a hand-written oxc lowering (oxc 0.134 has no CommonJS
  module transform) that matches esbuild's output contract: live-binding member rewrites for
  named/default imports (scope-correct via semantic), `__export` getter block + `__toCommonJS`,
  `export *` via `__reExport`, `export default`, and dynamic `import()` →
  `Promise.resolve(__toESM(require(x)))`. Opt out with `TURBO_NATIVE_CJS=0`. *Why:* drops the
  esbuild subprocess for app files — a step toward removing the esbuild/npm dependency. esbuild
  is still used for node_modules bundling (P2b), coverage source maps, decorator-metadata, and as
  the automatic fallback for any unhandled form.
- **Conformity harness (`scripts/conformity.mjs`).** Runs a target project both ways (esbuild
  baseline vs native) and diffs per-file pass/fail — `parity` mode guarantees behavioral
  equivalence; `coverage` mode (native-strict) measures the native handling rate. *Why:* the
  safety mechanism gating the cutover; it already caught a real dynamic-`import()` bug, now fixed.
  Validated on the payroll-app `staging` worktree: **1057 files / 10471 tests, 100% native
  handling, full parity.**
- **P2b: native package bundler (`src/bundler.rs`), default ON.** Replaces esbuild for node_modules:
  bundles a package's relative graph with lazy `__commonJS` init wrappers (circular-safe), bare
  imports stay external (shared via require cache), assets stubbed. Opt out with `TURBO_NATIVE_DEPS=0`.
  Validated: payroll 1057 files / 10471 tests full parity with native app **+ deps** — so **normal
  test runs no longer use esbuild at all** (esbuild remains only for coverage, decorator-metadata,
  and as the fallback). Key fix: `__reExport` passes `module.exports` as its 3rd arg so names
  re-exported after `__toCommonJS` (e.g. `@testing-library/react`'s `render`) aren't lost.
- **P2c (partial): single-pass emit + native coverage source maps (gated).** `emit` now does TS-strip
  + ESM→CJS on one AST and, under coverage, appends a correct inline oxc source map. Coverage still
  runs on esbuild for now: oxc's codegen map is less dense than esbuild's, so `coverage.rs`
  under-attributes inner functions/lines. Fully removing esbuild additionally needs native
  decorator-metadata + dropping the fallback.

## [0.2.16] — vitest CLI/API compatibility sweep + turbo-dom 0.2.5

### Changed
- **Bump `@miaskiewicz/turbo-dom` `^0.2.2` → `^0.2.5`.** Pulls in the latest published DOM runtime.

### Added
- **vitest CLI compatibility — P0 batch.** Closes the highest-value gaps for being a drop-in
  `vitest run` in CI (see `vitest.compat.md` for the full audit + tracker):
  - `-t, --testNamePattern <re>` — run only tests whose full `describe > it` name matches the regex
    (unanchored, case-sensitive, matching vitest). Plumbed `cli.js` → `TURBO_TEST_NAME_PATTERN` env
    → `__TT_NAME_PATTERN` global → `runtime.js` `runSuite` filter.
  - Leading `run`/`watch`/`dev` subcommand token is accepted and stripped (turbo-test is always a
    single run), so the canonical `vitest run …` invocation works unchanged.
  - `--passWithNoTests` — exit 0 (not 1) when discovery finds no test files.
  - Unknown `-`/`--` flags are now warned-and-ignored instead of being treated as test-file paths
    (which previously reached the runner as a hard load-error and flipped the exit code).
- **vitest CLI compatibility — P1/P2 batches** (developed in parallel worktrees, merged together):
  - **Execution control:** `--testTimeout <ms>` + per-test `{ timeout }` now ENFORCED (internal
    one-shot timer raced against the body, invisible to `vi.runAllTimers`/`getTimerCount`; a hung
    `await new Promise(()=>{})` test now fails cleanly instead of hanging the worker); `--retry <n>`
    global default; `--bail <n>` (shared cross-worker failure counter, file-granular); `--maxWorkers`
    alias of `--jobs`, `--minWorkers` accepted (no-op); `--silent` (test `console.*` no-op);
    `--allowOnly`/`--no-allowOnly` (per-file `.only` gate that flips the exit code).
  - **Reporters/output:** `--reporter junit` (per-testcase XML), `tap` (TAP v13), `verbose`, `dot`,
    `default`; `--outputFile <path>` for json/junit/tap. Unknown reporter values fall back to text
    (never error). Required retaining a per-test result list (`TestReport.tests`) through
    `runtime.js` → `runner.rs` → `turbo_test.rs`.
  - **Config/discovery/environment:** `-c/--config <path>` (force exact config), `--root`/`--dir`
    (discovery-root override), `--environment <node|jsdom|happy-dom>` + `// @vitest-environment`
    pragma (`node` skips turbo-dom DOM-global install via `TURBO_ENV`), `--isolate`/`--no-isolate`
    (→ `TURBO_NO_REUSE`/`TURBO_REUSE_ISOLATE`), `--changed [since]` (git changed-file filter),
    `--globals`/`--no-globals` (accepted; `--no-globals` a documented no-op).
  - **Test/`expect` API:** file snapshots (`toMatchSnapshot`, `toThrowErrorMatchingSnapshot`) +
    `-u/--update`; `toMatchInlineSnapshot` compare-only (no source auto-write); `expect.assertions(n)`/
    `hasAssertions()` enforcement; `it.fails`; `describe.todo/.skipIf/.runIf/.concurrent`; `it.extend`
    fixtures (best-effort).
  - Documented compatibility gaps for every partial item are tracked in `vitest.compat.md`.

### Tests
- Added `test/cli-compat.test.mjs` (the `test/` dir `npm test`/`node --test` already expected but
  which did not exist) + `test/compat-{runflags,reporters,config-env,api}.test.mjs` — **47 cases**
  total locking the behaviors above, plus `fixtures/compat/`.

### Docs
- Added `vitest.compat.md` — a living CLI/command + `vi`/`expect` API compatibility matrix and
  prioritized backlog, updated as each batch landed.

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
