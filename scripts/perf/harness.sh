#!/bin/bash
# ===========================================================================
# turbo-test ULTRA perf harness — one tool for all perf experiments.
# See scripts/perf/README.md for the full guide. Committed for future agents.
#
# Subcommands:
#   micro  <proj> [--sub N] [--jobs J] [--runs R] [-- cli-args...]
#       Warm-cache, then R timed runs over an even-stride sample of N test
#       files. Prints median wall + pass/fail. Flakiness/accuracy guarded.
#
#   ab     <proj> [--sub N] [--jobs J] [--pairs P] [--trim T] ENVA ENVB [LA LB]
#       Paired interleaved A/B (A,B,A,B,...). Per-pair delta cancels machine
#       drift. Reports TRIMMED-mean delta (drop T high+T low pairs) + win rate.
#       This is the decision tool for small (>=2%) effects. ENVA/ENVB are
#       space-separated VAR=VAL applied per side (e.g. "TURBO_V8_FLAGS=...").
#
#   full   <proj> [--jobs J] [--runs R] [-- cli-args...]
#       Whole-suite run (real config). Use to VALIDATE pass/fail + wall before
#       publishing. Slow.
#
#   profile <proj> [--sub N] [--jobs J]
#       macOS `sample` the binary during a warm run; bucket hot symbols
#       (GC / IC / String+RegExp / FS / parse).
#
# MEASUREMENT NOTES (learned the hard way):
#  * 8-job wall variance (~±30% per run) swamps a 2% effect. For small effects
#    use `ab` with --jobs 1 (serial => low variance) AND many --pairs; confirm
#    the winner with `full` at default jobs before publish.
#  * NEVER kill a run mid-flight: a killed esbuild leaves a truncated bundle in
#    the shared cache and poisons every later run (see docs/TODO-cache-poisoning).
#  * Cache is shared across projects in $TMPDIR/turbo-test-cache and is keyed by
#    content+mtime, independent of code changes to the runner, so warm once.
# ===========================================================================
set -u
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
CLI="$REPO/cli.js"

die(){ echo "harness: $*" >&2; exit 2; }
[ $# -lt 2 ] && die "usage: harness.sh <micro|ab|full|profile> <proj> [opts]"
CMD="$1"; shift
PROJ_IN="$1"; shift
PROJ="$(cd "$PROJ_IN" 2>/dev/null && pwd)" || die "no such project: $PROJ_IN"

SUB=40; JOBS=1; RUNS=3; PAIRS=12; TRIM=2; ALT=0
EXTRA=(); ENVA=""; ENVB=""; LA="A"; LB="B"
POS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --sub) SUB="$2"; shift 2;;
    --jobs) JOBS="$2"; shift 2;;
    --runs) RUNS="$2"; shift 2;;
    --pairs) PAIRS="$2"; shift 2;;
    --trim) TRIM="$2"; shift 2;;
    --alt) ALT=1; shift;;   # alternate A/B run order per pair: cancels the deterministic
                            # within-pair "second run is throttled" thermal bias seen at jobs=1
    --) shift; EXTRA=("$@"); break;;
    *) POS+=("$1"); shift;;
  esac
done
# --jobs env  => omit the --jobs flag so each A/B side's TURBO_JOBS env (or the host default)
# decides the worker count. Otherwise pass --jobs J (an explicit flag wins over TURBO_JOBS).
JOBFLAG=()
if [ -n "$JOBS" ] && [ "$JOBS" != "env" ]; then JOBFLAG=(--jobs "$JOBS"); fi

# ---- file selection: even-stride sample across src/ (representative, no e2e) ----
cd "$PROJ" || exit 1
ROOT="${SUB_ROOT:-src}"
ALL=()
while IFS= read -r f; do ALL+=("$f"); done < <(
  find "$ROOT" \( -name '*.test.ts' -o -name '*.test.tsx' -o -name '*.test.js' -o -name '*.test.jsx' \
            -o -name '*.spec.ts' -o -name '*.spec.tsx' \) -not -path '*/node_modules/*' 2>/dev/null | sort)
