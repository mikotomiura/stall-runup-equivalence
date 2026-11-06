#!/usr/bin/env bash
# 感度（事前登録の P8）の中間を作る: CS1 の中間 <work> から、停滞の定義を変えて replay --features だけを回し直す。
# 先に上書きなしで回し直し、元の onsets・observed_intervals と全件一致することを確かめる（陽性対照）。崩れたら止める。
# 使い方: build-variants.sh <work> <python> <gse-replay> [jobs]
set -euo pipefail
WORK="${1:?CS1 の中間}"
PY="${2:?Python}"
BIN="${3:?gse-replay}"
JOBS="${4:-8}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RB="$HERE/scripts/rebuild_onsets.py"
"$PY" -I "$RB" --selftest
CTRL="${WORK}_control"
rm -rf "$CTRL"
"$PY" -I "$RB" "$WORK" "$CTRL" "$BIN" "$JOBS"
"$PY" -I -c "import json,sys;s=json.load(open(sys.argv[1]));sys.exit(0 if s['units']>0 and s['same_as_input']==s['units']==s['ok'] and s['stderr_nonempty']==0 and not s['overrides'] else '陽性対照が崩れた: '+json.dumps(s))" "$CTRL/rebuild_summary.json"
cp "$CTRL/rebuild_summary.json" "${WORK}_control_summary.json"
rm -rf "$CTRL"
build() {
  local name="$1"; shift
  rm -rf "${WORK}_$name"
  "$PY" -I "$RB" "$WORK" "${WORK}_$name" "$BIN" "$JOBS" "$@"
}
build q995 --set q=0.995 --set warmup_count=200
build w15 --set w_ms=15000
build w60 --set w_ms=60000
