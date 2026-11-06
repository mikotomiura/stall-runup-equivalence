"""conditions.md F0〜F5: 凍結の前の計算（幅の換算・精度の見積もり）。見た書き手だけ。

使い方:
  python3 -I prefreeze.py <cs1_work2019> <holdout.json> <cs1_keystrokes_2019.csv> <cs1_landscape_results> \
      <klicke_work> <k6_results> <seed> <out_dir>
  python3 -I prefreeze.py --selftest
出力は集計値だけ（書き手の ID も番号も出さない）。CS1 の実の信号の AUC は書かない（F5）。
"""
import bisect
import glob
import json
import math
import os
import random
import statistics
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
K6_DIR = os.path.join(HERE, "../../20261101-klicke-early-stall-trigger/scripts")
CS1_DIR = os.path.join(HERE, "../../20261104-cs1-landscape/scripts")
sys.path.insert(0, K6_DIR)
sys.path.insert(0, CS1_DIR)
import analyze_item2 as k6  # noqa: E402
import analyze_cs1 as c1  # noqa: E402

SIGS = ("d_p_stuck", "d_ewma_x")
EVENTS = ("run", "paste", "switch")
STALL_GAP, CTRL_LO, CTRL_HI, WARM, D_WIN, N_MATCHED = 30_000, 2_000, 30_000, 60_000, 30_000, 5
FIRE_MIN = 2_000
Q_THR, MIN_CAND = 0.9, 10
N_DRAW, N_HOLD, N_BOOT = 1_000, 146, 2_000
H = 3_600_000


# ---------- 共通 ----------

def finite(v):
    return v is not None and isinstance(v, (int, float)) and math.isfinite(v)


def sig_at(w, u):
    a, b = w.snap(u), w.snap(u - D_WIN)
    if a is None or b is None:
        return None, "no_row"
    if not all(finite(x) for x in (a[1], a[2], b[1], b[2])):
        return None, "nonfinite"
    return {"d_p_stuck": a[1] - b[1], "d_ewma_x": a[2] - b[2]}, None


def wins_pairs(pos, neg, delta=0.0):
    """書き手の中の全ての対（停滞 × 対照）の勝ち（同値 0.5）と対の数と同値の数。"""
    if len(pos) == 0 or len(neg) == 0:
        return 0.0, 0, 0
    neg = np.sort(np.asarray(neg, dtype=float))
    p = np.asarray(pos, dtype=float) + delta
    lo = np.searchsorted(neg, p, side="left")
    hi = np.searchsorted(neg, p, side="right")
    ties = int((hi - lo).sum())
    return float(lo.sum() + 0.5 * ties), len(p) * len(neg), ties


def pooled_auc(writers, sig, delta=0.0):
    w = n = 0
    for r in writers:
        a, b, _ = wins_pairs(r["pos"][sig], r["neg"][sig], delta)
        w += a
        n += b
    return w / n if n else float("nan")


def solve_delta(writers, sig, target):
    lo, hi = -3.0, 3.0
    for _ in range(60):
        mid = (lo + hi) / 2
        if pooled_auc(writers, sig, mid) < target:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


def quart(xs):
    xs = sorted(xs)
    if not xs:
        return None
    q = lambda p: xs[min(len(xs) - 1, int(p * len(xs)))]  # noqa: E731
    return {"median": statistics.median(xs), "q25": q(0.25), "q75": q(0.75), "n": len(xs)}


# ---------- F3 幅の換算 ----------

