# M6 — Compatibility surface + hardening (ship gate): progress

## Done + verified
- **Test modifiers** (`src/runtime.js`): `it.skip`, `it.todo`, `it.only` + `describe.only`
  (only-mode skips siblings), `it.each` + `describe.each`, `it.concurrent` (API accepted),
  `it.skipIf`/`it.runIf`, and **retry** (`it(name, fn, { retry: n })`). Verified:
  retry passes on the 3rd attempt; `.only` runs solely the marked test; skipIf skips.
- **Sharding** (`--shard i/n`): deterministic index partition. Verified 1/2 + 2/2 = 10 files.
- **JSON reporter** (`--reporter json`): emits `numTotalTests`/`numPassedTests`/
  `numFailedTests`/`success`/`testResults[]` (Vitest-shaped).
- **Parity preserved** through all of it: `src/utils` 145/145 vs stock Vitest, 0 failed.

## Remaining for the ship gate (honestly tracked, not skipped)
- **Coverage**: V8 coverage API (`Profiler.takePreciseCoverage`) → istanbul format. Not yet
  wired. (V8 exposes it; needs the inspector/coverage binding + istanbul JSON emit.)
- **Source-map-correct stack traces (100% KPI)**: oxc codegen can emit source maps
  (`CodegenReturn.map`); need to attach them and remap thrown-error line numbers to original
  TS. The transform already produces output; sourcemap plumbing is the remaining step.
- **Full config compatibility**: read `vitest.config.ts`/`vite.config.ts` for
  `include`/`exclude`/`environment`/`setupFiles`/`alias`/`pool`. Currently CLI takes explicit
  files; resolver handles aliases via tsconfig only partially.
- **TAP / default reporters**: only default + JSON now.
- **Real @vitest/* + turbo-dom snapshot bundle** (M3-content): swaps the minimal runtime for
  the real framework — required for the long matcher/snapshot/automock tail and `.snap`
  byte-compatibility, and for DOM suites (RTL, components needing `document`).

## Verdict
**M6 partial.** Modifiers, sharding, and a JSON reporter are done and verified; coverage,
source maps, full config, and the real-framework/turbo-dom bundle remain before the ship
gate (full gauntlet ≥99.9% + 100% source-map correctness + coverage parity) is met.

## Node-API (napi) host — turbo-test is now Node-API capable (proven)

`src/napi_host.rs` implements the napi C ABI against rusty_v8 so native `.node` addons load
in turbo-test's (non-Node) V8 embedding:
- dlopen + `napi_register_module_v1` invocation with a real `napi_env`
- ~37 `napi_*` functions: value/object/string/number, arrays, **functions + cb_info
  trampoline**, arraybuffer/typedarray, errors/exceptions, references; threadsafe-fn family
  satisfied (GC-finalizer tsfn created at init returns a dummy handle)
- bridge: `napi_value` == `v8::Local<Value>` (transmute); handles created in the call's scope
- `#[no_mangle]` + `-Wl,-export_dynamic` (build.rs) so the addon resolves its imports against us
- wired into `require('.node')` (`runner.rs` native_require)

**Proof:** loaded turbo-dom's real `turbo-dom-parser.darwin-arm64.node` and ran
`parseBuffer('<!doctype html>...')` → valid result, 2/2 tests pass.

### Remaining to get turbo-dom DOM into the suites (not the napi ABI — node-module emulation)
turbo-dom's JS loader (`src/runtime/index.mjs`) reaches the `.node` via `node:module`
`createRequire` + `fs` + `path`. So full DOM needs: (1) node builtin stubs (`node:module`
createRequire→our require, `fs.existsSync/readFileSync`, `path`), then (2) run turbo-dom's
`installGlobals(globalThis)` as the environment setup before each test file (defines
`document`/`window`). The hard part (napi C ABI) is done; this is bounded node-builtin glue.
