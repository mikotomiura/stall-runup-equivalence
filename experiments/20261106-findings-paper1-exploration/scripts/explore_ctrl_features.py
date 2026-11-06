"""対照を休止 5・10 秒以上に限る（X14）と、学習器の量ごとの AUC（X15）。登録の外の探索・取り置き・判定に使わない。

X14: 確認のスクリプトの対照の下限 CTRL_LO を走りの間だけ 5,000 / 10,000 ms に替えて H1 の型を出す。
     門: 2,000 ms で回した値が確認の H1 の点推定と件数に一致。
X15: 確認の主の行の 18 の量を単独の得点として paired_auc（ε なし・主の層）。量が数値でない行はその量から除く。
     門: 推定器の 4 つの値を同じ関数で出した AUC が explore_comparison.json の point と一致。
集計の値だけを書く。
使い方: python -I explore_ctrl_features.py <取り置きの中間> <holdout.json> <confirm_holdout.json> <explore_comparison.json>
        <X14 の出力> <X15 の出力>
"""
import json
import math
import os
import random
import sys
from collections import Counter

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402

SEED = 20261106
TOL = 1e-12
NAMES9 = ["presses", "interval_q10", "interval_q50", "interval_q90", "intervals_ge_2s", "intervals_ge_10s",
          "backspace_share", "burst_mean", "char_presses_per_min"]
NAMES18 = ["level_" + n for n in NAMES9] + ["change_" + n for n in NAMES9]


def h1(writers, feats=False):
    cnt = Counter()
    rng = random.Random(SEED)
    rows = [C.h1_writer(u, rng, cnt, C.MAIN, feats=feats) for u in writers]
    return rows, dict(cnt)


def pooled(rows, sig, eps):
    wpt = [C.h1_pairs(r, sig, "none", eps) for r in rows]
    return sum(x[0] for x in wpt) / sum(x[1] for x in wpt)


def x14(writers, conf):
    out = {}
    saved = C.CTRL_LO
    try:
        for lo in (2_000, 5_000, 10_000):
            C.CTRL_LO = lo
            rows, cnt = h1(writers)
            summ = C.h1_summary(rows, SEED, "none", False, C.SIGS, light=False)
            e = {"counts": cnt}
            for s in C.SIGS:
                e[s] = {k: {"pooled_auc": summ[s][k]["pooled_auc"], "ci95": summ[s][k]["ci95"],
                            "branch_form": summ[s][k]["branch"], "pairs": summ[s][k]["pairs"]} for k in ("eps_0.001", "eps_0")}
                e[s]["weights"] = summ[s]["weights"]
            out["ctrl_lo_%d" % lo] = e
            if lo == 2_000:
                gate = cnt == conf["h1_counts"] and all(
                    abs(e[s][k]["pooled_auc"] - conf["H1"][s][k]["pooled_auc"]) <= TOL for s in C.SIGS for k in ("eps_0.001", "eps_0"))
                out["gate_2000_equals_confirm"] = gate
                if not gate:
                    sys.exit("X14 の門が外れた")
    finally:
        C.CTRL_LO = saved
    return out


def x15(writers, comp):
    rows, _ = h1(writers, feats=True)
    X, y, wt, g, cells, cids, miss = C.ml_rows(rows)
    keys = [(g[i], cells[i]) for i in range(len(y))]
    sig = {s: [] for s in C.SIGS + C.LEVELS}
    for r in rows:
        for c, sv, fv, ctrl in r:
            if c[1] != "none":
                continue
            for s in sig:
                sig[s].append(sv[s])
            for _, cs, _ in ctrl:
                for s in sig:
                    sig[s].append(cs[s])
    out = {"rows": int(len(y)), "stalls": int(y.sum()), "gate": {}, "features": {}}
    for s in sig:
        a = C.paired_auc(np.asarray(sig[s]), y, keys, cids)
        out["gate"][s] = abs(a - comp["point"][s]["paired_auc_eps0"]) <= TOL
    if not all(out["gate"].values()):
        sys.exit("X15 の門が外れた: " + json.dumps(out["gate"]))
    for j, name in enumerate(NAMES18):
        ok = np.isfinite(X[:, j])
        a = C.paired_auc(X[ok, j], y[ok], [k for k, m in zip(keys, ok) if m], [c for c, m in zip(cids, ok) if m])
        out["features"][name] = {"paired_auc_eps0": a, "stall_rows_dropped": int(((~ok) & (y == 1)).sum()),
                                 "control_rows_dropped": int(((~ok) & (y == 0)).sum())}
    return out


def main():
    hw, hold_p, conf_p, comp_p, out14, out15 = sys.argv[1:]
    writers, chk = C.load_cs1(hw, hold_p, "holdout")
    r14 = x14(writers, json.load(open(conf_p)))
    r14["checks_load"] = chk
    json.dump(r14, open(out14, "w"), indent=1)
    r15 = x15(writers, json.load(open(comp_p)))
    json.dump(r15, open(out15, "w"), indent=1)
    print(json.dumps({"x14_gate": r14["gate_2000_equals_confirm"], "x15_gate": r15["gate"]}))


if __name__ == "__main__":
    main()
