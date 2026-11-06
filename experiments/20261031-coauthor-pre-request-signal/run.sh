#!/usr/bin/env bash
# CoAuthor の要求の手前の信号（conditions.md）。データは repo の外に置く（判断 2）。
# Step 0 の逸脱: capabilities_gate.py は GSE が集めたログ用で、変換した公開データには当たらない。
#   代わりに変換時の行種の数え上げ（results/convert_stats.json）を残す（decisions.md 判断 5）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${COAUTHOR_DIR:=$HOME/gse-public-data/coauthor}"
DWELL="${DWELL:-100}"
RAW="$COAUTHOR_DIR/raw/unz/coauthor-v1.0"
WORK="$COAUTHOR_DIR/work_dwell${DWELL}"
OUT="$EXP_DIR/results/dwell${DWELL}"
BIN="$REPO_ROOT/src-tauri/target/release/gse-replay"
SEED="$(cat "$EXP_DIR/SEED")"
mkdir -p "$WORK" "$OUT"
[ -x "$BIN" ] || (cd "$REPO_ROOT/src-tauri" && cargo build --release --bin gse-replay)
: > "$WORK/convert_stats.ndjson"
: > "$WORK/replay_stderr.log"
# 古いセッションの中間が残らないように（レビュー指摘）
find "$WORK" -maxdepth 1 -name "*_events.json" -delete
for f in "$RAW"/*.jsonl; do
  sid="$(basename "$f" .jsonl)"
  python3 -I "$EXP_DIR/scripts/convert.py" "$f" "$WORK/$sid.ndjson" "$WORK/${sid}_events.json" "$DWELL" \
    | sed "s/^{/{\"sid\": \"$sid\", /" >> "$WORK/convert_stats.ndjson"
  "$BIN" "$WORK/$sid.ndjson" > "$WORK/${sid}_feat.json" 2>>"$WORK/replay_stderr.log"
  "$BIN" "$WORK/$sid.ndjson" --features > "$WORK/${sid}_ft.json" 2>>"$WORK/replay_stderr.log"
  # ディスク節約: analyze.py が読む項目だけを残す（dwell 100 で出力がバイト同一と確認・notes.md 逸脱 1）
  python3 -I "$EXP_DIR/scripts/slim.py" "$WORK/${sid}_feat.json" "$WORK/${sid}_ft.json"
done
python3 -I -c "
import json,sys,collections
c=collections.Counter(); n=0
for l in open(sys.argv[1]):
    d=json.loads(l); n+=1
    for k,v in d.items():
        if k!='sid': c[k]+=v
c['sessions']=n; json.dump(c,open(sys.argv[2],'w'),indent=1)" "$WORK/convert_stats.ndjson" "$OUT/convert_stats.json"
python3 -I "$EXP_DIR/scripts/analyze.py" "$WORK" "$COAUTHOR_DIR/raw/metadata_creative.csv" \
  "$COAUTHOR_DIR/raw/metadata_argumentative.csv" "$SEED" "$OUT"
# 来歴（レビュー指摘）
python3 -I -c "import json,sys,hashlib;json.dump({'git_head':sys.argv[1],'gse_replay_sha256':hashlib.sha256(open(sys.argv[2],'rb').read()).hexdigest(),'dwell_ms':int(sys.argv[3]),'replay_stderr_bytes':int(sys.argv[4])},open(sys.argv[5],'w'),indent=1)" "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$BIN" "$DWELL" "$(wc -c < "$WORK/replay_stderr.log")" "$OUT/provenance.json"
