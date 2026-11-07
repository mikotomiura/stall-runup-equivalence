#!/usr/bin/env bash
# Rerun the confirmation on the CS1 2019 holdout and compare the result with the released one.
#
# Usage: tools/rerun-holdout.sh <keystrokes_2019.csv> <holdout.json> <gse-replay> <work-dir> [jobs]
#   keystrokes_2019.csv  the CS1 2019 keystroke file (obtained from the data's authors)
#   holdout.json         the list of holdout writers, redrawn with
#                        experiments/20261104-cs1-landscape/scripts/make_holdout_cs1.py and seed 20261104
#                        (its SHA-256 must be a5a32fce...bdc47)
#   gse-replay           the replay tool: estimator/target/release/gse-replay, or the build the paper used
#   work-dir             an empty directory outside any git repository (it receives per-writer intermediates)
# Environment: PY (default python3) must have the packages of
#   experiments/20261105-findings-paper1-prefreeze/requirements-confirm.txt.
#
# The steps are those of experiments/20261105-findings-paper1-prefreeze/run-confirm-holdout.sh (build the holdout
# intermediates, check the holdout writers, build the sensitivity variants, run confirm_p1.py with seed 20261106),
# without its checks of the git history and the OSF records, which only the authors' run could pass
# (tools/rerun_confirm.py calls the unchanged confirm_p1.py with only that gate lifted).
# Exit code: 0 = the rerun's confirm_holdout.json is byte-identical to the released one / 1 = it differs or a step
# failed / 2 = usage error.
set -euo pipefail
[ $# -ge 4 ] && [ $# -le 5 ] || { sed -n 2,19p "$0" >&2; exit 2; }
K19="$1"; HOLD="$2"; BIN="$3"; WORKDIR="$4"; JOBS="${5:-4}"
PY="${PY:-python3}"
export OMP_NUM_THREADS=1  # confirm_p1.py requires it (small data; threads contend)
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
EXP="$ROOT/experiments/20261105-findings-paper1-prefreeze"
LAND="$ROOT/experiments/20261104-cs1-landscape"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$HOLD")" = a5a32fcefeb97b038afd170e2672262fd5a71230d5e851319eaa076b606bdc47 ] || { echo "holdout.json is not the registered list" >&2; exit 2; }
[ -x "$BIN" ] || { echo "gse-replay not found: $BIN" >&2; exit 2; }
mkdir -p "$WORKDIR"; WORKDIR="$(cd "$WORKDIR" && pwd -P)"
if git -C "$WORKDIR" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "work-dir is inside a git work tree; use a directory outside any repository" >&2; exit 2
fi
[ -z "$(ls -A "$WORKDIR")" ] || { echo "work-dir is not empty" >&2; exit 2; }
echo "gse-replay SHA-256: $(sha "$BIN")"
WORK="$WORKDIR/work2019_holdout"; OUT="$WORKDIR/results"; mkdir -p "$OUT"
"$PY" -I "$EXP/scripts/confirm_p1.py" --selftest
# the writers that are not in the holdout, so that the build skips them
"$PY" -I -c "import csv,json,sys;csv.field_size_limit(10**9);r=csv.reader(open(sys.argv[1],newline=''));i=next(r).index('SubjectID');s={x[i] for x in r};h=set(json.load(open(sys.argv[2]))['2019']);assert h<=s;json.dump({'2019':sorted(s-h)},open(sys.argv[3],'w'))" "$K19" "$HOLD" "$WORKDIR/skip_seen_2019.json"
"$PY" -I "$LAND/scripts/build_cs1.py" 2019 "$K19" "$BIN" "$WORKDIR/skip_seen_2019.json" "$WORK" "$JOBS"
"$PY" -I "$EXP/scripts/check_holdout_writers.py" "$K19" "$HOLD" "$WORK" "$OUT/holdout_writers_check.json"
bash "$EXP/build-variants.sh" "$WORK" "$PY" "$BIN" "$JOBS"
# confirm_p1.py itself refuses a holdout run outside the authors' procedure; the wrapper lifts only that gate
"$PY" -I "$ROOT/tools/rerun_confirm.py" "$WORK" "$HOLD" 20261106 "$OUT"
REF="$EXP/results_holdout/confirm_holdout.json"
if cmp -s "$OUT/confirm_holdout.json" "$REF"; then
  echo "confirm_holdout.json is byte-identical to the released one ($(sha "$REF"))"
else
  echo "confirm_holdout.json differs from the released one; differing entries (aggregate values only):" >&2
  "$PY" -I -c "import json,sys
def walk(a,b,p=''):
    if isinstance(a,dict) and isinstance(b,dict):
        for k in sorted(set(a)|set(b)): walk(a.get(k),b.get(k),p+'/'+str(k))
    elif a!=b: print(p or '/', 'rerun:', json.dumps(a)[:80], 'released:', json.dumps(b)[:80])
walk(json.load(open(sys.argv[1])),json.load(open(sys.argv[2])))" "$OUT/confirm_holdout.json" "$REF" | head -40 >&2
  echo "gse-replay SHA-256 above; compare it, the Python version and the packages with the released environment" >&2
  exit 1
fi