def conversion(writers, sig, delta):
    hit, exc, diff, rate = [], [], [], []
    fires = fires_stall = 0
    skipped = 0
    hit_n = hit_d = exc_n = exc_d = 0
    for r in writers:
        if not r["pos"][sig]:
            continue
        cand = sorted(r["cand"][sig])
        if len(cand) < MIN_CAND:
            skipped += 1
            continue
        thr = cand[min(len(cand) - 1, int(Q_THR * len(cand)))]
        h = sum(1 for v in r["pos"][sig] if v + delta > thr) / len(r["pos"][sig])
        e = sum(1 for c in cand if c > thr) / len(cand)
        f = fs = 0
        for v, is_stall, is_row in r["pauses"][sig]:
            if v + (delta if is_row else 0.0) > thr:
                f += 1
                fs += is_stall
        hit_n += sum(1 for v in r["pos"][sig] if v + delta > thr)
        hit_d += len(r["pos"][sig])
        exc_n += sum(1 for c in cand if c > thr)
        exc_d += len(cand)
        hit.append(h)
        exc.append(e)
        diff.append(h - e)
        rate.append(f / (r["obs_ms"] / H))
        fires += f
        fires_stall += fs
    return {"delta": delta, "writers": len(hit), "writers_skipped_few_candidates": skipped,
            "stall_hit_rate": quart(hit), "control_exceed_rate": quart(exc), "hit_minus_exceed": quart(diff),
            "stall_hit_rate_pooled": hit_n / hit_d if hit_d else None,
            "control_exceed_rate_pooled": exc_n / exc_d if exc_d else None,
            "fires_per_hour": quart(rate), "fires_total": fires,
            "share_of_fires_on_stall_pauses_pooled": fires_stall / fires if fires else None}


def binormal_hit(auc, fpr=0.10):
    nd = statistics.NormalDist()
    dprime = math.sqrt(2) * nd.inv_cdf(auc)
    return 1 - nd.cdf(nd.inv_cdf(1 - fpr) - dprime)


# ---------- KLiCKe（K6 の手順をそのまま） ----------

def klicke(work, seed, k6_res):
    paths = sorted(glob.glob(os.path.join(work, "w*.json")))
    summ = json.load(open(os.path.join(work, "build_summary.json")))
    if len(paths) != summ["members"] or summ["status"].get("ok") != summ["members"]:
        sys.exit("KLiCKe の中間の数が build_summary と合わない")
    ws, bad = [], 0
    for p in paths:
        w = k6.Writer(p)
        if k6.replicate_onsets(w.ca, w.theta) != [list(o) for o in w.onsets]:
            bad += 1
        if any(w.theta.at(s0) != (th, n) for s0, _, th, n in w.onsets):
            bad += 1
        ws.append(w)
    if bad:
        sys.exit("KLiCKe の θ・onset の再現が合わない")
    counts = {k: 0 for k in ("onsets_total", "onsets_no_text_before", "onsets_gap_lt_30s", "onsets_duplicate_anchor",
                             "stall_anchors", "anchor_inside_stall", "stall_ineligible", "stall_no_control",
                             "controls_dropped_inside_stall")}
    data = k6.replication(ws, seed, counts)
    out = []
    for w, rows in zip(ws, data):
        r = {"pos": {s: [v[s] for v, _ in rows] for s in SIGS},
             "neg": {s: [c[s] for _, cs in rows for c in cs] for s in SIGS},
             "cand": {s: [] for s in SIGS}, "pauses": {s: [] for s in SIGS},
             "obs_ms": sum(b - a for a, b in w.intervals)}

        def in_stall(t):
            return any(a - D_WIN <= t <= b for a, b, _, _ in w.onsets)
        # 対照の候補（K6 の replication と同じ条件・区切りの種類ごと）と、2 秒以上の休止
        cls_ok = {"boundary": False, "mid": False}
        pauses = []
        t0s = sorted(o[1] for o in w.onsets)
        for i in range(len(w.text) - 1):
            u, v = w.text[i], w.text[i + 1]
            if u - w.t_start < WARM or v - u < FIRE_MIN:
                continue
            sv = k6.sig_at(w, u)
            if sv is None:
                continue
            j = bisect.bisect_right(t0s, u)
            pauses.append((u, sv, j < len(t0s) and t0s[j] <= v))
            if CTRL_LO <= v - u < CTRL_HI and not in_stall(u):
                cls_ok["boundary" if w.text_cat[i] in k6.BOUNDARY else "mid"] = True
                for s in SIGS:
                    r["cand"][s].append(sv[s])
        # 行になった停滞の u（K6 の replication の条件を再評価する）
        row_u, seen = set(), set()
        for s0, t0, _, _ in w.onsets:
            i = bisect.bisect_right(w.text, t0) - 1
            if i < 0:
                continue
            u = w.text[i]
            nxt = w.text[i + 1] if i + 1 < len(w.text) else w.t_end
            if nxt - u < STALL_GAP or u in seen:
                continue
            seen.add(u)
            cls = "boundary" if w.text_cat[i] in k6.BOUNDARY else "mid"
            if u < s0 and u - w.t_start >= WARM and k6.sig_at(w, u) is not None and cls_ok[cls]:
                row_u.add(u)
        if len(row_u) != len(rows):
            sys.exit("KLiCKe の行の停滞の再評価が replication と合わない")
        for u, sv, is_stall in pauses:
            for s in SIGS:
                r["pauses"][s].append((sv[s], is_stall, u in row_u))
        out.append(r)
    ref = json.load(open(os.path.join(k6_res, "replication_auc.json")))
    chk = {}
    for s in SIGS:
        a = pooled_auc(out, s)
        b = ref[s]["pooled_within_writer"]["pooled_auc"]
        chk[s] = {"pooled_auc_reproduced": a, "k6": b}
        if round(a, 4) != round(b, 4):
            sys.exit("KLiCKe の対を合わせた AUC が K6 と合わない（F1）")
    return out, counts, chk