TOTAL=${#ALL[@]}
SELECT_FILES(){ # -> FILES[]
  FILES=()
  if [ "$SUB" -le 0 ] || [ "$TOTAL" -le "$SUB" ]; then FILES=("${ALL[@]}"); return; fi
  local stride=$(( TOTAL / SUB )); [ "$stride" -lt 1 ] && stride=1
  local k=0
  while [ "$k" -lt "$TOTAL" ] && [ "${#FILES[@]}" -lt "$SUB" ]; do FILES+=("${ALL[$k]}"); k=$((k+stride)); done
}
SUMMARY(){ grep -E 'files \|' | tail -1; }
WALL(){ grep -oE 'wall [0-9]+ ms' | grep -oE '[0-9]+'; }
PF(){ grep -oE '[0-9]+ passed \| [0-9]+ failed'; }
median(){ printf '%s\n' "$@" | sort -n | awk '{a[NR]=$0} END{print a[int((NR+1)/2)]}'; }
# trimmed mean: drop $1 lowest + $1 highest, mean the rest
tmean(){ local t="$1"; shift; printf '%s\n' "$@" | sort -n | awk -v t="$t" '{a[NR]=$0} END{s=0;c=0;for(i=t+1;i<=NR-t;i++){s+=a[i];c++} if(c>0)printf "%d", s/c; else print "NA"}'; }

case "$CMD" in
micro)
  SELECT_FILES
  echo "## micro $PROJ  files=${#FILES[@]}/$TOTAL  jobs=$JOBS runs=$RUNS args='${EXTRA[*]:-}'"
  L="$(node "$CLI" ${JOBFLAG[@]+"${JOBFLAG[@]}"} ${EXTRA[@]+"${EXTRA[@]}"} "${FILES[@]}" 2>/dev/null | SUMMARY)"; echo "WARMUP: $L"
  BPF="$(echo "$L" | PF)"; WS=()
  for r in $(seq 1 "$RUNS"); do
    L="$(node "$CLI" ${JOBFLAG[@]+"${JOBFLAG[@]}"} ${EXTRA[@]+"${EXTRA[@]}"} "${FILES[@]}" 2>/dev/null | SUMMARY)"
    P="$(echo "$L" | PF)"; W="$(echo "$L" | WALL)"; F=""
    [ "$P" != "$BPF" ] && F="  <<< PF DRIFT (was $BPF)"
    echo "run$r: wall=${W}ms | $P$F"; WS+=("$W")
  done
  echo "MEDIAN wall_ms=$(median "${WS[@]}")  pf=$BPF  walls=(${WS[*]})"
  ;;
ab)
  [ "${#POS[@]}" -lt 2 ] && die "ab needs ENVA ENVB"
  ENVA="${POS[0]}"; ENVB="${POS[1]}"; LA="${POS[2]:-A}"; LB="${POS[3]:-B}"
  SELECT_FILES
  run(){ env $1 node "$CLI" ${JOBFLAG[@]+"${JOBFLAG[@]}"} "${FILES[@]}" 2>/dev/null | SUMMARY; }
  echo "## ab $PROJ  files=${#FILES[@]}/$TOTAL  jobs=$JOBS pairs=$PAIRS trim=$TRIM"
  echo "## A=$LA [$ENVA]  vs  B=$LB [$ENVB]   (negative dB% => B faster)"
  run "$ENVA" >/dev/null; run "$ENVB" >/dev/null   # warm
  DS=(); BWIN=0; SA=0; SB=0; DRIFT=0
  for ((i=1;i<=PAIRS;i++)); do
    if [ "$ALT" = 1 ] && [ $((i%2)) -eq 0 ]; then
      RB="$(run "$ENVB")"; RA="$(run "$ENVA")"   # B-first on even pairs: penalize A this time
    else
      RA="$(run "$ENVA")"; RB="$(run "$ENVB")"
    fi
    WA="$(echo "$RA"|WALL)"; WB="$(echo "$RB"|WALL)"; PA="$(echo "$RA"|PF)"; PB="$(echo "$RB"|PF)"
    G=""; [ "$PA" != "$PB" ] && { G="  <<< PF DRIFT A=$PA B=$PB"; DRIFT=1; }
    D=$((WB-WA)); PCT="$(awk -v a="$WA" -v d="$D" 'BEGIN{printf "%+.1f",(d*100.0)/a}')"
    [ "$D" -lt 0 ] && BWIN=$((BWIN+1))
    SA=$((SA+WA)); SB=$((SB+WB)); DS+=("$D")
    echo "pair$i: A=${WA} B=${WB}  dB=${D}ms (${PCT}%)$G"
  done
  TM="$(tmean "$TRIM" "${DS[@]}")"; MD="$(median "${DS[@]}")"
  MA=$((SA/PAIRS))
  TMPCT="$(awk -v a="$MA" -v d="$TM" 'BEGIN{printf "%+.1f",(d*100.0)/a}')"
  MDPCT="$(awk -v a="$MA" -v d="$MD" 'BEGIN{printf "%+.1f",(d*100.0)/a}')"
  echo "---"
  echo "trimmed-mean dB=${TM}ms (${TMPCT}%) | median dB=${MD}ms (${MDPCT}%) | B faster $BWIN/$PAIRS | meanA=$MA meanB=$((SB/PAIRS))"
  [ "$DRIFT" = 1 ] && echo "!! ACCURACY: pass/fail drifted between A and B — result INVALID until fixed"
  VV="A faster/neutral"; awk -v t="$TM" 'BEGIN{exit !(t<0)}' && VV="B(exp) faster"
  echo "VERDICT(trimmed): $VV"
  ;;
