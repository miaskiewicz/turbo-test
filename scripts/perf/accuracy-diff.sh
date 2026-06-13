#!/bin/bash
# accuracy-diff.sh — prove two runner configs produce IDENTICAL results per file.
# Matching TOTAL pass/fail counts is necessary but not sufficient (a false-pass in
# one file can cancel a false-fail in another). This captures the per-file
# "PASS/FAIL <path> (N passed, M failed)" lines for two env configs and diffs them,
# so ANY divergence on ANY file shows up. Use for the isolate-reuse accuracy gate.
#
# Usage: accuracy-diff.sh <project-dir> "<ENV_A>" "<ENV_B>" [labelA labelB]
#   e.g. accuracy-diff.sh ../ui-design-components "" "TURBO_REUSE_ISOLATE=1" fresh reuse
set -u
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
CLI="$REPO/cli.js"
PROJ="$(cd "$1" && pwd)"; ENVA="$2"; ENVB="$3"; LA="${4:-A}"; LB="${5:-B}"
cd "$PROJ" || exit 1
TA="/tmp/acc_${LA}_$$.txt"; TB="/tmp/acc_${LB}_$$.txt"
# Keep only the per-file result lines, normalize to "path passed=N failed=M", sort by path.
norm() { grep -E '^(PASS|FAIL|ERROR) ' \
  | sed -E 's/^(PASS|FAIL|ERROR)  ([^ ]+)  \(([0-9]+) passed, ([0-9]+) failed\).*/\2 passed=\3 failed=\4/' \
  | sed -E 's/^ERROR ([^ ]+).*/\1 LOAD_ERROR/' | sort; }
echo "## accuracy-diff $PROJ : A=$LA [$ENVA]  vs  B=$LB [$ENVB]"
env $ENVA node "$CLI" 2>/dev/null | norm > "$TA"
env $ENVB node "$CLI" 2>/dev/null | norm > "$TB"
echo "A files: $(wc -l < "$TA")   B files: $(wc -l < "$TB")"
D="$(diff "$TA" "$TB")"
if [ -z "$D" ]; then
  echo "RESULT: IDENTICAL per-file pass/fail across all files. Accuracy preserved. ✅"
else
  echo "RESULT: DIVERGENCE — these files differ between $LA and $LB:"
  echo "$D" | head -60
  echo "(< = $LA, > = $LB)"
fi
rm -f "$TA" "$TB"
