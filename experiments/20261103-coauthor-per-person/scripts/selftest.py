"""陽性対照（conditions.md の Q7）。P-D1 は check_same.py、P-D2 は perperson.py --pd2 が受け持つ。

ここで受け持つ: P-EM1・P-EM2・P-EM3・P-EM4・P-L1・P-L2・P-S。合成のデータだけを使い、実データを読まない。
返り値: 0 = すべて通った、3 = どれかが赤。--only で対照を絞る（変異の確かめ用）。
"""
import json
import os
import sys
from concurrent.futures import ProcessPoolExecutor
from itertools import permutations

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hmm  # noqa: E402
import perperson as pp  # noqa: E402

L = 650
DECREASES = []


def _set_mut(mut):
    hmm.MUT.clear(); hmm.MUT.update(mut)
    pp.MUT.clear(); pp.MUT.update(mut)


def persistent_A():
    A = hmm.A0.copy()
    for i in range(3):
        off = A[i].copy(); off[i] = 0
        A[i] = off / off.sum() * 0.02
        A[i, i] = 0.98
    return A


def align_err(est, true):
    (pi_e, A_e, B_e), (pi_t, A_t, B_t) = est, true
    best = None
    for perm in permutations(range(3)):
        p = list(perm)
        d = np.abs(B_e[p] - B_t).sum()
        if best is None or d < best[0]:
            best = (d, p)
    p = best[1]
    A_al = A_e[np.ix_(p, p)]
    return float(max(np.abs(A_al - A_t).max(), np.abs(B_e[p] - B_t).max()))


def _em1_job(args):
    which, k, rep, mut = args
    _set_mut(mut)
    A = hmm.A0 if which == "a" else persistent_A()
    true = (hmm.PI0, A, hmm.B0)
    rng = np.random.default_rng([7, ord(which), k, rep])
    seqs = hmm.sample(*true, [L] * k, rng)
    est, diag = hmm.fit_em(seqs, rng)
    return which, k, align_err(est, true), diag["decreases"]


def p_em1(pool, mut):
    jobs = [(w, k, r, mut) for w in "ab" for k in (1, 5, 15) for r in range(10)]
    res = list(pool.map(_em1_job, jobs))
    out, ok = {}, True
    for w in "ab":
        med = {k: float(np.median([e for ww, kk, e, _ in res if ww == w and kk == k])) for k in (1, 5, 15)}
        good = med[15] < 0.05 and med[1] > med[15]
        out[w] = dict(median_max_err=med, passed=good)
        ok &= good
    DECREASES.extend(d for *_, d in res)
    return dict(models=out, passed=ok)


def p_em3(mut):
    _set_mut(mut)
    rng = np.random.default_rng(11)
    seqs = hmm.sample(hmm.PI0, hmm.A0, hmm.B0, [200] * 4, rng)
    O, M = hmm.pad(seqs)
    worst = {}
    alpha, _ = hmm.forward(hmm.PI0, hmm.A0, hmm.B0, O, M)
    for name, (pi, A, B) in dict(fa=(hmm.PI0, hmm.A0, hmm.B0),
                                 em=(rng.dirichlet(np.ones(3)), rng.dirichlet(np.ones(3), 3), rng.dirichlet(np.ones(26), 3))).items():
        al, _ = hmm.forward(pi, A, B, O, M)
        d = hmm.pred_dist(pi, A, B, al[M])
        worst[name] = float(np.abs(d.sum(1) - 1).max())
    _, _, sums = hmm.fb_engine(O, M)
    worst["fb"] = float(np.abs(sums - 1).max())
    worst["r0"] = float(abs(hmm.unigram(seqs).sum() - 1))
    worst["r1"] = float(np.abs(hmm.markov1(seqs).sum(1) - 1).max())
    return dict(worst=worst, passed=all(v <= 1e-9 for v in worst.values()))


