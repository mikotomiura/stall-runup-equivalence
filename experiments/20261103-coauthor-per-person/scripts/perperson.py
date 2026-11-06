"""CoAuthor の 1 人ごとの学習の学習曲線（conditions.md の Q2〜Q8）。

使い方:
  python3 -I perperson.py --pd2 <work_obs> <out.json>                    P-D2（engine の前向き＋表示の層の再現）
  python3 -I perperson.py --selftest <work_obs> <seed> <out.json> [--mutate mX] [--only NAME,...]
  python3 -I perperson.py <work_obs> <seed> <out_dir> <jobs>             本計算
出力は集計値だけ。書き手・セッションの番号を書かない。代理ラベルは詰まりの推定ではない（conditions.md「読まないこと」）。
"""
import glob
import json
import math
import os
import sys
from concurrent.futures import ProcessPoolExecutor

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hmm  # noqa: E402

WIN_START, WIN_END = 31_000, 1_000
LABEL_MS = 60_000
GAP_MS = 6 * 60_000
MAX_PROBES = 3
H = 5
GRID = [1, 2, 3, 5, 8, 12, 15, 20, 30]
P15_K = [1, 2, 3, 5, 8, 12, 15]
M_GRID = [1, 2, 3, 5, 8, 15]
LAM = 1.0
B_BOOT = 2000
MUT = set()


# ---------------------------------------------------------------- Q1・Q2 データ
def load(work):
    s2w = json.load(open(os.path.join(work, "sessions.json")))
    by_w = {}
    for f in sorted(glob.glob(os.path.join(work, "s[0-9]*.json"))):
        sid = os.path.basename(f)[1:-5]
        if sid not in s2w:
            continue
        d = json.load(open(f))
        if len(d["observed_intervals"]) != 1:
            continue
        a, b = d["observed_intervals"][0]
        F = np.array([r[:7] for r in d["feat"]], dtype=np.float64)
        alpha = np.array([r[7] for r in d["feat"]], dtype=np.float64)
        by_w.setdefault(s2w[sid], []).append(dict(
            sid=int(sid), a=a, b=b,
            keys=np.sort(np.array([k[0] for k in d["keys"]], dtype=np.int64)),
            bs=np.sort(np.array([k[0] for k in d["keys"] if k[1] == "backspace"], dtype=np.int64)),
            ft=F[:, 0], pf=F[:, 1], pi=F[:, 2], ps=F[:, 3], ex=F[:, 4], obs=F[:, 6].astype(np.int64), alpha=alpha,
            gets=np.sort(np.array([e[0] for e in d["events"] if e[1] == "suggestion-get"], dtype=np.int64))))
    return [sorted(by_w[w], key=lambda s: s["a"]) for w in sorted(by_w)]


# ---------------------------------------------------------------- Q5 問い・特徴（前の空回しの K3〜K5 を写した）
def draw_probes(rng, a, b):
    allowed = [(a + 31_000, b - LABEL_MS)]
    allowed = [(x, y) for x, y in allowed if y > x]
    out = []
    while allowed and len(out) < MAX_PROBES:
        lens = np.array([y - x for x, y in allowed], dtype=np.float64)
        u = rng.uniform(0, lens.sum())
        i = min(int(np.searchsorted(np.cumsum(lens), u, side="right")), len(allowed) - 1)
        x, y = allowed[i]
        t = min(max(int(x + (u - (lens[:i].sum()))), x), y)
        out.append(t)
        lo, hi = t - GAP_MS, t + GAP_MS
        nxt = []
        for x, y in allowed:
            if y <= lo or x >= hi:
                nxt.append((x, y))
            else:
                if x < lo:
                    nxt.append((x, lo))
                if y > hi:
                    nxt.append((hi, y))
        allowed = [(x, y) for x, y in nxt if y > x]
    return sorted(out)


