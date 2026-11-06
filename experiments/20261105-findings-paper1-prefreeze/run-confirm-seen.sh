#!/usr/bin/env bash
# 凍結の前の計算 3（conditions-3.md）: 確認の解析スクリプトを見た書き手で空回しする。判定の値は書かない。
# 感度（P8）の中間（replay の回し直し）を作ってから回す。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXP_REL="experiments/20261105-findings-paper1-prefreeze"
: "${DATA_DIR:=$HOME/gse-public-data}"
PY="${CONFIRM_PY:-$DATA_DIR/venv-findings1/bin/python}"
BIN="${GSE_REPLAY:-$DATA_DIR/klicke/bin/gse-replay}"
WORK="$DATA_DIR/cs1/work2019"
OUT="$EXP_DIR/results"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
[ "$("$PY" -m pip freeze)" = "$(cat "$EXP_DIR/requirements-confirm.txt")" ] || { echo "環境が requirements-confirm.txt と違う" >&2; exit 1; }
[ "$(sha "$DATA_DIR/cs1/holdout.json")" = a5a32fcefeb97b038afd170e2672262fd5a71230d5e851319eaa076b606bdc47 ] || { echo "取り置きの一覧が違う" >&2; exit 1; }
[ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || { echo "gse-replay が違う" >&2; exit 1; }
export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
"$PY" -I "$EXP_DIR/scripts/confirm_p1.py" --selftest
if [ -z "${SKIP_VARIANTS:-}" ]; then
  bash "$EXP_DIR/build-variants.sh" "$WORK" "$PY" "$BIN" "${JOBS:-8}"
fi
cp "${WORK}_control_summary.json" "$OUT/rebuild_control_summary_seen.json"
for v in q995 w15 w60; do cp "${WORK}_$v/rebuild_summary.json" "$OUT/rebuild_summary_${v}_seen.json"; done
"$PY" -I "$EXP_DIR/scripts/confirm_p1.py" seen "$WORK" "$DATA_DIR/cs1/holdout.json" "$DATA_DIR/klicke/work" 20261106 "$OUT"
"$PY" -I -c "import json,sys;json.dump({'git_head':sys.argv[1],'git_dirty_paths':[l for l in sys.argv[2].splitlines() if l],'manifest_now':sys.argv[3].splitlines()},open(sys.argv[4],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$(git -C "$REPO_ROOT" status --porcelain -- experiments/)" \
  "$(cd "$REPO_ROOT" && shasum -a 256 -c "$EXP_REL/confirm-manifest.sha256" 2>&1 || true)" "$OUT/provenance_confirm_seen.json"
