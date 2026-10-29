#!/usr/bin/env bash
# Duplicate-V8 guard: the build graph must link exactly one V8, turbo-dom's
# rusty_v8 fork `turbo-dom-v8` (shared by turbo-test and turbo-dom-browser-env).
# No crate may pull in upstream `v8`, `v8x` or the retired `turbo-surf-v8`.
#
# Usage: scripts/check-single-v8.sh   (from anywhere; uses this repo's Cargo.toml)
#
# Uses `cargo tree -i` over normal + build edges for every target, i.e. the
# resolved graph; `cargo metadata`'s raw resolve also lists inactive optional
# packages and must not be used for this check.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
for name in v8 v8x turbo-surf-v8; do
  if out=$(cargo tree --locked -e normal,build --target all -i "$name" 2>&1); then
    if [ -n "$out" ]; then
      echo "error: forbidden V8 package '$name' is in the dependency graph:" >&2
      echo "$out" >&2
      fail=1
    else
      echo "ok: no $name"
    fi
  elif grep -q 'did not match any packages' <<<"$out"; then
    echo "ok: no $name"
  else
    echo "error: cargo tree -i $name failed:" >&2
    echo "$out" >&2
    fail=1
  fi
done

if ! out=$(cargo tree --locked -e normal,build --target all -i turbo-dom-v8 --depth 0 2>&1); then
  # Two turbo-dom-v8 versions make the bare spec ambiguous ("multiple ... packages").
  echo "error: expected exactly one turbo-dom-v8; cargo tree -i turbo-dom-v8 failed:" >&2
  echo "$out" >&2
  exit 1
fi
count=$(grep -c '^turbo-dom-v8 v' <<<"$out" || true)
if [ "$count" -ne 1 ]; then
  echo "error: expected exactly one turbo-dom-v8, found $count:" >&2
  echo "$out" >&2
  fail=1
else
  echo "ok: one turbo-dom-v8 ($(grep '^turbo-dom-v8 v' <<<"$out"))"
fi
exit "$fail"
