#!/usr/bin/env bash
# check-versions.sh — verify Cargo.toml and package.json carry the same version string.
#
# These are the files CI's release-guard reads on a tag: a release where they disagree
# is a bug (CLAUDE.md). Also grep for suspicious drift in CHANGELOG.md's newest section
# heading. Exits non-zero on any mismatch.
#
# Usage: scripts/agent/check-versions.sh [--help]
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

cargo_v=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
npm_v=$(node -p "require('./package.json').version")

echo "Cargo.toml  version: $cargo_v"
echo "package.json version: $npm_v"

fail=0
if [[ "$cargo_v" != "$npm_v" ]]; then
  echo "error: Cargo.toml ($cargo_v) != package.json ($npm_v)" >&2
  fail=1
fi

if [[ -f CHANGELOG.md ]]; then
  changelog_v=$(grep -E '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | head -n1 | sed -E 's/^## \[([^]]+)\].*/\1/')
  echo "CHANGELOG.md newest:  $changelog_v"
  if [[ "$changelog_v" != "$cargo_v" ]]; then
    echo "warn: newest CHANGELOG.md entry ($changelog_v) does not match Cargo.toml ($cargo_v)" >&2
    echo "  (ok if the current version has no release notes yet)" >&2
  fi
fi

exit "$fail"
