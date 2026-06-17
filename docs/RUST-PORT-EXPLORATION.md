# Rust port — exploration (branch `rust-port`)

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

### What CANNOT (and should not) become Rust

`src/runtime.js` (1875 LOC) is the in-isolate test harness — `describe`/`it`/`expect`, the
event-loop + timers, console/process shims, `vi.*`. **V8 executes JS, not Rust.** It is already
baked into the binary via `include_str!` (`runner.rs:3364`) and ships inside it — it is not a Node
dependency. "Entirely in Rust" means *the toolchain and distribution* are pure Rust; the harness
glue that runs *inside the isolate* stays JS (the only alternative is Wasm, not worth it). Be
honest about this in any "100% Rust" claim.

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

## Coupling 2 — `esbuild` subprocess → native bundler (HIGH lift)

This is the real work. Two transform strategies coexist today:

1. **Module-runner path** — per-module: oxc_resolver resolves each specifier, oxc transforms
   TS/JSX, V8 loads it as a module. Already pure Rust.
2. **esbuild bundle path** (`esbuild_bundle_full`) — spawns esbuild `--bundle --format=esm
   --platform=browser` to flatten a test file + **all its node_modules deps** into one file,
   with mock externalization rewrites, CSS/asset loaders, and tsconfig `paths`. Cached by content
   hash under `cache_dir()`.

esbuild exists because the module-runner historically didn't cover the messy node_modules world
(CJS/ESM interop, `__toESM` default-interop, deep dep trees, singleton preservation for
react/@mui/@emotion — see the long `base_resolve_options` comment). Removing esbuild means one of:

- **A. Extend the module-runner to cover node_modules too.** oxc_resolver already resolves bare
  specifiers (it's used for the affected graph). The gaps to close: CJS↔ESM interop
  (`__toESM`/`__commonJS` shims oxc_transformer can emit), `paths` aliases (already wired),
  asset/CSS loaders (map to empty/text/dataurl in the loader, easy), and the singleton concern
  (one resolved build per package — the resolver conditions already enforce this). This reuses the
  most code and removes the subprocess + the npm `esbuild` dep entirely. **Preferred.**
- **B. Embed a Rust bundler** — `rolldown` (oxc-family, Rust, designed as the esbuild/rollup
  replacement) as a library, or `swc_bundler`. Adds a heavy dep but is closest to drop-in for the
  bundle semantics we already rely on.
- **C. Keep esbuild but vendor a static binary** — sidesteps "all Rust" (still a non-Rust
  subprocess) but kills the npm/node_modules dependency. Cheap fallback if A/B slip.

**Decision needed** (A vs B vs C) — see open questions. A is the cleanest "all Rust" answer and
leans on machinery we already have; B is lower-risk for bundle parity; C is a stopgap.

**Oracle:** the `payroll 10006/0` + `ui 6189/0` real-world suites in memory. Any bundler swap must
hold those green and the ~5.6–5.8× benchmark. Bundle cache (`esb-*.mjs`) keying logic carries over.

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
2. **Native bundler spike** (Coupling 2, decide A/B/C). Biggest lift, biggest payoff — kills the
   last runtime subprocess and the npm dep tree. Gate on the payroll/ui suites + benchmark.
3. **turbo-dom Rust crate** (Coupling 3). Lands when the turbo-dom port is ready; retire
   `napi_host.rs`.
4. **Distribution** — `cargo-dist` static binaries per platform; npm package becomes an optional
   thin wrapper (or is dropped). Update `scripts/npm-build.sh`, CI, README.

After 1–4: no `node` on `PATH`, no `node_modules`, one Rust binary. `runtime.js` (+ optional DOM
install JS) remain baked in as in-isolate harness source — by design.

## Open questions (for the user)

- **Bundler strategy** — A (extend module-runner), B (embed rolldown/swc_bundler), or C (vendor
  static esbuild as a stopgap)?
- **Keep an npm package?** Distribute via npm thin-wrapper (familiar `npx turbo-test`) vs pure
  cargo/`cargo-dist` release only?
- **turbo-dom crate API** — what's the planned Rust surface, and is the in-isolate `installGlobals`
  JS staying or being replaced by direct V8 binding?
