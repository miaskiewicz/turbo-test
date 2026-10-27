# Changelog

All notable changes to `@miaskiewicz/turbo-test`. Format based on
[Keep a Changelog](https://keepachangelog.com/); this project uses semantic versioning.

## [Unreleased]

### Changed — expect(x) is a chai Assertion (turbo-test#160)
- As in vitest, `expect(x)` is now a chai Assertion. turbo-test had no BDD chain:
  `expect(1).to.equal(1)` threw `Invalid Chai property: to`. It also lacked vitest's sinon-style
  members (`called`, `calledWith`, ...), `expect.assert` and the `vitest` module's `assert` /
  `chai` / `should`, and its matcher errors were not `chai.AssertionError`s. The npm package now
  depends on `chai` 6.3.0 (vitest 4.1.12's, pinned exactly). `cli.js` passes its location to the
  binary as `TURBO_CHAI`; the binary loads it once into the runtime snapshot through the native
  ESM→CJS emitter, and every jest-style matcher becomes a method of chai's Assertion, as
  vitest's JestChaiExpect does. Running the binary without chai is now an error.

### Performance — a failing file runs once (K1, turbo-test#126, #144, #150)
- Under isolate reuse (vitest `isolate: false`, or `--no-isolate`) every file with a failure —
  or with nothing passed — was run a second time on a fresh isolate and that result reported,
  doubling the cost of each failing file (a profiling hotspot in #79). vitest runs each file
  once on the shared worker and reports that run, so a run whose tests were collected and whose
  failures are all assertion failures (`expect` / `assert`) is now final, and a report is never
  re-run on the reused isolate (its module is already evaluated there: a second run collected no
  tests). A run that may be the reused isolate's own artifact — a test throwing something other
  than an assertion error (a stale DOM wrapper's `Illegal invocation`), a failed hook, a load
  error, a panic, the heap guard's out-of-memory abort — still gets one run on a fresh isolate
  (an out-of-memory file is still not retried on the same kind of isolate, turbo-test#124). `test/compat-reuse-retry.test.mjs`
  pins the outcome and the run count against vitest 4.1.12 on `fixtures/reuse-fail-once`
  (`tests/data/reuse-fail-once-vitest-4.1.12.json`).
- Isolate reuse keeps one document for the worker (turbo-test#150). The worker's DOM was
  installed once but dropped after every file, so the next DOM file read a dead `document`
  (`TypeError: Illegal invocation`) and passed only on its fresh-isolate re-run. As in vitest
  `isolate: false`, the document now lives as long as the worker's isolate (a later file sees
  the earlier file's document and body); a fresh-isolate run or a recycled isolate installs it
  again. Pinned by `tests/runner_b_reuse.rs` against vitest 4.1.12 with happy-dom.
- A file whose tests all fail runs once (turbo-test#144). Any report with 0 passed and ≥1 failed
  (or a "document / window / DOM is not defined" failure) was re-run twice more on the same kind
  of isolate, isolated or not, and the best attempt kept — a guard for a DOM init race in the old
  shared turbo-dom `.node` addon, which the in-process turbo-dom-browser-env binding does not
  have. vitest runs the file once; now only a file that did not run its tests (a load error, a
  panic) is retried. The same test pins it with `--isolate` (vitest's report is identical).

### Fixed
- Fake-timer runs are sinon's (turbo-test#177): `runOnlyPendingTimers` (and its async form)
  advances to the last pending timer, so intervals and timers set on the way run (it fired a
  snapshot of the pending timers once each), and a timer callback's error is thrown by the
  advance call — `advanceTimersByTime` / `runOnlyPendingTimers` after running the rest of the
  range, `runAllTimers` / `advanceTimersToNextTimer` at once, the async ones by rejecting —
  instead of being logged as `uncaught`.
- The real `performance.now()` and `process.hrtime()` / `hrtime.bigint()` move (turbo-test#178):
  they were constant 0, so elapsed-time code measured nothing. They count the real clock (real
  time plus the drive loop's jumps, as test durations do) since the runtime started
  (`performance.timeOrigin`).
- The fake-timer API is vitest 4.1.12's (turbo-test#171, #172). Without fake timers the advance /
  run functions and `getTimerCount` throw vitest's `A function to advance timers was called but
  the timers APIs are not mocked. …` (they silently did nothing, so a test that forgot
  `useFakeTimers` passed); `vi.useFakeTimers(config)` reads `toFake`, `now`,
  `shouldAdvanceTime` / `advanceTimeDelta` and `loopLimit` (it ignored its argument);
  `performance` and `process.hrtime` are faked by default and count the fake clock's time (they
  were stuck at 0); `runAllTimers` aborts an endless timer chain or interval after `loopLimit`
  (10000) timers as sinon does, instead of stopping at the first interval; restoring real timers
  gives back the same `Date`; `advanceTimersToNextFrame` and `runAllTicks` exist. The `jest.*`
  timer functions stay no-ops without fake timers (jest warns). `tests/runner_c_fake_timer_api.rs`.
- `vi.waitFor` / `vi.waitUntil` are vitest's (turbo-test#174): the timeout errors are
  `Timed out in waitFor!` / `Timed out in waitUntil!` located at the call (they were
  `vi.waitFor timed out` at the await), a number is the timeout, `waitFor` resolves with a
  `false` / `null` result, `waitUntil` rejects with the callback's error at once, and a promise
  result is awaited before polling again. `tests/runner_c_fake_timer_api.rs`.
- Fake timers fire only when the test moves the fake clock (turbo-test#168). The drive loop fired
  the earliest timer whenever the test was pending, fake or not, so a faked `setTimeout` ran as
  soon as the test awaited anything — a test vitest times out passed. Fake timers now live on
  their own clock as with vitest's @sinonjs/fake-timers (`shouldAdvanceTime` off):
  `useFakeTimers` swaps the global timer functions and `Date`, timers set before it, captured
  timer functions, `node:timers` / `node:timers/promises` and the test timeout stay real (and
  advancing the fake clock no longer brings the timeout closer), `useRealTimers` drops the fake
  clock's pending timers, `setSystemTime` no longer resets the real clock, the `*Async` advance
  functions yield a real macrotask around each timer (sinon's `tickAsync` & co.), `vi.waitFor` /
  `vi.waitUntil` advance fake timers by their interval, and each file starts on real timers.
  Pinned against vitest 4.1.12 by `tests/runner_c_fake_clock.rs`.
- A stack frame `at async /abs/file.js:L:C` (an anonymous function suspended at an await) kept
  its location (turbo-test#169): only a method name lost its `async ` prefix, so this frame was
  dropped and an error made after an await — `expect.poll`'s `Matcher did not succeed in time.`,
  a helper's error awaited by the test — lost its test-file frame and code frame
  (`tests/runner_c_async_frame.rs`, against vitest 4.1.12).
- The `expect` surface is now vitest 4.1.12's, checked member by member against vitest
  (`tests/runner_c_expect_surface.rs`, turbo-test#152–#155). The asymmetric matchers are
  vitest's classes, with `$$typeof`, `asymmetricMatch`, `toString`, `getExpectedType` and
  `toAsymmetricMatcher`; `equals` and the printers also accept a user-defined matcher. Added
  `expect.closeTo`, `schemaMatching`, `toSatisfy`, `addEqualityTesters`, `fail`, `unreachable`,
  `getState` / `setState`, and `expect.<name>` / `expect.not.<name>` for every `expect.extend`
  matcher. Added 17 missing matchers (the `toBeCalled*` / `toReturn*` aliases, the
  returned-times and nth / last returned family, the resolved family, called-before / after,
  `toBeNullable`); for these, vi.fn now records `invocationCallOrder` and `settledResults`.
  An unknown matcher throws chai's `Invalid Chai property` (it used to be `undefined`, so
  `is not a function`), and a passing matcher returns the assertion. `.resolves` / `.rejects`
  on a non-promise throw vitest's TypeError. A failed toMatchObject diffs against the received
  subset (it printed `Received: undefined`). Still missing: chai's BDD chain and `assert`
  (#160), `expect.poll` (#161), `addSnapshotSerializer` (#162), and vitest's `this.utils`
  printers (#163).
- `process.env` has vitest's `TEST`, `VITEST_MODE`, `FORCE_TTY`, `VITEST_POOL_ID` (the worker)
  and `VITEST_WORKER_ID` (per file, or per worker on a reused isolate), in vitest's key order
  (turbo-test#156). `PROD` / `DEV` follow `NODE_ENV` as in vite (`'1'` / `''` under
  `production`), so `import.meta.env.DEV` is false in a production build.
- runtime.js no longer defines `__ttRestoreFileApis` (turbo-test#147). It restored Blob / File /
  FileReader after the JS turbo-dom bootstrap replaced them; that bootstrap went in v0.3.0 and the
  native DOM (`turbo-dom-browser-env`) never replaces them, so nothing called it. Wiring it
  between files would also diverge from vitest, whose `isolate: false` keeps a file's global
  changes for the next file.
- `process.env` is the host environment (turbo-test#143): it was a fixed stub, so a variable
  the user exported (`K=v turbo-test`) was `undefined` in tests. As in vitest 4.1.12 it now holds
  the environment turbo-test was started with, the host's `NODE_ENV` wins (`'test'` only when
  unset), vitest's `VITEST` / `MODE` / `DEV` / `PROD` / `SSR` / `BASE_URL` replace the host's, and
  the stub's `TZ: 'UTC'` (not the real time zone) and `TURBO_DOM_PARSER` are gone.
- `toBeOneOf` and the asymmetric `expect.toBeOneOf` / `expect.not.toBeOneOf` (turbo-test#140)
  were missing (`expect(...).toBeOneOf is not a function`). They now compare as vitest 4.1.12's
  (`equals` per item, a Set by `has` first, an empty array / Set passes, anything else throws
  vitest's TypeError), fail with its `expect(received).toBeOneOf()` message, and print as
  `toBeOneOf<…>` / `not.toBeOneOf` in messages and diffs.
- Default-export interop with CommonJS (turbo-test#122), checked by an 89-row table against
  vitest 4.1.12 (`tests/interop_table.rs`, happy-dom / node, isolate on and off, both loaders).
  On the ESM graph loader (a project without esbuild, as a vite 8 install) `import X from` tsc /
  babel output (`exports.default` + `__esModule`) gave the whole exports object, so React
  reported `Element type is invalid … got: object`; it now gives `exports.default`, as vitest.
  On the module runner a package's `exports.default` without `__esModule` is no longer its
  default (vitest gives the whole `module.exports`), while an app `.cjs` file's own `default`
  is (both loaders); `export { default } from` reads through the same interop; `vi.importActual` /
  `importMock` called from a test body work on the ESM graph loader (they resolved from no
  directory and gave `undefined`); a `vi.mock` factory without `default` throws vitest's
  `[vitest] No "default" export is defined on the "x" mock. Did you forget to return it from
  "vi.mock"?` on access instead of giving the factory object (the missing-name message is
  vitest 4.1.12's too). Remaining differences are tracked in turbo-test#133–#137.
- Module namespaces as vitest gives them (turbo-test#133, #141; the last KNOWN rows of the #122
  interop table, both loaders). An ES module's namespace (`import * as`, `import()`,
  `vi.importActual`) no longer reads `__esModule`, lists its names in source order and is tagged
  `[object Module]` (esbuild's sorted export registration is put in source order; the ESM graph
  loader reads an app module's namespace through a view, V8 sorts it). A package's CommonJS
  module is Node's namespace of it: `default` and `'module.exports'` are `module.exports` (an
  `import * as` of `module.exports = fn` gave the function itself), `export *` re-exports Node's
  names (cjs-module-lexer's for a `module.exports = { … }` literal); a CommonJS module marked
  `__esModule` reads it on the ESM graph loader too; a package's ES module that Node runs as one
  (`.mjs`, `"type": "module"`) imports CommonJS as Node does (its default is `module.exports`);
  `require()` of an ES module from CommonJS has Node's enumerable `__esModule: true`; an
  automock of a package's CommonJS module mocks Node's namespace (`import { Comp }` reads
  `undefined`, #136).
- A package's ES module's namespace lists Node's sorted names on the module runner too, as
  vitest imports a package through Node (turbo-test#159; app modules keep source order, #141).
  On the ESM graph loader `import()` of a `vi.mock` factory's module without `default` gives the
  mock, whose `default` throws vitest's `No "default" export…` on every read, as `import * as`
  does (turbo-test#158; it gave #137's stand-in function).
- `require()` (and `createRequire`) resolves a dual package's `require` build and reads `main`
  before `module`, an import reads `main` before `module` too, as Node does for vitest
  (turbo-test#134): a CommonJS consumer requiring a dual package got its ES build's exports
  object (`Element type is invalid … got: object`). A require() of an import-only package still
  resolves it as an import.
- ESM graph loader: every read of the default import of a `vi.mock` factory without `default`
  throws vitest's `[vitest] No "default" export is defined on the "x" mock…` — `typeof X`,
  `if (X)`, `X === y` too (turbo-test#151), where #137's stand-in threw only once used. An app
  module importing such a default is compiled with each read of the binding wrapped in a check
  (`src/mock_read.rs`); every other module's source is unchanged. Four interop-table rows.
- Mocks and packages (turbo-test#135, #136, #137; rows of the #122 interop table): a `vi.mock`
  no longer reaches a package's own `require()` — vitest loads an external through Node, so
  `require('x')` inside `node_modules` gets the real `x` (a tsc package's `__importDefault`
  wrapped the mock into `{ default: { default: Mocked } }`, an object where a component was
  expected). An automocked function keeps its name (`Comp.name`, was `fn`). On the ESM graph
  loader a `vi.mock` factory without `default` makes the default import throw vitest's
  `[vitest] No "default" export is defined on the "x" mock…` when it is used (read, called,
  constructed, its keys listed — a synthetic module's binding cannot throw on the read itself),
  instead of giving the factory object; a `jest.mock` factory stays its own default there.
  Still differs: a named import of an automocked CommonJS module reads the mock where vitest
  reads `undefined` (needs #133's namespace).
- A project with no `node_modules/.bin/esbuild` — pnpm, whose transitive esbuild lives only under
  `node_modules/.pnpm`, or one that does not depend on esbuild — now runs on turbo-test's own
  esbuild (PR #80 by alphajew420, turbo-test#79): `cli.js` passes it as `TURBO_ESBUILD`, every
  esbuild call site goes through one `esbuild_bin(root)` (the project's own first), and the
  project root falls back to the nearest directory with `node_modules`. Such projects took the
  ESM-graph loader, where `vi.spyOn` on a module's export was not seen by the module importing
  it (vitest: it is).
- `import.meta` on every loader path (turbo-test#123): the native CJS emitter and dep bundler
  left `import.meta` in CommonJS output (`Cannot use 'import.meta' outside a module` when the
  esbuild retry was unavailable or failed; an empty object when it ran), esbuild dep bundles
  gave an empty object, and no path had `import.meta.env`. Each module now gets its own
  `url` / `filename` / `dirname` plus vitest's `env` (live `process.env`, `DEV` / `PROD` / `SSR`
  booleans), and `process.env` carries vite's `MODE` / `DEV` / `PROD` / `SSR` / `BASE_URL` (`SSR`
  off under a `jsdom` / `happy-dom` environment), as vitest sets them.
- An `import.meta.glob` pattern that is not relative, `/` or an alias fails its module, as vite
  does at transform (turbo-test#149), with vite's `Invalid glob: "<pattern>" (resolved: …). It
  must start with '/' or './'`; the call used to throw only when it ran, so the file's other
  tests passed.
- Asset imports give vite's strings (turbo-test#148): `import t from './t.txt?raw'`, `?url` and
  plain imports of images / fonts / media / `.txt` gave `'default'` on the module runner and `{}`
  on the ESM graph. Now `?raw` is the file's text, `?url` and known asset types the root-relative
  URL (the rest of the query kept), a small `.svg` vite's inline `data:` URL
  (`src/vite_asset.rs`), so `import.meta.glob(..., { query: '?raw' })` / `as: 'raw'` work too.
- `mockRestore` / `vi.restoreAllMocks` put the spied property's original descriptor back
  (turbo-test#157), as vitest does: they redefined it as an enumerable data property holding the
  original, so on the module runner a restored re-export (`export *`, `export { x } from`) was no
  longer live (a later spy on the source module did not show through it) and a restored class
  method became enumerable.
- `--reporter tap`: a file that fails to load gets vitest's block (turbo-test#166): `error:` with
  its `name` and `message` (YAML-quoted, line breaks kept) and `at: "<file>:<line>:<col>"`,
  indented four spaces. It printed `message: module errored: …` with the loader's prefix and the
  line breaks flattened.
- The summary line of a file that failed to load reads `ERROR a.test.ts  (Error: boom)` on the
  ESM-graph loader too (turbo-test#164); it carried the loader's internal load-phase prefix
  (`module errored: ` / `evaluate threw: ` / `instantiate: `), unlike the module runner.
- A module that only re-exports (`export * from`, `export { x } from`) loaded through the
  esbuild transform threw `ReferenceError: __toESM is not defined` (turbo-test#165): its
  `__reExport` called a helper esbuild had not emitted. It calls runtime.js `__ttToESM` now.
- A namespace spy on the ESM-graph loader reaches the importers only, as in vitest (turbo-test#146):
  it replaced the export's binding, so the exporting module's own calls (`greetTwice()` calling
  `greet`) and other modules' re-exports saw the spy too. Importers now read imported bindings
  through the spy overrides (`__ttR` / `__ttNs`, also for an `import()` namespace) and modules
  register their own names, re-exports and `export *` sources (`src/esm_spy.rs`); the module's
  own references keep the original, as vitest's exports objects do. Reading a name a `vi.mock`
  factory did not return now throws vitest's error on this loader too.
- `vi.spyOn` on a module's export on the ESM-graph loader (turbo-test#131, the binary run without
  `TURBO_ESBUILD` in a project without esbuild): V8's namespace objects are read-only, so the spy
  was dropped and the importing module kept calling the original (`expected 'hi a!' to be
  'spied!'`). Each app module now registers a setter for its own namespace (`src/esm_spy.rs`):
  `export const` / `let` / `function` / `class`, renamed locals, `export default` (expression or
  anonymous declaration) and re-exports (forwarded to the source module); `mockRestore` puts the
  original back.
- `import.meta.glob` options and order (turbo-test#145): `base`, `query` (string or object), the
  deprecated `as` and `resolve.alias` patterns threw vitest's "statically replaced" error instead
  of expanding, a module taking the decorator-metadata (tsc) transform was not expanded at all,
  and eager glob imports ran after the module's own imports. They now expand as vite does, the
  eager imports first (on the module's first line, so line numbers hold).
- `import.meta.resolve` / `main` / `glob` / `vitest` (turbo-test#130): every module's
  `import.meta` was `url, filename, dirname, env` only. It now has vitest 4.1.12's members in its
  order: `resolve` (Node's, from the module: relative paths and URLs against it, builtins as
  `node:` URLs, packages by the ESM conditions, `ERR_MODULE_NOT_FOUND` otherwise), `main`
  (`false`), `glob` and, in the test file, the non-enumerable `vitest`. A full-name
  `import.meta.glob(...)` call with literal arguments is expanded before the module's transform,
  as vite does (`src/import_glob.rs`: eager / lazy, `import`, negated and array patterns,
  `exhaustive`, relative and `/`-root patterns, the module itself, dot files and `node_modules`
  skipped); any other call throws vitest's "statically replaced" error.
- Config discovery (turbo-test#43 item 1): `setupFiles` are read from every config file vitest
  resolves, in its order (`vitest.config` then `vite.config`, each `.ts .mts .cts .js .mjs
  .cjs`); `vitest.config.{cts,mjs,cjs}` and `vite.config.{mts,cts,js,mjs,cjs}` were ignored and
  `vite.config.ts` beat `vitest.config.js`.
- Matchers build their failure message only when the assertion fails (turbo-test#43 item 2):
  a passing `toBe` no longer throws when formatting an operand throws (a native class
  prototype whose members check their receiver: "Illegal invocation").
- Node-API references keep a count (turbo-test#43 item 3): `napi_create_reference` honours
  `initial_refcount`, `napi_reference_ref` / `napi_reference_unref` move it, and a reference at
  0 is weak, as in Node.
- `napi_define_properties` leaves a missing accessor half out of the definition (turbo-test#43
  item 4), so redefining an existing accessor with one half keeps the other.
- Node-API (turbo-test#44): every function of Node 26's `js_native_api.h` and `node_api.h` up to
  NAPI_VERSION 10 is implemented with Node's semantics — the error family and
  `napi_get_last_error_info` (every call records its status; a NULL env is
  `napi_invalid_arg`), numbers / bigints / dates, Latin-1 / UTF-16 / external strings and
  property keys, symbols, coercions, the property family, comparisons and type checks,
  arraybuffers / typed arrays / dataviews, external buffers over the addon's memory (no
  longer copied, their finalizer runs), promises, externals, `napi_new_instance` (it threw "not
  implemented"), `napi_run_script`, handle scopes, instance data, cleanup hooks (run at env
  teardown in Node's order), `napi_module_register`, `node_api_get_module_file_name`, the
  versions (`process.versions.napi` is `'10'`) and the declared module API version, async work
  on a four-thread pool, threadsafe functions, async contexts / `napi_make_callback` / callback
  scopes. Their completions and calls run on the JS thread between macrotasks, and in-flight
  work keeps the event loop alive. An addon importing a function the host does not provide now
  fails its `require()` with an error naming it instead of crashing the whole run (SIGSEGV), and
  load errors carry the loader's message. The inventory (function, version, tests, host
  differences) is [docs/node-api.md](docs/node-api.md).
- Node-API core entrypoints check a NULL result / value argument as Node (`napi_invalid_arg`)
  instead of crashing or returning `napi_ok` (turbo-test#50); `napi_create_typedarray` accepts
  a SharedArrayBuffer (turbo-test#51); the buffer creators hand back `Buffer`s, not plain
  `Uint8Array`s (turbo-test#52); an exception a finalizer leaves pending is reported as
  uncaught from the event loop, as in Node, instead of being dropped (turbo-test#53).
- `structuredClone` is V8's structured clone with `{ transfer }` (transferred buffers are
  detached) and Node's errors (turbo-test#46).
- `require.cache` is the shared module cache: deleting an entry makes the next `require()` load
  the module again, a set entry is served as is (turbo-test#47). `require.resolve()` resolves
  (it returned its argument) and a missing module is Node's `MODULE_NOT_FOUND` with the require
  stack (turbo-test#56).
- `fs.symlinkSync` / `fs.promises.symlink` exist (turbo-test#48); `process.cwd()` is the process's
  working directory, not the nearest package.json directory (turbo-test#54); `os.tmpdir()` /
  `os.homedir()` follow the environment as Node's (turbo-test#55).
- An unresolved `import()` rejects as vitest's: `Cannot find module '/sub/x.js' imported from
  <importer>` / `Cannot find package 'x' …` (`ERR_MODULE_NOT_FOUND`), `No such built-in module`
  (turbo-test#61).
- Launcher, as vitest 4.1.12: every config lookup uses vitest's config names and order
  (turbo-test#49); default discovery matches `.mjs`/`.cjs`/`.mts`/`.cts` test files and skips
  only `node_modules` and `.git` (hidden, `dist`, `build`, `coverage`, `target`… are searched,
  turbo-test#57); a config `exclude` applies without a config `include` (turbo-test#60);
  positional arguments are filters on the discovered files (a directory, a file path or any
  substring of the root-relative path, case-insensitively — a directory argument failed with
  "graph load failed", turbo-test#45 / turbo-test#58); discovered files are named relative to the
  working directory (the JSON report keeps absolute names, as vitest's; the duration history
  is keyed by absolute path) and nothing to run prints vitest's `No test files found, exiting
  with code N` with its filter / include / exclude lines (turbo-test#59). A missing file
  argument is no longer dropped with a warning: like any filter, it matches nothing.
- Tests and `describe` blocks run, and are reported, in declaration order (turbo-test#64): a
  block's own tests ran before its nested `describe`s, so a test declared after a `describe` ran
  before that block's tests.
- `.only` runs what vitest runs (turbo-test#65): a `describe.only` block's tests ran nowhere —
  every test not itself marked `.only` was skipped. Now a test runs when it or an enclosing
  suite is `.only`, except that inside a suite holding `.only` tasks only those run.
- `-t` / `--testNamePattern` is matched against vitest's full name, the `describe` names and the
  test's joined with a space (turbo-test#66); it was matched against `describe > test`, so
  `-t "outer inner"` ran nothing and `-t "outer > inner"` ran what vitest skips.
- `test.each` / `it.each` / `describe.each` titles are formatted as vitest formats them
  (turbo-test#67): printf placeholders (`%s %d %i %f %j %o %O %c %%`), `%#` / `%$`, `$key` /
  `$a.b` attributes and the tagged-template table, values rendered like vitest's `inspect`. They
  were the template plus ` [i]` (`describe.each` kept the bare template), and a tagged table was
  read as one row per cell.
- `test.for` / `it.for` / `describe.for` (turbo-test#68): a file using one failed to load
  (`TypeError: test.for is not a function`). Each row is one argument (not spread), followed for
  a test by its context; titles are formatted like `.each`. Typed in the `.d.ts` shim.
- Suite-level outcomes follow vitest (turbo-test#69): a throwing `beforeAll` skips its suite's
  tests and nested suites (its `afterAll` still runs) instead of running them, and neither a
  throwing `beforeAll` nor `afterAll` counts as a failed test any more — it fails the suite and
  file. An empty `describe` fails its file with `No test found in suite <name>` (it passed). A
  file that reports such an error with no failed test fails the run (FAIL line, dot `x`, the
  `Test Files` count, JSON `status` / `success`, exit code 1).
- `process.stdout.write` / `process.stderr.write` write to stdout / stderr (turbo-test#70); they
  dropped their text. Stdout text goes to stderr while a json/junit/tap report owns stdout, as
  console output does, and `--silent` drops it; a trailing callback runs.
- `--reporter json` writes vitest 4.1.12's JSON report (turbo-test#63 item 1): per-test
  `assertionResults` (`ancestorTitles`, `fullName`, `status`, `title`, `duration`,
  `failureMessages`, `meta`, `tags`), the suite / pending / todo counts (the files and their
  `describe` blocks are the suites), the `snapshot` summary, `startTime`, and per file
  `startTime` / `endTime` / `message`, in vitest's key order. A file that fails to load is
  `failed` with its error's `message` (it was `error`); the per-file `numPassingTests` /
  `numFailingTests` (not vitest keys) are gone. Tests that do not run are reported as vitest
  reports them, in every per-test reporter: `-t`-filtered tests and the tests of a
  `describe.skip` block (whose callback is now collected, as in vitest) are skipped, `it.todo` /
  `describe.todo` ones are todo, and a `describe` none of whose tests runs runs no hooks.
- `--includeTaskLocation` / `test.includeTaskLocation: true` (turbo-test#63 item 1): the JSON
  report's `assertionResults` carry vitest's `location` (`{ line, column }` of the call that
  registered the test, in the original source: 1-based, UTF-16 columns). The flag was an
  unsupported flag. The call site's V8 position is mapped back through the transform's source
  map (the native emitter's, following the `vi.mock` hoisting; oxc's on the ESM path).
- `--root` names files relative to the root (turbo-test#63 item 2), as vitest does, in the
  default / verbose / dot output, JUnit and TAP (the JSON report keeps absolute names): they were
  relative to the working directory (absolute for a root above it). `--dir` keeps
  working-directory names.
- `node_modules` and `.git` are skipped only by the effective exclude (turbo-test#63 item 3): a
  config `test.exclude` replaces vitest's default exclude, so test files there are discovered
  unless the config lists them; they were never searched. Discovery still prunes any directory a
  `<prefix>/**` exclude covers.
- A hoisted `vi.mock` factory sees the test file's top-level bindings (turbo-test#72): the
  prepass ran it outside the file's scope, so without esbuild any binding it read was
  `not defined`, and with esbuild only a shared `let` reached it (a `const`, function, class or
  import did not, and the mock was dropped). The prepass now runs in a scope proxy the file fills
  when it runs; without esbuild a TypeScript file's factories are type-stripped first (their
  types made the prepass a syntax error, so the mock was dropped).
- Failed matchers throw what vitest 4.1.12 throws (turbo-test#71): chai's `AssertionError` with
  vitest's message for every built-in matcher, positive and negated (`expected 1 to be 2 //
  Object.is equality`, `expected [ 1 ] to include 2`, the spy matchers' call / return listings with
  vitest's diff), instead of a plain `Error` with turbo-test's wording; a snapshot mismatch is
  ``Snapshot `<name> <n>` mismatched`` (inline snapshots now share the test's snapshot counter),
  a custom matcher's `message` must be a function, as in vitest. Spies record a throwing call in
  `mock.results` (`type: 'throw'`) and a `vi.spyOn` spy is named after its method.
- Snapshots follow vitest 4.1.12 (turbo-test#73): snapshot files are written byte for byte as
  vitest writes them (header, natural key order, vitest's snapshot serializer — `[Function]`,
  `[MockFunction name] { calls, results }`, `10n`, `-0`, raw strings, escaped regexes —, single-line
  values without line breaks, `<name> > <hint> <n>` keys, property matchers) and files vitest wrote
  pass (they mismatched); thrown values are snapshotted (`[TypeError: boom]`, not the message);
  `toMatchFileSnapshot` is added; inline snapshots are compared as vitest compares them and new /
  `-u`-updated ones are written back into the test file; obsolete snapshots are reported and
  removed by `-u` (an emptied snapshot file deleted); on CI a missing snapshot fails instead of
  being written; the JSON report's `snapshot` summary has vitest's counts.
- CI detection is std-env's `isCI`, as vitest 4.1.12 decides it (std-env 4.3.0; turbo-test#74):
  `CI` set to any non-empty value counts (`CI=false` and `CI=0` included), and otherwise the
  first provider row of std-env's table whose variable is non-empty decides — 58 rows, among them
  Vercel `NOW_BUILDER`, Netlify, Bamboo, Cloudflare Pages and Google Cloud Run, while
  `NETLIFY_LOCAL`, `VERCEL`, `VERCEL_ENV` and `CODESANDBOX_*` do not count. turbo-test checked
  13 fixed variables (so a missing snapshot was written on most providers' CI, and `TF_BUILD` /
  `BITBUCKET_BUILD_NUMBER` or an empty value counted when vitest's does not). The table is data
  (`src/ci_providers.json`, read by `src/ci.rs`), checked against an installed std-env by
  `scripts/check-ci-detection.mjs`.
- `.only` fails on CI (turbo-test#75): `allowOnly` defaults to `!isCI` as in vitest, so a stray
  `.only` committed to a branch fails the CI run instead of silently narrowing it to one test.
  `--allowOnly` / `--no-allowOnly` still decide when given (the last one wins).
- A forbidden `.only` fails as in vitest (turbo-test#76): with allowOnly off each `.only` task
  fails in place with vitest's message (`[Vitest] Unexpected .only modifier. Remove it or pass
  --allowOnly argument to bypass this error`) and does not run — a `test.only` is a failed test
  (no hooks, no body), a `describe.only` a failed suite whose hooks do not run and whose tests
  are skipped. turbo-test ran the `.only` tests (they passed) and added one file-level failure,
  so the counts and the JSON report differed from vitest's.
- A `test.only` inside `describe.skip` / `describe.todo` is reported `pending` (turbo-test#77):
  vitest does not interpret `.only` inside those blocks, so the test keeps mode `only`, never
  runs and counts only in the total; turbo-test reported it `skipped` / `todo`.
- The summary footer counts as vitest's `getStateString` (turbo-test#78): `Test Files` and `Tests`
  read `failed | passed | expected fail | skipped | todo (total)` with the zero parts left out
  (no more `0 passed`), `no tests` when there is none, a passing `it.fails` is an `expected
  fail`, the `Tests` total counts every test, and a file none of whose tasks ran is a `skipped`
  file. turbo-test counted only failed and passed tests.
- Drop-in blockers from a 529-file React/TS vitest suite (turbo-test#79), each with the
  report's repro as a test and vitest 4.1.12 / Node 26 as the reference:
  - `vi.mock` factories with TypeScript or JSX (item 1): on the ESM graph loader (a project
    without esbuild) the entry's mock prepass stripped types only for `.ts` entries and never
    transformed JSX, then dropped a prepass that did not compile — the mock fell through
    silently to the real module (`Unexpected token ':'`, `as`, `<`, `...`). The prepass now
    gets the file's own TS/JSX transform (its JSX runtime `require`, `import.meta` = the
    entry's), the `vi.mock` calls are read from the AST (a `)` inside a factory's string no
    longer cuts it; calls in comments are not mocks) and a prepass that cannot compile fails
    the file with its SyntaxError instead of falling through.
  - `vi.dynamicImportSettled()` (item 2): vitest's `waitForImportsToResolve` — a real-clock
    macrotask, then every pending `import()`, until none is.
  - `node:fs` `readdirSync` (item 3): Node's sorted order, `withFileTypes` Dirents
    (`name`, `parentPath`, `is*()`), an encoding argument, `recursive`, a `file:` URL argument
    and Node's `scandir` errors (a URL listed `[]`, a missing directory returned `[]`).
  - SVG elements have `style` (item 4): fixed in turbo-dom-browser-env (turbo-dom#87, `style` on
    `SVGElement` and a new `MathMLElement`, as Chrome 156); the pin moves to that branch.
  - A package `exports` subpath to tsc-emitted CommonJS (item 5): a `.js` file without a
    package.json `"type"` is an ES module only with ES module syntax (Node's detection, by
    parsing); a comment mentioning "import " made the substring sniff load CommonJS as ESM
    (`does not provide an export named 'blockFields'`). A file with no module syntax at all is
    now CommonJS, as in Node.
  - The V8 abort (`# Fatal error … Check failed: has_exception()`): under `isolate: false` a
    file whose module threw while the graph evaluated left later modules linked to its mocks;
    the next file evaluating one ran a synthetic module with no registry entry and V8 aborted
    the whole run. The reuse reset drops modules that never finished evaluating, and a
    synthetic module whose entry is gone fails its importer instead of the process.
- `import()` of a module with top-level await settles after that module has evaluated
  (turbo-test#82); it resolved at once with a namespace whose bindings were uninitialised.
- `node:fs` follows Node (turbo-test#83, #86): `statSync` / `lstatSync` return `Stats` (real
  kind, size, times, `fs.Stats`; `throwIfNoEntry`) — `statSync` called every path a 0-byte file;
  `fs.Dirent`; every fs call takes a `file:` URL; `readFileSync` without an encoding is a
  `Buffer`; `writeFileSync` / `appendFileSync` / `mkdirSync` no longer create missing parents
  (`mkdirSync` honours `recursive` and returns the first directory created); `rmSync` needs
  `recursive` for a directory (`ERR_FS_EISDIR`) and `force` for a missing path; `rmdirSync` /
  `unlinkSync` / `rmSync` / the read and write calls throw Node's errors (`code`, `errno`,
  `syscall`, `path`, message); `fs/promises` gains `stat`, `lstat`, `rmdir`, `unlink`.
- `node:url` (turbo-test#84): `fileURLToPath` percent-decodes and rejects a non-file scheme, a
  host and an encoded `/` with Node's errors; `pathToFileURL` returns an encoded `URL` resolved
  against `process.cwd()` (it returned a string or a plain object). A file URL's `localhost` is
  the empty host and its origin `null`.
- `URL` percent-encodes the path and fragment (turbo-test#85): `new URL('file:///a b').href` is
  `file:///a%20b`; input loses leading/trailing C0 controls and spaces and inner tabs/newlines;
  `pathname` / `hash` setters encode.
- `import.meta.url` is percent-encoded like Node's `pathToFileURL` (turbo-test#87) on both
  loaders; a project path with a space or a non-ASCII character gave a raw `file://` string.
- `--reporter junit` writes vitest 4.1.12's report (turbo-test#81): vitest's element layout and
  attributes, skipped / todo cases, failed suites as cases, a load error as a failing case named
  after the file, each test's console output in `<system-out>` / `<system-err>`, and the
  failure printed as vitest prints it — the expected / actual diff and stack frames mapped back
  to the test file and the app modules (`src/failure.rs`). With `--outputFile` stdout says
  `JUNIT report written to <path>` (`JSON report written to <path>` for json) instead of the
  footer. turbo-test wrote its own `<testsuites name="turbo-test">` format without skipped
  cases, failure bodies or output. vitest's comparison and call-count matchers
  (`toBeGreaterThan`, `toBeCloseTo`, `toHaveBeenCalledTimes`, …) now raise errors without a diff,
  as vitest's do.
- The default and verbose reporters print what vitest 4.1.12 prints without colours
  (turbo-test#94): the ` RUN  v4.1.12 <root>` banner, `stdout | file > task` console blocks,
  vitest's module lines (` ❯ a.test.ts (3 tests | 1 failed) 12ms`, a failed file's tests under
  it, a test over the 300ms slowTestThreshold even in a passing file) or verbose test lines with
  each failure's messages, `(retry xN)`, the Failed Suites / Failed Tests sections on stderr with
  code frames, numbered dividers and identical errors merged, and the footer with the Snapshots
  lines (obsolete / removed keys per file), `Start at` and the duration breakdown. Without
  `--reporter`, under an AI agent (std-env's `isAgent`: `AI_AGENT`, `CLAUDECODE`, `CODEX_*`, …),
  the default is vitest's agent reporter: failed files and tests only, and only failed tasks'
  console output. turbo-test printed its own `FAIL  a.test.ts  (0 passed, 1 failed)` lines and
  a `1 files | 0 passed | … | cache 67% hit` line; `TURBO_LEGACY_REPORTER=1` keeps them. A
  retried test keeps every attempt's error (json `failureMessages`, junit, the reporters).
- The default and verbose reporters are coloured as vitest's (turbo-test#94): without
  `NO_COLOR` (and with no AI agent; tinyrainbow's rules: `TERM=dumb`, `FORCE_TTY=false`,
  `FORCE_COLOR`, `CI`, `--color` / `--no-color`) the labels, symbols, counts, durations, frames
  and dividers carry vitest's ANSI colours, code frames are syntax-highlighted as vitest's
  tinyhighlight does (a port of js-tokens, `src/highlight.rs`), console values are coloured as
  Node's `util.inspect` colours them (RegExps highlighted by group depth), and diffs are
  coloured with the changed part of a string inverse. Diffs now follow vitest's algorithm in
  plain output too: jest's diff-sequences for lines, the cleaned-up character diff with aligned
  lines for multi-line strings, and the `Compared values have no visual difference.` /
  `serialize to the same structure` messages without a misplaced `- Expected` header.
- `--reporter dot` prints what vitest 4.1.12's DotReporter prints (turbo-test#100): the RUN
  banner and console blocks, one character per test as it finishes (`·` passed, `x` failed, `-`
  skipped or todo; green / red / dim gray, each after an empty yellow run as vitest writes them
  in a pipe), an empty line, the Failed Suites / Failed Tests sections on stderr and the default
  footer. turbo-test printed one character per file (`.` / `x` / `!`), `[js] [log] …` console
  lines and its own `1 files | …` line (still printed with `TURBO_LEGACY_REPORTER=1`); like
  vitest's, the dot reporter now writes nothing to `--outputFile`.
- A test's duration counts the time it waited on timers (turbo-test#95): the event loop jumps
  its clock to the next timer instead of sleeping, so `await new Promise(r => setTimeout(r,
  320))` took 0ms in every reporter; it is now ~320ms, as on Node.
- `console.*`, `util.inspect` and `util.format` format as Node's (turbo-test#93): `util.inspect`
  is a port of Node's (depth 2, breakLength 80, compact 3: quoting, `[Object]`, `<ref *1>`,
  `Map(n) {…}`, `[class X]`, grouped long arrays, multi-line objects, `util.inspect.custom`);
  it was `JSON.stringify`, `util.format` ignored `%s %d …` and `console.log('a', {})` printed
  `a [object Object]`.
- Every test function gets vitest's TestContext (turbo-test#96); it was called with no argument,
  so any test using it failed with a TypeError. `ctx.task` (name, ids, `fullName`, its suite
  and file tasks, `result` while it runs), `ctx.skip(note?)` / `ctx.skip(condition, note?)`
  (the test is reported skipped at run time, with its duration and the `[note]` the human
  reporters print), `onTestFinished` (after afterEach, the last registered first) and
  `onTestFailed` handlers (a throw fails the test), `ctx.signal` (aborted when the test times
  out), `ctx.expect`, `ctx.annotate(message, type?)` (junit `<properties>`, the verbose
  reporter and the Failed Tests section print them). beforeEach / afterEach hooks get the
  context, `it.for` rows and `it.extend` fixtures see the same one (fixture functions get it
  too); `it.each` handlers get the row only, as in vitest (under `it.extend` they also got the
  fixtures). Each hook / handler call's console output is an entry of its own, as vitest's.
- Thrown values that are not `Error`s with a message, and error properties, print as vitest's
  (turbo-test#97): a thrown primitive is `Unknown Error: <value>` (junit `<failure
  message="…">` with no `type`; it printed `: plain string` with `type=""`), a value without a
  message its serialized form (`{ a: 1, stacks: [] }`, junit `<failure>`; it printed `Unknown
  Error: `), and the serialized error's own properties follow its frames as `Serialized Error:
  { code: 'E_CODE' }` in junit and the verbose reporter (a cause's in every reporter). The JSON
  reporter's `failureMessages` hold `null` for an error with no stack and no message (it held
  `[object Object]`), the verbose reporter `→ undefined`.
- `toStrictEqual` checks the types as vitest does (turbo-test#98): both sides must have the same
  `constructor`, so a class instance never strictly equals a plain object with the same
  properties (nor a null-prototype object `{}`, an `Array` subclass `[]`); it passed.
- A test file that fails to compile — itself or a module it imports — fails with V8's
  `SyntaxError` as vitest reports it (turbo-test#99): the file's load error, the json
  `message` and the junit failing case (`type="SyntaxError"`, no frames) carry it, and the
  default / verbose reporters' Failed Suites print one module's compile error once, under every
  file whose import of it failed (a different module's apart), as vitest. turbo-test
  said `graph load failed` and V8 printed `Uncaught SyntaxError` to stdout three times, with a
  line number of the transformed code.
- A failure frame at a call of an imported function that does not start its statement
  (`const v = boom()`, `() => boom()`, `{ k: boom() }`, `[1, boom()]`) points at the callee, as
  vitest's does, on the esbuild loader too (turbo-test#110): the native ESM→CJS emitter's
  `import_h.boom` rewrite had no source position, so the frame mapped to an earlier token
  (`a.test.ts:3:23` for vitest's `3:27`).
- `toEqual` / `toStrictEqual` compare as vitest's `equals` (turbo-test#103): Dates by time,
  RegExps by source and flags, boxed primitives by value (never equal to the primitive), URLs
  by href, Sets / Maps by members in any order, other iterables in order, symbol keys, the
  extra properties of an array, Errors by name / message / cause / properties, DOM nodes by
  `isEqualNode`, typed arrays by kind, circular structures; `toStrictEqual` also tells a hole
  from `undefined` and ArrayBuffers by bytes. The matchers on the same comparison use vitest's
  testers each: the spy matchers as `toEqual`, `toMatchObject` (and snapshot property matchers)
  with `subsetEquality`, `toContainEqual` / `toHaveProperty` / `toHaveReturnedWith` /
  `objectContaining` / `arrayContaining` without `iterableEquality`. Only own enumerable string
  keys were compared, so `new Date(1)` equalled `new Date(2)`, `new Map([[1, 2]])` equalled
  `new Map([[1, 3]])`, `new Error('a')` equalled `new Error('b')`.
- `toThrow(error)` / `toThrow(object)` compare the thrown value as vitest's
  `equals(thrown, expected, [iterableEquality])` (turbo-test#111): an Error by name, message,
  cause and own properties, an object by its keys (a plain object never equals an Error). An
  Error expected was compared by message only and an object by `message` substring, so
  `toThrow(new Error('abc'))` passed for a `TypeError('abc')` and `toThrow({ message: 'ab' })`
  for `Error('abc')`.
- `URLSearchParams` follows Node 26 across its surface (turbo-test#117): names and values are
  converted with `String()` (a Symbol throws TypeError; lone surrogates become U+FFFD) in every
  method, so `append(1, 'x')` is found by `get('1')` and keys are strings (they stayed numbers
  and `get('1')` was `null`); the constructor takes a record (own enumerable keys), any iterable
  of `[name, value]` pairs (a pair that is not an iterable of exactly two throws TypeError; it
  was only arrays) or a primitive as a string (`null` is `null=`; was ignored); string input is
  percent-decoded as UTF-8 with U+FFFD for invalid bytes (`%E2%82` stayed raw); `has` / `delete`
  honour the value argument; missing arguments throw TypeError; `size` exists; `forEach` and the
  `keys` / `values` / `entries` iterators are live (`[object URLSearchParams Iterator]`);
  `toString()` escapes `!'()~` as form-urlencoding does.
- `TextEncoder` / `TextDecoder` follow Node 26 (turbo-test#118): lone surrogates encode as
  U+FFFD (`EF BF BD`; a lone high surrogate became a bogus 4-byte sequence); `encodeInto` exists
  and stops before a character that does not fit, counting a surrogate pair as 2 read; invalid
  or truncated UTF-8 decodes to one U+FFFD per maximal subpart (it produced garbage code units),
  `fatal: true` throws TypeError `ERR_ENCODING_INVALID_ENCODED_DATA`, a leading BOM is stripped
  unless `ignoreBOM`, and `stream: true` keeps a partial sequence for the next call; `decode`
  takes an ArrayBuffer, SharedArrayBuffer or any view (offset and length honoured; a
  `Uint16Array`/`DataView` decoded the whole buffer) and throws TypeError for anything else;
  `encoding` / `fatal` / `ignoreBOM` are prototype getters; the label is trimmed and
  case-folded, accepting the UTF-8 labels (the legacy encodings followed in turbo-test#119), and
  an unknown label throws RangeError `ERR_ENCODING_NOT_SUPPORTED`. `Buffer#toString` keeps a BOM, as Node.
- `TextDecoder` decodes the legacy encodings Node 26 does (turbo-test#119): every WHATWG
  Encoding label is accepted (`latin1` / `ascii` are `windows-1252`, `utf-16` is `utf-16le`,
  `sjis` is `shift_jis`, ...; the `replacement` labels throw RangeError
  `The "replacement" encoding is not supported`) and the single-byte, UTF-16LE/BE and CJK
  multi-byte encodings decode natively through encoding_rs, with `fatal` (TypeError naming the
  encoding), `ignoreBOM` and `stream` (a partial sequence or ISO-2022-JP's mode carries over).
  Every other label threw RangeError `ERR_ENCODING_NOT_SUPPORTED`. Node's ICU big5 / euc-jp /
  euc-kr / shift_jis decoders differ from the Encoding Standard on some bytes; turbo-test follows
  the standard there (vitest.compat.md).
- `expect(value, message)` keeps its message (turbo-test#138): a failing matcher's error (chai
  AssertionError or a jest-style / `expect.extend` matcher's Error, also through `.not`,
  `.resolves` / `.rejects` and `expect.soft`) starts with `message: `, as vitest's
  (`GOT<x>: expected 1 to be 2 // Object.is equality`); it was dropped from the error, the
  default reporter and the json reporter's `failureMessages`.
- `FileReader` behaves as jsdom's (turbo-test#139): a read sets `readyState` LOADING at once and
  fires `loadstart`, `progress` (with `loaded` / `total`), `load` and `loadend` (only `load` /
  `loadend` fired); `addEventListener` keeps every listener (it replaced `on<type>`) and
  `removeEventListener` removes the given one; `on<type>` handlers hold their slot;
  `readAsText(blob, encoding)` decodes with the label after BOM sniffing (it always used UTF-8);
  `abort()` while LOADING cancels the read and fires `abort` / `loadend`; a read while LOADING
  throws `InvalidStateError`, a non-Blob argument TypeError; `EMPTY` / `LOADING` / `DONE` exist
  and a FileReader is an `EventTarget`.
- `FileReader` reads the Blob's bytes (turbo-test#132): `readAsArrayBuffer` and `readAsDataURL`
  took char codes of the decoded text (`'€'` read as `[0xAC]`, now `[0xE2, 0x82, 0xAC]`; binary
  that is not UTF-8 was mangled), and `readAsBinaryString` exists, as in jsdom / happy-dom.
- `Blob` / `File` match Node 26 (turbo-test#128): `size` counts UTF-8 bytes (`['€']` was 1, now
  3), `type` is lower-cased (or `''` with a character outside U+0020..U+007E), `arrayBuffer()`
  returns the bytes (it was zero-filled), `slice(start, end, type)` honours its arguments
  (negative and clamped bounds; it returned an empty Blob), `stream()` is a ReadableStream of one
  chunk per part (it was `null`), `bytes()` and `textStream()` exist, the parts are copied at
  construction, `endings: 'native'` turns CR / CRLF into LF, `size` / `type` / `name` /
  `lastModified` are prototype accessors (`ERR_INVALID_THIS` off a Blob), toStringTag is
  `Blob` / `File` (it was `Object`), `File` defaults `lastModified` to `Date.now()`, and the
  constructors throw Node's errors (a non-iterable `blobParts`, a non-object `options`, a bad
  `endings`, `File` with fewer than two arguments).
- `Blob#text()` UTF-8-decodes the concatenated bytes of all parts once, as Node 26
  (turbo-test#120): a sequence split across parts is one character (`[E2 82] + [AC]` gave two
  U+FFFD, now `€`), one leading BOM is stripped (it was kept), and a nested Blob contributes its
  bytes (its parts were joined as strings).
- `URLSearchParams.set` replaces the first pair of the name in place and removes the others, as
  Node's (turbo-test#116); it moved the pair to the end (`a=1&b=2&a=3` set `a` gave `b=2&a=9`,
  Node `a=9&b=2`), in `toString()`, iteration and a URL's `href`.
- The URL global keeps its query as parsed (turbo-test#115), percent-encoded with WHATWG's query
  set (`'` too for a special scheme): `http://a/?q`, `?a%20b=c%20d`, `?x=1&&y` and a bare `?` stay
  as written, and the query is re-serialized from `searchParams` only when that object changes
  (an emptied list leaves no `?`). `searchParams` stays the same object across `search` / `href`
  sets. The query was always rebuilt through URLSearchParams (`?q` became `?q=`, `%20` `+`, empty
  pairs dropped).
- The URL global parses the userinfo (turbo-test#114): `http://u:p@a.com:8080/` has
  `username` `u`, `password` `p`, `host` `a.com:8080` and origin `http://a.com:8080` (both
  percent-encoded with the userinfo set, kept in `href` and by relative resolution), and its parts
  are no own enumerable keys (`Object.keys(url)` is `[]`), as Node's. The authority was taken
  whole as the host (`hostname` `u`, `port` `p@a.com`) with `username` / `password` undefined.
- A URL / URLSearchParams prints as Node's (turbo-test#112): `URL { href: 'http://a/', origin: …,
  searchParams: URLSearchParams {}, hash: '' }` / `URLSearchParams { 'a' => '1' }` in matcher
  messages (vitest's loupe hands them to Node's inspect) and `util.inspect` / `console.log`;
  `JSON.stringify` and vitest's diffs show a URL's href, a URLSearchParams has no own keys, and
  `Object.prototype.toString` gives `[object URL]` / `[object URLSearchParams]`. They printed the
  polyfill's internals (`URL{ protocol: 'http:', …(8) }`, `_hierarchical`, `_sp`).
- A test file whose static import does not resolve, or whose JavaScript does not parse, fails
  with vitest's error (turbo-test#108): `Cannot find module './x.mjs' imported from <importer>`
  (`Cannot find package 'p' imported from …` for a bare specifier, `No such built-in module`
  for an unknown `node:` one) on both loader paths — a `require()` the source wrote keeps
  Node's `MODULE_NOT_FOUND` with the require stack —, and for a `.js` / `.mjs` / `.cjs` module
  vite's parse errors: its import analysis's (`Failed to parse source for import analysis …`)
  when es-module-lexer rejects the source, else rolldown's `RolldownError: Parse failure: Parse
  failed with 1 error: …` with the code frame and `At file: /syn.mjs:1:13`; a `require()`d
  module and a node_modules dependency keep V8's SyntaxError, as Node gives it to vitest.
  turbo-test said `graph load failed` (printing `cannot resolve …` to stderr three times) and
  V8's SyntaxError.
- Modules that do not transform or parse fail as vitest reports them (turbo-test#113): a
  `.ts` / `.tsx` / `.mts` / `.cts` / `.jsx` module oxc does not parse (or whose syntax its
  semantic check rejects, e.g. a redeclared `let`) fails with vite's `Transform failed with N
  error(s):` and rolldown's diagnostics, rendered by rolldown's ariadne fork (new dependency
  `rolldown-ariadne` 0.6.0, the version vite 8's rolldown links) — coloured, as vitest gives
  them even under NO_COLOR —, where turbo-test said `graph load failed` / `failed to load
  ./dep.ts` or V8's SyntaxError; a JavaScript module's `Parse failure` frame shows the code
  rolldown parses, the module with its imports rewritten to vite's URLs; the default reporter's
  Failed Suites entry prints vite's `Plugin:` / `File: <module>[:line:col]` / code frame lines
  under such an error (vitest's `__vitest_rollup_error__`); and the loaders print nothing to
  stderr about the files they could not read, transform or compile (`transform …: parse
  errors`, `cjs read/transform failed`, `cjs compile failed`, up to three times per file) unless
  `TURBO_CJS_DEBUG` (or the native emitter's `TURBO_NATIVE_CJS_STRICT` measurement) is set.
- Hooks run in vitest's default `sequence.hooks: 'stack'` order (turbo-test#101): `afterEach` /
  `afterAll` last registered first (a suite's own before its parent's), and the functions
  `beforeEach` / `beforeAll` return run after the `afterEach` / `afterAll` hooks, last returned
  first (they were ignored). A throwing `afterEach`, cleanup or `afterAll` fails the test / suite
  and stops the hooks after it (a throwing `afterEach` was swallowed: the test passed); `it.fails`
  flips the outcome after the test's hooks, as vitest.
- `expect.assertions(n)`, `expect.hasAssertions()` and a test timeout fail with vitest's errors
  (turbo-test#102): `expected number of assertions to be 2, but got 1`, `expected any number of
  assertion, but got none` (framed where `hasAssertions` was called) and `Test timed out in 30ms.`
  + vitest's `If this is a long-running test, …` hint (framed at the `it(` call); they carried
  turbo-test's own messages and no frame.

- A file that runs out of heap fails alone (turbo-test#124, from alphajew420's #104): V8's answer
  to an exhausted heap is to abort the process, which lost every result of a 529-file
  `isolate: false` run (#79). A near-heap-limit callback now terminates the file, it is reported
  as `JavaScript heap out of memory (limit N MB): the file was aborted and its isolate discarded`
  (not retried on the same kind of isolate), the run continues and exits 1 — vitest likewise
  fails the run without losing the other files. Under isolate reuse a worker whose live heap
  after a full GC passes `TURBO_HEAP_RECYCLE_MB` (default 60% of the heap limit) gets a new
  isolate for its next file (setup files re-run), instead of growing until V8 aborts.
  `TURBO_MAX_HEAP_MB` sets the per-isolate limit, `TURBO_HEAP_LOG=1` prints the heap after
  every file. The setup-mock snapshot handles are now dropped before their isolate.
- A finished file's spies no longer stay reachable under isolate reuse (turbo-test#124, from
  alphajew420's #107): the registry behind `vi.clearAllMocks` / `resetAllMocks` /
  `restoreAllMocks` held every `vi.fn()` / `vi.spyOn()` strongly for the worker's life, and each
  keeps every call's arguments and receiver, so a reused isolate's live heap grew by every file's
  call history (~70 MB per file in the fixture) until it ran out. It now holds spies weakly. Unlike
  #107 a finished file's spies are not `mockClear()`ed: vitest 4.1.12 with `isolate: false` keeps a
  spy's `mock.calls` across files when the spy is still referenced, and so does turbo-test.

### Changed
- `src/runtime.js` has one WHATWG UTF-8 decoder (turbo-test#121): `TextDecoder`, `URLSearchParams`
  percent-decoding, `querystring.unescape` and `StringDecoder` share `__utf8Run` instead of three
  copies of the maximal-subpart decoder. No behaviour change.

### CI
- `lint` job: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` (toolchain
  1.98.0). The 28 clippy findings are fixed and the crate is rustfmt-clean (turbo-test#62).

### Tests
- Node-API fixtures `api.c`, `async.c`, `cleanup.c`, `apiversion.c`, `refs.c`, `selfreg.c`,
  `fatal.c` and `unknown.c` with their specs; every spec also passes against Node 26.11 with the
  same compiled addon (but its host-only cases).
- `coloured_output_is_vitests` no longer flakes under load (turbo-test#109): a file or test over
  vitest's 300ms slowTestThreshold turns its duration yellow where vitest's reference is green.
  The coloured normalizer now checks each duration's colour against its measured value, then
  spells it the same whatever the colour.

## [0.6.0] - 2026-10-26 — turbo-dom consolidation

The browser binding and the V8 fork moved into the
[turbo-dom](https://github.com/miaskiewicz/turbo-dom) workspace so turbo-test,
turbo-surf and turbo-html2pdf share one DOM, one V8 fork and one release train
instead of three diverging copies. turbo-test jumps from 0.4.8 to 0.6.0 to line up
with the turbo-dom 0.6.0 crates it now links (turbo-dom-v8 keeps its upstream-V8
150.4.x numbering). Requires `turbo-dom-browser-env` 0.6.0 and `turbo-dom-v8 ~150.4`
from crates.io.

### Changed
- turbo-dom consolidation: the browser binding and the V8 fork moved to
  [turbo-dom](https://github.com/miaskiewicz/turbo-dom). The rtdom <-> V8 binding
  (`src/browser_env*`, its tests, probes and Chrome controls: 398 files) is now the
  `turbo-dom-browser-env` 0.6.0 crate, and the runner calls
  `turbo_dom_browser_env::browser_env::{install, enabled, reset}`. V8 is turbo-dom's
  rusty_v8 fork `turbo-dom-v8 ~150.4` (V8 15.0), replacing the registry crate
  `turbo-surf-v8 =150.4.0` the runner had just moved to from the vendored 149.3.0
  tree. The fork carries the interceptor `ShouldThrowOnError` fix, so strict-mode
  writes through DOM interceptors report the caller's mode. Dependencies only the
  binding used (url, psl, lightningcss, cssparser, turbo-dom, dev sha2) are gone.
  Until the turbo-dom crates are published, `[patch.crates-io]` resolves them from
  git; the section is removed before release (RELEASING.md).
- The binding's Chrome-control collectors moved with it: the 14
  `scripts/*-chrome-control.mjs`, `scripts/typed-om-registry.mjs`,
  `scripts/trusted-types-control/` and `docs/live-css-host.md` now live in
  turbo-dom-browser-env's `scripts/` and `docs/` (moved verbatim from this repository
  at 39b5911), next to the probes and controls they read and write. The README's
  "Native DOM lifecycle controls" section is replaced by a pointer to that crate's
  README.
- Results: every remaining Rust test, `npm test` and fixtures/compat are identical per
  test; the 350 binding tests now run (identically) in turbo-dom-browser-env.

### Fixed
- `npm ci` failed on npm 11 (EUSAGE: `Missing: @esbuild/aix-ppc64@0.21.5 from lock
  file`), so `typescript` was never installed and `npm test` silently skipped its two tsc
  tests (turbo-test#30). `package-lock.json` is regenerated with npm 11 and records every
  optional `@esbuild/*` platform entry; npm 10.9 and npm 11.20 install byte-identical
  trees from it without rewriting it. New CI job `npm-lockfile` runs `npm ci` under
  npm 10 and npm 11 so the lock cannot regress.

The pre-existing bugs found by the coverage push (turbo-test#31-#35) are fixed below.
Each fix has a regression test that fails without it; expected behaviour follows
vitest / jest / Node / Node-API (and tsc / vitest's v8 coverage provider where noted).

Runner and mocking (turbo-test#31):
- JSON jest configs are read: a package.json `"jest"` block (only the top-level key) and
  `jest.config.json` supply `setupFiles` / `setupFilesAfterEnv` / `rootDir` /
  `testEnvironment`, as `jest.config.js` always did. The config-key scanner did not
  accept quoted keys (item 1).
- Under isolate reuse, re-mocking a dependency no longer empties an importer that is
  itself mocked (`require()` returned `{}`), and a file that runs on a fresh isolate under
  the reuse switch (the fresh-isolate retry, `--coverage`) registers a `doMock` as a new
  module instead of patching the real exports object a test already holds (item 2).
- `await import()` of a `TURBO_LAZY_STUBS` barrel settles: the stub namespace was treated
  as a thenable and the import hung (item 3).
- A factory-less `vi.mock('./a')` no longer takes the factory of a later `vi.mock` (whose
  own mock was dropped) or a following test callback (item 4).
- A factory-less `vi.mock(path)` automocks the module without esbuild too. The shared
  automock now follows vitest's algorithm: class statics and prototype methods are mocked,
  and so are class-instance methods. On the ESM graph, a mock's `default` key is its
  default export (item 5).
- `node:buffer`, `fs/promises`, `process`, `timers`, `timers/promises`, `vm`,
  `worker_threads`, `querystring`, `string_decoder`, `constants`, `inspector` and
  `assert/strict` no longer import as an empty module. Each has working named and default
  exports with Node's behaviour on both loader paths (`assert` is Node's full API); what a
  single context on one thread cannot do throws a clear "not available" error. The
  supported set is documented in vitest.compat.md (item 6).
- A bare `vi.hoisted(...)` after a declared one no longer duplicates the earlier
  declaration in the mock prepass (a SyntaxError that dropped the file's prepass mocks);
  bare calls are replayed too, as vitest hoists every `vi.hoisted` (item 7).
- With `emitDecoratorMetadata` and no esbuild, a plain `import { SomeInterface }` used as a
  decorated member's type no longer fails to link ("does not provide an export named");
  type-only imports are read through a namespace import (TypeScript's TS1272 remedy), so
  `design:type` is `Object` (item 8).
- Module-runner load errors report the module's own error: an entry import of a throwing
  dependency shows `Error: <message>` instead of `entry load failed`; `require()` of a
  module whose body throws (or does not compile) throws that error instead of
  `failed to load <spec>`, and `import()` rejects with it (item 9).
- Module-runner mode: `import.meta.url` / `filename` / `dirname` are the module's own
  location instead of undefined (the ESM-graph path gains `filename` / `dirname`), and
  CSS-module default imports are class-name proxies (`s.foo === 'foo'`) (item 10).
- A `vi.mock` / `vi.doMock` factory that throws or rejects no longer silently falls back to
  the real module: importing the mocked module fails with vitest's "There was an error when
  mocking a module … Caused by: <error>"; `jest.mock` rethrows the factory's error (item 11).
- A setup file's `vi.mock` is hoisted above the setup file's own imports (module-runner),
  so the setup sees its own mocks and shares them with the test files; a test file no
  longer reads a setup file's `vi.hoisted` values. Reading an export a `vi.mock` factory
  did not return throws vitest's `No "x" export is defined on the "<spec>" mock` on access
  (module-runner; `jest.mock` keeps jest's `undefined`) (item 12).
- The lenient mock synthetic no longer picks up names from an earlier import's `{...}`
  when the import it reads is default-only (item 13).
- `fs.appendFileSync` is a real `O_APPEND` append instead of read-then-rewrite, so
  parallel workers appending to one file no longer lose lines; `fs.promises.appendFile`
  is added (item 14).
- `TURBO_NO_MR` (legacy bundle path): entry-file `vi.mock` / `jest.mock` targets are
  externalized like setup mocks, so they intercept (they were bundled into the entry and
  never did); the legacy prepass resolves automocks and `vi.importActual` from the entry
  dir. Partial (`importOriginal`) and React-building factories still stay bundled there.
  The "builds React" heuristic no longer mistakes `() => ({ ... })` for JSX (item 15).

CLI and reporters (turbo-test#32):
- A NaN/inf entry in the duration-history cache is ignored instead of panicking the run
  (item 1).
- Every value flag accepts vitest's inline form (`--reporter=json`, `--testTimeout=5000`,
  `--shard=1/2`, `--outputFile=…`); these were warned about and ignored (item 2).
- A value flag without a value (`turbo-test a.test.js --reporter`, a trailing `-t`) is
  dropped with a warning instead of taking the test file as its value. A `-t` pattern that
  starts with `-` needs the inline form (`--testNamePattern=-x`), as with vitest's parser
  (item 3).
- The TAP reporter reports a file that fails to load as `not ok N - <file>` with the load
  error; it was dropped (item 4).
- `--outputFile` with the text/dot/verbose reporters creates missing parent directories,
  and a failed `--outputFile` write fails the run (exit 1) for every reporter (item 5).
- Test `console.*` output no longer corrupts `--reporter json|junit|tap` stdout: it goes to
  stderr while the artifact is on stdout (item 6).
- A test file that collects no tests fails the run with vitest's "No test suite found in
  file <path>" in every reporter, unless `--passWithNoTests` is given (as in vitest). Files
  whose tests are all skipped or filtered out print PASS instead of FAIL (item 7).
- Alias parsing: `__dirname + '/z'` targets resolve, a quoted `find` starting with `/` is a
  prefix match, and `obj.in / 2` no longer hides an alias on the same line (item 8).

Node-API host (turbo-test#33):
- Re-entrant native callbacks (native -> JS -> native, or `require()` of a `.node` inside a
  callback) no longer abort the run (item 1), and each loaded addon gets its own env, so a
  second addon no longer takes over the first addon's functions and references (item 2).
- Pending exceptions follow Node-API: `napi_is_exception_pending` /
  `napi_get_and_clear_last_exception` work, calls that can run JS return
  `napi_pending_exception` while one is pending, and the exception is thrown when the
  callback returns (item 3).
- `napi_get_cb_info` fills missing argv slots with `undefined` instead of NULL (which
  crashed the process) (item 4); `napi_get_value_string_utf8` never writes past
  `bufsize`, copies whole UTF-8 characters and returns `napi_string_expected` for
  non-strings (item 5); `napi_typeof` reports `napi_symbol` / `napi_bigint` (item 6);
  `napi_create_string_utf8` replaces invalid UTF-8 with U+FFFD (item 7); number getters
  wrap like ToUint32 and return `napi_number_expected` for non-numbers, the bool getter
  `napi_boolean_expected` (item 8); a throwing setter makes `napi_set_named_property` /
  `napi_set_element` return `napi_pending_exception` (item 9).
- Status codes and semantics match Node: ToObject / `object_expected`, `array_expected`,
  `string_expected`, an unknown typed-array type is `invalid_arg`, property names include
  the prototype chain, error `code` and function `name` are honoured, a NULL `recv` and a
  raw ArrayBuffer passed to `napi_get_buffer_info` are `invalid_arg` (item 10).
- No panics across the FFI boundary: over-long strings return `napi_generic_failure`, and
  bad typed-array ranges throw Node's RangeErrors (item 11).

Coverage (turbo-test#34; expected values = vitest's v8 provider):
- esbuild's ESM `__export` getters are no longer reported as functions (an uncalled export
  was counted twice, both on line 1) (item 1), and esbuild's ESM prologue no longer marks a
  blank or type-only source line 1 as executed (item 6).
- Top-level module statements and branches are counted (they always read 0 or "not
  reached") (item 2).
- A never-taken braceless `if (c) throw …;` / `return …;` counts as an uncovered statement:
  gen->source columns no longer drift past esbuild's leading-blank segments (item 3).
- Lines of switch cases that never ran (and their closing braces) read 0 instead of the
  function's count (item 4).
- `coverage-summary.json` stays valid JSON with zero file rows (latent) (item 5).

Transform (turbo-test#35):
- TypeScript enums are lowered like tsc: string members no longer get a bogus reverse
  mapping (`enum E { A = 'a', B = A }` emits `E["B"] = "a"`), member references fold to
  constants, and debug builds no longer panic on oxc's enum debug_assert; the oxc semantic
  pass now evaluates enum members (item 1).
- Native ESM->CJS: string-named imports and re-exports whose name is not an identifier
  (`import { "a-b" as ab }`, `export { "a-b" as c } from`) use computed access
  `import_x["a-b"]` instead of `import_x.a-b` ("b is not defined"); export keys with `\` or
  line terminators are escaped correctly (item 2).

The bugs found while fixing those (turbo-test#37-#41) are filed and fixed too.

Native ESM->CJS emitter (turbo-test#37):
- A module whose only import is a dynamic `import()` no longer fails with `__toESM is not
  defined` (item 1).
- `import()` of a module that throws returns a rejected promise instead of throwing
  synchronously: the load is deferred (`Promise.resolve().then(() => __toESM(require(x)))`),
  a computed specifier is evaluated at the call as in Node/vitest, and in the native
  dependency bundler `import()` of an already-bundled module reuses that instance (item 2).
- Module specifiers containing `"` or `\` are emitted as escaped string literals in
  `require(...)`; the invalid native output used to be silently replaced by the esbuild
  retry (item 3).

Runtime (turbo-test#38):
- The `URL` polyfill resolves relative references like Node's WHATWG `URL` (it appended the
  reference to the whole base: `new URL('./x.json', 'file:///d/a.test.ts').pathname` was
  `/d/a.test.ts/./x.json`), so `new URL(rel, import.meta.url)` works; dot segments are
  normalized (item 1).
- An entry file's hoisted `vi.mock` / `jest.mock` factory runs once (it ran in the mock
  prepass and again in the module body) (item 2).
- A setup file that throws (or fails to load or link) fails every test file using it with
  `setup file <path>: <error>`, as in vitest; it was silently ignored. Under isolate reuse
  the error fails every file on that worker (item 3).
- Without esbuild, the entry's mock factories see the file's `vi.hoisted` values (item 4)
  and async factories are awaited before the entry links (item 5).

Mock resolution (turbo-test#39):
- `vi.doMock` followed by `import()` returns the mock on the ESM graph and the `TURBO_NO_MR`
  bundle path too, also for a module imported (real) before (item 1).
- A mock whose specifier has a `?query` is a module id of its own: `vi.mock('./x.js?raw', …)`
  no longer replaces the mock of `./x.js` (item 2).

Coverage (turbo-test#40):
- An `if` without `else` whose consequent always ran was reported as then 0 / else N, and a
  later nested function's range could be taken as the else-count. The then-count is now
  sampled at the consequent (as vitest's v8 provider does) and else = if-count - then;
  matches vitest 4.1.12 on a 119-module corpus of if/else/ternary/logical shapes (item 1).

Node-API host (turbo-test#41):
- `napi_call_function` with `argc > 0` and a NULL `argv`, `napi_get_value_string_utf8` with
  a NULL `buf` and NULL `result`, and `napi_delete_reference(NULL)` return
  `napi_invalid_arg` as in Node (items 1-3).
- `napi_wrap`, `napi_unwrap` and `napi_remove_wrap` are implemented with Node semantics
  (one wrap per object, weak optional result reference, `remove_wrap` doesn't finalize,
  finalizers on collection and at env teardown), so ObjectWrap-style addons can construct
  objects (item 4).
- Addon module init runs under the addon lock like every other entry into addon code, so
  two workers loading the same `.node` no longer run its init concurrently; a nested
  `require()` of a `.node` during init still works (item 5).

Node-API host, runner and mocks (turbo-test#42):
- `napi_define_class` builds the class from its property descriptors, as Node: instance
  methods (with the class as receiver signature: another receiver is "Illegal invocation"),
  accessors and values on the prototype, `napi_static` members on the constructor, with
  Node's attributes. node-addon-api `ObjectWrap` classes (`InstanceMethod` /
  `InstanceAccessor` / `InstanceValue` / `StaticMethod` / `StaticAccessor` / `StaticValue`)
  get their members. `napi_define_properties` and `napi_get_new_target` (which ObjectWrap's
  constructor calls) are new exports. Descriptors Node aborts on (an object or NULL value in
  a class template, no name) are refused with `napi_invalid_arg` / `napi_name_expected`
  (item 1).
- `napi_type_tag_object`, `napi_check_object_type_tag` and `napi_add_finalizer` are exported
  with Node semantics (one type tag per object; any number of finalizers per object, run on
  collection and at env teardown). An addon importing them crashed the run (item 2).
- Finalizers still pending at env teardown run in a handle scope, as Node: value calls
  (`napi_get_reference_value`, `napi_create_object`, ...) work, while calls that would run JS
  stay `napi_pending_exception`. A finalizer's own result reference has no value any more
  when it runs (as Node). Before, such a value call aborted the run (item 3).
- Mock hoisting reads specifiers with JS string escapes (and esbuild's template-literal
  form), so `vi.mock("./a\"b.js", ...)` mocks `./a"b.js` instead of a truncated id
  (item 4).
- Under `TURBO_NO_MR`, a setup file's `?query` mock (`vi.mock('./x.js?raw', ...)`) is
  externalized like an entry file's, so imports of `./x.js?raw` reach it (item 5).
- `vi.doMock` / `jest.doMock` with an async factory, then `import()`, waits for the factory
  as vitest does, on the module-runner, the ESM graph and `TURBO_NO_MR`: the import returns
  the mock (it returned the real module, or timed out on the module-runner), a rejecting
  factory rejects the import with vitest's mocking error, and a later `doMock` / `doUnmock`
  of the module wins over a factory still pending (item 6).

### Removed
- runner.rs dead code listed in turbo-test#31 (`kind_of`, `mock_specifiers`,
  `is_react_family`), and the "mocked but no exports" fallbacks of `native_require` /
  `load_dep`, which could only serve an empty `{}` and were reachable only through the
  turbo-test#31 item 2 eviction bug.

### Security
- esbuild runtime dependency `^0.21.0` (locked 0.21.5) -> `^0.28.2` (turbo-test#36):
  esbuild <= 0.24.2 is affected by GHSA-67mh-4wv8-2f99 (its `serve` dev server lets any
  website read responses). turbo-test only spawns esbuild for transforms and bundles and
  never starts that server, but consumers' `npm audit` flagged the dependency. The
  helpers esbuild emits that the runner patches (`__defProp`, `__toESM`) are unchanged in
  0.28; every Rust test, `npm test` and fixtures/compat give the same results. The lock is
  regenerated with npm 11; npm 10.9 and npm 11.20 `npm ci` install byte-identical trees
  without rewriting it. `test/deps-advisories.test.mjs` keeps the declared, locked and
  installed esbuild outside the affected range.

### Tests
- Line-coverage gate for the Rust crate: 6.09% (361/5926) -> 99.87% of production
  lines, enforced at `fail-under = 99` by the new CI `coverage` job (cargo-tarpaulin,
  Llvm engine, `tarpaulin.toml`). 365 new behaviour-asserting tests: unit tests in
  `src/*_tests.rs` and CLI integration tests in `tests/*.rs` (shared helper
  `tests/common/mod.rs`) that spawn the real binary, the spike binaries and C Node-API
  addons compiled at test time against throwaway projects. Every file is at 100% except
  `src/runner.rs` (8 lines: three branches reachable only through pre-existing bugs, which
  are filed separately, and a V8-unreachable empty-ranges skip; listed in tarpaulin.toml).
- After the turbo-test#31-#35 fixes the crate is at 99.98% (6423/6424 lines) and the gate
  is raised to `fail-under = 99.98`. The one uncovered line is coverage_accumulate's
  "script with no ranges" skip, which V8 precise coverage never triggers. 61 new
  regression tests (455 Rust tests, 64 `npm test`); new integration files
  `tests/runner_c_config.rs`, `tests/runner_c_mocks.rs`, `tests/runner_c_node.rs`.
  After the turbo-test#37-#41 fixes: 6578/6579 lines (99.98%), 476 Rust tests, new files
  `tests/runner_c_runtime.rs` and `tests/runner_c_mockres.rs`.
  After the turbo-test#42 fixes: 6835/6836 lines (99.99%), 484 Rust tests; new C Node-API
  fixtures `tests/napi_support/objectwrap.c` (a node-addon-api-style ObjectWrap class) and
  `typetag.c`.
- Testability refactors, output byte-identical: `src/bin/turbo_test.rs` is now a thin
  wrapper over the new `cli` module (argument parsing, the retry/fresh-isolate policy,
  the worker pool and pure reporter renderers); `launcher::prepare` wraps a side-effect-free
  `plan()`; coverage reporting, napi panic guards, bundler/esm_cjs/runner helpers were
  split into pure functions.
- CI: `npm ci` now runs before `cargo test` on every non-Windows build leg (the
  integration tests link its esbuild/typescript installs).
- `tests/interceptor_should_throw.rs` is no longer ignored: its 4 tests (consumer copies
  of turbo-dom-v8's) pass against the fork. `release_v8_teardown.rs` and
  `weak_teardown.rs` stay as consumer checks.
- `scripts/check-single-v8.sh` (CI build job): exactly one `turbo-dom-v8`, and no `v8`,
  `v8x` or `turbo-surf-v8`, in the resolved build graph.

### Removed
- `vendor/` (the vendored rusty_v8 149.3.0 fork, its patch, source pins and the
  update/release/test scripts, `vendor/RUSTY_V8.md`): the fork's patches, pins and
  generator now live in turbo-dom's `crates/turbo-dom-v8`.
- CI: the tag-only `publish-v8` job and the manual `recover-v047-v8.yml` recovery
  workflow (both published or verified `turbo-surf-v8` 149.3.0). This also drops the
  unreleased registry-API absence check and v0.4.7 recovery procedure that lived in
  those scripts; turbo-test publishes no V8 crate any more.

## [0.4.8] - 2026-10-25 — Chrome-parity batch for turbo-surf#28

Closes a large set of measured missing DOM members and behaviours from
turbo-surf issue 28's paired API inventory. Every closed surface has an
independently captured Chrome 156.0.8078.47 / V8 15.6.75.19 control
(`src/*_chrome156.json`, probe SHA recorded) and an offline regression that runs
the same probe in the binding and asserts equality; irreducible deltas are pinned
as explicit KNOWN_GAP assertions, never skipped. No presence-only stubs: members
that cannot be truthful without a browser stay open. Integration PR: #29.

### Added
- HTMLMeterElement / HTMLProgressElement / HTMLOutputElement reflection, clamping,
  `htmlFor` token list and Chrome's output default-value/reset behaviour.
- HTMLSelectElement collection API (`add`/`remove`/`item`/`namedItem`/`length`/
  `size`/`type`/iterator, indexed access) and a live, same-object
  `HTMLOptionsCollection` for `select.options`; live `selectedOptions`.
- HTMLInputElement / HTMLTextAreaElement members (`stepUp`/`stepDown` with Blink's
  Decimal arithmetic, `valueAsDate`/`valueAsNumber`, `maxLength`/`minLength`,
  `size`/`cols`/`rows`/`wrap`, `indeterminate`, `list`, `textLength`, `type`, …)
  and Chrome value sanitization per input type.
- File-state inputs: `FileList`, `input.files`, `webkitEntries`, fakepath values,
  file form entries, and the `DataTransfer` / `DataTransferItemList` family.
- Invoker commands (`command`, `commandForElement`, `popoverTargetAction/Element`,
  `CommandEvent`) and genuine `HTMLDialogElement` behaviour (`show`/`showModal`/
  `close`/`requestClose`/`returnValue`/`closedBy`, events, `:modal`/`:open`,
  focusing steps, `method=dialog`).
- `getHTML`/`setHTMLUnsafe`, ShadowRoot init state (`clonable`/`serializable`/
  `delegatesFocus`), `activeElement`, `styleSheets`, `onslotchange`; declarative
  shadow DOM in the main document parse and `Document.parseHTML[Unsafe]`.
- HTML Sanitizer API and safe `Element`/`ShadowRoot` `setHTML`.
- `ShadowRoot.referenceTarget`, interest invokers (`interestForElement`,
  `InterestEvent`), and `focusgroup` arrow-key navigation.
- `document.all` (undetectable `HTMLAllCollection`), `domain`, `lastModified`,
  inert fullscreen / pointer-lock / picture-in-picture state.
- Pointer capture and pointer lock state machines driven by trusted input.
- Web Animations (`Element.animate`, `getAnimations`, `document.timeline`,
  `Animation`, `KeyframeEffect`, …) applied through computed style.
- CSS Typed OM Level 1 (`computedStyleMap`, `attributeStyleMap`, `styleMap`, all
  value classes) with a Chromium-156-generated property registry.
- CSS `zoom` grammar and computed value; `Element.pseudo()` / `CSSPseudoElement`;
  `Document.fragmentDirective`; `webkitMatchesSelector`; eight Element-level event
  handlers; CustomEvent/PopStateEvent/HashChangeEvent/ProgressEvent/CloseEvent;
  native link URL members on `<a>`/`<area>`.

### Fixed
- Strict CSSOM selector admission: unknown pseudo-classes are rejected like Chrome
  (forgiving `:is()`/`:where()` preserved); `querySelector`/`matches`/`closest`
  now throw `SyntaxError` for selectors Chrome rejects. The previously ignored
  unknown-pseudo regression now passes.
- Implicit inherited `color`/`visibility` see CSSOM rule mutations.
- Input-only members removed from the wrong control prototypes (HTMLLabelElement,
  HTMLSelectElement, HTMLTextAreaElement, HTMLOptionElement); control members
  enumerable as in Chrome.
- `document.activeElement` retargets out of shadow trees; `form.elements` works for
  forms inside shadow roots; a plain `new Event('click')` no longer runs checkbox
  activation; `<noscript>` serializes literally with scripting enabled.

### Known gaps
- Composed events dispatched inside a shadow tree still don't reach the host's
  ancestors; rusty_v8 `should_throw_on_error()` deltas on interceptors; prototype
  own-key order (`constructor` first). Tracked in turbo-surf issue 28.

## [0.4.7] - 2026-10-21

Release the combined browser ownership/lifecycle and computed-style follow-ups
from PRs #26 and #27. Publish the reviewed Rusty V8149.3.0 teardown fix as
`turbo-surf-v8` before the runner crate, preserving the upstream SDK version and
native archive URLs. Packaged consumers resolve the fork through a real package
alias rather than losing a root-workspace patch. Live DOM, forms, CSSOM and constructor controls retain their
independent Chrome evidence. Computed style reuses ancestor snapshots only within
one public read and skips declaration work for unmatched private selectors.
The unknown-pseudo CSSOM admission regression remains explicitly ignored with
Chrome's expected SyntaxError; its explicit expected failure is not counted as
passing conformance. Broader browser parity remains tracked in turbo-surf issue28.

- Match private style-rule selectors before reading declarations, avoiding full
  serialization of unmatched rules on every ancestor calculation. The captured
  native matcher receives complete selector lists, preserving quoted braces and
  commas; non-style-rule fallbacks and import/source order remain unchanged.
  Add work-count, live CSSOM and hostile-getter controls without a persistent
  computed-style cache. Preserve a separate Chrome-expected, explicitly ignored
  regression for preexisting unknown-pseudo CSSOM admission (turbo-surf issue28);
  its expected failure is recorded rather than treating acceptance as correct.
  Browser-specific strict/forgiving selector admission remains a separate feature.

- Reuse all resolved ancestor computed-style snapshots within one public read,
  including percentage lookups through auto-sized ancestors before font
  inheritance revisits an earlier node. Capture private WeakMap intrinsics at
  bootstrap and expire the cache before the next live declaration read. Bounded
  scan, mutation/reparenting and hostile-prototype controls prevent redundant
  ancestor work without introducing a persistent computed-style cache.

- Preserve parsed `!important` priority per computed longhand, including shorthand
  expansion and inline styles. Later normal declarations no longer overwrite
  important stylesheet values. Priority stays private; live stylesheet adoption,
  mutation and computed color/length normalization still apply. Include private
  document-adopted sheets after ordinary sheets without importing them into shadow
  scopes. The temporary adopted-sheet copy has no Array prototype, so numeric
  prototype setters cannot observe or alter this private collection. Genuine Chrome155
  controls cover the nine measured border precedence cases.

- Implement the measured constructible Text and DocumentFragment interfaces using
  captured native document factories. Preserve instance prototypes, constructor
  inheritance (including ShadowRoot), subclass `new.target` prototypes and owner
  documents, with lossless UTF-16 and Web IDL string conversion. Public factory
  poisoning does not intercept construction. Update stale compatibility checks
  against genuine Chrome constructor, shadow insertion and computed-color controls.

- Preserve Node constructor inheritance when installing the listener-backed
  EventTarget constructor; keep the original EventTarget instance prototype and
  native Node listener behavior intact. Add relational constructor/dispatch
  controls for the public interfaces.

- Seed measured Chrome155 button declarations at user-agent origin for computed
  style reads, including the normal Arial 13.3333px font reset, spacing and border
  widths. Author universal rules, inline values, explicit inheritance and initial
  font values take precedence; `revert` restores the button UA declaration.
  Source-bound independent Chrome controls cover default and inherited parent
  fonts, authored overrides and CSS-wide font keywords. Other form-control UA
  defaults and broader computed-value behavior remain outside this bounded fix.

- Normalize ASCII casing for the existing supported computed-font units and
  grammar keywords using a captured private lowercase intrinsic. Preserve quoted
  and custom font-family spelling and custom-property names. Native deferred-value
  admission now recognizes the same five CSS-wide keywords case-insensitively
  without accepting arbitrary identifiers. Additional independent Chrome155
  cases reproduce and prevent mixed-case units yielding `NaNpx`, uppercase
  shorthand misclassification and valid `INITIAL` resets being discarded.
  Preserve scalar `calc()` line-height as an inherited factor rather than
  misclassifying it as pixels. Unit-cancelling division remains unresolved.

- Resolve computed font longhands and shorthand from actual owner-document
  preferences and inherited values, including unitless versus absolute
  line-height, relative sizes, arithmetic `calc()`, CSS-wide keywords and native
  named/manual slot inheritance. Live declarations refresh after property-name
  conversion, retain receiver brand checks, and return empty font values after
  disconnection or browsing-context retirement. Independent Chrome155 controls
  cover fractional serialization, inert/template ownership, realm borrowing and
  adoption. Native embedders can set private scalar initial font preferences;
  macOS `Times` is measured, while the other-platform `Times New Roman` default
  remains uncertified. Metric-dependent units, viewport units, `min`/`max`/`clamp`,
  custom-property substitution and other per-element UA defaults remain outside this
  bounded computed-font resolver; unresolved text survives inheritance without
  fabricated zero or `undefined` values.

- Add a host-only custom-element context retirement boundary. Retained frame
  registries, definitions and existing instances stay usable, while automatic
  construction, upgrades and reactions stop for the retired definition realm,
  including already queued reactions and callbacks borrowed from an active
  realm. Fresh Chrome155 controls cover detached-frame creation, innerHTML,
  contextual fragments, explicit upgrades and existing-element adoption.

- Clear constructed adopted stylesheets on every migrated ShadowRoot during
  cross-document adoption, including nested closed roots, before custom-element
  adopted callbacks. Preserve retained ObservableArray identity, same-document
  adoption and disconnect/reconnect state, while bypassing poisoned public
  ownerDocument/adoptedStyleSheets getters and array methods. Fresh Chrome155
  controls cover unread roundtrips, foreign-realm sheets and assignment rejection.

- Share custom-element reaction queues across nested CEReactions frames. Advance
  the shared cursor before invoking a callback so reentrant adoption preserves
  already queued callbacks, per-element order, exception reporting and the
  original execution limits. Fresh Chrome155 controls cover callback realms,
  error receivers, two-element interleaving and nested attribute reactions.
- Parse Range contextual fragments through the public turbo-dom development
  revision `aaeb290`, using the actual post-conversion boundary context,
  namespace/integration attributes, form ancestry and document quirks mode.
  Preserve foreign and template-associated owner documents, scoped registries
  and inert nested templates without invoking a context-element constructor.
- Parse template innerHTML into a detached native fragment before inert-owner
  adoption, preventing premature custom-element construction and attribute
  callbacks. Add a bounded independent Chrome155 parser/observer lifecycle
  matrix with19 context cases, mutation notification/reentrancy controls and
  reusable `scripts/dom-lifecycle-chrome-control.mjs` collection. XML parsing
  and complete browser lifecycle parity remain separate work.

- Preserve LF checkout bytes for native JavaScript sources and JSON controls so
  independent Chrome source-hash assertions also verify on Windows. Licensed
  XPath provider bytes retain their explicit no-conversion exceptions.

- Reuse each private CSS rule record already resolved during import flattening
  instead of looking it up again immediately in computed-style evaluation.
  Preserve import/media traversal, source order and live retained declarations;
  nested-import edits and detached-rule mutations have regression coverage.

- Read already resolved parent computed values from their private backing records
  within one calculation, avoiding exponential live-getter recursion through
  inherited colors and percentage sizes. Retained external declarations still
  refresh on subsequent reads. Independent Chrome mutation controls and bounded
  stylesheet-scan regressions cover inheritance plus width/height chains.

- Refresh the private stylesheet-list backing array once per computed-style
  calculation instead of rescanning every style/link for each live-list index
  and length read. Retained computed declarations still recompute on every
  property access; independently measured Chrome controls cover CSSOM edits,
  stylesheet insertion/removal/replacement, disabled sheets and zero public
  document/style-list getter calls. No persistent
  computed-style cache or browser-conformance claim is introduced.

- Add genuine live, readonly `labels` NodeLists on Input, Button, TextArea,
  Select, Output, Meter and Progress. Native light-tree associations preserve
  first-ID targeting, null-namespace attributes, exact UTF-16 IDs,
  hidden-input transitions, shadow scopes,
  adoption and receiver-realm collection identity; share that authority with
  ElementInternals and label.control, including custom controls before builtins.
  Independent Chrome controls cover descriptors, wrong receivers, live mutations
  and hostile public methods. Production iframe/retirement GC validation remains
  a separate renderer gate with an explicit real-task collection policy.

- Make the adopted-radio GC control resolve readiness without retaining the
  load Event, clear its listener and record an explicit real-task GC policy.
  Preserve the original non-repeatable WeakRef receipt, three fresh original
  runs, three settled Chrome controls and actual native PageSession proof;
  retained radios keep their genuine owner until release without extra roots.


- Preserve an independent Chrome adversarial form-owner control for first-ID
  non-form/SVG/HTML-case collisions, open/closed shadow boundaries, disabled
  radio groups, external reset order, split moves and genuine XML documents.


- Index selected radios by native node and prior native form owner, without
  JavaScript roots. Normalize checked descendants and batch fragment order on
  container moves, keep shadow-tree form lookup within its actual root, and
  reassociate external radios after form ID/removal changes. Route the seven
  existing control `.form` getters through genuine native ownership with readonly
  per-interface brands and descriptors. Preserve detached
  unowned radio behavior, genuine checked-accessor brands, cloning/adoption and
  native storage reclamation with independent Chrome controls. Empty indexes
  skip descendant traversal on ordinary non-radio moves.


- Keep radio grouping in the native checkedness intrinsic, including checked/name/
  type/form attribute changes, parsed subtrees, directly moved radios, and form reset. Restore
  the prior selected radio after canceled clicks without guest checked hooks.
  Preserve native checkedness overrides across actual cross-document adoption;
  source-pinned Chrome controls cover dirty defaults, cloning, live old/new form
  collections, cancellation and forced-GC observations.


- Add genuine live HTMLFormControlsCollection and RadioNodeList interfaces, form
  elements/length/iteration, owner-aware external/custom controls and cached named
  groups. Private records support borrowed base accessors across real V8 contexts,
  preserve measured named/index ordering and mutation behavior, and release owner
  realms after retained collections are dropped. Independent Chrome controls cover
  descriptor shapes, live membership, radio values, collisions and DOMString errors.
  Broader radio dirty/default/reset/adoption lifecycle remains active follow-up;
  genuine production iframe/retirement controls are supplied for integration gates.

- Expose genuine Element.customElementRegistry with native receiver checks and
  private creation/parser associations. Preserve scoped/null registries across
  moves; global associations follow adoption to the new owner document without
  retaining the former global registry; inert-null adoption rebinds correctly,
  including unread descendants, ShadowRoots and multi-hop moves. Parse-completion
  hooks record ownership before any public getter read, without enabling plain-DOM
  reaction recording.
  Independent Chrome controls cover unread parser nodes, adjacent HTML, inert
  documents, cross-realm borrowing/adoption and property descriptors.

- Reflect Input/Button formAction, formEnctype, formMethod and formTarget on their
  own prototypes with native owner-document/base URL resolution, per-interface
  string conversion, missing/invalid enum defaults and genuine receiver checks.
  Independent Chrome controls include malformed URLs, attribute writes, borrowed
  accessors, wrong namespace/local-name receivers and hostile public getters.

- Match computed styles through captured genuine native selector and owner-document
  intrinsics. Page overrides of matches/ownerDocument are not invoked internally;
  live stylesheet mutations remain visible without new cascade caching.

- Keep descriptor rules out of nested cssRules lists so the private native CSSHost
  snapshot retains @property/@view-transition descriptor bodies. Independent Chrome
  shape controls and poisoned-public-getter export regressions cover the boundary.

- Validate complete @property and @view-transition descriptor blocks before
  checking rule shells. Preserve validated registration descriptors and typed
  navigation/types instead of dropping them as ordinary CSS declarations. This
  retains CSSOM source; it does not implement property registration semantics.

- Make the genuine ValidityState constructor prototype read-only, matching Chrome;
  extend the independent form control oracle with descriptor and native brand checks.

- Match Input/Button own prototype placement, enumerable control descriptors and
  text-selection conversion/errors; setRangeText performs real replacement and
  selection updates rather than an inert compatibility method.

- Add measured native HTMLFormElement reflected attributes and live relList,
  including action URL resolution from the host-installed document URL, enum
  normalization, encoding alias, USVString action conversion and branded accessors.
  Independent Chrome controls include relative/base and malformed URL serialization.

- Suppress native form submission for disconnected forms before validation/events
  and recheck connection after submit listeners, using the captured native Node
  accessor. Independent Chrome controls cover removal and reconnection timing.

- Add branded ElementInternals and CustomStateSet with native custom-element
  admission, form ownership/validation, live labels, default ARIA and closed-shadow
  access. Association, disabled and reset callbacks use native reaction queues.
  Private V8 records preserve cross-realm brands and object-owned lifetimes.
- Match native `:state()` selectors with exact UTF-16 membership, fresh clone/import
  state and adoption migration. Branded setlike iterators resume after exhaustion
  when new entries appear and retain their actual owner without host roots.
- Add genuine FormDataEvent identity/descriptor behavior and host-only trusted
  dispatch plus guarded form-entry construction transactions. Runtime adapters
  provide real private File/FormData snapshots; a standalone binding supplies no
  networking backend. Native FileList and radio grouping remain follow-up work. Dirty checkbox state, reset, clone and typed :checked now use native records.
- Pin the public turbo-dom development revision
  `fb60e6f90a5d373165cfd1045b090407ca945d47` for native custom-state selectors;
  this does not imply registry publication of its 0.5.7 candidate.

- Preserve FormData optional argument conversion and current-document authority
  when no form is supplied; reject invalid HTMLElement submitters before the
  empty result. Add a native base-target authority accessor for navigation adapters.

- Expose a host-only native form entry-list constructor for private FormData
  adapters, preserving submitter validation, duplicate ordering and UTF-8 hidden
  charset values without dispatching validation or submit events.

- Capture an optional host-only File/FormData value reader through document-owned
  private V8 edges, so runtime adapters can provide genuine immutable snapshots
  without trusting guest object shape or public metadata properties.

- Add native-document-owned submission admission and queue callbacks with immutable
  ordered text entry snapshots, submitter overrides and hidden charset-entry metadata.
  Preserve validation/cancellation before transport; native FileList and full select
  state support remain explicit follow-up work. Independent Chrome155 FormData
  controls cover duplicate names, disabled/checked controls, text, charset and submitter.

- Replace unconditional form-validity answers and no-op custom validity with live
  native-DOM control state, branded ValidityState objects, required/custom/type/
  pattern/numeric checks, trusted non-bubbling invalid events, and form validation.
  Add a branded SubmitEvent and requestSubmit argument/admission/validation/event
  behavior, preserving native submitter and Chrome155 composed-event differences.
  Capture live control values and native accessors instead of reading guest overrides;
  form-owner lookup includes explicit form IDs. Uncanceled navigation still needs a
  consumer transport and reports a clear NotSupportedError instead of pretending
  to submit; click default-action failures are reported as engine exceptions.

- Implement Chrome155's AbstractRange → NodeRange → Range/StaticRange interface
  hierarchy with native descriptors, constructor inheritance and genuine private
  static boundaries. Static offsets retain their original values through native
  tree mutations; getters accept authentic foreign-realm records and reject forged
  or proxied receivers. Match sorted Web IDL dictionary conversion, unsigned offsets,
  invalid node types, subclassing and poisoned public intrinsic controls. Keep live
  Range mutation tracking unchanged. Real iframe retirement is a separate consumer gate.

- Preserve quoted commas, comment-looking strings and escaped commas when CSSOM
  serializes selector preludes. A lexical scan normalizes only actual separators;
  fresh Chrome155 controls cover parsing, selectorText changes and cssText reloads.

- Add a host-only per-document form-submission admission flag for sandboxed
  consumers. Block the submit default action after an uncanceled control click
  while preserving the click event; keep policy isolated across native documents.
  The consumer supplies sandbox/CSP intersection; this does not implement form
  navigation or destination authorization.

- Add native private-state Trusted Types factory, policy and value brands, exact
  UTF-16/USVString conversion, callback conversion and Chrome155 interface descriptors.
  Cross-realm values retain their native brands without host roots. Authored Chrome
  controls cover sink-type mappings and conversion errors. Native HTML, script,
  attribute and Range sinks invoke immutable document-owned host authorization;
  additive exports let consumers enforce parser/timer sinks and supply exact private
  TrustedScript source to the engine without guest coercion. Production CSP and
  eval/timer execution remain responsibilities of the consumer runtime.

- Implement DocumentFragment.getElementById, inherited by ShadowRoot, through
  iterative native tree-scope lookup. Preserve exact UTF-16 IDs, first-match order,
  empty/missing behavior and native conversion/receiver errors without invoking
  replaceable query/getAttribute methods. Extend the independent installed Chrome155
  factory control with fragment/closed-shadow lookup, intrinsic-poisoning and ID changes;
  add deep lookup and scalar-repair collision regressions.

- Claim pending native mutation batches across all documents before invoking resource
  consumers. A consumer's nested query can no longer drain another document's old
  removal events into its inner custom-element reaction scope, so cross-realm adoption
  callbacks observe the final ownerDocument. Add a regression with querying consumers
  in both realms against the unchanged independent Chrome155 lifecycle control;
  genuine nested mutations, interruption and GC contracts remain covered.

- Implement genuine CustomElementRegistry and HTML element constructors over native nodes:
  autonomous/customized construction, late upgrade, live attribute/connect/disconnect/adopt
  callbacks, scoped registry lookup/initialization, clone/import behavior and trusted construction
  error reporting.
- Add native per-element custom-element reaction queues, including moveBefore connectedMoveCallback
  and fallback lifecycle ordering; native RAII scopes recover from interrupted constructors,
  definitions and callbacks without retaining page realms.
- Replace isolate-rooted JavaScript collection-record maps/functions with context-free V8 Private
  keys and per-object records; unused installed document contexts, custom-element nodes and scoped
  shadow cycles now collect. Preserve cross-realm brands, frozen/proxy record support and
  interruption behavior.
- Add six fresh, source-hashed normal Chrome155/V8 .23 control pairs plus native
  GC/termination/private-payload regressions. Form-associated ElementInternals lifecycle remains
  follow-up work; these tests establish the measured contracts rather than full browser conformance.

- Consume verified published `turbo-dom` 0.5.6 for parser provenance and native
  template extraction; the dependency resolves from crates.io with its published
  checksum, without a private DOM path override.

- Preserve the native Window `onstorage` listener slot when combining parser
  provenance and inert-template prerequisites. The handler keeps branded
  descriptors, listener ordering, replacement and non-callable clearing; its
  standalone regression is retained from the frozen storage-events checkpoint.

- Preserve parser-time style-attribute CSP checks inside genuine template contents.
  Native preorder follows the private template graph and passes the parsing
  document's policy without activating inert stylesheets or inventing source
  positions. The combined standalone cohort passes 214 contracts; the downstream
  Chrome control retains its document-targeted report for a template attribute.

- Combine the reviewed real associated inert template implementation with native
  parser provenance and original-unit CSS callbacks. Template contents use their
  actual separate document, so active queries and stylesheet authorization do
  not cross into them. Imported inert storage conservatively has no new lexical
  source token. The combined standalone cohort passes214 contracts; the additive
  backend dependency is published in turbo-dom0.5.6.

- Preserve authoritative original UTF-16 code units in native stylesheet and
  style-attribute authorization callbacks. Scalar policy compatibility views
  remain separate. A native regression covers independent lone high/low units
  in both sources, supporting exact CSP report-sample delivery downstream.

- Add opt-in original UTF-16 parser source positions for host installation and
  private native node/attribute provenance. CSS authorization receives trusted
  parser metadata and actual Window ownership. Inert template styles remain
  unprepared until active adoption; initial and dynamic authorization stay
  distinct. Native controls cover CRLF/non-BMP positions, illegal objects,
  poisoned guest document access and template activation. Requires the additive
  turbo-dom source-position API; full runtime report ingress is tested separately.

- Implement HTMLScriptElement.text through native direct child Text/CDATA code
  units and the existing native text replacement/preparation pipeline. Fresh
  Chrome controls cover brands, descriptors, coercion, original units and actual
  execution on connection or filling an empty connected script. Failed conversion
  preserves nodes, Range boundaries and observer queues. The merged binding passes
  179 standalone contracts; Trusted Types policy enforcement remains pending.

- Consume published turbo-dom 0.5.5 and preserve original UTF-16 payloads, names,
  namespaces, attributes, mutation records, parser input and inline handler/script
  source through native two-byte V8 strings. Scalar compatibility views remain at
  existing host boundaries. Independent document installation and iterative script
  preparation cover real ownership and 8192-level host-stack regressions. Fresh
  Chrome controls cover surrogate identity, BOMs, lookup/reflection, cloning and
  source text. The merged standalone binding passes 177 native contracts; Surf
  runtime module/frame source integration remains a separate pending gate.

- Expose host-only native composed children/parent identities for layout. Closed
  roots and manual assignment use private records; hidden fallback and unslotted
  children have no composed parent. A native regression poisons public slot APIs
  and preserves authentic internal projection without exposing closed roots.

- Implement native named/manual shadow-slot distribution, live assignedSlot,
  fallback flattening and trusted slotchange in the shared observer notification
  microtask. Original wrapper edges preserve foreign adoption and manual stealing;
  clearing an assignment releases its old slot. Source-pinned Chrome controls cover
  brands, dictionaries, closed roots, light slot elements and reentrant ordering.
  Actual V8 collection and a 4096-level iterative fallback walk guard retention
  and native stack behavior. The complete binding cohort passes 165 tests.

- Reflect HTMLIFrameElement.credentialless through genuine HTML iframe brands and captured native attribute operations; boolean conversion, descriptors, poisoned public methods and illegal receivers match a fresh Chrome 155 control.

- Add the native-branded, live iframe sandbox DOMTokenList with Chrome155 token
  support. Token-list PutForwards setters honor own value setters and failed
  ordinary assignment; source-pinned Chrome controls cover identity, descriptors
  and HTML namespace/local-name receiver behavior.

- Reflect HTMLMetaElement name/httpEquiv/content/media/scheme through native attributes, with genuine HTML namespace/receiver brands, DOMString conversion and private native method capture. Saved normal Chrome155 descriptor/property/attribute/illegal-receiver controls cover the complete narrow surface.

### Fixed
- Dispose cached native worlds safely after binding thread-local cells have been
  destroyed. Availability guards preserve normal isolate restoration while the
  isolate-owned saved state still clears before V8 handle invalidation. A real
  thread-exit regression covers the cached-runtime initialization/drop order.
- Partition native document, listener, observer, CSS and focus state by its actual
  V8 isolate. Scoped activation restores the previous isolate/document after
  nested work, cold installation and disposal; resetting or dropping one isolate
  preserves its peers. Eight regressions cover identity, independent observers,
  reset/disposal ordering and activation guards that outlive their isolate.
- Consume published turbo-dom 0.5.4 bounded storage collection through its public
  API. Genuine weak wrappers, private ownership edges and host-job node pins keep
  reachable nodes alive; successful collection also removes stale scalar records.
  Thirteen contracts cover retained/adopted nodes, pending mutations, job pins,
  budgets and teardown. Collection is explicit; automatic host-boundary scheduling
  remains consumer work, and multi-document calls are atomic per Tree.
- Resolve computed colors through the public cascade serialization and private
  linked-sheet values. Cover inherited/default CanvasText, CSS-wide keywords,
  currentColor, disconnected elements and live declarations against a fresh
  source-pinned Chrome 155 control. Broader system-color/color-scheme, variables
  and layer cascade behavior remains open.
- The combined canonical browser binding cohort passes 158 contracts on the
  default 149.3.0 SDK. Consumer runtime scheduling and genuine native155 checks
  remain separate; inherited runner warnings prevent claiming a strict full-repo
  gate from this focused result.
- Select HTML interfaces by exact namespace/local name rather than uppercasing
  XML-created names. Preserve genuine HTMLUnknownElement versus valid custom-name
  identity, reserved names and legacy generic/preformatted aliases. Control getters
  and methods check native namespace/local-name brands even when borrowed or when
  public prototypes are changed. A fresh 723-row authored Chrome155 matrix covers
  these HTML/null-namespace mappings and foreign control brands; SVGScriptElement
  and SVGImageElement projection/resource behavior remain separate open gaps.
- Allocate native ownership component/helper maps in the actual Document realm.
  A foreign script getter cannot retain its retired Window through another
  Document’s map shape. Observer event draining no longer dirties unchanged graphs.
  Two new regressions bring the current canonical cohort to 134 passing contracts.
- Store script parser/force-async/already-started flags as native scalar node state.
  Preserve clone/import/adoption and inert/retired-document behavior; prepare native
  candidates after observer fanout, including nested and shadow-root scripts. Six
  new contracts and a fresh Chrome155 control cover this subset. The integrated
  canonical cohort passes 132 tests; runtime fetch/execution wiring, parser reentry,
  SVG scripts, import maps and speculation rules remain open.
- Keep Rusty V8 weak handles live through native slot/finalizer disposal using a
  source-pinned, reproducible 149.3.0 dependency patch. Preserve its real SDK/native
  engine identity and upstream license; the original teardown regression fails,
  while 16 patched isolates and all 126 canonical binding contracts pass.
- Make private helper publication fallible during iframe budget interruption,
  preserving pending V8 termination and leaving interrupted helpers unpublished.
  Normal construction and a canceled-interrupt retry remain covered.
- Register inert document initializers on their callback creation realm. Shared
  embedder ops no longer overwrite the main Window initializer with the last iframe
  closure; a heap-traced retained-Document regression identified that native root.
- Pin the captured RegExp exec intrinsic on private XML-name validation, so replacing
  RegExp.prototype.exec cannot intercept native document factories. Validate doctype
  delimiters directly; the expanded Chrome155 control covers seven poisoned intrinsics.
- Replace native wrapper/helper/context roots with weak lookups and private
  Document/component ownership. Keep genuine receiver and Node-argument owners
  pinned during native callbacks, preserve ordinary exceptions and materialized
  V8 termination, and invalidate interrupted wrapper publication before retry.
  The combined canonical ownership/observer checkpoint passes 124 contracts;
  dependency disposal regressions are tracked separately from that binding gate.
- Deliver native MutationObserver records through one V8 notification microtask,
  with per-observer options, old values, namespace filtering, transient removed
  subtrees, static branded NodeLists and receiver checks. Compound insert/replace
  algorithms emit their semantic batches, preserving ordered source removals and
  failed cross-document conversion. Saved Chrome 155 controls and hostile builtin
  regressions cover this subset. Add a trusted Document-private mutation consumer
  for resource postconnection work, with observer fanout first and nested mutations.
  Broader lifecycle, shadow slots, custom elements and UTF-16 work remain open.
- Retain the native node projector privately on its owner Document, avoiding
  page-replaced constructor and public projector getters during first wrapping,
  including parser-created controls and independent documents. Reject inherited
  tag-map entries and keep interrupted wrapping out of the identity cache. The
  canonical browser binding cohort passes 119 contracts.
- Preserve XML element case and document-specific default namespaces for native
  createElement, including borrowed receivers and colon-containing XML names.
  Keep createElement local names distinct from createElementNS qualified names;
  saved Chrome controls cover HTML, XML, XHTML and SVG document factories.
- Create genuine independent HTML/XML document trees through branded
  DOMImplementation factories and Document constructors. Preserve native doctype
  adoption identity, title conversion, receiver checks and foreign stylesheet
  rejection, without page-replaced RegExp/String/Array/Map/Object hooks. Cover
  ordinary parsed documents without factory metadata as well as the saved Chrome
  155 factory control. XML parsing, CDATA and downstream XML helpers remain open.
- Capture native node projection methods so first-time wrapping of Text, Attr,
  Comment, elements and fragments cannot call page-replaced Function, WeakSet,
  Object, Array or String methods. Cover this alongside repeated document installs
  and the XPath private-intrinsic regression.
- Consume published turbo-dom 0.5.3, including its additive mutation-event metadata
  for observer delivery; browser observer queues remain a separate integration task.
- Preserve native Node traversal/equality descriptors when rebuilding a document
  in an existing realm; project compatibility delegates from that bootstrap's
  actual raw node prototype rather than a previous constructor's prototype.
- Adopt genuine foreign nodes during all native insertion methods after validating
  the full destination sequence. Preserve cached node/descendant identity, listener
  state and source Range adjustment. Single-node insertion validates references
  and Document hierarchies before changing an owner or parent. DocumentFragment and ShadowRoot
  insertion moves their children while keeping the original container and owner;
  repeated arguments retain their observed ordering. An independent Chrome 155
  collector covers borrowed methods, namespace data and single-node failure state.
- Match Chrome155 variadic conversion before destination checks: drain fragments,
  detach multi-node sequences, retain the last duplicate and resolve viable sibling
  references. Failed compound insertion leaves converted nodes detached in their
  source document and updates source Ranges; failed WebIDL conversion has no such
  effects. A shared authored Chrome fixture covers failure and fragment overlap.
- Expose the real empty, stable NodeList from Attr.childNodes with native receiver
  branding. Include Attr and CDATA kinds in the Node receiver predicate; the HTML
  binding still rejects createCDATASection and does not add an XML parser.
- Make binding cleanup safe during thread-local runtime destruction regardless
  of initialization order. Clear every remaining private handle cache while
  treating already-destroyed TLS cells as released, including diagnostic storage.
- Keep live Range boundaries synchronized with insertAdjacentHTML, outerHTML,
  innerText and outerText replacement. Match Chrome's parent-offset collapse when
  outerText appends and removes adjacent text, instead of applying normalize's
  boundary-transfer behavior. The exact shared mutation collector and saved Chrome
  155 control cover all four insertion positions and empty/multiline replacements.
- Read stylesheet text, media and title through captured native DOM accessors,
  preserving actual sheet identity across CSSOM edits despite page-defined getters.
  Add a Rust-only CSSHost live export with private rule/declaration state, disabled
  and media values, grouping and attached import provenance; bypass public getters,
  iterators and poisoned prototypes with explicit graph/text budgets. Correct
  font-face descriptor validation, mutable declaration edits and URL serialization.
  Seven offline contracts include an independent installed Chrome155.0.8059.79 control;
  this does not implement layout, font loading or complete CSS interface shapes.
- Expose native-only generic EventTarget listener observation for embedding lifetime
  management, using genuine private listener state and the creation realm. Cover
  add/remove/once/abort cleanup, foreign targets and forged receivers without guest
  getters or page-visible registration capabilities.
- Update live Range boundaries through private weak native records for text edits,
  UTF-16 offsets, splitting, insertion, removal, bulk child replacement, normalization,
  state-preserving moves and node adoption. Match an independent Chrome155.0.8059.79
  control without guest DOM getter/method hooks; cover foreign trees and actual V8
  garbage collection. Geometry, remaining parsing/text mutation seams and lossless
  unpaired UTF-16 storage remain open.
- Implement native-tree Range clone/extract/delete/surround content operations,
  real selected text, boundary ordering/offset/root validation, node intersections
  and atomic insertion preflight. Preserve extracted node identity and nested
  partial shells, genuine receiver checks and private final exception constructors.
  Saved Chrome155 content/error controls and hostile DOM-method regressions cover
  this subset; layout geometry and remaining mutation-seam coverage remain open.
- Build DOM collection snapshots, private event paths and returned composed-path
  copies through intrinsic own elements, bypassing guest numeric setters,
  constructors/species and iterators while retaining live/native node identities.
- Propagate native Node events through the actual owner Document and Window with capture,
  target and bubble phases, stable composed paths, stop/removal/once/passive contracts and
  immediate AbortSignal subscriptions. Nonbubbling resource errors reach Window capture;
  element load events exclude Window. Preserve inert-document and shadow-root boundaries,
  foreign event/signal brands, listener objects and native handler registration order.
  Window onerror receives genuine ErrorEvent's five source/error arguments and return-true
  cancellation; ordinary on* handlers receive one Event and cancel on return false.
  Capture final trusted ErrorEvent/AbortSignal interfaces and the default AbortError
  DOMException constructor after host environment installation. Expose dispatch records through
  a Rust-only native_event_record seam with genuine/foreign/forged Event brand checks.
  Generic EventTarget/subclass and AbortSignal listeners also use private native records:
  no public registry fields, native receiver/foreign brands, real onabort handlers and
  once/passive/signal/removal/stop semantics. Match measured registration order at direct
  Window/generic/Signal targets while preserving Node/Document capture and bubble phases.
  Create private listener/dispatch records without prototypes, append/copy/remove private
  arrays through own indices without guest setters/species/iterators, and allocate native
  facade data/construct arguments with intrinsic array creation. Expose Rust-only platform
  brands and an idempotent opaque host target initializer in the actual creation realm.
  Hostile index/species/iterator/state-setter controls retain callbacks with zero hooks.
  Native Node abort-hook data and Window ErrorEvent argument arrays also bypass inherited
  numeric setters during subscription and onerror dispatch.
  Complete shadow retargeting/composed host crossing and listener exception reporting remain
  explicit followups.
- Implement the HTML-standard legacy Document clear/captureEvents/releaseEvents no-effect
  methods with native Document receivers, descriptors and ignored-argument behavior.
- Implement Document XPath evaluation/compiled expressions and XPathEvaluator over bounded
  public-rtdom snapshots using the unmodified MIT xpath 0.0.35 provider in a private V8
  context. Preserve real Node/Attr identity, namespace and foreign-case matching, corrected
  following/preceding axes, scalar/snapshot/iterator contracts, mutation invalidation,
  opaque native brands and owner-realm results for borrowed foreign interfaces. Enforce
  explicit node/depth/string/expression/operation budgets; reject namespace-axis results
  and fragment/shadow contexts. Keep the provider's automatic page installation disabled.
- Capture popover task schedulers by the current native context document after host timer
  installation, preserving independent frame queues and the parent's private scheduler.
- Preserve createElement's actual Document receiver through control/iframe enrichment, with
  single tag coercion, native ownership for inert documents, and IDL/name validation. Reflect
  script/link integrity properties into loader attributes with native receiver checks. Add
  legacy createNSResolver as validated Node identity backed by native namespace resolution.

## [0.4.6] - 2026-10-13

- Implement stable ObservableArray-backed stylesheet adoption for genuine Documents
  and ShadowRoots. Private backing preserves ownership, conversion/error effects,
  indexed/deletion/descriptor behavior and native-sheet constructor-document checks.
  Renderer exports read captured private backing, bypassing page-owned adoption
  getters; independent Chrome 155 mutation/ownership controls cover this subset.

### Fixed
- Back popover show/hide/toggle with native per-document state, auto/hint/manual ancestry
  and source relationships, cancellable trusted beforetoggle events and coalesced trusted
  toggle tasks on a privately captured host scheduler. Keep live :popover-open selector
  composition and shadow-root separation without changing DOM attributes or styles. Handle
  ordinary removal/moves, state-preserving moveBefore, null-namespace attribute changes,
  inert documents and queued event identity across adoption. Preserve assigned on* source
  markers during cross-document adoption. Autofocus/restoration, light-dismiss/backdrop,
  fullscreen/modal integration, stateful CSS cascade and top-layer painting remain pending.
- Validate native DOM insertion before changing parent links: reject ordinary and shadow-host
  ancestor cycles, enforce final Document element/doctype/text constraints, preserve nodes on
  invalid insertions, and check native Node brands, references and removal membership. Route
  variadic, adjacent and replacement operations through the shared validator; preserve valid
  fragment transfer, repeated-node ordering, string insertion and self-reference behavior.
  Native hierarchy errors retain genuine DOMException identity through private constructors.
- Use published turbo-dom 0.5.2 as the canonical DOM dependency. Add native HTML innerText/outerText
  mutation and supported rendered-text flow, nullable popover reflection and branded ToggleEvent
  records; complete CSS layout and top-layer painting remain unsupported.
- Keep native `classList` and `part` token-list identity and live indexed state,
  validate mutations atomically, and reflect ARIA element references with private
  assignment state, attribute invalidation and shadow-tree scope checks. Add native
  `moveBefore` hierarchy/root checks and preserve node/focus/control/handler identity;
  complete Range/iterator/slot/custom-element move reactions remain unsupported.
- Compile authored HTML event-handler attributes as real V8 functions with native element,
  form and document scope; keep assigned-function overrides and host CSP authorization private.
- Initiate inline stylesheet imports through private host state and include imported declarations
  in live computed styles; cache native policy filtering by the actual tree mutation version.
- Create native ProcessingInstruction nodes with CharacterData inheritance, XML-name/data
  validation, serialization, identity-preserving adoption and clone/import support. CharacterData
  methods now use UTF-16 offsets and reject invalid ranges; HTML CDATA creation throws the
  measured NotSupportedError. Preserve actual supported HTML XML-metadata state.
- Apply host CSS authorization without changing authored DOM attributes; preserve parser policy
  position and separate CSP-blocked attribute text from applied declarations and CSSOM writes.
  Keep stylesheet loader capabilities private and guard opaque stylesheet rules/mutations.
- Share CSS interface records across same-origin V8 realms, retaining borrowed getter and method
  behavior while rejecting unbranded receivers.
- Preserve element and attribute namespace URIs, prefixes and qualified names across native
  creation, mutation, cloning and import; keep native Attr identity, live values and single-owner
  attachment semantics, with branded live NamedNodeMap access and namespace validation.
- Back CSSStyleSheet, StyleSheet and MediaList interfaces with private records and native branded
  callbacks; retain live lists, owner/import metadata and media mutations, detach removed rules,
  reject invalid single-rule insertions, and filter imports from constructed replacements.
- Share CSSStyleDeclaration native prototype callbacks across private inline, rule and
  read-only computed records; synchronize inline mutations and external attribute edits,
  retain stable identity and validate supported properties with the stylesheet parser.
- Reflect editing and accessibility attributes with native branded accessors, including enum
  normalization, hidden union conversion, spellcheck/translation inheritance, associated-form
  auto-capitalization/correction and draggable image/link defaults.
- Isolate DOM trees, native node ownership, style state and listeners by V8 context.
- Separate input/textarea live values from defaults; retain reflected hidden values, default select
  selection and intentional empty selections.
- Validate CSS selectors and supported property values with Lightning CSS, recover malformed rules,
  and retain strings, custom properties, grouping rules, and deferred `var()`/`env()` values.
- Represent fetched linked stylesheets in document order, preserve their URL and import rules,
  and detach stylesheet ownership when their link leaves the document.
- Expose CSSRuleList and StyleSheetList as branded indexed collections rather than arrays;
  rule-list identity survives replacement and the document stylesheet list tracks tree changes.
- Separate native Node/Element/HTMLElement/Document and character-data interface prototypes;
  shared native accessors remain live and reject invalid receivers. Document-specific instance
  properties and compatibility APIs still have partial Web IDL coverage.
- Normalize Event constructor required arguments, string types and optional initialization,
  including CustomEvent's default detail.
- Back CSS list methods/accessors and DOM interface constructors with native V8 callbacks;
  preserve constructor inheritance, read-only prototype properties and non-enumerable globals.
  Rebuilding a document refreshes interface constructors without losing native tree bindings.
- Preserve class construction, subclass `new.target`, live instance state and exceptions through
  the native callable bridge used by host runtimes.
- Create genuine native document fragments, flatten their children on insertion, and retain
  fragment clone/query identity. Native shadow roots preserve host/mode and scoped tree queries.
- Put Node constants, DOM event methods and form-control methods on their owning interfaces;
  retain native DOM listeners and subclass EventTarget state without exposing cascade helpers.
- Keep event state private, expose read-only event fields and distinguish host-generated trusted
  dispatch from script dispatch across DOM, custom EventTarget and window listeners.
- Preserve CSS list receiver identity across document replacement and same-origin frame access;
  new style elements use the current document's CSSOM closures.
- Project supported document collections and creation methods onto Document; normalize text nodes,
  replace children and compare actual native tree positions through Node methods.
- Store element event-handler properties privately and invoke them during native event dispatch;
  preserve measured legacy Window load/pageshow state and real DOMException errors.
- Read HTML/SVG/MathML namespace identity from the native tree, retain foreign-element case and
  namespaces during cloning, and resolve native namespace prefixes through ancestor elements.
- Reflect scalar ARIA and basic HTML attributes through native node storage; expose attribute
  enumeration and adjacent text insertion on Element, and move link/script/control properties
  onto their HTML interfaces without copying them onto unrelated elements.
- Put style/import rule fields behind native, branded prototype accessors with private state;
  selector/style setters update serialization, and import ownership uses an internal loader seam.

- Add live native indexed HTMLCollection/NodeList views over document collections and native
  children, with static querySelectorAll results and private cross-realm receiver records.
- Preserve native node/Attr/shadow identities, ownership, listeners and control/nonce state
  during adoption; clone/import supported private live values through captured document helpers.
- Read DocumentType metadata and compatibility mode from the real HTML parser; implement
  supported Document scalar reflection, handler ordering, nonce state and tabIndex conversion.

## [0.4.5] — 2026-10-10

### Fixed
- Parse style-element and constructed stylesheet rule lists, retain CSSStyleRule identity and selectors,
  refresh after text mutations, and track only connected style elements in document.styleSheets.
- Replacing style text creates a new stylesheet and detaches the previous sheet while retaining its rules.
- Parser-created form controls now receive their interface prototypes and populated IDL values.
- Frame construction accepts a host-provided window factory for engines with independent V8 contexts.

## [0.4.4] — hide internal window state

### Changed
- Move internal iframe/event state to the shared Symbol namespace.

## [0.4.3] — bump turbo-dom to 0.5.1 (faster cascade)

### Performance
- Upgraded the native DOM engine `turbo-dom` `0.4.0` → `0.5.1`. The headline win is a cascade
  fix: `getComputedStyle` no longer re-parses every `<style>` sheet on each DOM mutation. The
  parsed rule index used to be keyed on the DOM version (bumped by *any* mutation), so a React
  re-render — which restructures nodes but leaves the injected CSS untouched — forced a full
  re-parse of all stylesheets on the next `getComputedStyle`. `turbo-dom` 0.5.1 keys the index on
  the `<style>` sources, reusing it across version bumps whose CSS is unchanged. Measured ~3–5%
  faster wall on a 457-file / 7324-test React component suite, identical pass counts.
- Also picks up 0.5.0's rtdom internal-consistency fixes (stale parent pointers, unset event
  target) — neither reachable through `query_selector*`/serialize, so no behavior change here.

## [0.4.2] — browser_env: iframe realm, SwiftShader WebGL, content-dependent canvas

Fidelity additions to the rtdom↔V8 `browser_env` DOM binding (consumed by turbo-surf's
browserless render tier for its reCAPTCHA/anti-bot work):
- **Bridged second-realm `<iframe>`** — `contentWindow`/`contentDocument` is a real
  second window realm with two-way cross-realm `postMessage` (the channel a reCAPTCHA
  anchor↔bframe handshake needs).
- **SwiftShader WebGL context** — `getContext('webgl'|'webgl2')` returns a coherent
  headless-Chrome ANGLE/Vulkan-SwiftShader signature (getParameter incl. UNMASKED_*,
  ~40 limits, extension list, getShaderPrecisionFormat, deterministic readback);
  previously `null`.
- **Content-dependent canvas2d readback** — `toDataURL`/`toBlob`/`getImageData` now
  hash the recorded draw ops (deterministic, non-empty) instead of a constant stub.
## [0.4.1] — memoize per-file config/project walks

### Performance
- **Per-file project/config walks are now memoized (thread-local, per start directory).** Five
  pure-of-the-filesystem functions in the module-load hot path — `cjs_first_project`,
  `vitest_setup_files`, `project_root`, `nearest_pkg_type`, and `is_esm_module` — each walk up the
  directory tree re-reading `package.json` and up to ~11 config filenames per ancestor. Their result
  depends only on the *directory* chain (never the filename — except `is_esm_module`, which also
  keys on the file extension), yet they were recomputed for every test file and, for the
  per-module ones, for every module a file imports. Each now caches into a `thread_local` `HashMap`
  keyed by the start dir, exactly like the existing E12 `nearest_tsconfig` / `resolve_spec_as`
  memos and gated behind the same `TURBO_NO_E12` flag. Keying by the start directory (rather than
  the file path) lets sibling test files and co-located modules share one answer, turning repeat
  walks into hashmap hits. The syscalls saved are the config `is_file()`/`read_to_string` and
  `node_modules/.bin/esbuild` `exists()` probes that dominate warm-cache runs once transforms are
  already cached.
- **`cache_dir()` no longer calls `create_dir_all` on every invocation.** It is hit ~9× per module
  load; the directory only needs creating once. The creation is now guarded by a process-wide
  `OnceLock<Mutex<HashSet<PathBuf>>>` so the syscall fires exactly once per distinct cache dir
  (the set keys on the resolved path so a `TURBO_CACHE_DIR` override is still honored).

  Behavior is unchanged — identical resolution results, identical test outcomes. Measured on real
  suites (warm cache, ABBA-interleaved medians to cancel thermal/load drift; `cargo build --release`
  native binary, 12 jobs, Apple Silicon):
  - **payroll-app** (1583 files / 18418 tests): runner wall ~15.48s → ~15.19s (**~1.9% faster**).
  - **ui-design-components** (453 files / 7268 tests): ~12.48s → ~12.35s (**~1%**).
  - **website-global** (101 / 1476) and **standalone-chat-app** (68 / 761): within noise (walks
    are a tiny fraction of these short runs).
  - Cold-cache runs are transform-bound (esbuild dominates) and land within noise — never a
    regression. All four suites reported identical pass counts before and after.

## [0.4.0] — window self-reference + native plugin/alias support

### Fixed
- **A top-level window did not self-reference (issue #18).** `window.parent`, `window.top`, and
  `window.self` were all `undefined`, so `window.parent !== window` — the canonical "am I inside an
  iframe?" check — read `true` for a non-framed window. Real browsers, jsdom, and vitest all return
  `=== window` for a top-level document, so any component gated on top-level/iframe detection (e.g. a
  CMS live-preview hook that only mounts inside the preview iframe) behaved as if *always framed*
  under turbo-test, forcing tests to stub the framing globals by hand. The window object
  (`window === globalThis`) now defines `self`/`parent`/`top`/`frames` pointing at itself and
  `frameElement = null`. They're `configurable`, so a test can still stub framing explicitly
  (`Object.defineProperty(window, 'parent', { value: iframeWin })`) to exercise the framed path.

### Added
- **`resolve.alias` / `test.alias` from the vitest config are resolved natively.** Both the object
  form (`{ '@': path.resolve(__dirname, './src') }`) and the array form
  (`[{ find, replacement }]`) are parsed by the launcher and fed to the module resolver, so a prefix
  alias like `@` → `src` turns `@/foo` into `src/foo` while leaving `@scope/pkg` node_modules imports
  alone. The alias target is the last string literal in the value expression (covers `path.resolve`,
  `fileURLToPath(new URL(…))`, and bare literals); regex `find` keys are skipped. This is the
  non-tsconfig counterpart to the tsconfig `paths` aliases turbo-test already resolves.
- **`vite-plugin-svgr` (`?react`).** `import Icon from './x.svg?react'` now resolves to a
  render-safe React component — a dependency-free function component that returns a real `<svg>`
  React element with the props forwarded (className / data-testid / aria-* pass through), plus the
  legacy `ReactComponent` named export. Plain `import url from './x.svg'` still yields the file
  contents. Other Vite query suffixes (`?url`, `?raw`, …) no longer hard-error — they resolve the
  base file.
- **Documented native support** for the most common vitest plugins (`vite-tsconfig-paths`,
  `@vitejs/plugin-react`(`-swc`), `@vitest/coverage-v8`, `@testing-library/jest-dom`,
  `vitest-canvas-mock`) in the README — most suites need no plugin setup.
- **More vitest CLI flags accepted** instead of warned: `--run`, `--watch`/`-w`, `--pool`, `--mode`,
  `--project`, `--exclude`, `--maxConcurrency`, `--sequence.*`, `--logHeapUsage`,
  `--hideSkippedTests`, `--disableConsoleIntercept`, `--inspect*`, `--browser*` (accepted-and-ignored
  on a native single-run runner), and `--no-file-parallelism` (→ runs files serially, `jobs = 1`).
  Value-taking forms no longer leak their argument as a bogus test-file path.

### Changed
- **Reporter output gained a vitest-style summary footer** (`Test Files` / `Tests` / `Duration`) for
  the human reporters (default / dot / verbose), matching vitest's recognizable end-of-run block. The
  existing per-file `PASS`/`FAIL` lines and the turbo diagnostic line are unchanged; machine
  reporters (json / junit / tap) keep clean stdout.

## [0.3.15] — functional Web Streams + `expectTypeOf` on the vitest surface

### Fixed
- **`ReadableStream`/`WritableStream`/`TransformStream` were present but non-functional (issue #15).**
  The globals existed — so libraries that do `class X extends TransformStream` at module load kept
  loading — but the reader's `read()` unconditionally resolved `{ done: true }`, so any code that
  actually *consumed* a stream (fetch response-body streaming, SSE parsing) silently saw an empty
  stream. Replaced the stub with a working implementation: a chunk queue with `pull` backpressure,
  reader/writer locking, `cancel`/`abort` propagation to the underlying source/sink, `tee()`,
  async iteration (`for await`), and `pipeTo`/`pipeThrough`. The guard still defers to a host that
  already provides native streams. Not spec-exhaustive (no BYOB readers, no queuing-strategy
  `size`), but matches Node's observable behavior for the common consumer patterns.

### Added
- **`expectTypeOf` and `assertType` on the `vitest` compatibility surface (issue #16).** They were
  missing from both the runtime module shim (`import { expectTypeOf } from 'vitest'` threw
  `does not provide an export named 'expectTypeOf'`, and the namespace-member form failed at
  runtime as `not a function`) and the bundled type shim (so type-level assertions didn't
  type-check either). Type assertions are erased at compile time and have no runtime behavior in
  vitest, so the runtime is a chainable no-op (a proxy that answers every member with itself, with
  `then` excluded so an accidental `await` can't hang). The checking lives in the `.d.ts`:
  `ExpectTypeOf<Actual, Positive>` implements `toEqualTypeOf`/`toMatchTypeOf`/`toHaveProperty`/
  `toBe*`/`.not`/navigation (`items`/`returns`/`resolves`/…), where a mismatch surfaces as a
  compile error. Exported from `types/vitest.d.ts` and declared as globals in `types/globals.d.ts`,
  alongside `describe`/`it`/`expect`/`vi`.

### Tests
- `fixtures/compat/streams.test.ts` (15 cases) — the issue-#15 minimal repro plus lazy `pull`,
  async `start`, async iteration, `error`/`cancel` propagation, `tee`, subclassed
  `TransformStream`, and `pipeTo`/`pipeThrough`; asserted via the JSON reporter in
  `test/compat-api.test.mjs`.
- `fixtures/compat/expecttypeof.test.ts` (5 cases) — `expectTypeOf`/`assertType` resolve and run
  through named, namespace, and global access.
- `types/typetests/expecttypeof.test-d.ts` — compile-only guard (run under `tsc --strict` by
  `test/compat-types.test.mjs`) covering positive assertions and `@ts-expect-error` negative cases
  so the shim fails in both directions.

## [0.3.14] — setupFiles matchers no longer lost for a whole worker under reuse/coverage

### Fixed
- **`setupFiles`' `expect.extend` matchers (jest-dom's `toBeInTheDocument`, `toHaveTextContent`,
  `toBeDisabled`, …) intermittently missing for an entire worker's chunk of files under the
  isolate-reuse path — and under `--coverage` even with `--isolate`.** Symptom in the consuming app
  (payroll-app CI shards): a random subset of `.tsx` tests failing `expect(...).toBeInTheDocument is
  not a function`, bursty per-worker (a broken worker cascades its whole file chunk), unreproducible
  in isolation. `--isolate` masked the reuse trigger but coverage stayed flaky, so there was no CI
  workaround.
- **Root cause:** `esbuild_bundle_full` (used to build the setup-file bundle) wrote the output
  straight to the shared content-addressed cache path via esbuild's `--outfile`, which streams the
  file **non-atomically**. The cache-hit check is a bare `out.exists()`, so when many workers raced
  to create the *same* setup bundle at startup, a worker could read a **half-written or empty**
  bundle. An empty setup bundle imports nothing → `expect.extend` never runs → that worker's single
  reused `__customMatchers` registry stays empty for **every** file it owns (the whole-worker
  cascade). `run_setup_file` swallowed the resulting load, so the failure was silent. Every *other*
  esbuild cache path already wrote through `write_atomic` (temp + rename) for exactly this reason;
  this one path did not. It now builds to a unique temp file (applying the mock-rewrite pass there)
  and **atomically renames** into the cache — a racing worker sees either no file or the complete
  one, never a partial. Fixes both the reuse and coverage triggers (both go through this bundle).
- The `fresh-isolate retry` was auto-healing most of the reuse failures (re-running setup per file
  in a clean isolate), which is why the reuse trigger was flaky rather than constant and why it hid
  under `--isolate` — but it re-ran whole files at a cost, and could not help the coverage path.

### Added
- `TURBO_CACHE_DIR` env var to override the shared transform/bundle cache location — lets a test run
  against an isolated, freshly-cleared cache without disturbing other concurrent turbo-test processes
  (used by the new regression test).

### Tests
- `fixtures/jestdom-reuse/` — a vendored `jest-dom-ish` package whose top-level side-effect is
  `expect.extend({...})`, a setupFile that imports it, `isolate: false`, and 30 files that each call
  the custom matcher. `test/compat-reuse-matchers.test.mjs` runs it under reuse against a clean
  isolated cache (forcing the concurrent first-build) and asserts all matcher assertions run + pass.

## [0.3.13] — EventTarget is now a real subclassable base (React hydration)

### Fixed
- **`class Foo extends EventTarget` now works.** The ctor shim gave `EventTarget` an
  empty prototype, so `this.addEventListener(…)` inside any subclass (event buses,
  state stores, modal managers — Nike's global-nav modal manager is one) threw
  `addEventListener is not a function`. An uncaught throw mid-hydration makes React
  discard the SSR tree and client-render the whole root (#425), blanking
  server-rendered content. `EventTarget.prototype` now carries a working
  `addEventListener`/`removeEventListener`/`dispatchEvent` listener registry, so
  subclass instances behave as event targets and hydration completes over the SSR DOM.

## [0.3.12] — DOM binding: hasChildNodes/namedItem + form-control methods (React hydration)

### Added
- **`Node.hasChildNodes()`** on the native element binding. It was absent, so the
  NON_MASKING interceptor returned `undefined` and React's hydration/commit path threw
  `hasChildNodes is not a function` — aborting the client-render fallback.
- **`namedItem` on the array-like NodeList/HTMLCollection/NamedNodeMap** returned by
  `children`/`childNodes`/`querySelectorAll`/`getElementsBy*`/`attributes` (bound next
  to the existing `.item`). Libs calling `namedItem` no longer hit `not a function`.
- **Form-control methods** `setCustomValidity`, `setSelectionRange`, `setRangeText`,
  `select`, `scrollTo`, `scrollBy` (no-ops), plus `checkValidity`/`reportValidity`
  (always valid). Missing methods threw mid-hydration, making React discard the SSR
  tree and client-render the whole root (#425), losing server-rendered content.

## [0.3.7] — node-env jest: decorator-metadata via tsc + interface design:type partial

### Fixed
- **`emitDecoratorMetadata` files now route through the project's TypeScript on first load** — on
  both the CJS and ESM load paths — emitting `Object` for non-value types (ts-jest parity for local
  type aliases / nullable types) instead of oxc's identifier-based metadata. Fixes a decorated field
  typed as a **local** type alias and the nullable-metadata cases.
- **The routing now actually fires on ts-jest backends.** It required `project_root`, which only
  resolves a dir containing `node_modules/.bin/esbuild`; projects that ship `typescript` but NOT
  esbuild (the common ts-jest setup) hit `None` and silently fell back to oxc. New `tsc_root` walks
  up for `node_modules/typescript`, so tsc runs. Net on a real NestJS+Sequelize suite (flux-apis):
  **+79 passing tests** (decorated files whose metadata tsc lowers correctly now load).

### Known limit
- A decorated field typed as an **imported interface** (`@Column() declare x: SomeImportedIface`)
  still fails ESM instantiation (`does not provide an export named SomeImportedIface`). Neither oxc
  nor `ts.transpileModule` has a cross-file type checker, so both keep the type-only import and
  reference it in `design:type`; the interface has no runtime export. Fix is app-side
  `import type { … }` (elided by any transform) or a full TS program (too heavy for a runner).
  ~73 such cases remain on flux-apis.

## [0.3.6] — node-env jest: gated decorators + `.`/`..` re-exports + native-addon crash guard

Makes a real NestJS + Sequelize/Mongoose backend (flux-apis) load and run under turbo-test, from
two background spikes. On a 120-spec subset, clean-loading files went **30 → 53** and
decorator-syntax / TDZ load-errors **83 → 0**; and a native `.node` addon `require()` no longer
takes down the whole run.

### Fixed
- **TypeScript legacy decorators + `emitDecoratorMetadata` are lowered in the native transform —
  gated, on both load paths.** The oxc TS-strip pass used `TransformOptions::default()`, which
  doesn't lower *legacy* decorators, so a NestJS/Sequelize/Mongoose decorated class emitted either
  `export @Decorator class X {}` (→ `Unexpected token 'export'`) or oxc's 2022-standard lowering
  that re-reads the class binding mid-init (→ `Cannot access 'X' before initialization` TDZ) — and
  because both fail at *run* time, the esbuild retry-on-load was never taken → hard load-error. New
  `transform::transform_with` / `esm_cjs::emit_with` lower legacy decorators + metadata (babel
  helpers are global via `runtime.js`), called from both `read_transformed` (ESM) and
  `native_transform_cjs` (CJS), **gated** on `experimentalDecorators`/`emitDecoratorMetadata` being
  enabled AND the file actually using a decorator AND not being in `node_modules`, with the flag
  folded into the transform cache keys — so non-decorated files pay nothing (no blanket
  metadata-pass cost). The pre-existing CJS circular machinery (installing `module.exports` before
  the body runs) was already correct; decorators were the actual cause of the "circular init" TDZ.
- **`require(".")` / `require("..")` directory re-exports resolve.** NestJS packages
  (`@nestjs/sequelize`, `config`, `mapped-types`, …) chain `__export(require("./dist"))` and
  `dist/utils/x.js → require("..")` self-references. `bundler::is_relative` excluded bare `.`/`..`,
  so they were left as runtime externals resolved against the bundle's package-root dir →
  "cannot resolve ..". `is_relative` now accepts `.`/`..`; and `resolve_spec_as` falls through to
  index probing when oxc resolves such a spec to a *directory* instead of its index file.
- **A native `.node` N-API addon no longer SIGSEGVs the whole run.** turbo-test's host
  (`src/napi_host.rs`) exported ~37 napi symbols; a real napi-rs addon (e.g. `turbo-html2pdf`) calls
  ~14 more, and under macOS flat-namespace `-export_dynamic` those undefined symbols resolve to
  `0x0`, so the addon's first such call jumps to NULL (exit 139, zero output, every other test in
  the run killed). Implemented the safe missing symbols (buffers, array/element/property-name
  access, bool/double/int64 getters); routed genuinely-unsupported entrypoints
  (`napi_wrap`/`napi_unwrap`/`napi_new_instance`) to a **catchable JS throw**; and wrapped addon
  module-init + every native callback FFI call in `catch_unwind` — so a broken/unsupported addon
  fails its own file cleanly and the rest of the run continues.
- **`__dirname` / `__filename` on the ESM graph load path.** CJS modules get them as wrapper
  params; a real ES module (the `load_graph` path) had neither in scope, so an app `.ts` that reads
  `const DIR = __dirname` at top level threw `ReferenceError: __dirname is not defined` and the file
  failed to load. `inject_dirname_esm` now prepends Node-CJS-equivalent module-local `const`
  bindings, gated on actual reference + no local declaration, newline-free (line numbers/source maps
  preserved). (`browser_env` projects are unaffected — only files reaching `__dirname` change.)

### Tests
- `fixtures/compat/circular/` (plain ESM require cycle) and `fixtures/compat/circular-decorators/`
  (decorated, mutually-referencing models + local tsconfig) — both fail on the pre-fix binary, pass
  with the fix — wired into `test/compat-api.test.mjs`.
- `fixtures/napi/` (a synthetic misbehaving C addon, built at test time, `.node` gitignored) +
  `test/compat-napi.test.mjs`: asserts the run does not segfault (`signal !== 'SIGSEGV'`,
  `code !== 139`) and a sibling spec still executes after the bad-addon file.

### Known follow-ups (not here)
- A separate, pre-existing native-bundler bug: it can't resolve `tslib` inside
  `@aws-crypto/crc32c` (cascading through `@aws-sdk/client-s3`) — the bulk of the remaining
  flux-apis load-errors, unrelated to decorators/circular init.

## [0.3.5] — `.d.ts` shim round 4: adopt vitest's canonical types

Stops the `it.each` whack-a-mole by lifting vitest's real type definitions verbatim (read from
`@vitest/runner` / `@vitest/spy`) instead of hand-approximating, and audits the rest of the shim
against the upstream source.

### Fixed
- **`it.each` typing replaced with vitest's canonical overload set** (`ExtractEachCallbackArgs` +
  `EachFunctionReturn` + the four `TestEachFunction` overloads, verbatim from `@vitest/runner`).
  Tuple rows up to 10 columns (const or not) map to precise positional args; ragged / heterogeneous
  / >10-column rows land on the permissive `T[]` / `fallback` overload with array args — exactly the
  runner's behavior. Fixes the last residual (payroll-app `it.each` `1 → 0`) and supersedes the
  hand-rolled `Writable<T>` overloads from 0.3.4.
- **Test bodies and hooks now receive a context arg.** vitest's `TestFunction` is
  `(context) => …` and `beforeEach`/`afterEach`/`beforeAll`/`afterAll` callbacks receive
  `(context, suite)`; the shim typed them as zero-arg, so `it('x', ({ expect }) => …)` /
  `beforeEach((ctx) => …)` failed to assign (a callback with more params than the target is not
  assignable). Added a permissive `TestContext` (typed `task`/`signal`/`expect`/`skip`/`annotate`/
  `onTestFailed`/`onTestFinished` + an index signature) and widened `TestFunction` / `HookFunction`
  to pass it. No-arg bodies still assign.
- **`ExpectStatic` gained an index signature** so forward-compat statics (`expect.poll`,
  `expect.unreachable`, `expect.assertType`, `expect.addSnapshotSerializer`, …) resolve instead of
  erroring — matching the shim's permissive-subset philosophy.

### Tests
- `types/typetests/each.test-d.ts` extended: ragged non-const `it.each` rows (fallback path),
  context-receiving test bodies + destructured context, hooks with/without a context arg, and the
  forward-compat `expect` statics. Still gated by `test/compat-types.test.mjs` (`tsc --strict`).

## [0.3.4] — `.d.ts` shim round 3 (`it.each` `as const` rows) + constructable stylesheets

### Added
- **Constructable stylesheets** in the DOM bootstrap: `new CSSStyleSheet()` (with
  `cssRules`/`rules`, `insertRule` returning the index, `deleteRule`, `replace` → `Promise<this>`,
  `replaceSync`) plus a real, settable `document.adoptedStyleSheets` array. emotion/MUI do
  `new CSSStyleSheet()` then `document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet]`;
  there's no layout engine, so the rule list is an inert store that satisfies the API — but a
  missing `CSSStyleSheet` reference threw mid-chunk during hydration and blanked the tree.
  (`browser_env.js`.)

### Fixed
- **`it.each([...] as const)` regression.** The 0.3.3 tuple overload constrained rows to mutable
  `any[]`, so `[…] as const` (readonly tuples) fell through to the single-value overload and the
  multi-arg callback's params went implicit-`any` (payroll-app: `it.each` errors `2 → 30`). The
  spread overload now (1) accepts `readonly` rows via `ReadonlyArray<readonly [...T]>` to force
  per-position tuple inference (non-const heterogeneous rows + 1-tuples included), and (2) adds a
  second overload matching the union-of-readonly-tuples that `as const` produces, mapping it to
  mutable positional params with a `Writable<T>` helper so the spread assigns. `it.each` /
  `test.each` / `describe.each` and their `.only`/`.skip` variants all share `TestEachFunction`,
  so the fix covers every form.

### Tests
- `types/typetests/each.test-d.ts` — a compile-only guard exercising the it.each tuple/`as const`/
  union/scalar rows plus the rounds 1–2 surface (`vi.mocked`/`hoisted`/`fn<T>`, mock
  `importOriginal<T>`, `expect.fail`). Run by `test/compat-types.test.mjs` via `tsc --noEmit
  --strict` (new `typescript` devDependency; skips if absent). A shim regression now fails `npm test`.

## [0.3.3] — `.d.ts` shim round 2 (mock `importOriginal` generic + tuple-preserving `it.each`)

### Fixed
Round-2 type widening: after 0.3.2, payroll-app's `tsc` errors were `525 → 18` (+1 unrelated PNG).
The remaining 18 trace to two more type-only shim gaps in `types/turbo-test-api.d.ts`:
- **`importOriginal` is now generic** in the `vi.mock` / `vi.doMock` async-factory overloads — the
  factory's first arg is typed `<T extends Record<string, any>>() => Promise<T>` (vitest's
  signature), so `const actual = await importOriginal<typeof Mod>()` resolves and the spread /
  derived-`Parameters` usage downstream stops cascading off `{}`. (~16 errors: TS2347 ×11, TS2698,
  TS2339 ×4)
- **`it.each([...])` preserves per-position tuple types.** The 0.3.2 array overload widened each row
  to an element-union, so `it.each([['x', true]])(…, (label, isLast) => …)` saw
  `string | boolean`. The tuple overload now takes `ReadonlyArray<readonly [...T]>`, forcing each
  row to infer as a tuple → `(label: string, isLast: boolean)`. Ordered before the single-value
  overload; numeric-tuple spread and the explicit-union single-arg form (gap 5) still resolve. (2)

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