def row_le(s, t):
    """t 以前の最後の feat 行の添字（無ければ 0）。"""
    return max(int(np.searchsorted(s["ft"], t, side="right")) - 1, 0)


def logit(p):
    p = np.clip(p, 1e-6, 1 - 1e-6)
    return np.log(p / (1 - p))


def tab_features(s, t):
    w0, w1 = t - WIN_START, t - WIN_END
    k = s["keys"][(s["keys"] >= w0) & (s["keys"] < w1)]
    nb = int(((s["bs"] >= w0) & (s["bs"] < w1)).sum())
    gap = float(np.diff(k).max()) / 1000.0 if len(k) >= 2 else 30.0
    j = row_le(s, w1)
    return [float(len(k)), float(nb), gap, float(s["ps"][j]), float(s["ex"][j])], j


def probes_of(s, seed):
    rng = np.random.default_rng([seed, s["sid"]])
    out = []
    for t in draw_probes(rng, s["a"], s["b"]):
        tab, j = tab_features(s, t)
        y = int(((s["gets"] > t) & (s["gets"] <= t + LABEL_MS)).any())
        out.append(dict(t=t, row=j, tab=tab, recal=float(logit(np.array([s["ps"][j]]))[0]), y=y))
    return out


def grid_points(s):
    """標準化のためのラベル無しの点: 5 秒ごとの時刻 t（最初の feat 行 + 1 秒・観測区間の始まり + 31 秒以降）で、問いと同じく t − 1 秒以前の行を使う。"""
    pts = []
    t0 = int(s["ft"][0]) + WIN_END
    for t in range(max(t0, s["a"] + 31_000), int(s["ft"][-1]) + WIN_END + 1, 5_000):
        tab, j = tab_features(s, t)
        pts.append(dict(row=j, tab=tab, recal=float(logit(np.array([s["ps"][j]]))[0])))
    return pts


# ---------------------------------------------------------------- ロジスティック
def sigmoid(x):
    return 1 / (1 + np.exp(-np.clip(x, -35, 35)))


def fit_logistic(X, y, lam=0.0, offset=None, w=None, pseudo=False):
    """切片つき・切片は罰しない・Newton 法（許容 1e-8・最大 100 回）。pseudo は特徴 0 の y=1 と y=0 を重み 0.5 ずつ足す。"""
    X = np.zeros((len(y), 0)) if X is None else np.asarray(X, dtype=float)
    y = np.asarray(y, dtype=float)
    w = np.ones(len(y)) if w is None else np.asarray(w, dtype=float)
    off = np.zeros(len(y)) if offset is None else np.asarray(offset, dtype=float)
    if pseudo:
        X = np.vstack([X, np.zeros((2, X.shape[1]))])
        y = np.concatenate([y, [1.0, 0.0]])
        w = np.concatenate([w, [0.5, 0.5]])
        off = np.concatenate([off, [0.0, 0.0]])
    Xd = np.hstack([np.ones((len(y), 1)), X])
    beta = np.zeros(Xd.shape[1])
    R = np.eye(Xd.shape[1]) * lam
    R[0, 0] = 0.0
    conv = False
    for _ in range(100):
        p = sigmoid(Xd @ beta + off)
        g = Xd.T @ (w * (y - p)) - R @ beta
        Hs = (Xd * (w * p * (1 - p))[:, None]).T @ Xd + R
        step = np.linalg.solve(Hs + 1e-12 * np.eye(len(beta)), g)
        beta = beta + step
        if np.max(np.abs(step)) < 1e-8:
            conv = True
            break
    return beta, conv


def predict(beta, X):
    X = np.asarray(X, dtype=float)
    return sigmoid(beta[0] + X @ beta[1:])


