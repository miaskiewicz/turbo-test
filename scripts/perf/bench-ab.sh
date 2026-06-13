#!/bin/bash
# DEPRECATED shim — superseded by harness.sh. Kept for back-compat.
# Old: bench-ab.sh <proj> <pairs> <subN> ENVA ENVB [LA LB] -> harness.sh ab ...
DIR="$(cd "$(dirname "$0")" && pwd)"
PROJ="$1"; PAIRS="$2"; SUB="$3"; ENVA="$4"; ENVB="$5"; LA="${6:-A}"; LB="${7:-B}"
exec "$DIR/harness.sh" ab "$PROJ" --sub "$SUB" --jobs 1 --pairs "$PAIRS" "$ENVA" "$ENVB" "$LA" "$LB"
