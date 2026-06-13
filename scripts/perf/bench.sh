#!/bin/bash
# DEPRECATED shim — superseded by harness.sh. Kept for back-compat.
# Old: bench.sh <proj> [runs] [--sub N] [-- args]  ->  harness.sh micro ...
DIR="$(cd "$(dirname "$0")" && pwd)"
PROJ="$1"; shift || true
RUNS=3; SUB=40; EXTRA=()
while [ $# -gt 0 ]; do case "$1" in
  --sub) SUB="$2"; shift 2;;
  --) shift; EXTRA=("$@"); break;;
  *) [[ "$1" =~ ^[0-9]+$ ]] && RUNS="$1"; shift;;
esac; done
exec "$DIR/harness.sh" micro "$PROJ" --sub "$SUB" --jobs 1 --runs "$RUNS" ${EXTRA[@]+-- "${EXTRA[@]}"}
