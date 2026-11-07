#!/usr/bin/env bash
# benchmark-app.sh — benchmark an app's suite with ABBA runs and a load check.
#
# Usage: scripts/agent/benchmark-app.sh APP_DIR [FILE] [--runs N] [--out FILE]
#
# Order: A B B A (balances warmup drift). Before each run, waits until the 1-minute
# load average drops below 15. Records wall and user-CPU seconds; prints a results
# table and writes it to --out (default: ./bench-<timestamp>.tsv).
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || $# -eq 0 ]]; then
  sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

APP_DIR="$1"; shift
APP_DIR="$(cd "$APP_DIR" && pwd)"
FILE=""
RUNS=4
OUT="bench-$(date +%Y%m%d-%H%M%S).tsv"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    *) FILE="$1"; shift ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/bin/turbo-test-$(node -p 'process.platform')-$(node -p 'process.arch')"
[[ -x "$BIN" ]] || { echo "error: no binary at $BIN" >&2; exit 1; }
CHAI="$APP_DIR/node_modules/chai/index.js"
[[ -f "$CHAI" ]] || { echo "error: no chai at $CHAI" >&2; exit 1; }

wait_for_load() {
  while :; do
    local load
    load=$(uptime | sed -E 's/.*load averages?: *([0-9.]+).*/\1/')
    if awk -v l="$load" 'BEGIN{exit !(l+0 < 15)}'; then return; fi
    echo "  load $load >= 15, waiting 10s" >&2
    sleep 10
  done
}

# Per-run isolated cache so one run doesn't warm the next.
run_once() {
  local tag="$1"
  wait_for_load
  local cache
  cache="$(mktemp -d "${TMPDIR:-/tmp}/turbo-bench-XXXXXX")"
  local tmp; tmp="$(mktemp)"
  env TURBO_CHAI="$CHAI" TURBO_CACHE_DIR="$cache" \
    /usr/bin/time -lp "$BIN" ${FILE:+"$FILE"} >/dev/null 2>"$tmp" || true
  local wall user
  wall=$(awk '/^real /{print $2}' "$tmp")
  user=$(awk '/^user /{print $2}' "$tmp")
  rm -rf "$cache" "$tmp"
  printf '%s\t%s\t%s\n' "$tag" "$wall" "$user"
}

printf 'tag\twall_s\tuser_s\n' | tee "$OUT"
# ABBA pattern; with --runs N, interleave two halves.
labels=(A B B A)
for ((i=0; i<RUNS; i++)); do
  tag="${labels[$((i % 4))]}"
  run_once "$tag" | tee -a "$OUT"
done
echo "results in $OUT"
