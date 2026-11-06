"""書き手の全セッション（feat 行つき NDJSON）を、2,500 ms の間隔で詰めて 1 本につなぐ（conditions_round2.md R1）。

使い方: python3 -I concat_writer.py <out.ndjson> <out_offsets.json> <sess1.ndjson> [<sess2.ndjson> ...]
セッションは元の開始時刻の順に並べる。offsets は sid → 足した ms（元の時刻 = つないだ時刻 − offset）。
"""
import json
import os
import sys

GAP_MS = 2500
out, out_off, paths = sys.argv[1], sys.argv[2], sys.argv[3:]
sess = []
for p in paths:
    rows = [json.loads(line) for line in open(p, encoding="utf-8")]
    body = [r for r in rows if r["type"] in ("key", "feat")]
    if not body:
        continue
    sid = os.path.basename(p).split(".")[0]
    sess.append((body[0]["t"], sid, body))
sess.sort()
offsets = {}
cursor = None
with open(out, "w", encoding="utf-8") as f:
    for i, (t0, sid, body) in enumerate(sess):
        off = 0 if cursor is None else (cursor + GAP_MS) - t0
        offsets[sid] = off
        if i == 0:
            f.write(json.dumps({"type": "meta", "session_start": t0 + off, "hmm_version": "0.5.0"}) + "\n")
            f.write(json.dumps({"type": "session_start", "t": t0 + off}) + "\n")
        for r in body:
            r = dict(r)
            r["t"] = r["t"] + off
            f.write(json.dumps(r) + "\n")
        cursor = max(r["t"] for r in body) + off
json.dump(offsets, open(out_off, "w"))
