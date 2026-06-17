# Rust port — exploration (branch `rust-port`)

## STATUS (live)

| Phase | State | Notes |
|---|---|---|
| P1 — cli.js → Rust | ✅ DONE | `src/launcher.rs`; cli.js is a thin shim; compat suites green |
| P2a — app-file ESM→CJS emitter | ✅ DONE, default ON | `src/esm_cjs.rs`; payroll 1057 files/10471 tests full parity |
| Conformity harness | ✅ DONE | `scripts/conformity.mjs` (parity + coverage modes) |
| P2b — node_modules native bundler | ✅ DONE, default ON | `src/bundler.rs`; payroll full parity with native app **+ deps** |
| P2c — delete esbuild | 🟡 PARTIAL | coverage maps built but density-gap; metadata + fallback still esbuild |
| P3 — turbo-dom Rust crate | ⬜ TODO (other agent) | retire `napi_host.rs` |

**Normal test runs are now 100% native** (app + node_modules), validated at full parity on the
payroll oracle (1057 files / 10471 tests). esbuild is still invoked only for: **coverage runs**,
**decorator-metadata** files, and as the **automatic fallback** for any unhandled form.

**P2b (done):** `src/bundler.rs` bundles a package's relative graph, wrapping each module in a lazy
`__commonJS((exports,module)=>{…})` init closure (circular-safe — `module.exports` is assigned
early with live getters), reusing the per-file emit verbatim inside; bare imports stay external
(shared via require cache); assets stubbed. The naive per-file attempt's failure (barrel `export *`
under circular deps) is fixed by the bundle + a key correctness fix: `__reExport` must pass
`module.exports` as its 3rd arg (else `__toCommonJS`'s snapshot misses names re-exported afterward —
this broke `@testing-library/react`'s `render`).

**P2c (partial):** `emit` is now single-pass (TS-strip + ESM→CJS on one AST) and, under coverage,
appends an inline oxc source map (correct — maps to the right source lines). But oxc's codegen map
is **less dense** than esbuild's per-token map, so `coverage.rs` under-attributes inner
functions/lines → not parity. Coverage therefore stays on esbuild (gated) until the map density is
closed or `coverage.rs` is adapted. **Fully deleting esbuild needs:** (1) coverage-map density
parity, (2) native decorator-metadata (oxc can already lower metadata — wire it to native ESM→CJS),
(3) confidence to drop the fallback. The mock-hoist/shared-let passes need NO change — native emits
esbuild-shape `var import_ = require(…)` lines, so the existing string passes work (payroll mock
parity proves it).

Conformity worktrees live at `/Users/grzegorzmiaskiewicz/github-flux/.tt-conformity/{payroll-app,flux-apis}`
(detached on `origin/staging`, node_modules symlinked from the main checkouts). flux-apis only runs
under turbo-test in pure-logic dirs (its NestJS app-graph doesn't load under the esbuild baseline
either), so payroll-app is the primary oracle.

---


Goal: turbo-test runs as a **single self-contained Rust binary** — no Node.js process, no npm
runtime dependencies. Pairs with the upcoming **all-Rust turbo-dom build** (the DOM env links as
a Rust crate instead of a prebuilt `.node` addon).

## TL;DR — we are ~80% there already

The native `turbo-test` binary already embeds V8, transforms with **oxc** (`transform.rs`),
resolves modules with **oxc_resolver** (`runner.rs`), computes coverage natively
(`coverage.rs` / `coverage_branch.rs`), and even hosts native addons through its **own NAPI host**
(`napi_host.rs`) so it can `dlopen` turbo-dom's parser *without a Node runtime*. The binary does
not link or require Node at execution time today.

Three things still keep Node/npm in the loop:

| # | Node/npm coupling | Where | Lift to remove |
|---|---|---|---|
| 1 | **`cli.js` launcher** — flag parse, vitest-config scan, file discovery, `--changed` git filter | `cli.js` (383 LOC, runs under `node`) | **Low** — pure logic, port to `turbo_test.rs` |
| 2 | **`esbuild` subprocess** — bundles node_modules deps | `runner.rs::esbuild_bundle_full` spawns `node_modules/.bin/esbuild` | **High** — needs a native bundler |
| 3 | **`turbo-dom` `.node` addon** — DOM environment | `napi_host.rs` + `runner.rs` DOM bootstrap | **Medium** — gated on the all-Rust turbo-dom build (separate repo) |

`index.js` (50 LOC programmatic API) is a Node-consumer convenience, orthogonal to the binary.

### `src/runtime.js` — stays JS by CHOICE, not necessity

`src/runtime.js` (1875 LOC) is the in-isolate test harness — `describe`/`it`/`expect`, the
event-loop + timers, console/process shims, `vi.*`. It is already baked into the binary via
`include_str!` (`runner.rs:3364`) and ships inside it — **it is not a Node dependency.**

Could it become Rust? The `v8 = "149.3.0"` crate IS **rusty_v8** (Deno's bindings; crate renamed
`rusty_v8` → `v8`) — already our embedder. But rusty-v8 is the Rust↔V8 *FFI*; it lets Rust drive
V8 and bind native functions, it does **not** make V8 run Rust (V8 executes JS/Wasm). So one
*could* reimplement the harness as native `v8::FunctionTemplate` callbacks on `globalThis` (the
binary already does this for `log`, microtask draining). Not worth it:

- ergonomics collapse — matcher chaining (`expect(x).toBe(y)`), async test bodies interleaving with
  user JS, `vi.fn()` proxies are all far cleaner as JS than as cross-FFI native callbacks;
- **zero distribution payoff** — it's `include_str!`'d into the binary, removing nothing from the
  "all Rust" target.

So "entirely in Rust" means *the toolchain and distribution* are pure Rust; the in-isolate harness
glue stays JS by choice. Don't claim "no JS at all" — claim "no Node runtime, no npm deps."

---

## Coupling 1 — `cli.js` → native front-end (LOW lift)

`cli.js` is the npm `bin`. Everything it does is already expressible in Rust, and most has a Rust
analogue in-tree:

- **binary resolution / musl detect** — moot once we ship one Rust binary (no self-exec needed).
- **default test discovery** (`walk` + `TEST_RE`, `SKIP_DIR`) — trivial `std::fs` walk.
- **vitest include/exclude + coverage + environment config scan** (`globToRe`, `patternsFromText`,
  `vitestCoverage`, `configEnvironment`) — these are *string-scans* of the config file, not TS
  evaluation. Direct port; `globToRe` ↔ a small glob→regex with the `regex` crate or reuse the
  existing glob logic the coverage include/exclude path already has in Rust.
- **`--changed [since]` git filter** (`gitChanged`) — shell out to `git` from Rust (same as now),
  or use `gix`. Note the existing caveat: direct changed-FILE filter, no import graph — but
  `graph.rs` (M5 affected) already builds that graph, so the Rust port could *upgrade* `--changed`
  to transitive-affected for free.
- **flag splitting + forwarding** — currently cli.js parses launcher-only flags and forwards the
  rest to the binary as argv. In a single binary this collapses into one arg parser
  (`clap`, or hand-rolled to match the current bespoke parsing exactly).

**Plan:** move all of the above into `src/bin/turbo_test.rs` (or a `cli` module). Delete `cli.js`.
npm package (if still shipped) points `bin` straight at the platform binary via a thin shim — or
drop npm entirely and distribute via `cargo-dist` / GitHub releases.

**Risk:** the config readers are deliberately loose regex scans; replicate their exact behavior
(and the `vitest.compat.md` quirks) or some projects' discovery silently changes. Port with the
`test/cli-compat.test.mjs` + `compat-config-env.test.mjs` suites as the oracle.

## Coupling 2 — `esbuild` subprocess → **DECIDED: Option A** (oxc-native ESM→CJS emitter)

Decision (with user): **Option A — extend the in-tree oxc pipeline, no new bundler crate.** Not
rolldown/swc_bundler (option B), not vendoring a static esbuild (option C). Reuses what's linked,
removes the subprocess AND the npm `esbuild` dep, keeps resolution consistent with the module-runner
and affected graph (all oxc_resolver). The cost is writing the ESM→CJS emit ourselves.

### Why it's not a flag-flip — the core blocker

`transform.rs:67`: **`// oxc 0.134's CommonJS module transform is a no-op`.** esbuild is doing the
ESM→CJS conversion that oxc can't yet. Even in module-runner mode (`mr_enabled()`, default ON),
`read_transformed` still routes the CJS load path through esbuild:

- **`esbuild_transform_cjs`** (`runner.rs:1387`) — per **app file**: TS/JSX → CJS.
  oxc already does TS/JSX strip + decorator/metadata lowering (`transform.rs::transform`,
  `transform_decorators_with_metadata`). Missing piece = module-syntax → CJS emit.
- **`esbuild_bundle_dep_cjs`** (`runner.rs:1986`) — per **node_modules package entry**:
  `--bundle --format=cjs --packages=external`. Flattens a package's OWN relative files into one
  CJS module, **externalizes every bare import** so react/@mui/@emotion stay single-instance via
  the require cache. = a mini intra-package bundler (oxc_resolver walks the relative graph; deps
  are externalized, so it's bounded — not a general bundler).

### What Option A must build

1. **An oxc-based ESM→CJS emitter** matching esbuild's output **contract**, because that shape is
   load-bearing downstream:
   - `postprocess_mr_cjs` (`runner.rs:1952`) patches export getters → configurable, and overrides
     `__toESM` to an identity (so `import styled from '@emotion/styled'` yields the function, not
     the namespace);
   - `hoist_mock_setup` (`runner.rs:1497`) **string-matches `var import_… = require(…)` lines** to
     reorder requires below `vi.mock` setup;
   - `shared_mock_lets` / `rewrite_shared_lets` route mock-closed-over `let`s through a global;
   - react-family externalization keys off the same `var import_` / `require` shape.
   So we either emit byte-compatible `var import_X = require("…")` + `__toESM`/`__commonJS` +
   export-getter output, OR rewrite these consumers against an AST instead of strings (cleaner,
   bigger diff). **Leaning AST-based** for the hoist/share passes — string-matching esbuild's exact
   formatting is brittle.
2. **The per-package bundle mode** — walk a package's relative import closure with oxc_resolver,
   concatenate + CJS-wrap, externalize all bare specifiers. Reuse the `mrdep-*.cjs` content-cache.
3. **Coverage source maps** — esbuild emits `--sourcemap=inline` so `coverage.rs` remaps V8 byte
   ranges → original lines. oxc Codegen can emit a source map; wire it into the inline-map path
   (`coverage.rs:499` decodes it). Must stay byte-faithful enough for the existing decoder.
4. **Decorator-metadata path** — already oxc-capable (`transform_decorators_with_metadata`), but it
   currently hands ESM to a *second* esbuild `--format=cjs` pass (`esbuild_format_cjs`). Same
   emitter reused → drops that pass too. (tsc-parity path `tsc_transform_esm` is separate, opt-in
   on decorator-load-failure; can stay.)

### Sequencing within Option A

a. Emitter for **app files** first (`esbuild_transform_cjs` replacement) — smaller, no graph walk.
   Validate pass/fail + benchmark on one suite before touching node_modules.
b. Then **per-package bundle** (`esbuild_bundle_dep_cjs`) — the harder half (graph + externalize +
   singleton correctness).
c. Convert `hoist_mock_setup`/`shared_mock_lets` to AST passes over our own output.
d. Wire oxc source maps for coverage. Delete esbuild call sites + the npm dep + the
   `esbuild_bundle_full` ESM path (setup/DOM-boot — also still esbuild).

### Risks

- **CJS/ESM interop is the whole reason esbuild is here** — `__toESM`/`__commonJS` default-interop,
  `.default` semantics, live-binding mockability. Getting these subtly wrong = green→red on real
  suites in non-obvious ways (the `base_resolve_options` + `postprocess_mr_cjs` comments are a map
  of the landmines already hit).
- **oxc upstream** may ship a real CJS transform — worth checking current oxc before hand-rolling
  the emit; if it lands, step 1 shrinks to configuring `TransformOptions` + matching the shape.
- The legacy non-MR ESM path (`esbuild_bundle_full`, setup files / DOM boot) is *also* esbuild —
  Option A must cover it too for a clean kill, or keep MR-only and document that `TURBO_NO_MR` is
  gone.

**Oracle:** `payroll 10006/0` + `ui 6189/0` real-world suites (memory) + the ~5.6–5.8× benchmark.
Every step gated on holding those green. Content-cache keying (`mr-*.cjs`, `mrdep-*.cjs`,
`esb-*.mjs`) carries over with a new version tag to bust stale esbuild-shaped entries.

## Coupling 3 — `turbo-dom` `.node` → Rust crate (MEDIUM, gated on turbo-dom)

Today: `napi_host.rs` implements enough of the NAPI C ABI for the binary to `dlopen` turbo-dom's
prebuilt parser `.node` from a worker thread; `runner.rs` runs `install.mjs` to put
`window`/`document` on `globalThis`. The DOM *globals* are installed by turbo-dom's JS
(`installGlobals`), the *parser* is the native addon.

When turbo-dom ships an **all-Rust build**:

- Link turbo-dom as a **Cargo dependency** instead of dlopen'ing a `.node`. The parser becomes
  direct Rust calls — `napi_host.rs` can be retired (or kept only for *other* third-party addons).
- The `installGlobals` JS layer can stay (it's in-isolate DOM-shim JS, same category as
  `runtime.js`) or be reimplemented to bind the Rust DOM directly to V8 globals. Lower priority —
  the JS install path works; the win is dropping the `.node` + NAPI shim.
- Coordinate the turbo-dom crate API in that repo. **External dependency on the turbo-dom port.**

This coupling can stay as-is while 1 and 2 land; it's independently shippable.

---

## Suggested sequencing

1. **Port `cli.js` into the Rust binary** (Coupling 1). Self-contained, immediately removes the
   Node *launch* requirement, validated by the existing compat suites. Ship first.
2. **oxc-native ESM→CJS emitter** (Coupling 2, Option A — decided). Biggest lift, biggest payoff —
   kills the last runtime subprocess and the npm dep tree. Sub-sequenced a→d above. Gate every step
   on the payroll/ui suites + benchmark.
3. **turbo-dom Rust crate** (Coupling 3). Lands when the turbo-dom port is ready; retire
   `napi_host.rs`.
4. **Distribution** — `cargo-dist` static binaries per platform; npm package becomes an optional
   thin wrapper (or is dropped). Update `scripts/npm-build.sh`, CI, README.

After 1–4: no `node` on `PATH`, no `node_modules`, one Rust binary. `runtime.js` (+ optional DOM
install JS) remain baked in as in-isolate harness source — by design.

## Decisions made

- **Bundler strategy → Option A** (oxc-native ESM→CJS emitter). Rejected B (rolldown — would be the
  pick if we embedded a bundler, since it's oxc-family vs swc_bundler's dead/duplicate-AST path) and
  C (vendor static esbuild — stopgap only).
- **runtime.js → stays JS** (baked in, not a Node dep; native-callback port via rusty-v8 possible
  but not worth it).
- **Distribution → npm thin-wrapper.** Keep `@miaskiewicz/turbo-test` on npm shipping prebuilt
  per-platform binaries (familiar `npx turbo-test` / devDependency install), `bin` pointing at the
  Rust binary. The Node *launch* requirement still goes away (cli.js logic moves into the binary —
  Coupling 1); npm is just the delivery channel, not a runtime dep. `cargo-dist` releases can be
  added alongside later for non-npm users.
- **oxc upstream CJS transform → NOT available (checked Jun 2026).** Issue
  [oxc#4050](https://github.com/oxc-project/oxc/issues/4050) (babel-plugin-transform-modules-commonjs)
  open since Jul 2024, no implementation progress / assignee / target version. Local 0.134 only
  wires `Module::CommonJS` to TS-only `import x = require()` (`typescript/module.rs`) — no general
  ESM→CJS pass exists. **Option A step 1 stays full-scope: we hand-roll the emit.** Bumping oxc
  won't change this. (Aside: oxc wants the transform itself "for Rolldown app mode" — so rolldown's
  own module execution is blocked on the same gap; reinforces that B wouldn't have dodged it.)

## Open questions (for the user)

- **turbo-dom crate API** — what's the planned Rust surface, and is the in-isolate `installGlobals`
  JS staying or being replaced by direct V8 binding?
