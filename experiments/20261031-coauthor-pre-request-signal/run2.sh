#!/usr/bin/env bash
# 2 回目（conditions_round2.md）。データは repo の外（判断 2）。dwell 100 ms だけ。
# Step 0 の逸脱は 1 回目と同じ（decisions.md 判断 5）。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${COAUTHOR_DIR:=$HOME/gse-public-data/coauthor}"
DWELL=100
RAW="$COAUTHOR_DIR/raw/unz/coauthor-v1.0"
WORK="$COAUTHOR_DIR/work2_dwell${DWELL}"
OUT="$EXP_DIR/results/round2_dwell${DWELL}"
BIN="$REPO_ROOT/src-tauri/target/release/gse-replay"
SC="$EXP_DIR/scripts"
SEED="$(cat "$EXP_DIR/SEED")"
rm -rf "$WORK"
mkdir -p "$WORK/writers" "$OUT"
[ -x "$BIN" ] || (cd "$REPO_ROOT/src-tauri" && cargo build --release --bin gse-replay)
: > "$WORK/replay_stderr.log"
for f in "$RAW"/*.jsonl; do
  sid="$(basename "$f" .jsonl)"
  python3 -I "$SC/convert.py" "$f" "$WORK/$sid.ndjson" "$WORK/${sid}_events.json" "$DWELL" > /dev/null
  "$BIN" "$WORK/$sid.ndjson" > "$WORK/${sid}_featfull.json" 2>>"$WORK/replay_stderr.log"
  python3 -I "$SC/add_feat.py" "$WORK/$sid.ndjson" "$WORK/${sid}_featfull.json" "$WORK/$sid.withfeat.ndjson"
  python3 -I -c "import json,sys;a=json.load(open(sys.argv[1]));json.dump([{k:r[k] for k in ('timestamp','p_stuck','ewma_x')} for r in a],open(sys.argv[2],'w'))" \
    "$WORK/${sid}_featfull.json" "$WORK/${sid}_feat.json"
  rm "$WORK/${sid}_featfull.json"
done
python3 -I "$SC/group_writers.py" "$COAUTHOR_DIR/raw/metadata_creative.csv" "$COAUTHOR_DIR/raw/metadata_argumentative.csv" > "$WORK/writers/groups.txt"
while read -r wid sids; do
  files=()
  for s in $sids; do [ -f "$WORK/$s.withfeat.ndjson" ] && files+=("$WORK/$s.withfeat.ndjson"); done
  [ "${#files[@]}" -gt 0 ] || continue
  python3 -I "$SC/concat_writer.py" "$WORK/writers/$wid.ndjson" "$WORK/writers/${wid}_offsets.json" "${files[@]}"
  "$BIN" "$WORK/writers/$wid.ndjson" --features > "$WORK/writers/${wid}_ftfull.json" 2>>"$WORK/replay_stderr.log" < /dev/null
  python3 -I -c "import json,sys;d=json.load(open(sys.argv[1]));json.dump({'observed_intervals':len(d['provenance']['observed_intervals']),'rows':[{'t_ms':r['t_ms'],'stall_onset':{k:r['stall_onset'].get(k) for k in ('is_onset','theta_person_ms')}} for r in d['rows'] if r['stall_onset'].get('is_onset')]},open(sys.argv[2],'w'))" \
    "$WORK/writers/${wid}_ftfull.json" "$WORK/writers/${wid}_ft.json"
  rm "$WORK/writers/${wid}_ftfull.json" "$WORK/writers/$wid.ndjson"
done < "$WORK/writers/groups.txt"
rm -f "$WORK"/*.withfeat.ndjson
python3 -I "$SC/analyze2.py" "$WORK" "$COAUTHOR_DIR/raw/metadata_creative.csv" \
  "$COAUTHOR_DIR/raw/metadata_argumentative.csv" "$SEED" "$OUT"
python3 -I "$SC/dist.py" "$WORK" "$OUT/dist.json"
python3 -I -c "import json,sys,hashlib;json.dump({'git_head':sys.argv[1],'gse_replay_sha256':hashlib.sha256(open(sys.argv[2],'rb').read()).hexdigest(),'dwell_ms':int(sys.argv[3]),'replay_stderr_bytes':int(sys.argv[4])},open(sys.argv[5],'w'),indent=1)" \
  "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$BIN" "$DWELL" "$(wc -c < "$WORK/replay_stderr.log")" "$OUT/provenance.json"
