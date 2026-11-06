#!/usr/bin/env bash
# 項目 2（conditions.md）。データと中間は repo の外（~/gse-public-data/klicke/）。
# 1) zip の SHA-256 を照合 2) 1 本ずつ変換・replay（build_work.py）3) θ と onset の照合（止める門）4) 解析。
# Step 0（capabilities_gate.py）は当たらない（GSE のログ用・前の 2 実験と同じ逸脱）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${KLICKE_DIR:=$HOME/gse-public-data/klicke}"
ZIP="$KLICKE_DIR/raw/WritingTask.zip"
WORK="$KLICKE_DIR/work"
OUT="$EXP_DIR/results"
SC="$EXP_DIR/scripts"
BIN="${GSE_REPLAY:-$REPO_ROOT/src-tauri/target/release/gse-replay}"
SHA_EXPECTED=ffa4d753fdb9a54f5cadd6836426d56a0cabe73ceade8e7e40dddbbae178555d
SEED="$(cat "$EXP_DIR/SEED")"
[ "$(shasum -a 256 "$ZIP" | cut -d' ' -f1)" = "$SHA_EXPECTED" ] || { echo "WritingTask.zip の SHA-256 が違う" >&2; exit 1; }
[ -x "$BIN" ] || (cd "$REPO_ROOT/src-tauri" && cargo build --release --bin gse-replay)
mkdir -p "$OUT"
python3 -I "$SC/selftest_convert.py"
rm -rf "$WORK"
python3 -I "$SC/vocabulary.py" "$ZIP" "$OUT/key_vocabulary.json"
python3 -I "$SC/build_work.py" "$ZIP" "$BIN" "$REPO_ROOT/experiments/20261031-coauthor-pre-request-signal/scripts/add_feat.py" "$WORK" "${JOBS:-8}"
cp "$WORK/build_summary.json" "$OUT/build_summary.json"
python3 -I "$SC/analyze_item2.py" "$WORK" "$SEED" "$OUT"
# 来歴: 走らせたコードを固定する（git_head・作業ツリーの汚れ・各スクリプトの SHA-256）
python3 -I -c "import json,sys,hashlib,glob,os;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'gse_replay_sha256':h(sys.argv[3]),'zip_sha256':sys.argv[4],'scripts_sha256':{os.path.basename(p):h(p) for p in sorted(glob.glob(sys.argv[5]+'/*.py')+[sys.argv[6]])}},open(sys.argv[7],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/20261101-klicke-early-stall-trigger/scripts experiments/20261101-klicke-early-stall-trigger/run.sh experiments/20261031-coauthor-pre-request-signal/scripts/add_feat.py)" \
  "$BIN" "$SHA_EXPECTED" "$SC" "$REPO_ROOT/experiments/20261031-coauthor-pre-request-signal/scripts/add_feat.py" "$OUT/provenance.json"
