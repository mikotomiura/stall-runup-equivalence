#!/usr/bin/env bash
# CoAuthor の 1 人ごとの学習の学習曲線（conditions.md）。データと中間は repo の外。代理ラベルで、詰まりの推定ではない。
# Step 0（capabilities_gate.py）は当たらない（GSE のログ用・前の実験と同じ逸脱）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${DATA_DIR:=$HOME/gse-public-data}"
OLD="$DATA_DIR/landscape/coauthor_work"
NEW="$DATA_DIR/landscape/coauthor_work_obs"
CZIP="$DATA_DIR/coauthor/raw/coauthor.zip"
META=("$DATA_DIR/coauthor/raw/metadata_creative.csv" "$DATA_DIR/coauthor/raw/metadata_argumentative.csv")
BIN="$DATA_DIR/klicke/bin/gse-replay"
OUT="${OUT:-$EXP_DIR/results}"
SC="$EXP_DIR/scripts"
SEED="$(cat "$EXP_DIR/SEED")"
JOBS="${JOBS:-10}"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$CZIP")" = 8d7380ca3811c671e774e43bfb9f5d3265e2a5d425788a066a612831aa1e418b ] || { echo "coauthor.zip の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "${META[0]}")" = 66fd2ca541999412de4050a85c17274f02eea04424e83906b697f4cd824c65bf ] || { echo "metadata_creative の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "${META[1]}")" = cfadf031b21c07951f648b02422b6703850359c7ff6a78a8ef513624c8e4c0d1 ] || { echo "metadata_argumentative の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "gse-replay の SHA-256 が違う" >&2; exit 1; }
mkdir -p "$OUT"
# Q1 中間の作り直し（無ければ）と門 P-D1
if [ ! -f "$NEW/build_summary.json" ]; then
  rm -rf "$NEW"
  python3 -I "$SC/build_obs.py" "$CZIP" "$BIN" "$OLD" "$NEW" "$JOBS" "${META[@]}"
fi
python3 -I "$SC/check_same.py" "$OLD" "$NEW" > "$OUT/check_same.json"
# P-D2（engine の前向き＋表示の層の再現）
python3 -I "$SC/perperson.py" --pd2 "$NEW" "$OUT/pd2.json"
# 陽性対照（合成のデータだけ）
python3 -I "$SC/perperson.py" --selftest "$NEW" "$SEED" "$OUT/selftest.json"
# 変異（名指しの対照が赤になること）を本計算の前の門にする
OUT="$OUT" bash "$EXP_DIR/mutate.sh"
python3 -I "$SC/check_mutations.py" "$OUT/mutations" "$OUT/selftest.json"
# 本計算（書き手ごとの値は repo の外に残す）
GSE_PP_RAW="$DATA_DIR/landscape/coauthor_pp_writer_values.json" python3 -I "$SC/perperson.py" "$NEW" "$SEED" "$OUT" "$JOBS"
python3 -I -c "import json,sys,hashlib,glob,os;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();fs=sorted(glob.glob(sys.argv[3]+'/s[0-9]*.json'));json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'scripts_sha256':{os.path.basename(p):h(p) for p in sorted(glob.glob(sys.argv[4]+'/*.py'))},'gse_replay_sha256':h(sys.argv[6]),'input_files':len(fs),'input_sha256_of_sha256_list':hashlib.sha256(''.join(os.path.basename(f)+h(f) for f in fs).encode()).hexdigest(),'python':sys.version.split()[0],'numpy':__import__('numpy').__version__},open(sys.argv[5],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/20261103-coauthor-per-person/scripts experiments/20261103-coauthor-per-person/run.sh)" \
  "$NEW" "$SC" "$OUT/provenance.json" "$BIN"