def calib(y, p):
    lp = logit(np.asarray(p))
    y = np.asarray(y, dtype=float)
    if np.var(lp) < 1e-12:
        slope = None
    elif "m4" in MUT:
        slope = float(fit_logistic(lp[:, None], (np.asarray(p) > 0.5).astype(float))[0][1])
    else:
        slope = float(fit_logistic(lp[:, None], y)[0][1])
    off = None if "m5" in MUT else lp
    inter = float(fit_logistic(None, y, offset=off)[0][0])
    return inter, slope


def auc(y, p):
    y, p = np.asarray(y), np.asarray(p)
    pos, neg = p[y == 1], p[y == 0]
    if len(pos) == 0 or len(neg) == 0:
        return None
    allv = np.concatenate([pos, neg])
    order = np.argsort(allv, kind="mergesort")
    sv = allv[order]
    ranks = np.empty(len(sv))
    i = 0
    while i < len(sv):
        j = i
        while j + 1 < len(sv) and sv[j + 1] == sv[i]:
            j += 1
        ranks[i:j + 1] = (i + j) / 2 + 1
        i = j + 1
    r = np.empty(len(sv))
    r[order] = ranks
    return float((r[:len(pos)].sum() - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg)))


def losses(y, p):
    y, p = np.asarray(y, dtype=float), np.clip(np.asarray(p, dtype=float), 1e-6, 1 - 1e-6)
    return float(np.mean((p - y) ** 2)), float(-np.mean(y * np.log(p) + (1 - y) * np.log(1 - p)))


class Model:
    """標準化（平均・SD）＋ L2 ロジスティック（擬似観測つき）。"""

    def __init__(self, Xstd, X, y):
        X = np.asarray(X, dtype=float)
        Xstd = np.asarray(Xstd, dtype=float)
        self.mu = Xstd.mean(0)
        sd = Xstd.std(0)
        sd[sd == 0] = 1.0
        self.sd = sd
        self.one_class = len(set(y)) < 2
        self.beta, self.conv = fit_logistic((X - self.mu) / self.sd, y, lam=LAM, pseudo=True)
        self.sep = bool(np.any(np.abs(self.beta[1:]) > 10))

    def __call__(self, X):
        return predict(self.beta, (np.asarray(X, dtype=float) - self.mu) / self.sd)


# ---------------------------------------------------------------- EM の入力
def seqs_of(ss):
    return [s["obs"] for s in ss]


def em_feats(params, ss, rows_per_session):
    """因果の前向きのフィルタを指定の行で取り、S1〜S3 の加法対数比 2 つにする。"""
    pi, A, B = params
    O, M = hmm.pad(seqs_of(ss))
    alpha, _ = hmm.forward(pi, A, B, O, M)
    o = hmm.order_states(B)
    out = []
    for i, rows in enumerate(rows_per_session):
        f = np.clip(alpha[i, rows][:, o], 1e-6, None)
        out.append(np.stack([np.log(f[:, 0] / f[:, 2]), np.log(f[:, 1] / f[:, 2])], 1))
    return out


def version_X(items, ver, emx=None):
    if ver == "tab":
        return np.array([p["tab"] for p in items])
    if ver == "recal":
        return np.array([[p["recal"]] for p in items])
    if ver == "em":
        return emx
    raise ValueError(ver)


# ---------------------------------------------------------------- Q2 分け方
def split(n, k):
    """評価 = 後ろの H 本、学習 = 池の後ろの k 本。m6 は学習に評価の 1 本を混ぜる変異。"""
    test = list(range(n - H, n))
    train = list(range(n - H - k, n - H))
    if "m6" in MUT:
        train = train[1:] + [test[0]]
    return train, test


def check_split(n, k):
    tr, te = split(n, k)
    return len(set(tr) & set(te)) == 0 and min(te) > max(tr) and len(tr) == k and len(te) == H


# ---------------------------------------------------------------- 1 人ぶんの計算
_W = None


def _init(work, mut):
    global _W
    _W = load(work)
    hmm.MUT.clear(); hmm.MUT.update(mut)
    MUT.clear(); MUT.update(mut)