# ---------- CS1（草案 v2 の P3） ----------

def make_unit(d):
    w = object.__new__(k6.Writer)
    w.keys = d["keys"]
    w.press = [k[0] for k in w.keys]
    w.cats = [k[1] for k in w.keys]
    w.feat = d["feat"]
    w.feat_t = [f[0] for f in w.feat]
    w.onsets = d["onsets"]
    w.intervals = [tuple(iv) for iv in d["observed_intervals"]]
    w.theta = k6.Theta(w.press, w.intervals)
    w.ca = k6.CondA(w.press, w.cats, w.intervals)
    w.text = [t for t, c in w.keys if c in k6.TEXT]
    w.text_cat = [c for t, c in w.keys if c in k6.TEXT]
    w.t_start, w.t_end = w.press[0], w.press[-1]
    w.block_starts = [b[0] for b in d["blocks"]]
    w.ev = sorted(t for t, n in d["events"] if n in EVENTS)
    w.stderr_bytes = d["replay_stderr_bytes"]
    return w


def block_start(w, t):
    i = bisect.bisect_right(w.block_starts, t) - 1
    return w.block_starts[i] if i >= 0 else None


def has_event(w, a, b):
    i = bisect.bisect_left(w.ev, a)
    return i < len(w.ev) and w.ev[i] <= b