full)
  echo "## full $PROJ  jobs=$JOBS runs=$RUNS args='${EXTRA[*]:-}'"
  L="$(node "$CLI" ${JOBFLAG[@]+"${JOBFLAG[@]}"} ${EXTRA[@]+"${EXTRA[@]}"} 2>/dev/null | SUMMARY)"; echo "WARMUP: $L"
  BPF="$(echo "$L"|PF)"; WS=()
  for r in $(seq 1 "$RUNS"); do
    L="$(node "$CLI" ${JOBFLAG[@]+"${JOBFLAG[@]}"} ${EXTRA[@]+"${EXTRA[@]}"} 2>/dev/null | SUMMARY)"
    P="$(echo "$L"|PF)"; W="$(echo "$L"|WALL)"; F=""; [ "$P" != "$BPF" ] && F="  <<< PF DRIFT (was $BPF)"
    echo "run$r: wall=${W}ms | $P$F"; WS+=("$W")
  done
  echo "MEDIAN wall_ms=$(median "${WS[@]}")  pf=$BPF"
  ;;
profile)
  SELECT_FILES
  BIN="$REPO/bin/turbo-test-darwin-arm64"; [ -x "$BIN" ] || BIN="$REPO/target/release/turbo-test"
  "$BIN" ${JOBFLAG[@]+"${JOBFLAG[@]}"} "${FILES[@]}" >/dev/null 2>&1   # warm
  "$BIN" ${JOBFLAG[@]+"${JOBFLAG[@]}"} "${FILES[@]}" >/dev/null 2>&1 &
  PID=$!; OUT="/tmp/tt_sample_$$.txt"
  sample "$PID" 10 -file "$OUT" >/dev/null 2>&1; wait "$PID" 2>/dev/null
  SEC="$(sed -n '/Sort by top of stack/,/Binary Images/p' "$OUT")"
  bucket(){ echo "$SEC" | grep -iE "$1" | grep -oE '[0-9]+$' | paste -sd+ - | bc; }
  tot="$(echo "$SEC"|grep -oE '[0-9]+$'|paste -sd+ -|bc)"
  wait_s="$(bucket 'psynch_cvwait|ulock_wait|kevent|__wait4|mach_msg')"
  echo "## profile $PROJ files=${#FILES[@]} jobs=$JOBS  (busy=$((tot-wait_s)) of $tot samples)"
  echo "  GC:        $(bucket 'Scaveng|Marking|Sweeper|RecordWrite|FreeList|Evacuate|IteratePointers|Heap::')"
  echo "  IC/props:  $(bucket 'LoadIC|StoreIC|KeyedLoad|KeyedStore|HashMap|MapPrototypeSet|Megamorphic')"
  echo "  String/RX: $(bucket 'String|RegExp|Scanner|murmur|cityhash|Hash')"
  echo "  FS sys:    $(bucket 'getattrlist|\bread\b|\bstat\b|__open|getdirentries|fstat')"
  echo "  full sample: $OUT"
  ;;
*) die "unknown cmd '$CMD'";;
esac
