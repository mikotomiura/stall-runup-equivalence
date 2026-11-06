#!/usr/bin/env bash
# 俯瞰（conditions.md）。データと中間は repo の外。記述だけ・検定なし。
# Step 0（capabilities_gate.py）は当たらない（GSE のログ用・前の実験と同じ逸脱）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${DATA_DIR:=$HOME/gse-public-data}"
LAND="$DATA_DIR/landscape"
KZIP="$DATA_DIR/klicke/raw/WritingTask.zip"
KTAR="$DATA_DIR/klicke/work.tar.gz"
CZIP="$DATA_DIR/coauthor/raw/coauthor.zip"
META=("$DATA_DIR/coauthor/raw/metadata_creative.csv" "$DATA_DIR/coauthor/raw/metadata_argumentative.csv")
BIN="${GSE_REPLAY:-$DATA_DIR/klicke/bin/gse-replay}"
OUT="${OUT:-$EXP_DIR/results}"
SC="$EXP_DIR/scripts"
SEED="$(cat "$EXP_DIR/SEED")"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$KZIP")" = ffa4d753fdb9a54f5cadd6836426d56a0cabe73ceade8e7e40dddbbae178555d ] || { echo "WritingTask.zip の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$CZIP")" = 8d7380ca3811c671e774e43bfb9f5d3265e2a5d425788a066a612831aa1e418b ] || { echo "coauthor.zip の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "${META[0]}")" = 66fd2ca541999412de4050a85c17274f02eea04424e83906b697f4cd824c65bf ] || { echo "metadata_creative の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "${META[1]}")" = cfadf031b21c07951f648b02422b6703850359c7ff6a78a8ef513624c8e4c0d1 ] || { echo "metadata_argumentative の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "gse-replay の SHA-256 が T2 と違う" >&2; exit 1; }
avail_kb="$(df -k "$DATA_DIR" | awk 'NR==2{print $4}')"
[ "$avail_kb" -gt $((3 * 1024 * 1024)) ] || { echo "空きが 3 GB を切っている" >&2; exit 1; }
python3 -I "$SC/analyze_landscape.py" --selftest
mkdir -p "$LAND" "$OUT"
python3 -I "$SC/make_holdout.py" 4992 "$SEED" "$LAND/holdout.json" "${META[@]}" > "$OUT/holdout_summary.json"
python3 -I "$SC/klicke_side.py" "$KZIP" "$LAND/holdout.json" "$LAND/klicke_side.json" > "$OUT/klicke_side_summary.json"
rm -rf "$LAND/coauthor_work"
python3 -I "$SC/build_coauthor.py" "$CZIP" "$BIN" "$LAND/holdout.json" "$LAND/coauthor_work" "${JOBS:-6}" "${META[@]}" ${LIMIT:+--limit "$LIMIT"}
cp "$LAND/coauthor_work/build_summary.json" "$OUT/coauthor_build_summary.json"
python3 -I "$SC/analyze_landscape.py" "$KTAR" "$LAND/klicke_side.json" "$LAND/coauthor_work" "$LAND/holdout.json" "$OUT" "$CZIP" "${META[@]}"
# 事後（conditions.md に無い・判定に使わない）
python3 -I "$SC/posthoc.py" "$KTAR" "$LAND/klicke_side.json" "$LAND/coauthor_work" "$LAND/holdout.json" "$OUT/posthoc.json" "$CZIP" "${META[@]}" > /dev/null
python3 -I -c "import json,sys,hashlib,glob,os;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'gse_replay_sha256':h(sys.argv[3]),'klicke_work_tar_sha256':h(sys.argv[4]),'scripts_sha256':{os.path.relpath(os.path.realpath(p),os.path.realpath(sys.argv[6]+'/..')):h(p) for p in sorted(glob.glob(sys.argv[5]+'/*.py'))+[sys.argv[6]+'/../20261101-klicke-early-stall-trigger/scripts/analyze_item2.py',sys.argv[6]+'/../20261101-klicke-early-stall-trigger/scripts/convert_klicke.py',sys.argv[6]+'/../20261031-coauthor-pre-request-signal/scripts/convert.py',sys.argv[6]+'/../20261031-coauthor-pre-request-signal/scripts/add_feat.py']}},open(sys.argv[7],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/20261102-public-data-landscape/scripts experiments/20261102-public-data-landscape/run.sh)" \
  "$BIN" "$KTAR" "$SC" "$EXP_DIR" "$OUT/provenance.json"
