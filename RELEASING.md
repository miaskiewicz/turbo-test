# Releasing turbo-test

The release publishes `@miaskiewicz/turbo-test` on npm and `turbo-test` on
crates.io. This repository has no PyPI release. The `vX.Y.Z` tag runs the four
supported binary builds, their tests, and both registry publications.

Before release, merge all reviewed stacked work into the final release PR. Bump
`Cargo.toml`, the runner entry in `Cargo.lock`, `package.json` and both root
versions in `package-lock.json` to the same next version (normally the next patch;
the turbo-dom consolidation release is 0.6.0, aligned with the turbo-dom 0.6.0
crates). Move the complete unreleased changelog into that release's dated section.
Preserve earlier versions and unrelated dependency versions.

## Native dependency provenance

The runner links V8 through turbo-dom's rusty_v8 fork, the registry crate
`turbo-dom-v8 ~150.4` (V8 15.0), under its local `v8` alias, and installs the
browser environment from `turbo-dom-browser-env` 0.8.4 (currently a pre-publication
candidate). Both are built and
published from [miaskiewicz/turbo-dom](https://github.com/miaskiewicz/turbo-dom):
the fork's patches, pins and generator live in its `crates/turbo-dom-v8`, and the
binding (formerly this repo's `src/browser_env*`) in `crates/turbo-dom-browser-env`.
This repository carries no vendored V8 source and publishes no V8 crate.

The fork carries the interceptor `ShouldThrowOnError` fix;
`tests/interceptor_should_throw.rs`, `tests/release_v8_teardown.rs` and
`tests/weak_teardown.rs` are consumer copies of turbo-dom-v8's own tests and run
against the resolved fork. `scripts/check-single-v8.sh` (CI build job) requires
exactly one `turbo-dom-v8` in the graph and no `v8`, `v8x` or `turbo-surf-v8`.

Until the turbo-dom 0.8.4 crates are on crates.io, this release branch resolves
`turbo-dom`, `turbo-dom-layout` and `turbo-dom-browser-env` from one exact git
revision through `[patch.crates-io]`. The V8 fork stays on its registry release
150.4.1. This pin is for development validation, not registry publication.
Publish turbo-dom 0.8.4 first, following its own procedure, then verify that all
three crates are available. Remove the pin and re-lock just those dependencies:

```sh
for c in turbo-dom turbo-dom-layout turbo-dom-browser-env; do
  curl -sf "https://index.crates.io/tu/rb/$c" | grep -q '"vers":"0.8.4"' || echo "MISSING $c"
done
scripts/agent/drop-turbo-dom-pin.sh
! grep -n '^\[patch' Cargo.toml
! grep -n 'git+https://github.com/miaskiewicz/turbo-dom' Cargo.lock
bash scripts/check-single-v8.sh
cargo test --locked
cargo package --locked -p turbo-test --allow-dirty
```

Drop the matching pre-publish notes in `CLAUDE.md` ("Where the DOM and V8 live")
and here in the same commit. The tag-only `release-guard` CI job fails a `v*` tag
whose `Cargo.toml` still has a `[patch]` section, whose `Cargo.lock` still has git
sources, or whose `Cargo.toml` / `package.json` version differs from the tag; both
publish jobs depend on it. The Rust job verifies the normalized runner package
against real registry dependencies (`cargo package --locked`) before publishing
`turbo-test`; it also fails while git patches remain. Cargo publication
verification remains enabled.
The jobs use the existing `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` secrets; never
record token values.

Use published compatible turbo-dom crates before the runner crate release.
A Git-pinned development version with the same number does not prove that its
required APIs are available to registry consumers.

## Gates and final merge tag

Run `npm ci` (npm 10 and npm 11 must both install the committed lock unchanged; CI job
`npm-lockfile`), the complete locked Rust tests, the Node compatibility/type tests, all
nine native DOM compatibility fixtures used by `.github/workflows/ci.yml`, and the
line-coverage gate (`cargo tarpaulin`, `fail-under = 99.9` in tarpaulin.toml; CI job
`coverage`), and the lints (`cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`; CI job `lint`). The complexity jobs (`complexity / rust (cc <= 7)`,
`complexity / js (cc <= 7)`) are blocking since 0.8.0: a release needs them green
too (README "Gates and CI"). The binding's
Chrome-parity tests (including the explicit unknown-pseudo CSSOM admission ignore
and its Chrome-expected failure) now run in turbo-dom-browser-env, not here. Check the actual normalized runner package from a
clean consumer and inspect the npm allowlist and staged platform binaries.

Merge all open reviewed PRs with merge commits. Require the integrated final PR's
four platform checks to pass. Tag the **final main merge commit**, once, only after
all intended changes have landed:

```sh
git tag -a vX.Y.Z FINAL_MERGE_SHA -m vX.Y.Z
git push origin vX.Y.Z
```

No separate Python tag exists here. Verify the tagged workflow and both actual
registries after publication; do not infer success from a green build job or
from an existing older package. Keep each failed attempt with its actual source
identity. A tag is an irreversible publication request.
