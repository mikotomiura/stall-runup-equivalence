"""計器の確かめ: 全セッションの FeatSnapshot の p_stuck・ewma_x の分布（conditions_round2.md R6-5）。

使い方: python3 -I dist.py <work_dir> <out.json>
"""
import glob
import json
import os
import sys

ps, ex, n = [], [], 0
for f in sorted(glob.glob(os.path.join(sys.argv[1], "*_feat.json"))):
    n += 1
    for r in json.load(open(f)):
        ps.append(r["p_stuck"])
        ex.append(r["ewma_x"])
ps.sort()
ex.sort()
q = lambda a, p: a[int(p * (len(a) - 1))]
json.dump({"sessions": n, "rows": len(ps),
           "p_stuck_q": {k: q(ps, v) for k, v in {"50": .5, "90": .9, "99": .99, "max": 1}.items()},
           "p_stuck_share_ge_0.6": sum(x >= 0.6 for x in ps) / len(ps),
           "ewma_x_q": {k: q(ex, v) for k, v in {"50": .5, "90": .9, "99": .99}.items()}},
          open(sys.argv[2], "w"), indent=1)