def cs1_writer(units, rng, cnt):
    """1 人（課題をまたぐ）。層ごとの pos/neg/cand/pauses と件数。"""
    stalls = []   # (unit 番号, u, sig, cell, layer)
    cands = {}    # (cell, layer) -> [sig]
    pauses = []   # (unit 番号, u, sig, is_stall, layer)
    rec = {L: {"pos": {s: [] for s in SIGS}, "neg": {s: [] for s in SIGS}, "cand": {s: [] for s in SIGS},
               "pauses": {s: [] for s in SIGS}} for L in ("none", "event")}
    obs = 0
    for k, w in enumerate(units):
        obs += sum(b - a for a, b in w.intervals)
        anchors = {}
        for s0, t0, _, _ in w.onsets:
            cnt["onsets_total"] += 1
            i = bisect.bisect_right(w.text, t0) - 1
            if i < 0:
                cnt["onsets_no_text_before"] += 1
                continue
            u = w.text[i]
            nxt = w.text[i + 1] if i + 1 < len(w.text) else w.t_end
            if nxt - u < STALL_GAP:
                cnt["onsets_gap_lt_30s"] += 1
                continue
            if u in anchors:
                cnt["onsets_duplicate_anchor"] += 1
                continue
            anchors[u] = (s0, nxt, "newline" if w.text_cat[i] == "enter" else "other")
        for u in sorted(anchors):
            s0, nxt, cell = anchors[u]
            cnt["stall_anchors"] += 1
            if u >= s0:
                cnt["anchor_inside_stall"] += 1
                continue
            bs = block_start(w, u)
            if bs is None or u - bs < WARM:
                cnt["stall_warm_or_block"] += 1
                continue
            sv, why = sig_at(w, u)
            if sv is None:
                cnt["stall_" + why] += 1
                continue
            stalls.append((k, u, sv, cell, "event" if has_event(w, u - D_WIN, nxt) else "none"))

        def in_stall(t):
            return any(a - D_WIN <= t <= b for a, b, _, _ in w.onsets)
        t0s = sorted(o[1] for o in w.onsets)
        for i in range(len(w.text) - 1):
            u, v = w.text[i], w.text[i + 1]
            if v - u < FIRE_MIN:
                continue
            bs = block_start(w, u)
            if bs is None or u - bs < WARM:
                continue
            sv, _ = sig_at(w, u)
            if sv is None:
                continue
            layer = "event" if has_event(w, u - D_WIN, v) else "none"
            j = bisect.bisect_right(t0s, u)
            pauses.append((k, u, sv, j < len(t0s) and t0s[j] <= v, layer))
            if CTRL_LO <= v - u < CTRL_HI:
                if in_stall(u):
                    cnt["controls_dropped_inside_stall"] += 1
                    continue
                cell = "newline" if w.text_cat[i] == "enter" else "other"
                cands.setdefault((cell, layer), []).append(sv)
                cnt["cand_%s_%s" % (cell, layer)] += 1
                for s in SIGS:
                    rec[layer]["cand"][s].append(sv[s])
    row_keys = set()
    for k, u, sv, cell, layer in stalls:
        pool = cands.get((cell, layer), [])[:]
        rng.shuffle(pool)
        ctrl = pool[:N_MATCHED]
        if not ctrl:
            cnt["stall_no_control"] += 1
            continue
        cnt["stall_rows_%s_%s" % (cell, layer)] += 1
        row_keys.add((k, u))
        for s in SIGS:
            rec[layer]["pos"][s].append(sv[s])
            rec[layer]["neg"][s] += [c[s] for c in ctrl]
    for k, u, sv, is_stall, layer in pauses:
        for s in SIGS:
            rec[layer]["pauses"][s].append((sv[s], is_stall, (k, u) in row_keys))
    for L in rec:
        rec[L]["obs_ms"] = obs
    return rec