def eval_writer(job):
    """job: dict(i=書き手の添字, seed, wseq, ks, m_grid)。本人と他人のセッションはワーカーが読んだ _W から取る。返り値は数だけ。"""
    if _W is not None:
        job = dict(job, ss=_W[job["i"]], others=[s for j, ss in enumerate(_W) if j != job["i"] for s in ss])
    ss, others, seed = job["ss"], job["others"], job["seed"]
    n = len(ss)
    rng = np.random.default_rng([seed + 10, job["wseq"]])
    # 共通の EM（他人の全セッション）
    pooled, pdiag = hmm.fit_em(seqs_of(others), rng)
    # 問い
    P_self = [probes_of(s, seed) for s in ss]
    P_oth = [probes_of(s, seed) for s in others]
    oth_items = [p for ps in P_oth for p in ps]
    y_oth = [p["y"] for p in oth_items]
    base_pool = float(np.mean(y_oth))
    # 共通のロジスティック（他人の問いで標準化・学習）
    oth_em = em_feats(pooled, others, [[p["row"] for p in ps] for ps in P_oth])
    oth_emx = np.concatenate([x for x in oth_em if len(x)]) if any(len(x) for x in oth_em) else np.zeros((0, 2))
    pooled_models = {}
    for ver in ("tab", "recal", "em"):
        X = version_X(oth_items, ver, oth_emx)
        pooled_models[ver] = Model(X, X, y_oth)
    _, test = split(n, 1)
    te_ss = [ss[i] for i in test]
    te_items = [p for i in test for p in P_self[i]]
    y_te = [p["y"] for p in te_items]
    te_rows = [[p["row"] for p in P_self[i]] for i in test]
    pooled_te_em = np.concatenate([x for x in em_feats(pooled, te_ss, te_rows) if len(x)]) if te_items else np.zeros((0, 2))
    pooled_pred = {ver: pooled_models[ver](version_X(te_items, ver, pooled_te_em)) if te_items else np.zeros(0)
                   for ver in ("tab", "recal", "em")}
    # 尤度の相手（k によらない）
    te_seqs = seqs_of(te_ss)
    ll_pool = hmm.ll_hmm(*pooled, te_seqs)
    ll_fa = hmm.ll_hmm(hmm.PI0, hmm.A0, hmm.B0, te_seqs)
    O, M = hmm.pad(te_seqs)
    lp_fb, _, _ = hmm.fb_engine(O, M)
    ll_fb = (float(lp_fb.sum()), int(M.sum()))
    per = lambda x: x[0] / x[1]  # noqa: E731
    pflags = {v: dict(one_class=m.one_class, sep=m.sep, nonconv=not m.conv) for v, m in pooled_models.items()}
    out = dict(n=n, pooled_diag=pdiag, pooled_flags=pflags, test_labels=len(y_te), test_pos=int(sum(y_te)), base_pool=base_pool,
               ll=dict(pooled=per(ll_pool), fa=per(ll_fa), fb=per(ll_fb)), k={}, m={},
               pooled_pred={v: pooled_pred[v].tolist() for v in pooled_pred}, y_test=y_te)
    fits = {}
    for k in job["ks"]:
        train, test2 = split(n, k)
        assert test2 == test
        tr_ss = [ss[i] for i in train]
        params, diag = hmm.fit_em(seqs_of(tr_ss), rng)
        fits[k] = (params, tr_ss)
        r0 = hmm.unigram(seqs_of(tr_ss))
        r1 = hmm.markov1(seqs_of(tr_ss))
        rec = dict(em_diag=diag, ll=dict(self=per(hmm.ll_hmm(*params, te_seqs)), r0=per(hmm.ll_r0(r0, te_seqs)),
                                         r1=per(hmm.ll_r1(r0, r1, te_seqs))))
        rec.update(logistic_block(params, tr_ss, [P_self[i] for i in train], te_ss, te_items, te_rows))
        out["k"][str(k)] = rec
    if job.get("m_grid") and 15 in fits:
        params, em_ss = fits[15]
        for m in M_GRID:
            train, _ = split(n, m)
            lab_ss = [ss[i] for i in train]
            out["m"][str(m)] = logistic_block(params, lab_ss, [P_self[i] for i in train], te_ss, te_items, te_rows,
                                              std_ss=em_ss, only=("em",))
    return out


