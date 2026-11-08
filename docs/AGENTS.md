# AGENTS.md — short guide for AI agents (and humans in a hurry)

turbo-test's common workflows wrapped in one-shot scripts under `scripts/agent/`. These
run on the user's machine (where V8, Chrome and the chai dependency live); an agent calls
them and reads the output.

## Build loop

The runtime is a Rust binary on V8. `cli.js` prefers a prebuilt `bin/turbo-test-<platform>-<arch>`
over `target/release`, so after any source change the bin dir needs refreshing.

```bash
scripts/agent/build-release.sh         # cargo build --release + copy into bin/
scripts/agent/run-app.sh ../my-app     # run the app's suite with the fresh binary
```

`build-release.sh` sets `RUSTY_V8_ARCHIVE` to the cached V8 (so it doesn't redownload) and
uses a per-session `CARGO_TARGET_DIR` under `$TMPDIR` to avoid colliding with sibling
sessions sharing `target/`.

## `TURBO_CHAI` — mandatory for standalone runs

The binary loads the `chai` npm package into its V8 snapshot at startup from the path in
`TURBO_CHAI` (chai's `index.js`). `cli.js` sets it automatically from `node_modules/chai`
of the project being tested; but when you run `target/release/turbo-test` (or the copied
`bin/turbo-test-…`) directly, you must set it yourself, pointing at the app's chai:

```bash
TURBO_CHAI="$PWD/node_modules/chai/index.js" bin/turbo-test-darwin-arm64
```

`run-app.sh` does this for you.

## jsdom-media and transitions — opt-in

Two headline 0.7.x changes default to the fast path:

- **Chrome-oracled media queries**. Media is evaluated against the Chrome-collected
  oracles; pass `--chrome-media` (or env `TURBO_CHROME_MEDIA=1`, or `test.chromeMedia: true`
  in the vitest config) to opt back into the slow jsdom-driven path for a suite that
  depends on it.
- **CSS transitions off**. Transitions don't fire by default (independent of media);
  `--css-transitions` (env `TURBO_CSS_TRANSITIONS=1`, `test.cssTransitions: true`) turns
  them on for suites asserting on `transitionend`.

`run-app.sh` passes both flags through.

## Pinning turbo-dom by git rev pre-publish

The DOM, layout and V8 fork live in `miaskiewicz/turbo-dom`. Between a fix landing there
and its crates.io release, patch in a git pin:

```bash
scripts/agent/pin-turbo-dom.sh a5e05113        # add [patch.crates-io] + re-lock
# …build and test…
scripts/agent/drop-turbo-dom-pin.sh            # strip the section, re-lock from crates.io
```

A tag must build from crates.io: CI's `release-guard` fails if `Cargo.toml` still has a
`[patch]` section or `Cargo.lock` still has a `git+` source. Always drop the pin in the
same commit that bumps the version.

## Cache directory trap

The saved window snapshot is now fingerprinted by engine scripts, so the same
`TURBO_CACHE_DIR` across builds is safe for normal runs. **For benchmarks, use a separate
cache per run** (benchmark-app.sh does this with `mktemp -d`): sharing warms the snapshot
across runs and hides first-run cost.

## Benchmark method

`benchmark-app.sh` uses an ABBA order (`A B B A`) to balance warmup drift between two
revisions, and before each run waits until the 1-minute load average drops below 15 so
other processes don't skew the timing. It records `/usr/bin/time -lp` wall and
user-CPU seconds; compare by user CPU seconds (wall absorbs scheduler jitter).

## Coverage floor

`cargo tarpaulin` (reads `tarpaulin.toml`) enforces `fail-under = 99.9` in CI. Locally:

```bash
scripts/agent/coverage.sh
```

New code needs behaviour-asserting tests — never add coverage-only no-op tests. The two
known uncovered lines (each explained in `tarpaulin.toml`) stay excluded via context, not
via `#[cfg]` skips.

## Pre-commit hook gotchas

- CI runs both npm 10 and npm 11 against the committed lock; if deps change, regenerate
  with npm 11: `npm install --package-lock-only`.
- `bash scripts/check-single-v8.sh` must pass — exactly one `turbo-dom-v8` in the graph,
  no stray `v8`/`v8x`/`turbo-surf-v8`.
- Version bumps touch BOTH `package.json` and `Cargo.toml` in the same commit; every
  tagged release gets a `CHANGELOG.md` entry. `scripts/agent/check-versions.sh` catches
  drift.
- Never use `--no-verify`. If a hook fails, fix the cause.

## Scripts

| Script | One-line what it does |
|---|---|
| `build-release.sh` | `cargo build --release` with cached V8, per-session target dir; copies into `bin/`. |
| `run-app.sh APP_DIR [FILE…]` | Runs an app's suite with the local binary and the app's chai. Passes `--chrome-media`, `--css-transitions`. |
| `benchmark-app.sh APP_DIR [FILE]` | ABBA runs with load check and isolated caches; wall + user CPU seconds to a TSV. |
| `pin-turbo-dom.sh REV` | Adds `[patch.crates-io]` for the turbo-dom crates at REV and re-locks. |
| `drop-turbo-dom-pin.sh` | Removes the pin and re-locks from crates.io; fails if git sources remain. |
| `coverage.sh` | `cargo tarpaulin` against the 99.9% floor in `tarpaulin.toml`. |
| `check-versions.sh` | Confirms `Cargo.toml` and `package.json` agree on the version. |