def cs1(work, hold_p, csv19, land_res, seed):
    hold_ids = set(json.load(open(hold_p))["2019"])
    n_kept = c1.n_not_held(csv19, hold_ids)
    n_built = c1.check_build(work, hold_ids, hold_p)
    if n_built != n_kept:
        sys.exit("中間の書き手の数が、取っておく書き手を除いた数と違う")
    bs = json.load(open(os.path.join(work, "build_summary.json")))
    ref = json.load(open(os.path.join(land_res, "build_summary_2019.json")))
    for k in ("writers_read", "holdout_writers_skipped", "units", "status", "blocks_kept", "blocks_few_presses",
              "row_kinds", "holdout_sha256"):
        if bs[k] != ref[k]:
            sys.exit("作り直した中間の build_summary が俯瞰と違う: " + k)
    uw = json.load(open(os.path.join(work, "units.json")))
    by_w = {}
    for p in sorted(glob.glob(os.path.join(work, "u[0-9]*.json"))):
        by_w.setdefault(uw[os.path.basename(p)[1:-5]], []).append(p)
    rng = random.Random(seed)
    cnt = {k: 0 for k in ("onsets_total", "onsets_no_text_before", "onsets_gap_lt_30s", "onsets_duplicate_anchor",
                          "stall_anchors", "anchor_inside_stall", "stall_warm_or_block", "stall_no_row",
                          "stall_nonfinite", "stall_no_control", "controls_dropped_inside_stall")}
    for a in ("newline", "other"):
        for L in ("none", "event"):
            cnt["cand_%s_%s" % (a, L)] = 0
            cnt["stall_rows_%s_%s" % (a, L)] = 0
    chk = {"units": 0, "onset_mismatch_units": 0, "theta_mismatch": 0, "stderr_nonempty_units": 0}
    writers, excluded_short = [], 0
    for wid in sorted(by_w):
        units = [make_unit(json.load(open(p))) for p in by_w[wid]]
        for w in units:
            chk["units"] += 1
            if k6.replicate_onsets(w.ca, w.theta) != [list(o) for o in w.onsets]:
                chk["onset_mismatch_units"] += 1
            chk["theta_mismatch"] += sum(1 for s0, _, th, n in w.onsets if w.theta.at(s0) != (th, n))
            chk["stderr_nonempty_units"] += w.stderr_bytes != 0
        if chk["onset_mismatch_units"] or chk["theta_mismatch"] or chk["stderr_nonempty_units"]:
            sys.exit("CS1 の照合が崩れた（F0）: " + json.dumps(chk))
        if sum(sum(b - a for a, b in w.intervals) for w in units) < 10 * 60_000:
            excluded_short += 1
            continue
        writers.append(cs1_writer(units, rng, cnt))
    chk["writers_eligible"] = len(writers)
    chk["writers_excluded_obs_lt_10min"] = excluded_short
    if len(writers) != 319:
        sys.exit("見た書き手が 319 人にならない（F0）")
    return writers, cnt, chk


# ---------- F4 精度の見積もり ----------

def precision(writers, sig, seed):
    wp = np.array([wins_pairs(r["pos"][sig], r["neg"][sig])[:2] for r in writers], dtype=float)
    nst = np.array([len(r["pos"][sig]) for r in writers])
    nct = np.array([len(r["neg"][sig]) for r in writers])
    rng = np.random.default_rng(seed)
    widths, st, ct, pr, wr = [], [], [], [], []
    lo_i, hi_i = int(0.025 * N_BOOT), min(N_BOOT - 1, int(0.975 * N_BOOT))
    for _ in range(N_DRAW):
        sel = rng.choice(len(writers), N_HOLD, replace=False)
        sub = wp[sel]
        b = rng.integers(0, N_HOLD, (N_BOOT, N_HOLD))
        num, den = sub[b, 0].sum(1), sub[b, 1].sum(1)
        vals = np.sort(num / den)
        widths.append(float(vals[hi_i] - vals[lo_i]))
        st.append(int(nst[sel].sum()))
        ct.append(int(nct[sel].sum()))
        pr.append(int(sub[:, 1].sum()))
        wr.append(int((nst[sel] > 0).sum()))
    wq = np.quantile(widths, [0.05, 0.5, 0.95])
    return {"width_median": float(wq[1]), "width_p05": float(wq[0]), "width_p95": float(wq[2]),
            "share_width_le_010": float(np.mean(np.array(widths) <= 0.10)),
            "per_draw_median": {"stalls": statistics.median(st), "controls": statistics.median(ct),
                                "pairs": statistics.median(pr), "writers_with_stalls": statistics.median(wr)}}


def degeneracy(writers, sig):
    t = n = zero = m = 0
    for r in writers:
        _, b, ties = wins_pairs(r["pos"][sig], r["neg"][sig])
        t += ties
        n += b
        vals = r["pos"][sig] + r["neg"][sig]
        zero += sum(1 for v in vals if v == 0)
        m += len(vals)
    return {"pair_tie_share": t / n if n else None, "anchor_zero_change_share": zero / m if m else None,
            "pairs": n, "anchors": m}


