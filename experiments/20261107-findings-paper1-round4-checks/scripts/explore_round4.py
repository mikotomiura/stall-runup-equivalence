"""X18（t − 30 秒以前の最後の推定の行の時間差の監査）。登録の外・取り置き・判定に使わない。

条件は ../conditions.md（計算の前にコミット）。集計の値だけを書く。
使い方: python -I explore_round4.py <取り置きの中間> <holdout.json> <confirm_holdout.json> <出力の json>
"""
import bisect
import json
import os
import sys
from collections import Counter

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261107-findings-paper1-round3-checks", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402
import explore_round3 as R3  # noqa: E402

D = 30_000
LAG_MAX, SPAN_LO, SPAN_HI = 2_000, 28_000, 32_000


def row_time(w, t):
    i = bisect.bisect_right(w.feat_t, t)
    return w.feat_t[i - 1] if i > 0 else None


def interval_of(w, t):
    for k, (a, b) in enumerate(w.intervals):
        if a <= t <= b:
            return k
    return None


def audit_point(w, u):
    a, b = row_time(w, u), row_time(w, u - D)
    lag_a, lag_b, span = u - a, (u - D) - b, a - b
    ia, ib = interval_of(w, a), interval_of(w, b)
    same = ia is not None and ia == ib
    bad = lag_a > LAG_MAX or lag_b > LAG_MAX or not (SPAN_LO <= span <= SPAN_HI) or not same
    return {"lag_a": lag_a, "lag_b": lag_b, "span": span, "same": same, "bad": bad}


def summarise(recs):
    if not recs:
        return {"n": 0}
    la = np.array([r["lag_a"] for r in recs], float)
    lb = np.array([r["lag_b"] for r in recs], float)
    sp = np.array([r["span"] for r in recs], float)
    q = lambda x: {"median": float(np.median(x)), "q99": float(np.quantile(x, 0.99)), "max": float(x.max())}
    return {"n": len(recs),
            "lag_u": q(la), "lag_u_minus_30s": q(lb), "span": q(sp),
            "share_lag_over_2s": float(np.mean((la > LAG_MAX) | (lb > LAG_MAX))),
            "share_span_outside_28_32s": float(np.mean((sp < SPAN_LO) | (sp > SPAN_HI))),
            "share_not_same_interval": float(np.mean([not r["same"] for r in recs])),
            "bad": int(sum(r["bad"] for r in recs)), "share_bad": float(np.mean([r["bad"] for r in recs]))}


def pooled(rows_by_writer, sig, eps):
    w = n = 0
    for rows in rows_by_writer:
        a, b, _ = C.h1_pairs(rows, sig, "none", eps)
        w, n = w + a, n + b
    return w / n if n else None, n


def main():
    hw, hold_p, conf_p, out_p = sys.argv[1:5]
    writers, chk = C.load_cs1(hw, hold_p, "holdout")
    rows, meta, cnt = R3.rows_with_s(writers)
    conf = json.load(open(conf_p))
    g0 = R3.gate_g0(writers, rows, conf["h1_counts"], cnt)
    if not all(g0.values()):
        sys.exit("門 G0 が外れた: " + json.dumps(g0))
    st, ct, bad_st, bad_ct = [], [], set(), set()
    for wi, (units, wr, wm) in enumerate(zip(writers, rows, meta)):
        seen = set()
        for r, m in zip(wr, wm):
            cell, _, _, ctrl = r
            if cell[1] != "none":
                continue
            a = audit_point(units[m[0]], m[1])
            st.append(a)
            if a["bad"]:
                bad_st.add((wi, m[0], m[1]))
            for cid, _, _ in ctrl:
                key = (cell, cid)
                if key in seen:
                    continue
                seen.add(key)
                c = audit_point(units[cid[0]], cid[1])
                ct.append(c)
                if c["bad"]:
                    bad_ct.add((wi, cid))
    out = {"checks_load": chk, "gate_g0": g0, "stalls": summarise(st), "controls": summarise(ct)}
    # 主の統計量（ε = 0.001）の点: 全部と、外れた行を除いた版
    base = {s: pooled(rows, s, C.EPS) for s in C.SIGS}
    out["main_point_all"] = {s: {"auc": v[0], "pairs": v[1]} for s, v in base.items()}
    if bad_st or bad_ct:
        kept = []
        for wi, (wr, wm) in enumerate(zip(rows, meta)):
            nw = []
            for r, m in zip(wr, wm):
                cell, sv, fv, ctrl = r
                if (wi, m[0], m[1]) in bad_st and cell[1] == "none":
                    continue
                c2 = [c for c in ctrl if (wi, c[0]) not in bad_ct]
                if not c2:
                    continue
                nw.append((cell, sv, fv, c2))
            kept.append(nw)
        ex = {s: pooled(kept, s, C.EPS) for s in C.SIGS}
        out["main_point_without_bad"] = {s: {"auc": v[0], "pairs": v[1]} for s, v in ex.items()}
    out["reading_rule"] = "none_bad" if not (bad_st or bad_ct) else "some_bad"
    out["eps"] = C.EPS
    json.dump(out, open(out_p, "w"), indent=1)
    print(json.dumps({"g0": g0, "reading": out["reading_rule"], "stalls_bad": out["stalls"].get("bad"),
                      "controls_bad": out["controls"].get("bad")}))


if __name__ == "__main__":
    main()
