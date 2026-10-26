# turbo-test — agent notes

## Releasing / version bumps
**Always bump the version in BOTH files together — they must never drift:**
- `package.json` → `"version"`
- `Cargo.toml` → `version`

A release where these disagree is a bug. Bump both in the same commit.

**Every tagged release MUST add a `CHANGELOG.md` entry — this is mandatory, no exceptions.**
- Add a `## [x.y.z] — <short title>` section (newest first, above the previous release and below
  `## [Unreleased]`) in the SAME commit as the version bump.
- Use Keep-a-Changelog headings (`### Added` / `### Fixed` / `### Performance` / `### Tests`, etc.).
- Record the *why*, not just the *what* — a future reader should understand the motivation.
- A tag without a matching `CHANGELOG.md` entry is an incomplete release. Do not tag without it.

## Build / test loop
- Native binary: `cargo build --release` → produces `target/release/turbo-test`.
- `cli.js` prefers the prebuilt `bin/turbo-test-<platform>-<arch>` over `target/release`.
  After rebuilding, refresh it: `cp target/release/turbo-test bin/turbo-test-darwin-arm64`
  (or run `scripts/npm-build.sh`). Otherwise you'll test a stale binary.
- Coverage needs `node_modules/.bin/esbuild` in the project being tested (the module-runner
  CJS transform path emits the inline source map only via esbuild). A fixture without esbuild
  silently collects no coverage.

## Tests and the coverage gate
- `npm ci` first (npm 10 and npm 11 both supported; CI checks both against the committed
  lock — regenerate it with npm 11 `npm install --package-lock-only` if deps change).
- `cargo test --locked` runs unit tests (`src/*_tests.rs`, wired with `#[path]` from the
  module they test) and CLI integration tests (`tests/*.rs`, helper `tests/common/mod.rs`:
  throwaway projects outside the repo, isolated TMPDIR/TURBO_CACHE_DIR, spawning the real
  binary). Integration tests and the path/V8-sensitive unit modules are `cfg(unix)`.
- `cargo tarpaulin` (reads `tarpaulin.toml`: Llvm engine, `target/cov`, `fail-under = 99.98`)
  is the CI `coverage` gate. New code needs behaviour-asserting tests (happy path + edge
  cases) that keep every line covered; never add coverage-only no-op tests. The single
  remaining uncovered line (a V8-unreachable guard) is explained in tarpaulin.toml.
- CI detection (snapshot update mode `none` on CI) is std-env's `isCI`: `src/ci.rs` over the
  provider table `src/ci_providers.json`. When vitest's std-env changes, re-check the table with
  `node scripts/check-ci-detection.mjs <dir with vitest installed>` (0 mismatches). The test
  harnesses (`tests/common`, `test/compat-api.test.mjs`) drop every variable of that table.
- In-process V8 unit tests must boot isolates from `runner::framework_snapshot()`: the
  read-only heap is process-wide, so a default-snapshot isolate alive alongside
  framework-snapshot ones aborts the test binary.

## Complexity gates
- CI jobs `complexity / rust (cc <= 15)` (`npm run complexity:rust`: turbo-html2pdf's `cc-check`,
  installed from its home at the pinned rev in ci.yml, over every tracked `.rs` file) and
  `complexity / js (cc <= 15)` (`npm run complexity:js`: oxlint `complexity` over cli.js,
  index.js, src/runtime.js). One limit, no allow-list, no baseline.
- Both are expected red (`continue-on-error`) until the post-coverage refactor clears the
  backlog listed in README "Complexity gates". New code stays <= 15; never raise the limit.

## Where the DOM and V8 live
- The DOM binding (window/document/CSSOM/forms/events, its probes, Chrome controls and
  tests) is the `turbo-dom-browser-env` crate in miaskiewicz/turbo-dom
  (`crates/turbo-dom-browser-env`); the runner only calls
  `turbo_dom_browser_env::browser_env::{install, enabled, reset}`. Fix DOM behaviour there.
- V8 is turbo-dom's rusty_v8 fork `turbo-dom-v8` (aliased `v8`). No V8 source is vendored
  here. `bash scripts/check-single-v8.sh` must pass (one turbo-dom-v8, no v8/v8x/turbo-surf-v8).
- Pre-publish, `Cargo.toml`'s `[patch.crates-io]` pins those crates to git revs; drop it
  before release (RELEASING.md).

## Coverage CLI (v0.2.3+)
`--coverage` `--coverage-dir DIR` `--coverage-thresholds lines=,functions=,branches=,statements=`
`--coverage-per-file` `--coverage-reporter lcov,json-summary,text,html`
`--coverage-include GLOB` `--coverage-exclude GLOB`.
Thresholds/include/exclude are auto-read from the vitest config `coverage` block by `cli.js`
when not passed explicitly (flags win).
`statements` (v0.2.6+) is DERIVED: oxc parses each source once (shared with the branch pass —
no extra parse cost) and each executable statement's position is correlated with V8's covered
ranges, the same as branches. It appears in json-summary / text / html and is gateable; lcov has
no statement field so it's omitted there. Tracks lines closely (c8-style).
Include/exclude globs support `{a,b}` brace alternation. The multi-glob separator is a TOP-LEVEL
comma only — commas inside `{ts,tsx}` are NOT split (v0.2.7 fix; the canonical vitest include
`src/**/*.{ts,tsx}` is forwarded comma-joined and was being torn into malformed globs).
Under `--coverage`, 0 instrumented files is a hard FAIL (non-zero exit), never a vacuous 0/0
green — guards against a misconfigured include silently covering nothing.
Mapping V8 ranges back through esbuild's inline map (src/coverage.rs) must respect three things
(turbo-test#34; expected values = vitest's v8 provider, checked against node's V8):
the ESM->CJS interop prologue (export getters + `module.exports = __toCommonJS(...)`) maps to
(0, 0) and is NOT source code (no FN/DA/ranges on line 1 from it); top-level code is counted
only by the module-wide ranges, which open on the wrapper line and close past the map comment
(clamped to the source start/end); and segments often begin with blank columns (indentation,
the space before a token) — skip that lead-in before applying the 1:1 column delta, and credit
a line whose first segment is blank only with the count of the token it points back to.
