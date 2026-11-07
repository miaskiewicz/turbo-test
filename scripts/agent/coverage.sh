#!/usr/bin/env bash
# coverage.sh — run tarpaulin against the 99.9% floor from tarpaulin.toml.
#
# Usage: scripts/agent/coverage.sh [-- <extra cargo-tarpaulin args>]
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  sed -n '2,4p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

V8_ARCHIVE="${RUSTY_V8_ARCHIVE:-/Users/grzegorz/.cache/turbo-consolidation/v8cache/librusty_v8_simdutf_release_aarch64-apple-darwin.a.gz}"
[[ -f "$V8_ARCHIVE" ]] && export RUSTY_V8_ARCHIVE="$V8_ARCHIVE"

if ! command -v cargo-tarpaulin >/dev/null; then
  echo "error: cargo-tarpaulin not installed (cargo install cargo-tarpaulin)" >&2
  exit 1
fi

args=()
if [[ "${1:-}" == "--" ]]; then shift; args=("$@"); fi

exec cargo tarpaulin "${args[@]}"
