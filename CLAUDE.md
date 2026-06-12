# turbo-test — agent notes

## Releasing / version bumps
**Always bump the version in BOTH files together — they must never drift:**
- `package.json` → `"version"`
- `Cargo.toml` → `version`

A release where these disagree is a bug. Bump both in the same commit.

## Build / test loop
- Native binary: `cargo build --release` → produces `target/release/turbo-test`.
- `cli.js` prefers the prebuilt `bin/turbo-test-<platform>-<arch>` over `target/release`.
  After rebuilding, refresh it: `cp target/release/turbo-test bin/turbo-test-darwin-arm64`
  (or run `scripts/npm-build.sh`). Otherwise you'll test a stale binary.
- Coverage needs `node_modules/.bin/esbuild` in the project being tested (the module-runner
  CJS transform path emits the inline source map only via esbuild). A fixture without esbuild
  silently collects no coverage.

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