def _em4_writer(args):
    """合成の書き手 1 人: 共通は他人の全セッション、本人は split() の k 本。"""
    w, params, seqs_all, ks, mut = args
    _set_mut(mut)
    rng = np.random.default_rng([44, w])
    others = [s for j, ss in enumerate(seqs_all) if j != w for s in ss]
    pooled, d0 = hmm.fit_em(others, rng)
    mine = seqs_all[w]
    _, test = pp.split(len(mine), 1)
    te = [mine[i] for i in test]
    true_ll = hmm.ll_honest(*params[w], te)
    out, decs = {}, [d0["decreases"]]
    for k in ks:
        tr, _ = pp.split(len(mine), k)
        est, d = hmm.fit_em([mine[i] for i in tr], rng)
        decs.append(d["decreases"])
        s = hmm.ll_hmm(*est, te)
        p = hmm.ll_hmm(*pooled, te)
        out[k] = dict(self_minus_pooled=(s[0] - p[0]) / s[1], self_minus_true=(s[0] - true_ll[0]) / s[1])
    return out, decs


def p_em4(pool, mut):
    ks = (1, 5, 15)
    res = {}
    for scen in ("i", "ii"):
        rng = np.random.default_rng(4400 + (scen == "ii"))
        params = []
        for _ in range(17):
            if scen == "i":
                A = hmm.A0.copy()
                for r in range(3):
                    dg = np.clip(A[r, r] + rng.uniform(-0.1, 0.15), 0.05, 0.98)
                    off = A[r].copy(); off[r] = 0
                    A[r] = off / off.sum() * (1 - dg); A[r, r] = dg
                B = np.array([rng.dirichlet(50 * hmm.B0[r]) for r in range(3)])
                B = np.clip(B, 1e-6, None); B /= B.sum(1, keepdims=True)
                params.append((hmm.PI0, A, B))
            else:
                params.append((hmm.PI0, hmm.A0, hmm.B0))
        seqs_all = [hmm.sample(*params[w], [L] * 20, np.random.default_rng([45, w, scen == "ii"])) for w in range(17)]
        out = list(pool.map(_em4_writer, [(w, params, seqs_all, ks, mut) for w in range(17)]))
        DECREASES.extend(d for _, ds in out for d in ds)
        res[scen] = {k: dict(self_minus_pooled_median=float(np.median([o[k]["self_minus_pooled"] for o, _ in out])),
                             self_minus_true_median=float(np.median([o[k]["self_minus_true"] for o, _ in out]))) for k in ks}
    leak_ok = all(res[s][k]["self_minus_true_median"] <= 0.002 for s in res for k in ks)
    ok_i = res["i"][15]["self_minus_pooled_median"] > 0
    ok_ii = res["ii"][15]["self_minus_pooled_median"] <= 0.002
    return dict(result={s: {str(k): v for k, v in r.items()} for s, r in res.items()}, leak_ok=leak_ok,
                i_ok=ok_i, ii_ok=ok_ii, passed=leak_ok and ok_i and ok_ii)


def p_l1(mut):
    _set_mut(mut)
    rng = np.random.default_rng(31)
    X = rng.normal(size=(20000, 3))
    beta = np.array([-0.3, 0.8, -0.5, 0.2])
    y = (rng.random(20000) < pp.sigmoid(beta[0] + X @ beta[1:])).astype(float)
    b, _ = pp.fit_logistic(X, y)
    rec = bool(np.all(np.abs(b - beta) <= 0.05))
    b1, _ = pp.fit_logistic(X[:10], np.ones(10), lam=1.0, pseudo=True)
    xs = np.array([[-1.0], [-0.5], [0.5], [1.0]])
    b2, _ = pp.fit_logistic(xs, np.array([0.0, 0.0, 1.0, 1.0]), lam=1.0, pseudo=True)
    fin = bool(np.all(np.isfinite(b1)) and np.all(np.isfinite(b2)))
    return dict(recovery=rec, finite=fin, passed=rec and fin)


def _synth_writers(rng, hetero):
    Ws = []
    common = rng.normal(0, 1, 5)
    for _ in range(17):
        beta = rng.normal(0, 1, 5) if hetero else common
        b0 = rng.normal(-1.5, 0.5) if hetero else -1.5  # 基準率を 0.5 から離す（変異 m5 が当たるように・判断 9）
        sess = []
        for _ in range(20):
            X = rng.normal(size=(2, 5))
            p = pp.sigmoid(b0 + X @ beta)
            sess.append((X, p))
        Ws.append(sess)
    return Ws


