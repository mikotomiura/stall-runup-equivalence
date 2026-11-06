#!/usr/bin/env bash
# Equivalence gate: the paper's pipelines, run with gate/compare-replay.sh standing in for gse-replay.
# Usage: run-gate.sh <cs1|cs1-variants|coauthor|klicke> [--limit N]   (cs1 and cs1-variants accept --limit;
#        cs1-variants needs the cs1 intermediate from a cs1 run with the same GATE_WORK)
# Environment: GSE_REPO (GSE checkout with experiments/), DATA_DIR (default ~/gse-public-data),
#   GATE_OLD (reference gse-replay; its SHA-256 must be 26cae610...33bb3), GATE_NEW (default: this crate's
#   target/release/gse-replay), GATE_WORK (scratch dir outside any repo), JOBS (default 6).
# Writes the per-call log to $GATE_WORK/<part>.log and compares the rebuilt intermediates with the ones the
# paper used. Nothing is written inside the repository.
# Exit code: 0 = every call matched and every intermediate is byte-identical / 1 = a mismatch or a failed step /
#   2 = usage or setup error.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PART="${1:?cs1 | cs1-variants | coauthor | klicke}"; shift
LIMIT=(); if [ "${1:-}" = "--limit" ]; then LIMIT=(--limit "${2:?N}"); shift 2; fi
[ $# -eq 0 ] || { echo "unknown argument: $1" >&2; exit 2; }
: "${GSE_REPO:?GSE checkout}" "${GATE_WORK:?scratch dir}"
: "${DATA_DIR:=$HOME/gse-public-data}" "${JOBS:=6}"
: "${GATE_OLD:=$DATA_DIR/klicke/bin/gse-replay}" "${GATE_NEW:=$HERE/../target/release/gse-replay}"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$GATE_OLD")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "GATE_OLD is not the reference gse-replay" >&2; exit 1; }
[ -x "$GATE_NEW" ] || { echo "GATE_NEW not built" >&2; exit 1; }
case "$GATE_WORK" in /?*) ;; *) echo "GATE_WORK must be an absolute path other than /" >&2; exit 2 ;; esac
mkdir -p "$GATE_WORK"
# the log keeps copies of mismatched inputs (raw data), so the scratch dir must not be inside a git work tree
command -v git >/dev/null || { echo "git is needed to check that GATE_WORK is outside any repository" >&2; exit 2; }
if git -C "$GATE_WORK" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "GATE_WORK is inside a git work tree; use a scratch dir outside any repository" >&2; exit 2
fi
export GATE_OLD GATE_NEW GATE_LOG="$GATE_WORK/$PART.log"
: > "$GATE_LOG"
CMP="$HERE/compare-replay.sh"
EXP="$GSE_REPO/experiments"
FAIL=0
# compare <rebuilt dir> <reference dir> <glob>: byte-compares the intermediates the pipeline wrote; fails on any
# difference, on an empty rebuild, and (without --limit) on a reference file the rebuild did not write
compare() {
  python3 -I - "$1" "$2" "$3" "${#LIMIT[@]}" <<'PY' || FAIL=1
import fnmatch, filecmp, json, os, sys
new, ref, pat, limited = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] != "0"
names = sorted(n for n in os.listdir(new) if fnmatch.fnmatch(n, pat))
refs = sorted(n for n in os.listdir(ref) if fnmatch.fnmatch(n, pat))
diff = [n for n in names if not os.path.exists(os.path.join(ref, n)) or not filecmp.cmp(os.path.join(new, n), os.path.join(ref, n), shallow=False)]
missing = [] if limited else sorted(set(refs) - set(names))
print(json.dumps({"rebuilt": len(names), "differs_from_reference": len(diff), "missing_from_rebuild": len(missing)}))
sys.exit(1 if diff or missing or not names else 0)
PY
}
case "$PART" in
  cs1)
    W="$GATE_WORK/cs1_work"; rm -rf "$W"
    python3 -I "$EXP/20261104-cs1-landscape/scripts/build_cs1.py" 2019 "$DATA_DIR/cs1/raw/2019/keystrokes.csv" "$CMP" \
      "$DATA_DIR/cs1/holdout.json" "$W" "$JOBS" ${LIMIT[@]+"${LIMIT[@]}"} >/dev/null || FAIL=1
    compare "$W" "$DATA_DIR/cs1/work2019" "u[0-9]*.json" ;;
  cs1-variants)
    W="$GATE_WORK/cs1_work"; [ -d "$W" ] || { echo "run the cs1 part first" >&2; exit 1; }
    bash "$EXP/20261105-findings-paper1-prefreeze/build-variants.sh" "$W" python3 "$CMP" "$JOBS" >/dev/null || FAIL=1
    for v in q995 w15 w60; do echo -n "$v "; compare "${W}_$v" "$DATA_DIR/cs1/work2019_$v" "u[0-9]*.json"; rm -rf "${W}_$v"; done
    rm -f "${W}_control_summary.json" ;;
  coauthor)
    W="$GATE_WORK/coauthor_onsets"; rm -rf "$W"
    python3 -I "$HERE/coauthor_gate.py" "$GSE_REPO" "$DATA_DIR/coauthor/raw/coauthor.zip" \
      "$DATA_DIR/coauthor/raw/metadata_creative.csv" "$DATA_DIR/coauthor/raw/metadata_argumentative.csv" \
      "$DATA_DIR/landscape/coauthor_work/sessions.json" "$CMP" "$W" "$JOBS" >/dev/null || FAIL=1
    compare "$W" "$DATA_DIR/landscape/coauthor_writer_onsets" "w[0-9]*.json" ;;
  klicke)
    W="$GATE_WORK/klicke_work"; rm -rf "$W"
    python3 -I "$EXP/20261101-klicke-early-stall-trigger/scripts/build_work.py" "$DATA_DIR/klicke/raw/WritingTask.zip" "$CMP" \
      "$EXP/20261031-coauthor-pre-request-signal/scripts/add_feat.py" "$W" "$JOBS" >/dev/null || FAIL=1
    compare "$W" "$DATA_DIR/klicke/work" "w[0-9]*.json" ;;
  *) echo "unknown part: $PART" >&2; exit 2 ;;
esac
python3 -I "$HERE/summarize_log.py" "$GATE_LOG" || FAIL=1
exit "$FAIL"
