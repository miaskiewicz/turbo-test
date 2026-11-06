#!/usr/bin/env bash
# run-app.sh — run an app's vitest suite with the locally built turbo-test binary.
#
# Usage: scripts/agent/run-app.sh APP_DIR [--chrome-media] [--css-transitions] [FILE ...]
#
# Sets TURBO_CHAI from APP_DIR/node_modules/chai/index.js (required: the binary loads
# chai into its V8 snapshot from this path).
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || $# -eq 0 ]]; then
  sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

APP_DIR="$1"; shift
[[ -d "$APP_DIR" ]] || { echo "error: $APP_DIR is not a directory" >&2; exit 1; }
APP_DIR="$(cd "$APP_DIR" && pwd)"

CHAI="$APP_DIR/node_modules/chai/index.js"
[[ -f "$CHAI" ]] || { echo "error: no chai at $CHAI — run 'npm ci' in the app first" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/bin/turbo-test-$(node -p 'process.platform')-$(node -p 'process.arch')"
[[ -x "$BIN" ]] || { echo "error: no binary at $BIN — run scripts/agent/build-release.sh" >&2; exit 1; }

FLAGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --chrome-media) FLAGS+=("--chrome-media"); shift ;;
    --css-transitions) FLAGS+=("--css-transitions"); shift ;;
    *) break ;;
  esac
done

cd "$APP_DIR"
export TURBO_CHAI="$CHAI"
exec "$BIN" "${FLAGS[@]}" "$@"