def logistic_block(params, lab_ss, lab_probes, te_ss, te_items, te_rows, std_ss=None, only=("tab", "recal", "em")):
    """本人の学習（ラベル lab_ss の問い・標準化 std_ss のラベル無しの点）→ 評価の問いの予測。"""
    std_ss = lab_ss if std_ss is None else std_ss
    items = [p for ps in lab_probes for p in ps]
    y = [p["y"] for p in items]
    gp = [grid_points(s) for s in std_ss]
    g_items = [p for ps in gp for p in ps]
    res = dict(train_labels=len(y), train_pos=int(sum(y)), base_self=(sum(y) + 0.5) / (len(y) + 1), pred={}, flags={})
    for ver in only:
        if ver == "em":
            g_em = np.concatenate(em_feats(params, std_ss, [[p["row"] for p in ps] for ps in gp]))
            l_em = em_feats(params, lab_ss, [[p["row"] for p in ps] for ps in lab_probes])
            l_em = np.concatenate([x for x in l_em if len(x)]) if items else np.zeros((0, 2))
            t_em = np.concatenate([x for x in em_feats(params, te_ss, te_rows) if len(x)]) if te_items else np.zeros((0, 2))
            Xs, X, Xt = g_em, l_em, t_em
        else:
            Xs, X, Xt = version_X(g_items, ver), version_X(items, ver), version_X(te_items, ver)
        if len(y) == 0:
            X = np.zeros((0, Xs.shape[1]))
        mdl = Model(Xs, X, y)
        res["pred"][ver] = mdl(Xt).tolist() if te_items else []
        res["flags"][ver] = dict(one_class=mdl.one_class, sep=mdl.sep, nonconv=not mdl.conv)
    return res


# ---------------------------------------------------------------- Q6 集計
def q(xs):
    xs = [x for x in xs if x is not None]
    if not xs:
        return None
    a = np.array(xs, dtype=float)
    return dict(median=float(np.median(a)), q25=float(np.quantile(a, 0.25)), q75=float(np.quantile(a, 0.75)), n=len(xs))


def boot_median(vals, rng):
    a = np.array(vals, dtype=float)
    if len(a) < 2:
        return None
    meds = np.array([np.median(a[rng.integers(0, len(a), len(a))]) for _ in range(B_BOOT)])
    m = float(np.median(a))
    se = float(meds.std())
    return dict(pct=[float(np.quantile(meds, 0.025)), float(np.quantile(meds, 0.975))], normal=[m - 1.96 * se, m + 1.96 * se])


def pooled_metrics(ys, ps):
    y = np.concatenate([np.asarray(v, dtype=float) for v in ys]) if ys else np.zeros(0)
    p = np.concatenate([np.asarray(v, dtype=float) for v in ps]) if ps else np.zeros(0)
    if len(y) == 0:
        return None
    inter, slope = calib(y, p)
    return dict(brier=float(np.mean((p - y) ** 2)), auc=auc(y, p), cal_intercept=inter, cal_slope=slope, n=int(len(y)))


