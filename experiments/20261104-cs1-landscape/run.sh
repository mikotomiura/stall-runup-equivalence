#!/usr/bin/env bash
# CS1 の俯瞰（conditions.md）。データと中間は repo の外。記述だけ・検定なし。
# Step 0（capabilities_gate.py）は当たらない（GSE のログ用・元の俯瞰と同じ逸脱）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${DATA_DIR:=$HOME/gse-public-data}"
CS1="$DATA_DIR/cs1"
K19="$CS1/raw/2019/keystrokes.csv"
K21="$CS1/raw/2021/keystrokes.csv"
BIN="${GSE_REPLAY:-$DATA_DIR/klicke/bin/gse-replay}"
OUT="${OUT:-$EXP_DIR/results}"
# 部分実行（LIMIT）は正式の results に書かない（レビューの後）。解析の側も部分実行の中間を拒む
# 比べるのは正規化したパス（末尾の /・相対パス・シンボリックリンクで抜けない・2 巡目のレビューの後）
if [ -n "${LIMIT:-}" ] && python3 -I -c 'import os,sys;a,b=(os.path.realpath(x) for x in sys.argv[1:]);sys.exit(0 if a==b or a.startswith(b+os.sep) else 1)' "$OUT" "$EXP_DIR/results"; then
  echo "LIMIT のときは OUT を results の外に指定する" >&2; exit 1
fi
SC="$EXP_DIR/scripts"
SEED="$(cat "$EXP_DIR/SEED")"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$(sha "$K19")" = b54d8cf56b7719ae0ec9ecf19e5b3788168f087bd4a50238dca1e06f6bfe7955 ] || { echo "2019 年版の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$K21")" = 950a587bb3a410efc4aad0e64974d6442806345fddd13af76676dfb8006a22b6 ] || { echo "2021 年版の SHA-256 が違う" >&2; exit 1; }
[ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "gse-replay の SHA-256 が T2 と違う" >&2; exit 1; }
avail_kb="$(df -k "$DATA_DIR" | awk 'NR==2{print $4}')"
[ "$avail_kb" -gt $((3 * 1024 * 1024)) ] || { echo "空きが 3 GB を切っている" >&2; exit 1; }
python3 -I "$SC/build_cs1.py" --selftest
python3 -I "$SC/analyze_cs1.py" --selftest
python3 -I "$SC/../../20261102-public-data-landscape/scripts/analyze_landscape.py" --selftest
mkdir -p "$OUT"
python3 -I "$SC/make_holdout_cs1.py" "$SEED" "$CS1/holdout.json" "$K19" "$K21" > "$OUT/holdout_summary.json"
for y in 2019 2021; do
  rm -rf "$CS1/work$y"
  K="$CS1/raw/$y/keystrokes.csv"
  python3 -I "$SC/build_cs1.py" "$y" "$K" "$BIN" "$CS1/holdout.json" "$CS1/work$y" "${JOBS:-6}" ${LIMIT:+--limit "$LIMIT"}
  cp "$CS1/work$y/build_summary.json" "$OUT/build_summary_$y.json"
done
python3 -I "$SC/analyze_cs1.py" "$CS1/work2019" "$CS1/work2021" "$CS1/holdout.json" "$K19" "$K21" "$OUT"
python3 -I -c "import json,sys,hashlib,glob,os;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();e=sys.argv[5];json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'gse_replay_sha256':h(sys.argv[3]),'scripts_sha256':{os.path.relpath(os.path.realpath(p),os.path.realpath(e+'/..')):h(p) for p in sorted(glob.glob(sys.argv[4]+'/*.py'))+[e+'/../20261102-public-data-landscape/scripts/analyze_landscape.py',e+'/../20261101-klicke-early-stall-trigger/scripts/analyze_item2.py',e+'/../20261031-coauthor-pre-request-signal/scripts/convert.py',e+'/../20261031-coauthor-pre-request-signal/scripts/add_feat.py']}},open(sys.argv[6],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/20261104-cs1-landscape/scripts experiments/20261104-cs1-landscape/run.sh)" \
  "$BIN" "$SC" "$EXP_DIR" "$OUT/provenance.json"