def run(argv):
    w19, hold_p, csv19, land_res, kwork, k6_res, seed, out = argv
    seed = int(seed)
    os.makedirs(out, exist_ok=True)
    cs, ccnt, cchk = cs1(w19, hold_p, csv19, land_res, seed)
    cs_none = [r["none"] for r in cs]
    cs_event = [r["event"] for r in cs]
    kl, kcnt, kchk = klicke(kwork, 20261101, k6_res)
    json.dump({"cs1": cchk, "klicke_k6_reproduction": kchk}, open(os.path.join(out, "checks.json"), "w"), indent=1)
    json.dump({"cs1": ccnt, "klicke": kcnt}, open(os.path.join(out, "counts.json"), "w"), indent=1)

    conv = {"binormal_hit_at_fpr010": {"auc_055": binormal_hit(0.55), "auc_050": 0.10}}
    for name, ws, show_real in (("klicke", kl, True), ("cs1_none", cs_none, False)):
        conv[name] = {}
        for s in SIGS:
            d55, d50 = solve_delta(ws, s, 0.55), solve_delta(ws, s, 0.50)
            e = {"auc_055": conversion(ws, s, d55), "auc_050": conversion(ws, s, d50)}
            if show_real:
                e["real_delta0"] = conversion(ws, s, 0.0) | {"pooled_auc": pooled_auc(ws, s)}
            else:
                # δ の符号と大きさは実の AUC の向きを示す（F5）。CS1 では書かない（初回の出力で漏れた・notes の逸脱 1）
                for v in e.values():
                    v.pop("delta")
            conv[name][s] = e
    json.dump(conv, open(os.path.join(out, "width_conversion.json"), "w"), indent=1)

    prec = {"cs1_none": {s: precision(cs_none, s, seed + 1) for s in SIGS},
            "degeneracy": {"cs1_none": {s: degeneracy(cs_none, s) for s in SIGS},
                           "cs1_event": {s: degeneracy(cs_event, s) for s in SIGS},
                           "klicke": {s: degeneracy(kl, s) for s in SIGS}},
            "cs1_writers": len(cs), "draw": {"n_draw": N_DRAW, "n_hold": N_HOLD, "n_boot": N_BOOT}}
    json.dump(prec, open(os.path.join(out, "precision.json"), "w"), indent=1)
    print(json.dumps({"checks": cchk, "klicke": kchk}))


def selftest():
    # 対の勝ちは総当たりと一致する
    pos, neg = [0.1, 0.2, 0.2, -0.5], [0.2, 0.0, -1.0, 0.3, 0.2]
    brute = sum(1.0 if a > b else 0.5 if a == b else 0.0 for a in pos for b in neg)
    w, n, t = wins_pairs(pos, neg)
    assert abs(w - brute) < 1e-12 and n == 20 and t == 4, (w, brute, t)
    assert wins_pairs([], neg) == (0.0, 0, 0)
    # δ の二分法は目標の AUC に届く
    rr = random.Random(1)
    ws = [{"pos": {s: [rr.gauss(0, 1) for _ in range(20)] for s in SIGS},
           "neg": {s: [rr.gauss(0, 1) for _ in range(100)] for s in SIGS}} for _ in range(30)]
    d = solve_delta(ws, "d_p_stuck", 0.55)
    assert abs(pooled_auc(ws, "d_p_stuck", d) - 0.55) < 1e-3
    # 解析の参考値
    assert abs(binormal_hit(0.5) - 0.10) < 1e-9 and 0.13 < binormal_hit(0.55) < 0.14
    # まとまりの開始と出来事の判定
    w = object.__new__(k6.Writer)
    w.block_starts, w.ev = [0, 5_000_000], [100_000, 5_050_000]
    assert block_start(w, 4_999_999) == 0 and block_start(w, 5_000_000) == 5_000_000 and block_start(w, -1) is None
    assert has_event(w, 90_000, 100_000) and not has_event(w, 100_001, 5_049_999)
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        run(sys.argv[1:])
