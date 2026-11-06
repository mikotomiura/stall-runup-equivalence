"""セッションの NDJSON に replay の既定の出力を feat 行として足す（GSE のログと同じ形・conditions_round2.md R1）。

使い方: python3 -I add_feat.py <in.ndjson> <feat.json> <out.ndjson>
同じ時刻では key 行を先に置く。推定の値は変えない（feat 行は FeatureExtractor に入らない）。
"""
import json
import sys

KEYS = ["f1", "f3", "f4", "f5", "f6", "p_flow", "p_inc", "p_stuck", "raw_x", "raw_y",
        "ewma_x", "ewma_y", "obs", "alpha", "keys", "bs_keys"]
rows = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
head = [r for r in rows if r["type"] in ("meta", "session_start")]
body = [r for r in rows if r["type"] not in ("meta", "session_start")]
feat = [dict(type="feat", t=f["timestamp"], **{k: f[k] for k in KEYS}) for f in json.load(open(sys.argv[2]))]
body = sorted(body + feat, key=lambda r: (r["t"], 0 if r["type"] == "key" else 1))
with open(sys.argv[3], "w", encoding="utf-8") as f:
    for r in head + body:
        f.write(json.dumps(r) + "\n")
