#!/usr/bin/env bash
# build-release.sh — cargo build --release, then place the binary where cli.js looks for it.
#
# Uses a per-session CARGO_TARGET_DIR under the scratchpad (or $TMPDIR) so parallel
# sibling sessions don't thrash the shared target/ dir, and reuses the cached V8 archive
# via RUSTY_V8_ARCHIVE so builds don't re-download turbo-dom-v8.
#
# Usage: scripts/agent/build-release.sh [--help]
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

V8_ARCHIVE="${RUSTY_V8_ARCHIVE:-/Users/grzegorz/.cache/turbo-consolidation/v8cache/librusty_v8_simdutf_release_aarch64-apple-darwin.a.gz}"
if [[ -f "$V8_ARCHIVE" ]]; then
  export RUSTY_V8_ARCHIVE="$V8_ARCHIVE"
else
  echo "warn: RUSTY_V8_ARCHIVE not found at $V8_ARCHIVE — a full V8 download will happen" >&2
fi

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  BASE="${TMPDIR:-/tmp}"
  export CARGO_TARGET_DIR="$BASE/turbo-test-agent-target-$$"
  mkdir -p "$CARGO_TARGET_DIR"
fi
echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"

cargo build --release --bin turbo-test

PLATFORM="$(node -p 'process.platform')"
ARCH="$(node -p 'process.arch')"
DST="bin/turbo-test-${PLATFORM}-${ARCH}"
mkdir -p bin
cp "$CARGO_TARGET_DIR/release/turbo-test" "$DST"
chmod +x "$DST"
echo "placed $DST"
