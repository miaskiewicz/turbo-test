#!/usr/bin/env bash
# pin-turbo-dom.sh — add a [patch.crates-io] pin for the turbo-dom crates at a git REV.
#
# Usage: scripts/agent/pin-turbo-dom.sh REV
#
# Patches turbo-dom, turbo-dom-layout and turbo-dom-browser-env to
# https://github.com/miaskiewicz/turbo-dom at REV, then re-locks. Use pre-publish while
# the registry hasn't caught up; drop before tagging (release-guard refuses a tag with a
# [patch] section or any git source in Cargo.lock). See drop-turbo-dom-pin.sh.
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || $# -lt 1 ]]; then
  sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

REV="$1"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if grep -q '^\[patch.crates-io\]' Cargo.toml; then
  echo "error: Cargo.toml already has a [patch.crates-io] section — drop it first" >&2
  exit 1
fi

cat >>Cargo.toml <<EOF

[patch.crates-io]
turbo-dom = { git = "https://github.com/miaskiewicz/turbo-dom", rev = "$REV" }
turbo-dom-layout = { git = "https://github.com/miaskiewicz/turbo-dom", rev = "$REV" }
turbo-dom-browser-env = { git = "https://github.com/miaskiewicz/turbo-dom", rev = "$REV" }
EOF

cargo update -p turbo-dom -p turbo-dom-layout -p turbo-dom-browser-env
echo "pinned turbo-dom crates to $REV"
