"""比べの表（X11）と、停滞の実効の長さ・観測の端・較正の引きの実効の書き手（X12）。登録の外の探索・判定に使わない。

X11: 取り置きの主の層の同じ組で、推定器の 4 つの信号と学習器 2 つの AUC（paired_auc・ε なし）と、確認の boot_oof と
     同じ引き直しの上での差のパーセンタイル区間。門: 学習器の点推定と区間・推定器の ε = 0.001 の点推定が確認の値と一致。
X12 (a)(b): 取り置きの停滞の行の t0 − u と G(u)・観測の端の数。門: 行 4,554・件数が確認と同じ。
X12 (c): 見た書き手で確認の boundary_calibration の引きを同じ乱数で作り直し、各 draw の実効の書き手。
     門: 見た書き手の主の行の対の数と重みが confirm_seen.json と一致。較正の分岐は回し直さない。
集計の値だけを書く。書き手ごとの値は出さない。
使い方: python -I explore_comparison.py <取り置きの中間> <見た書き手の中間> <holdout.json> <confirm_holdout.json>
        <confirm_seen.json> <X11 の出力> <X12 の出力>
"""
import bisect
import json
import os
import random
import sys
from collections import Counter

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402
import prefreeze as pf  # noqa: E402

SEED = 20261106
TOL = 1e-12
SIG4 = C.SIGS + C.LEVELS
LEARNERS = ("logreg", "hgb")

ORIG_PAUSE_INFO = C.pause_info
ORIG_ANCHORS = C.stall_anchors
ANCHOR_INFO = {}


def pause_info_u(w, u, end, cs1):
    r = ORIG_PAUSE_INFO(w, u, end, cs1)
    r["_wu"] = (id(w), u)
    return r


def anchors_rec(w, cfg=None):
    out = ORIG_ANCHORS(w, cfg)
    for u, s0, nxt, i, t0 in out:
        ANCHOR_INFO.setdefault((id(w), u), (s0, t0, nxt))
    return out


def q(v, qs=(0.1, 0.25, 0.5, 0.75, 0.9)):
    v = np.asarray(v, dtype=float)
    return {str(x): float(np.quantile(v, x)) for x in qs} | {"mean": float(v.mean()), "n": int(len(v))}


def sig_arrays(rows):
    """ml_rows と同じ順の推定器の値（停滞・対照）。"""
    out = {s: [] for s in SIG4}
    for r in rows:
        for c, sv, fv, ctrl in r:
            if c[1] != "none":
                continue
            for s in SIG4:
                out[s].append(sv[s])
            for _, cs, _ in ctrl:
                for s in SIG4:
                    out[s].append(cs[s])
    return {s: np.asarray(v, dtype=float) for s, v in out.items()}


def x11(writers, rows, conf):
    X, y, wt, g, cells, cids, _ = C.ml_rows(rows)
    sg = sig_arrays(rows)
    assert all(len(v) == len(y) for v in sg.values())
    specs = C.models(SEED, LEARNERS)
    keys = [(g[i], cells[i]) for i in range(len(y))]
    res = {"rows": int(len(y)), "stalls": int(y.sum()), "point": {}, "gates": {}}
    for s in SIG4:
        res["point"][s] = {"paired_auc_eps0": C.paired_auc(sg[s], y, keys, cids)}
    for m, spec in specs.items():
        a = C.oof_auc(X, y, wt, g, cells, cids, spec, SEED)
        res["point"][m] = {"paired_auc_eps0": a}
        res["gates"]["point_%s_equals_confirm" % m] = abs(a - conf["H3"][m]["auc"]) <= TOL
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
    for m in LEARNERS:
        ci, _ = C.pct_ci([r[m] for r in reps if r[m] is not None])
        fails = sum(1 for r in reps if r[m] is None)
        res["point"][m]["ci95"] = ci
        res["point"][m]["boot_failures"] = fails
        res["gates"]["ci_%s_equals_confirm" % m] = (fails == conf["H3"][m]["boot_failures"] and
                                                   all(abs(a - b) <= TOL for a, b in zip(ci, conf["H3"][m]["ci95"])))
    for s in SIG4:
        res["point"][s]["ci95_same_resamples"] = C.pct_ci([r[s] for r in reps])[0]
    diffs = {}
    ok = [r for r in reps if all(r[m] is not None for m in LEARNERS)]
    for m in LEARNERS:
        for s in SIG4:
            diffs["%s_minus_%s" % (m, s)] = {"point": res["point"][m]["paired_auc_eps0"] - res["point"][s]["paired_auc_eps0"],
                                             "ci95": C.pct_ci([r[m] - r[s] for r in ok])[0]}
    diffs["logreg_minus_hgb"] = {"point": res["point"]["logreg"]["paired_auc_eps0"] - res["point"]["hgb"]["paired_auc_eps0"],
                                 "ci95": C.pct_ci([r["logreg"] - r["hgb"] for r in ok])[0]}
    diffs["p_stuck_minus_d_p_stuck"] = {"point": res["point"]["p_stuck"]["paired_auc_eps0"] - res["point"]["d_p_stuck"]["paired_auc_eps0"],
                                        "ci95": C.pct_ci([r["p_stuck"] - r["d_p_stuck"] for r in ok])[0]}
    diffs["ewma_x_minus_d_ewma_x"] = {"point": res["point"]["ewma_x"]["paired_auc_eps0"] - res["point"]["d_ewma_x"]["paired_auc_eps0"],
                                      "ci95": C.pct_ci([r["ewma_x"] - r["d_ewma_x"] for r in ok])[0]}
    res["differences"] = diffs
    res["n_rep"] = len(reps)
    return res


