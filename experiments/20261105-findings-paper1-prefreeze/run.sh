#!/usr/bin/env bash
# 論文 1 の凍結の前の計算（conditions.md）。データと中間は repo の外。見た書き手だけ。
# CS1 2019 年版の中間は俯瞰の build_cs1.py で作り直す（取り置きの書き手は build が読み飛ばす）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${DATA_DIR:=$HOME/gse-public-data}"
CS1="$DATA_DIR/cs1"
K19="$CS1/raw/2019/keystrokes.csv"
BIN="${GSE_REPLAY:-$DATA_DIR/klicke/bin/gse-replay}"
KWORK="$DATA_DIR/klicke/work"
LAND="$REPO_ROOT/experiments/20261104-cs1-landscape"
K6="$REPO_ROOT/experiments/20261101-klicke-early-stall-trigger"
OUT="$EXP_DIR/results"
SEED="$(cat "$EXP_DIR/SEED")"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$K19")" = b54d8cf56b7719ae0ec9ecf19e5b3788168f087bd4a50238dca1e06f6bfe7955 ] || { echo "2019 年版の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "gse-replay の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$CS1/holdout.json")" = a5a32fcefeb97b038afd170e2672262fd5a71230d5e851319eaa076b606bdc47 ] || { echo "取り置きの一覧の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$LAND/scripts/build_cs1.py")" = b64bdf74326f3975661f7700ebae8ec132d264ccb98276fdefef8904bd8bb30e ] || { echo "build_cs1.py が俯瞰と違う" >&2; exit 1; }
[ -d "$KWORK" ] || { echo "KLiCKe の中間が無い（work.tar.gz を展開する）" >&2; exit 1; }
python3 -I "$EXP_DIR/scripts/prefreeze.py" --selftest
if [ ! -e "$CS1/work2019/build_summary.json" ]; then
  rm -rf "$CS1/work2019"
  python3 -I "$LAND/scripts/build_cs1.py" 2019 "$K19" "$BIN" "$CS1/holdout.json" "$CS1/work2019" "${JOBS:-6}"
fi
mkdir -p "$OUT"
python3 -I "$EXP_DIR/scripts/prefreeze.py" "$CS1/work2019" "$CS1/holdout.json" "$K19" "$LAND/results" "$KWORK" "$K6/results" "$SEED" "$OUT"
python3 -I -c "import json,sys,hashlib,os;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();import numpy;r=sys.argv[4];ps=['experiments/20261105-findings-paper1-prefreeze/scripts/prefreeze.py','experiments/20261104-cs1-landscape/scripts/build_cs1.py','experiments/20261104-cs1-landscape/scripts/analyze_cs1.py','experiments/20261102-public-data-landscape/scripts/analyze_landscape.py','experiments/20261101-klicke-early-stall-trigger/scripts/analyze_item2.py','experiments/20261031-coauthor-pre-request-signal/scripts/convert.py','experiments/20261031-coauthor-pre-request-signal/scripts/add_feat.py'];json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'gse_replay_sha256':h(sys.argv[3]),'python':sys.version.split()[0],'numpy':numpy.__version__,'scripts_sha256':{p:h(os.path.join(r,p)) for p in ps}},open(sys.argv[5],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/20261105-findings-paper1-prefreeze/scripts experiments/20261105-findings-paper1-prefreeze/run.sh)" \
  "$BIN" "$REPO_ROOT" "$OUT/provenance.json"
