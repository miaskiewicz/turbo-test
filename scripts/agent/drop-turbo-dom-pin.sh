#!/usr/bin/env bash
# drop-turbo-dom-pin.sh — remove the [patch.crates-io] turbo-dom pin and re-lock from crates.io.
#
# Usage: scripts/agent/drop-turbo-dom-pin.sh
#
# Fails if, after unpinning, Cargo.lock still has a git source for any turbo-dom crate.
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

# Drop everything from [patch.crates-io] onward if that section is at the end of the file.
if grep -n '^\[patch.crates-io\]' Cargo.toml >/dev/null; then
  line=$(grep -n '^\[patch.crates-io\]' Cargo.toml | head -n1 | cut -d: -f1)
  # Expect it was appended at the end (pin-turbo-dom.sh appends). Trim everything from
  # that line to EOF, plus any trailing blank line before it.
  awk -v stop="$line" 'NR < stop { print }' Cargo.toml > Cargo.toml.new
  mv Cargo.toml.new Cargo.toml
fi

cargo update -p turbo-dom -p turbo-dom-layout -p turbo-dom-browser-env

if grep -n '^source = "git+' Cargo.lock | grep -E 'turbo-dom'; then
  echo "error: Cargo.lock still has a git source for a turbo-dom crate" >&2
  exit 1
fi
echo "unpinned; Cargo.lock is clean of turbo-dom git sources"