def _cov_rep(args):
    rep, mut = args
    _set_mut(mut)
    pp.B_BOOT = 500
    rng = np.random.default_rng([52, rep])
    Ws = _synth_writers(rng, hetero=True)
    ys, ps = [], []
    for sess in Ws:
        te = sess[-pp.H:]
        p = np.concatenate([s[1] for s in te])
        ys.append((rng.random(len(p)) < p).astype(float))
        ps.append(p)
    ci, _ = pp.boot_pooled(ys, ps, np.random.default_rng([53, rep]))
    f = lambda c, v: bool(c is not None and c[0] <= v <= c[1])  # noqa: E731
    gi, gs = ci["cal_intercept"], ci["cal_slope"]
    return (f(gi and gi["pct"], 0), f(gi and gi["normal"], 0), f(gs and gs["pct"], 1), f(gs and gs["normal"], 1))


def _self_vs_pooled(args):
    """合成 1 回: k = 15 の本人 V-tab と共通の Brier の差（本人 − 共通）の書き手の平均。"""
    seed, hetero, mut = args
    _set_mut(mut)
    rng = np.random.default_rng(seed)
    Ws = _synth_writers(rng, hetero)
    labs = [[(X, (rng.random(len(p)) < p).astype(float)) for X, p in sess] for sess in Ws]
    d = []
    for w in range(17):
        tr, te = pp.split(20, 15)
        X = np.concatenate([labs[w][i][0] for i in tr]); y = np.concatenate([labs[w][i][1] for i in tr])
        Xt = np.concatenate([labs[w][i][0] for i in te]); yt = np.concatenate([labs[w][i][1] for i in te])
        Xo = np.concatenate([labs[j][i][0] for j in range(17) if j != w for i in range(20)])
        yo = np.concatenate([labs[j][i][1] for j in range(17) if j != w for i in range(20)])
        d.append(pp.losses(yt, pp.Model(X, X, y)(Xt))[0] - pp.losses(yt, pp.Model(Xo, Xo, yo)(Xt))[0])
    return float(np.mean(d)), float(np.mean(np.array(d) < 0))


def p_l2(pool, mut):
    """第 2 版（conditions.md Q7 の注）: 被覆は結果、門は粗い下限 0.80。見分けは 20 回の平均の差。"""
    cov = np.array(list(pool.map(_cov_rep, [(r, mut) for r in range(200)])), dtype=float).mean(0)
    cov_ok = cov[0] >= 0.80 and cov[2] >= 0.80
    di = list(pool.map(_self_vs_pooled, [(1000 + r, True, mut) for r in range(20)]))
    dii = list(pool.map(_self_vs_pooled, [(2000 + r, False, mut) for r in range(20)]))
    mi, mii = float(np.mean([x[0] for x in di])), float(np.mean([x[0] for x in dii]))
    ok = bool(cov_ok and mi < 0 and mii > 0)
    return dict(coverage=dict(intercept_pct=cov[0], intercept_normal=cov[1], slope_pct=cov[2], slope_normal=cov[3]),
                coverage_gate_ok=bool(cov_ok), mean_dbrier_i=mi, mean_dbrier_ii=mii,
                self_better_share_i=float(np.mean([x[1] for x in di])), self_better_share_ii=float(np.mean([x[1] for x in dii])),
                passed=ok)


def p_s(mut):
    _set_mut(mut)
    ok = all(pp.check_split(n, k) for n in range(6, 90) for k in pp.GRID if k <= n - pp.H)
    return dict(passed=ok)


def main(work, seed, out, only, mut):
    _set_mut(mut)
    names = only or ["P-EM1", "P-EM3", "P-EM4", "P-L1", "P-L2", "P-S"]
    R = dict(mutate=mut)
    with ProcessPoolExecutor(10) as pool:
        for n in names:
            if n == "P-EM1":
                R[n] = p_em1(pool, mut)
            elif n == "P-EM3":
                R[n] = p_em3(mut)
            elif n == "P-EM4":
                R[n] = p_em4(pool, mut)
            elif n == "P-L1":
                R[n] = p_l1(mut)
            elif n == "P-L2":
                R[n] = p_l2(pool, mut)
            elif n == "P-S":
                R[n] = p_s(mut)
            print(n, json.dumps(R[n]), flush=True)
    if DECREASES:
        R["P-EM2"] = dict(decreases=int(sum(DECREASES)), fits=len(DECREASES), passed=sum(DECREASES) == 0)
    json.dump(R, open(out, "w"), indent=1)
    return 0 if all(v.get("passed", True) for k, v in R.items() if isinstance(v, dict)) else 3
