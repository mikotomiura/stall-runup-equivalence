"""離散の HMM（K 状態 × M 記号）の前向き・後ろ向き・MAP-EM（numpy・系列を詰めて時刻ごとに一括）。conditions.md の Q3・Q4。

系列は長さの違う整数の配列の list。詰めた行列 O（n × T）とマスク（n × T）で扱う。各系列は π から始まる。
変異の切り替え（MUT）は perperson.py の --mutate が設定する。
"""
import itertools

import numpy as np

# HMM 0.5.0（src-tauri/src/analysis/engine.rs の new_for_domain と INITIAL_PROBS）。放出は行の和が 1 でない。
A0 = np.array([0.75, 0.17, 0.08, 0.12, 0.80, 0.08, 0.06, 0.18, 0.76]).reshape(3, 3)
PI0 = np.array([0.80, 0.15, 0.05])
_E = [
    0.01, 0.02, 0.08, 0.12, 0.14, 0.01, 0.02, 0.08, 0.12, 0.13, 0.01, 0.01, 0.02, 0.01, 0.08,
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
    0.15, 0.10, 0.04, 0.03, 0.02, 0.14, 0.08, 0.04, 0.03, 0.02, 0.10, 0.08, 0.03, 0.02, 0.01,
    0.05, 0.04, 0.01, 0.01, 0.01, 0.04, 0.03, 0.01, 0.01, 0.01, 0.01,
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.02, 0.04, 0.04, 0.05, 0.01,
    0.10, 0.16, 0.05, 0.07, 0.01, 0.16, 0.22, 0.12, 0.05, 0.02, 0.99,
]
B0_RAW = np.array(_E).reshape(3, 26)
B0 = B0_RAW / B0_RAW.sum(axis=1, keepdims=True)
NSYM = 26
PSEUDO = dict(pi=1.0, A=1.0, B=0.5)
TOL, MAX_ITER, N_STARTS = 1e-7, 500, 5

MUT = set()  # "m1" 漏れ・"m2" F-b の予測を正規化しない・"m3" 前向きで A の転置


def pad(seqs):
    n, T = len(seqs), max(len(s) for s in seqs)
    O = np.zeros((n, T), dtype=np.int64)
    M = np.zeros((n, T), dtype=bool)
    for i, s in enumerate(seqs):
        O[i, :len(s)] = s
        M[i, :len(s)] = True
    return O, M


def _A(A):
    return A.T if "m3" in MUT else A


def forward(pi, A, B, O, M):
    """スケーリングした前向き。alpha[:, t] はフィルタ p(z_t | o_1..t)、c[:, t] = p(o_t | o_<t)。"""
    n, T = O.shape
    Af = _A(A)
    alpha = np.zeros((n, T, len(pi)))
    c = np.ones((n, T))
    a = pi[None, :] * B[:, O[:, 0]].T
    c[:, 0] = a.sum(1)
    alpha[:, 0] = a / c[:, 0:1]
    for t in range(1, T):
        a = (alpha[:, t - 1] @ Af) * B[:, O[:, t]].T
        s = a.sum(1)
        m = M[:, t]
        c[m, t] = s[m]
        alpha[m, t] = a[m] / s[m, None]
        alpha[~m, t] = alpha[~m, t - 1]
    return alpha, c


def pred_logprob(pi, A, B, O, M):
    """1 歩先の予測の対数確率（n × T・マスクの外は 0）。m1 は o_t を見たフィルタから o_t を予測する漏れ。"""
    alpha, c = forward(pi, A, B, O, M)
    if "m1" in MUT:
        lp = np.log(np.einsum("ntk,knt->nt", alpha, B[:, O]))
    else:
        lp = np.log(c)
    return np.where(M, lp, 0.0), alpha


def pred_dist(pi, A, B, alpha_prev):
    """フィルタ（n × K）から次の記号の予測の分布（n × M）。P-EM3 用。"""
    return (alpha_prev @ _A(A)) @ B


def fb_engine(O, M, mutate_unnorm=None):
    """F-b: engine の前向き（放出の表を正規化しない）。返り値: 予測の対数確率（n × T）・フィルタ（n × T × 3）・予測の分布の和（n × T）。"""
    unnorm = ("m2" in MUT) if mutate_unnorm is None else mutate_unnorm
    n, T = O.shape
    Af = _A(A0)
    filt = np.tile(PI0, (n, 1))
    lp = np.zeros((n, T))
    F = np.zeros((n, T, 3))
    sums = np.ones((n, T))
    for t in range(T):
        pred = filt @ Af
        dist = pred @ B0_RAW
        z = dist.sum(1, keepdims=True)
        if not unnorm:
            dist = dist / z
        sums[:, t] = dist.sum(1)
        p_o = dist[np.arange(n), O[:, t]]
        a = pred * B0_RAW[:, O[:, t]].T
        a = a / a.sum(1, keepdims=True)
        m = M[:, t]
        lp[m, t] = np.log(p_o[m])
        filt = np.where(m[:, None], a, filt)
        F[:, t] = filt
    return np.where(M, lp, 0.0), F, np.where(M, sums, 1.0)


def backward(A, B, O, M, c):
    n, T = O.shape
    Af = _A(A)
    beta = np.ones((n, T, A.shape[0]))
    for t in range(T - 2, -1, -1):
        m = M[:, t + 1]
        b = (B[:, O[:, t + 1]].T * beta[:, t + 1]) @ Af.T
        beta[m, t] = b[m] / c[m, t + 1, None]
    return beta


def log_prior(pi, A, B):
    return float(PSEUDO["pi"] * np.log(pi).sum() + PSEUDO["A"] * np.log(A).sum() + PSEUDO["B"] * np.log(B).sum())


