"""取り置きでの、対照の引きの種だけを替えた H1 の点推定の幅（事前登録 v10 の P10・記述・判定に使わない）。

確認のスクリプトの load_cs1（mode=holdout）と h1_writer・h1_pairs をそのまま使い、主の設定で、
対照の引きの種を 20261106 + 100 + k（k = 0..K-1）に替えて全員分の対の AUC（ε = 0.001 と 0）を出す。
主の値（種 20261106）も並べる。区間は出さない。書き手ごとの値は出さない。
使い方: python -I seed_spread.py <取り置きの中間> <holdout.json> <出力の json> [K]
"""
import json
import os
import random
import statistics
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402


def pooled(rows, sig, eps):
    w = n = 0
    for r in rows:
        a, b, _ = C.h1_pairs(r, sig, "none", eps)
        w, n = w + a, n + b
    return w / n if n else None


def main(work, hold_p, out, k=20):
    k = int(k)
    writers, chk = C.load_cs1(work, hold_p, "holdout")
    res = {"checks": chk, "seeds": [], "main_seed": 20261106}
    for seed in [20261106] + [20261106 + 100 + i for i in range(k)]:
        rng = random.Random(seed)
        cnt = Counter()
        rows = [C.h1_writer(u, rng, cnt, C.MAIN) for u in writers]
        res["seeds"].append({"seed": seed, **{f"{s}_eps_{e}": pooled(rows, s, e) for s in C.SIGS for e in C.EPS_SET}})
    summ = {}
    for key in [kk for kk in res["seeds"][0] if kk != "seed"]:
        vals = [d[key] for d in res["seeds"][1:]]
        summ[key] = {"main": res["seeds"][0][key], "min": min(vals), "max": max(vals), "sd": statistics.pstdev(vals)}
    res["summary"] = summ
    json.dump(res, open(out, "w"), indent=1)
    print(json.dumps(summ, indent=1))


if __name__ == "__main__":
    main(*sys.argv[1:5])
