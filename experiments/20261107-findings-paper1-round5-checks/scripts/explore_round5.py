"""X19a（比べを s − u = 30 秒の停滞に限る）・X19b（対照の θ の非対称）・X19c（閾値の表の差の区間）。
登録の外・取り置き・判定に使わない。条件は ../conditions.md（計算の前にコミット）。集計の値だけを書く。
使い方: python -I explore_round5.py <取り置きの中間> <holdout.json> <confirm_holdout.json> <explore_comparison.json> <出力の json>
環境: CONFIRM_JOBS（既定 8）・OMP_NUM_THREADS=1
"""
import json
import os
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261107-findings-paper1-round3-checks", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261106-findings-paper1-exploration", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402
import explore_comparison as EC  # noqa: E402
import explore_round3 as R3  # noqa: E402

SEED = 20261106
TOL = 1e-12
SIG4 = C.SIGS + C.LEVELS
CHANGES = C.SIGS
LEARNERS = ("logreg", "hgb")


def compare(rows):
    """X11 と同じ: 学習器 2 つ（残した行の中で学習し直す）と 4 つの信号の AUC、200 回の引き直しでの差。
    向きを問わない差（学習器 − max(信号, 1 − 信号)）も添える。"""
    X, y, wt, g, cells, cids, _ = C.ml_rows(rows)
    sg = EC.sig_arrays(rows)
    assert all(len(v) == len(y) for v in sg.values())
    specs = C.models(SEED, LEARNERS)
    keys = [(g[i], cells[i]) for i in range(len(y))]
    res = {"rows": int(len(y)), "stalls": int(y.sum()), "point": {}}
    for s in SIG4:
        res["point"][s] = C.paired_auc(sg[s], y, keys, cids)
    for m, spec in specs.items():
        res["point"][m] = C.oof_auc(X, y, wt, g, cells, cids, spec, SEED)
    from joblib import Parallel, delayed
    rng = np.random.default_rng(SEED + 4)
    ids = np.unique(g)
    idx_by = {k: np.where(g == k)[0] for k in ids}
    picks = [rng.choice(ids, len(ids), replace=True) for _ in range(C.N_BOOT_ML)]

    def one(pick):
        ix = np.concatenate([idx_by[k] for k in pick])
        copy_ = np.concatenate([np.full(len(idx_by[k]), c) for c, k in enumerate(pick)])
        kk = [(copy_[j], cells[i]) for j, i in enumerate(ix)]
        cc = [cids[i] for i in ix]
        o = {}
        for m, spec in specs.items():
            pred = C.oof_pred(X[ix], y[ix], wt[ix], g[ix], spec, SEED)
            o[m] = None if pred is None else C.paired_auc(pred, y[ix], kk, cc)
        for s in SIG4:
            o[s] = C.paired_auc(sg[s][ix], y[ix], kk, cc)
        return o
    reps = Parallel(n_jobs=int(os.environ.get("CONFIRM_JOBS", "8")))(delayed(one)(p) for p in picks)
    ok = [r for r in reps if all(r[m] is not None for m in LEARNERS)]
    res["n_rep"], res["n_rep_ok"] = len(reps), len(ok)
    for m in LEARNERS:
        res["point"][m + "_ci95"] = C.pct_ci([r[m] for r in ok])[0]
    diffs = {}
    nodir = lambda a: max(a, 1 - a)  # noqa: E731
    for m in LEARNERS:
        for s in SIG4:
            pt = res["point"][m] - res["point"][s]
            diffs["%s_minus_%s" % (m, s)] = {"point": pt, "ci95": C.pct_ci([r[m] - r[s] for r in ok])[0]}
            ptn = res["point"][m] - nodir(res["point"][s])
            diffs["%s_minus_%s_no_direction" % (m, s)] = {"point": ptn, "ci95": C.pct_ci([r[m] - nodir(r[s]) for r in ok])[0]}
    res["differences"] = diffs
    return res


def x19b(writers, rows, meta):
    st = ct = st_u = ct_u = 0
    bad_st, bad_ct = set(), set()
    for wi, (units, wr, wm) in enumerate(zip(writers, rows, meta)):
        seen = set()
        for r, m in zip(wr, wm):
            cell, _, _, ctrl = r
            if cell[1] != "none":
                continue
            df = units[m[0]].theta.defined_from
            st += 1
            if df is None or m[1] < df:
                st_u += 1
                bad_st.add((wi, m[0], m[1]))
            for cid, _, _ in ctrl:
                if (cell, cid) in seen:
                    continue
                seen.add((cell, cid))
                ct += 1
                dfc = units[cid[0]].theta.defined_from
                if dfc is None or cid[1] < dfc:
                    ct_u += 1
                    bad_ct.add((wi, cid))
    kept = []
    for wi, (wr, wm) in enumerate(zip(rows, meta)):
        nw = []
        for r, m in zip(wr, wm):
            cell, sv, fv, ctrl = r
            if cell[1] == "none" and (wi, m[0], m[1]) in bad_st:
                continue
            c2 = [c for c in ctrl if (wi, c[0]) not in bad_ct]
            if c2:
                nw.append((cell, sv, fv, c2))
        kept.append(nw)
    pt = {}
    for s in CHANGES:
        w = n = 0
        for r in kept:
            a, b, _ = C.h1_pairs(r, s, "none", C.EPS)
            w, n = w + a, n + b
        pt[s] = {"auc": w / n if n else None, "pairs": n}
    share_ct = ct_u / ct if ct else None
    return {"stalls": st, "stalls_theta_undefined_at_u": st_u, "controls": ct, "controls_theta_undefined_at_u": ct_u,
            "share_controls_theta_undefined": share_ct, "main_point_theta_defined_both": pt,
            "reading_rule": "count_only" if share_ct is not None and share_ct < 0.01 else "report_point"}