def em_step(pi, A, B, O, M):
    alpha, c = forward(pi, A, B, O, M)
    beta = backward(A, B, O, M, c)
    gamma = alpha * beta
    gamma /= gamma.sum(2, keepdims=True)
    K = len(pi)
    xi = np.zeros((K, K))
    for t in range(O.shape[1] - 1):
        m = M[:, t + 1]
        if not m.any():
            break
        x = alpha[m, t][:, :, None] * A[None] * (B[:, O[m, t + 1]].T * beta[m, t + 1])[:, None, :]
        xi += (x / c[m, t + 1][:, None, None]).sum(0)
    em = np.zeros((NSYM, K))
    np.add.at(em, O[M], gamma[M])
    pi_n = gamma[:, 0].sum(0) + PSEUDO["pi"]
    A_n = xi + PSEUDO["A"]
    B_n = em.T + PSEUDO["B"]
    obj = float(np.log(c[M]).sum()) + log_prior(pi, A, B)
    return pi_n / pi_n.sum(), A_n / A_n.sum(1, keepdims=True), B_n / B_n.sum(1, keepdims=True), obj


def objective(pi, A, B, O, M):
    _, c = forward(pi, A, B, O, M)
    return float(np.log(c[M]).sum()) + log_prior(pi, A, B)


def em_from(pi, A, B, O, M):
    """1 つの開始点から。返り値: 母数・1 記号あたりの目的・回数・収束したか・減った回の数。"""
    nobs = int(M.sum())
    prev, dec = None, 0
    conv = False
    for it in range(1, MAX_ITER + 1):
        pi2, A2, B2, obj = em_step(pi, A, B, O, M)  # obj は更新前の母数の目的
        o = obj / nobs
        if prev is not None:
            if o < prev - 1e-9:  # 1 記号あたりで 1e-9 より大きく減った（収束の判定と同じ尺度）
                dec += 1
            if abs(o - prev) < TOL:
                conv = True
                break
        prev = o
        pi, A, B = pi2, A2, B2
    return pi, A, B, objective(pi, A, B, O, M) / nobs, it, conv, dec


def fit_em(seqs, rng):
    """開始点 5 つ（凍結の正規化 ＋ Dirichlet(1) 4 つ）から、目的がいちばん大きいものを採る。"""
    O, M = pad(seqs)
    starts = [(PI0, A0, B0)]
    for _ in range(N_STARTS - 1):
        starts.append((rng.dirichlet(np.ones(3)), rng.dirichlet(np.ones(3), 3), rng.dirichlet(np.ones(NSYM), 3)))
    res = [em_from(p, a, b, O, M) for p, a, b in starts]
    objs = [r[3] for r in res]
    order = np.argsort(objs)[::-1]
    best = res[order[0]]
    diag = dict(winner_frozen=bool(order[0] == 0), gap12=float(objs[order[0]] - objs[order[1]]),
                nonconv=sum(not r[5] for r in res), decreases=sum(r[6] for r in res), iters=[r[4] for r in res])
    return (best[0], best[1], best[2]), diag


def order_states(B):
    """状態を放出の x_bin の期待値の昇順に並べる添字（名前は付けない・S1〜S3）。"""
    xb = np.array([o // 5 if o < 25 else 5 for o in range(NSYM)], dtype=float)
    return np.argsort(B @ xb)


def unigram(seqs):
    cnt = np.full(NSYM, 0.5)
    for s in seqs:
        np.add.at(cnt, s, 1)
    return cnt / cnt.sum()


def markov1(seqs):
    T = np.full((NSYM, NSYM), 0.5)
    for s in seqs:
        np.add.at(T, (s[:-1], s[1:]), 1)
    return T / T.sum(1, keepdims=True)


def ll_r0(p, seqs):
    return float(sum(np.log(p[s]).sum() for s in seqs)), int(sum(len(s) for s in seqs))


def ll_r1(p0, T, seqs):
    tot = 0.0
    for s in seqs:
        tot += np.log(p0[s[0]]) + np.log(T[s[:-1], s[1:]]).sum()
    return float(tot), int(sum(len(s) for s in seqs))


def ll_hmm(pi, A, B, seqs):
    O, M = pad(seqs)
    lp, _ = pred_logprob(pi, A, B, O, M)
    return float(lp.sum()), int(M.sum())


def ll_honest(pi, A, B, seqs):
    """変異 m1 の影響を受けない対数尤度（前向きの c から）。P-EM4 の真の母数の尤度に使う。"""
    O, M = pad(seqs)
    _, c = forward(pi, A, B, O, M)
    return float(np.log(c[M]).sum()), int(M.sum())


def sample(pi, A, B, lengths, rng):
    seqs = []
    for L in lengths:
        z = np.empty(L, dtype=np.int64)
        z[0] = rng.choice(len(pi), p=pi)
        u = rng.random(L)
        cA = np.cumsum(A, 1)
        for t in range(1, L):
            z[t] = min(int(np.searchsorted(cA[z[t - 1]], u[t], side="right")), len(pi) - 1)
        cB = np.cumsum(B, 1)
        v = rng.random(L)
        s = np.array([min(int(np.searchsorted(cB[zz], vv, side="right")), B.shape[1] - 1) for zz, vv in zip(z, v)])
        seqs.append(s)
    return seqs


def brute_loglik(pi, A, B, s):
    tot = 0.0
    for path in itertools.product(range(len(pi)), repeat=len(s)):
        p = pi[path[0]] * B[path[0], s[0]]
        for t in range(1, len(s)):
            p *= A[path[t - 1], path[t]] * B[path[t], s[t]]
        tot += p
    return float(np.log(tot))
