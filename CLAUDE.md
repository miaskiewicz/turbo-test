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
`--coverage` `--coverage-dir DIR` `--coverage-thresholds lines=,functions=,branches=`
`--coverage-per-file` `--coverage-reporter lcov,json-summary,text,html`
`--coverage-include GLOB` `--coverage-exclude GLOB`.
Thresholds/include/exclude are auto-read from the vitest config `coverage` block by `cli.js`
when not passed explicitly (flags win). No `statements` metric — V8 has none (intentional).