def boot_pooled(ys, ps, rng):
    keys = ("brier", "auc", "cal_intercept", "cal_slope")
    acc, dropped = {k: [] for k in keys}, 0
    W = len(ys)
    for _ in range(B_BOOT):
        idx = rng.integers(0, W, W)
        yy = [ys[i] for i in idx]
        if len(set(np.concatenate([np.asarray(v) for v in yy]).tolist())) < 2:
            dropped += 1
            continue
        m = pooled_metrics(yy, [ps[i] for i in idx])
        for k in keys:
            if m[k] is not None:
                acc[k].append(m[k])
    pt = pooled_metrics(ys, ps)
    out = {}
    for k, v in acc.items():
        if not v:
            out[k] = None
            continue
        sd = float(np.std(v))
        c = pt[k] if pt and pt[k] is not None else float(np.median(v))
        out[k] = dict(pct=[float(np.quantile(v, 0.025)), float(np.quantile(v, 0.975))], normal=[c - 1.96 * sd, c + 1.96 * sd])
    return out, dropped


def crossing(ks, deltas, better):
    """Δ が良い側になり、格子の残りでも続く最小の k（無ければ None＝打ち切り）。"""
    for i, k in enumerate(ks):
        if all(better(d) for d in deltas[i:]):
            return k
    return None


def summarize(results, panels, rng, with_crossing=True):
    S = {}
    comps_ll = ("pooled", "fb", "fa", "r0", "r1")
    for pname, (wids, ks) in panels.items():
        P = {"writers": len(wids), "sessions_per_writer": q([results[w]["n"] for w in wids]), "k": {}}
        for k in ks:
            rows = [results[w]["k"][str(k)] for w in wids if str(k) in results[w]["k"]]
            ws = [w for w in wids if str(k) in results[w]["k"]]
            if not rows:
                continue
            e = dict(writers=len(rows), train_labels=q([r["train_labels"] for r in rows]),
                     train_pos=q([r["train_pos"] for r in rows]), ll={}, loss={}, pooled={}, flags={}, em_diag={})
            for c in comps_ll:
                d = [r["ll"]["self"] - (r["ll"][c] if c in ("r0", "r1") else results[w]["ll"][c]) for r, w in zip(rows, ws)]
                e["ll"]["self_minus_" + c] = dict(q(d), better_share=float(np.mean([x > 0 for x in d])), ci=boot_median(d, rng))
            for ver in ("tab", "em", "recal"):
                for comp in ("pooled_same", "b0_pool", "b0_self"):
                    d_b, d_l = [], []
                    for r, w in zip(rows, ws):
                        y = results[w]["y_test"]
                        if not y:
                            continue
                        p_self = r["pred"][ver]
                        if comp == "pooled_same":
                            pc = results[w]["pooled_pred"][ver]
                        elif comp == "b0_pool":
                            pc = [results[w]["base_pool"]] * len(y)
                        else:
                            pc = [r["base_self"]] * len(y)
                        bs, ls = losses(y, p_self)
                        bc, lc = losses(y, pc)
                        d_b.append(bs - bc)
                        d_l.append(ls - lc)
                    e["loss"][f"{ver}_minus_{comp}"] = dict(brier=dict(q(d_b), better_share=float(np.mean([x < 0 for x in d_b])),
                                                                       ci=boot_median(d_b, rng)),
                                                            logloss=q(d_l))
                ys = [results[w]["y_test"] for w in ws if results[w]["y_test"]]
                ps = [r["pred"][ver] for r, w in zip(rows, ws) if results[w]["y_test"]]
                m = pooled_metrics(ys, ps)
                ci, dropped = boot_pooled(ys, ps, rng)
                e["pooled"][ver] = dict(self=m, self_ci=ci, dropped=dropped)
                e["flags"][ver] = {f: int(sum(r["flags"][ver][f] for r in rows)) for f in ("one_class", "sep", "nonconv")}
            for ver in ("tab", "em", "recal"):
                ys = [results[w]["y_test"] for w in ws if results[w]["y_test"]]
                ps = [results[w]["pooled_pred"][ver] for w in ws if results[w]["y_test"]]
                e["pooled"][ver]["pooled_model"] = pooled_metrics(ys, ps)
            e["em_diag"] = dict(winner_frozen_share=float(np.mean([r["em_diag"]["winner_frozen"] for r in rows])),
                                gap12=q([r["em_diag"]["gap12"] for r in rows]),
                                nonconv_starts=int(sum(r["em_diag"]["nonconv"] for r in rows)),
                                decreases=int(sum(r["em_diag"]["decreases"] for r in rows)))
            P["k"][str(k)] = e
        if not with_crossing:
            S[pname] = P
            continue
        # 越える k
        cr = {}
        targets = [("ll", "self_minus_pooled", lambda d: d > 0), ("brier", "tab_minus_pooled_same", lambda d: d < 0),
                   ("brier", "em_minus_pooled_same", lambda d: d < 0), ("brier", "tab_minus_b0_pool", lambda d: d < 0),
                   ("brier", "em_minus_b0_pool", lambda d: d < 0)]
        for kind, name, better in targets:
            vals, labs = [], []
            for w in wids:
                dk = []
                for k in ks:
                    r = results[w]["k"][str(k)]
                    if kind == "ll":
                        dk.append(r["ll"]["self"] - results[w]["ll"]["pooled"])
                    else:
                        ver, comp = name.split("_minus_")
                        y = results[w]["y_test"]
                        if not y:
                            dk = None
                            break
                        pc = results[w]["pooled_pred"][ver] if comp == "pooled_same" else [results[w]["base_pool"]] * len(y)
                        dk.append(losses(y, r["pred"][ver])[0] - losses(y, pc)[0])
                if dk is None:
                    continue
                c = crossing(ks, dk, better)
                vals.append(c)
                if c is not None:
                    labs.append(results[w]["k"][str(c)]["train_labels"])
            big = max(ks) * 10
            ranked = [v if v is not None else big for v in vals]
            cr[name] = dict(writers=len(vals), censored=sum(v is None for v in vals),
                            k_quartiles_with_censored_as_beyond=q(ranked), train_labels_at_cross=q(labs))
        P["crossing_k"] = cr
        S[pname] = P
    return S