def x12_ab(writers, rows):
    ws = {}
    for units in writers:
        for w in units:
            ws[id(w)] = w
    lead, gap = [], []
    for r in rows:
        for c, sv, _, _ in r:
            if c[1] != "none":
                continue
            s0, t0, nxt = ANCHOR_INFO[sv["_wu"]]
            u = sv["_wu"][1]
            lead.append(t0 - u)
            gap.append(nxt - u)
    gap_a = np.asarray(gap, dtype=float)
    a = {"stall_rows": len(lead), "onset_minus_u_ms": q(lead), "resume_minus_u_ms": q(gap),
         "resume_ge_60s_share": float((gap_a >= 60_000).mean()), "resume_ge_120s_share": float((gap_a >= 120_000).mean()),
         "resume_ge_300s_share": float((gap_a >= 300_000).mean())}
    # (b) 観測の端: onset の u ごと（同じ u は 1 度）
    cnt = Counter()
    for units in writers:
        for w in units:
            seen = set()
            for s0, t0, _, _ in w.onsets:
                i = bisect.bisect_right(w.text, t0) - 1
                if i < 0:
                    cnt["onset_before_first_text"] += 1
                    continue
                u = w.text[i]
                if u in seen:
                    continue
                seen.add(u)
                cnt["onset_u_unique"] += 1
                if i + 1 >= len(w.text):
                    nxt = w.t_end
                    cnt["no_next_text_press"] += 1
                    cnt["no_next_text_press_gap_ge_30s"] += nxt - u >= C.STALL_GAP
                elif pf.block_start(w, w.text[i + 1]) != pf.block_start(w, u):
                    cnt["next_text_press_in_later_block"] += 1
    return a, dict(cnt)


def x12_c(seen_writers, conf_seen):
    rows, summ = C.h1_run(seen_writers, C.MAIN, SEED, hide=True, light=False)
    gate = {s: summ["H1"][s]["weights"] == conf_seen["H1"][s]["weights"] and
            summ["H1"][s]["eps_0.001"]["pairs"] == conf_seen["H1"][s]["eps_0.001"]["pairs"] for s in C.SIGS}
    p = np.array([C.h1_pairs(r, "d_p_stuck")[1] for r in rows], dtype=float)
    p2 = np.array([C.h1_pairs(r, "d_ewma_x")[1] for r in rows], dtype=float)
    out = {"gates": gate, "pairs_same_both_signals": bool((p == p2).all()),
           "full_seen_effective_writers": float(p.sum() ** 2 / (p ** 2).sum())}
    n_hold = round(146 * 319 / 338)
    for how, rep in (("without_replacement", False), ("with_replacement", True)):
        rng = np.random.default_rng(SEED + 8)
        eff, top = [], []
        for _ in range(500):
            sel = rng.choice(len(rows), n_hold, replace=rep)
            rng.integers(1 << 31)
            ps = p[sel]
            eff.append(ps.sum() ** 2 / (ps ** 2).sum())
            srt = np.sort(ps[ps > 0])[::-1]
            k = max(1, int(np.ceil(0.1 * len(srt))))
            top.append(srt[:k].sum() / srt.sum())
        out[how] = {"effective_writers": q(eff, (0.025, 0.1, 0.5, 0.9, 0.975)),
                    "top10pct_share": q(top, (0.1, 0.5, 0.9))}
    out["n_hold"] = n_hold
    return out


def main():
    hw, sw, hold_p, conf_h_p, conf_s_p, out11, out12 = sys.argv[1:]
    conf_h, conf_s = json.load(open(conf_h_p)), json.load(open(conf_s_p))
    C.pause_info = pause_info_u
    C.stall_anchors = anchors_rec
    writers, chk = C.load_cs1(hw, hold_p, "holdout")
    cnt = Counter()
    rng = random.Random(SEED)
    rows = [C.h1_writer(u, rng, cnt, C.MAIN, feats=True) for u in writers]
    gates = {"counts_equal_confirm": dict(cnt) == conf_h["h1_counts"]}
    for s in SIG4:
        wpt = [C.h1_pairs(r, s) for r in rows]
        a = sum(x[0] for x in wpt) / sum(x[1] for x in wpt)
        gates["h1_%s_eps0.001_equals_confirm" % s] = abs(a - conf_h["H1"][s]["eps_0.001"]["pooled_auc"]) <= TOL
    if not all(gates.values()):
        sys.exit("門が外れた: " + json.dumps(gates))
    a, b = x12_ab(writers, rows)
    if a["stall_rows"] != 4554:
        sys.exit("停滞の行が 4,554 でない")
    C.pause_info = ORIG_PAUSE_INFO
    C.stall_anchors = ORIG_ANCHORS
    seen_writers, _ = C.load_cs1(sw, hold_p, "seen")
    c = x12_c(seen_writers, conf_s)
    if not all(c["gates"].values()):
        sys.exit("X12 (c) の門が外れた")
    r12 = {"checks_load_holdout": chk, "gates_holdout": gates, "a_length": a, "b_observation_edge": b,
           "b_confirm_stall_warm": conf_h["h1_counts"]["stall_warm"], "c_calibration_effective_writers": c}
    json.dump(r12, open(out12, "w"), indent=1)
    r11 = x11(writers, rows, conf_h)
    r11["gates_holdout"] = gates
    json.dump(r11, open(out11, "w"), indent=1)
    if not all(r11["gates"].values()):
        sys.exit("X11 の門が外れた: " + json.dumps(r11["gates"]))
    print(json.dumps({"x11_gates": r11["gates"], "x12_gates": c["gates"]}))


if __name__ == "__main__":
    main()