def x19c(rows, conf):
    q = 0.9
    out = {}
    rng = np.random.default_rng(SEED + 5)
    per_sig = {}
    for sig in CHANGES:
        per = []
        for r in rows:
            pos = [sv[sig] for cell, sv, _, _ in r if cell[1] == "none"]
            neg = list({cid: cs[sig] for cell, _, _, ctrl in r if cell[1] == "none" for cid, cs, _ in ctrl}.values())
            if not pos or not neg:
                continue
            cv = sorted(neg)
            th = cv[min(len(cv) - 1, int(q * len(cv)))]
            per.append((sum(1 for v in pos if v > th), len(pos), sum(1 for c in cv if c > th), len(cv)))
        per_sig[sig] = np.asarray(per, dtype=float)
    n = len(next(iter(per_sig.values())))
    b = rng.integers(0, n, (C.N_BOOT, n))
    for sig, a in per_sig.items():
        hit, exc = a[:, 0].sum() / a[:, 1].sum(), a[:, 2].sum() / a[:, 3].sum()
        ref = conf["threshold_table"][sig][str(q)]
        gate = bool(abs(hit - ref["hit_pooled"]) <= TOL and abs(exc - ref["exceed_pooled"]) <= TOL)
        dif = a[b, 0].sum(1) / a[b, 1].sum(1) - a[b, 2].sum(1) / a[b, 3].sum(1)
        ci = C.pct_ci(dif)[0]
        out[sig] = {"stall_exceed": float(hit), "control_exceed": float(exc), "difference": float(hit - exc), "ci95": ci,
                    "gate_g5_equals_confirm": gate, "writers": int(n),
                    "reading": "includes_zero" if ci[0] <= 0 <= ci[1] else "excludes_zero"}
    return out


def main():
    hw, hold_p, conf_p, cmp_p, out_p = sys.argv[1:6]
    writers, chk = C.load_cs1(hw, hold_p, "holdout")
    rows, meta, cnt = R3.rows_with_s(writers)
    conf, cmpj = json.load(open(conf_p)), json.load(open(cmp_p))
    g0 = R3.gate_g0(writers, rows, conf["h1_counts"], cnt)
    if not all(g0.values()):
        sys.exit("門 G0 が外れた: " + json.dumps(g0))
    out = {"checks_load": chk, "gate_g0": g0}
    out["x19c"] = x19c(rows, conf)
    if not all(v["gate_g5_equals_confirm"] for v in out["x19c"].values()):
        sys.exit("門 G5 が外れた: " + json.dumps(out["x19c"]))
    out["x19b"] = x19b(writers, rows, meta)
    full = compare(rows)
    g4 = {"logreg_equals_confirm": bool(abs(full["point"]["logreg"] - conf["H3"]["logreg"]["auc"]) <= TOL),
          "hgb_equals_confirm": bool(abs(full["point"]["hgb"] - conf["H3"]["hgb"]["auc"]) <= TOL)}
    for s in SIG4:
        g4[s + "_equals_x11"] = bool(abs(full["point"][s] - cmpj["point"][s]["paired_auc_eps0"]) <= TOL)
    if not all(g4.values()):
        sys.exit("門 G4 が外れた: " + json.dumps(g4))
    s30 = [[r for r, m in zip(wr, wm) if m[2] - m[1] == 30_000] for wr, wm in zip(rows, meta)]
    part = compare(s30)
    d = part["differences"]
    los = [d["logreg_minus_%s_no_direction" % s]["ci95"] for s in CHANGES]
    reading = ("survives" if all(lo[0] > 0 for lo in los) else
               "not_shown" if all(lo[0] <= 0 for lo in los) else "between")
    out["x19a"] = {"gate_g4": g4, "all": full, "s_minus_u_eq_30s": part, "reading_rule": reading}
    json.dump(out, open(out_p, "w"), indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o))
    print(json.dumps({"g0": g0, "g4": all(g4.values()), "x19a": reading, "x19b": out["x19b"]["reading_rule"],
                      "x19c": {s: v["reading"] for s, v in out["x19c"].items()}}))


if __name__ == "__main__":
    main()