def summarize_m(results, wids, rng):
    out = {}
    for m in M_GRID:
        d_pool, d_same_k = [], []
        for w in wids:
            r = results[w]
            if str(m) not in r["m"] or not r["y_test"]:
                continue
            y = r["y_test"]
            p15 = r["m"][str(m)]["pred"]["em"]
            d_pool.append(losses(y, p15)[0] - losses(y, r["pooled_pred"]["em"])[0])
            if str(m) in r["k"]:
                d_same_k.append(losses(y, p15)[0] - losses(y, r["k"][str(m)]["pred"]["em"])[0])
        out[str(m)] = dict(em15_labels_m_minus_pooled_em=dict(q(d_pool), better_share=float(np.mean([x < 0 for x in d_pool])),
                                                              ci=boot_median(d_pool, rng)) if d_pool else None,
                           em15_labels_m_minus_em_m_labels_m=dict(q(d_same_k), better_share=float(np.mean([x < 0 for x in d_same_k])),
                                                                  ci=boot_median(d_same_k, rng)) if d_same_k else None,
                           train_labels=q([results[w]["m"][str(m)]["train_labels"] for w in wids if str(m) in results[w]["m"]]))
    return out


# ---------------------------------------------------------------- 本計算
def run(work, seed, out, jobs):
    writers = load(work)
    ns = [len(ss) for ss in writers]
    elig = [i for i, n in enumerate(ns) if n >= 1 + H]
    jobl = []
    for seq, i in enumerate(elig):
        ks = [k for k in GRID if k <= ns[i] - H]
        jobl.append(dict(i=i, seed=seed, wseq=seq, ks=ks, m_grid=ns[i] >= 20))
    assert all(check_split(ns[i], k) for i in elig for k in GRID if k <= ns[i] - H)
    with ProcessPoolExecutor(jobs, initializer=_init, initargs=(work, sorted(MUT))) as ex:
        res = list(ex.map(eval_writer, jobl))
    results = {i: r for i, r in zip(elig, res)}
    raw = os.environ.get("GSE_PP_RAW")  # 書き手ごとの値（repo の外・事後の確かめ用）
    if raw:
        json.dump({str(k): v for k, v in results.items()}, open(raw, "w"))
    p15 = [i for i in elig if ns[i] >= 20]
    p30 = [i for i in elig if ns[i] >= 35]
    panels = {"P15": (p15, P15_K), "P30": (p30, GRID)}
    rng = np.random.default_rng(seed + 4)  # 区間の乱数はすべてこの 1 本の流れ（Q6 の SEED + 4）
    S = summarize(results, panels, rng)
    S.update(summarize(results, {"all_secondary": (elig, GRID)}, rng, with_crossing=False))
    S["m_grid_P15"] = summarize_m(results, p15, rng)
    S["four_tuple"] = dict(writers=len(elig), sessions_per_writer=q([ns[i] for i in elig]),
                           labels_test=int(sum(results[i]["test_labels"] for i in elig)),
                           positives_test=int(sum(results[i]["test_pos"] for i in elig)), stall_episodes="使わない（0）")
    S["pooled_em_diag"] = dict(winner_frozen_share=float(np.mean([results[i]["pooled_diag"]["winner_frozen"] for i in elig])),
                               nonconv_starts=int(sum(results[i]["pooled_diag"]["nonconv"] for i in elig)),
                               gap12=q([results[i]["pooled_diag"]["gap12"] for i in elig]),
                               decreases=int(sum(results[i]["pooled_diag"]["decreases"] for i in elig)),
                               decrease_rule="1 記号あたりの目的が 1e-9 より大きく減った回（判断 9）")
    S["pooled_logistic_flags"] = {v: {f: int(sum(results[i]["pooled_flags"][v][f] for i in elig)) for f in ("one_class", "sep", "nonconv")}
                                  for v in ("tab", "recal", "em")}
    S["writers_total_single_interval"] = len(writers)
    json.dump(S, open(os.path.join(out, "summary.json"), "w"), indent=1, ensure_ascii=False)


# ---------------------------------------------------------------- P-D2
def pd2(work, out):
    writers = load(work)
    tot = ok = 0
    worst = 0.0
    for ss in writers:
        for s in ss:
            O, M = hmm.pad([s["obs"]])
            _, F, _ = hmm.fb_engine(O, M)
            disp = hmm.PI0.copy()
            for t in range(len(s["obs"])):
                a = 0.60 if (s["obs"][t] == 25 and s["alpha"][t] == 0.3) else 0.40
                disp = a * F[0, t] + (1 - a) * disp
                disp = disp / disp.sum()
                d = max(abs(disp[0] - s["pf"][t]), abs(disp[1] - s["pi"][t]), abs(disp[2] - s["ps"][t]))
                tot += 1
                ok += int(d <= 1e-3)
                worst = max(worst, d)
    r = dict(rows=tot, within_1e3=int(ok), share=ok / tot, worst=float(worst), passed=bool(ok / tot >= 0.999), mutate=sorted(hmm.MUT))
    json.dump(r, open(out, "w"), indent=1)
    print(json.dumps(r))
    return r


if __name__ == "__main__":
    argv = sys.argv[1:]
    if "--mutate" in argv:
        i = argv.index("--mutate")
        MUT.add(argv[i + 1]); hmm.MUT.add(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    if argv[0] == "--pd2":
        r = pd2(argv[1], argv[2])
        sys.exit(0 if r["passed"] else 3)
    elif argv[0] == "--selftest":
        import selftest
        only = None
        if "--only" in argv:
            i = argv.index("--only")
            only = argv[i + 1].split(",")
            argv = argv[:i] + argv[i + 2:]
        sys.exit(selftest.main(argv[1], int(argv[2]), argv[3], only, sorted(MUT)))
    else:
        run(argv[0], int(argv[1]), argv[2], int(argv[3]))
