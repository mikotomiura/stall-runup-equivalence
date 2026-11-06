"""conditions.md K4〜K6: 止まりの初期の発火（主）と案 A の追試（副）。

使い方: python3 -I analyze_item2.py <work_dir> <seed> <out_dir> [--check-only]
work_dir は build_work.py の出力（w*.json）。出力は集計値だけ（書き手の ID も番号も出さない）:
  checks.json（θ と onset の再現の照合・計算の前に止める門）・main.json・curve.json・
  replication_counts.json・replication_auc.json・replication_describe.json
"""
import bisect
import glob
import json
import math
import os
import random
import statistics
import sys

TEXT = {"alphabetic", "numeric", "punctuation", "sentence_punctuation", "space", "enter", "tab"}
DELETE = {"backspace", "delete"}
BOUNDARY = {"space", "enter", "sentence_punctuation"}
W_MS, Q, WARMUP, HOLE_GAP = 30_000, 0.99, 100, 2_000  # p1-episode.json
ELIG_MIN_MS = 10 * 60_000  # K4
KS = (0.5, 0.75)
CURVE_K = [0.25 * i for i in range(1, 13)]
CURVE_ABS = [2_000, 5_000, 10_000, 15_000, 20_000]
TRIG_P, TRIG_HOLD, EP_END_HOLD, AWAY = 0.60, 30_000, 60_000, 180_000  # invitation.rs
N_BOOT = 2_000
MIN_FIRES_PER_WRITER = 5
# 副（案 A の S3〜S6）
STALL_GAP, CTRL_LO, CTRL_HI, WARM, D_WIN = 30_000, 2_000, 30_000, 60_000, 30_000
N_MATCHED, MIN_STALL, MIN_WRITERS = 5, 5, 30
SIGNALS = ["d_p_stuck", "d_ewma_x", "p_stuck", "ewma_x"]


# ---------- GSE の規則の再現（K4） ----------

def observed_intervals(feat_ts):
    ts = sorted(set(feat_ts))
    if not ts:
        return []
    out, start, prev = [], ts[0], ts[0]
    for t in ts[1:]:
        if t - prev > HOLE_GAP:
            out.append((start, prev))
            start = t
        prev = t
    out.append((start, prev))
    return out


class Theta:
    """θ(t): t 以前に確定した隣接の押下の間隔（観測区間の中）の 0.99 分位・nearest-rank・100 未満は None。"""

    def __init__(self, press, intervals):
        pooled = []
        for a, b in intervals:
            lo, hi = bisect.bisect_left(press, a), bisect.bisect_right(press, b)
            seg = press[lo:hi]
            pooled += [(y, y - x) for x, y in zip(seg, seg[1:])]
        pooled.sort(key=lambda r: r[0])
        self.ts = [r[0] for r in pooled]
        self.step = []
        vals = []
        for _, g in pooled:
            bisect.insort(vals, g)
            n = len(vals)
            self.step.append(vals[min(max(math.ceil(Q * n), 1), n) - 1] if n >= WARMUP else None)
        self.defined_from = self.ts[WARMUP - 1] if len(self.ts) >= WARMUP else None

    def at(self, t):
        i = bisect.bisect_right(self.ts, t)
        return (self.step[i - 1], i) if i > 0 and self.step[i - 1] is not None else (None, i)


class CondA:
    """条件 A（30 秒の窓の正味の産出 < 1）のタイムライン。区間ごとに評価時刻と値を持つ。"""

    def __init__(self, press, cats, intervals):
        self.press = press
        pref = [0]
        for c in cats:
            pref.append(pref[-1] + (1 if c in TEXT else -1 if c in DELETE else 0))
        self.pref = pref
        inst = sorted(set(press) | {t + W_MS for t in press})
        self.iv = []
        for a, b in intervals:
            start = a + W_MS
            if start > b:
                continue
            lo, hi = bisect.bisect_left(inst, start), bisect.bisect_right(inst, b)
            ins = sorted(set([start] + inst[lo:hi]))
            self.iv.append((a, b, ins, [self.net(t) < 1 for t in ins]))

    def net(self, t):
        # 窓 (t - W, t]: timestamp + W > t かつ timestamp <= t
        hi = bisect.bisect_right(self.press, t)
        lo = bisect.bisect_right(self.press, t - W_MS)
        return self.pref[hi] - self.pref[lo]

    def rises(self):
        for a, b, ins, val in self.iv:
            prev = False
            for i, v in enumerate(val):
                if v and not prev:
                    yield (a, b, ins, val, i)
                prev = v

    @staticmethod
    def holds(ins, val, i, until):
        j = i + 1
        while j < len(ins) and ins[j] <= until:
            if not val[j]:
                return False
            j += 1
        return True


def replicate_onsets(ca, theta):
    out = []
    for a, b, ins, val, i in ca.rises():
        s = ins[i]
        th, n = theta.at(s)
        if th is None:
            continue
        t0 = s + th
        if t0 <= b and CondA.holds(ins, val, i, t0):
            out.append([s, t0, th, n])
    return out


# ---------- 書き手 1 人 ----------

class Writer:
    def __init__(self, p):
        d = json.load(open(p))
        self.keys = d["keys"]
        self.press = [k[0] for k in self.keys]
        self.cats = [k[1] for k in self.keys]
        self.feat = d["feat"]
        self.feat_t = [f[0] for f in self.feat]
        self.onsets = d["onsets"]
        # 観測区間は replay の封筒の値（idle_trim の後）。feat 行からの再計算は刈り込みを含まない（K4 の照合で判明）
        self.intervals = [tuple(iv) for iv in d["observed_intervals"]]
        self.intervals_untrimmed = observed_intervals(self.feat_t)
        self.theta = Theta(self.press, self.intervals)
        self.ca = CondA(self.press, self.cats, self.intervals)
        self.text = [t for t, c in self.keys if c in TEXT]
        self.text_cat = [c for t, c in self.keys if c in TEXT]
        self.t_start, self.t_end = self.press[0], self.press[-1]

    def snap(self, t):
        i = bisect.bisect_right(self.feat_t, t)
        return self.feat[i - 1] if i > 0 else None

    def pause_of(self, f):
        """f を含む休止 (u, u⁺)。u は f 以前の最後の文字の入力。"""
        i = bisect.bisect_right(self.text, f) - 1
        if i < 0:
            return None
        return self.text[i], (self.text[i + 1] if i + 1 < len(self.text) else self.t_end)

    def reaches_stall(self, f):
        p = self.pause_of(f)
        if p is None:
            return False, None
        u, nxt = p
        t0s = [o[1] for o in self.onsets if f < o[1] <= nxt]
        return (True, min(t0s) - f) if t0s else (False, None)


def fires_clock_a(w, k=None, abs_ms=None):
    out = []
    for i, u in enumerate(w.text):
        th, _ = w.theta.at(u)
        if th is None:
            continue
        nxt = w.text[i + 1] if i + 1 < len(w.text) else w.t_end
        d = k * th if k is not None else abs_ms
        if nxt - u > d:
            out.append(u + d)
    return out


def fires_clock_b(w, k):
    out = []
    for a, b, ins, val, i in w.ca.rises():
        s = ins[i]
        th, _ = w.theta.at(s)
        if th is None:
            continue
        f = s + k * th
        if f <= b and CondA.holds(ins, val, i, f):
            out.append(f)
    return out


def fires_current(w):
    """invitation.rs の track_episode の再現（契機の部分だけ）。"""
    out = []
    stuck_since = below_since = None
    in_ep = False
    for t, p, _ in w.feat:
        j = bisect.bisect_right(w.press, t)
        idle = t - w.press[j - 1] if j > 0 else 0
        if idle >= AWAY:
            in_ep, stuck_since, below_since = False, None, None
            continue
        p = p if p is not None and math.isfinite(p) else 0.0
        if p >= TRIG_P:
            below_since = None
            if stuck_since is None:
                stuck_since = t
            if not in_ep and t - stuck_since >= TRIG_HOLD:
                in_ep = True
                out.append(t)
        else:
            stuck_since = None
            if in_ep:
                if below_since is None:
                    below_since = t
                if t - below_since >= EP_END_HOLD:
                    in_ep, below_since = False, None
    return out


def summarize_fires(w, fires):
    lo = w.theta.defined_from
    fs = [f for f in fires if lo is not None and lo <= f <= w.t_end]
    hit, lead = 0, []
    for f in fs:
        ok, dt = w.reaches_stall(f)
        if ok:
            hit += 1
            lead.append(dt)
    # 取りこぼしの参考: 停滞の t0 より前に、同じ休止の中で発火があったか
    caught = 0
    for o in w.onsets:
        p = w.pause_of(o[1])
        if p and any(p[0] < f < o[1] for f in fs):
            caught += 1
    return {"fires": len(fs), "hit": hit, "lead": lead, "stalls": len(w.onsets), "caught": caught}


def boot(vals_by_writer, stat, rng):
    n = len(vals_by_writer)
    out = sorted(stat([vals_by_writer[rng.randrange(n)] for _ in range(n)]) for _ in range(N_BOOT))
    return [out[int(0.025 * N_BOOT)], out[min(N_BOOT - 1, int(0.975 * N_BOOT))]]


def quart(xs):
    xs = sorted(xs)
    if not xs:
        return None
    q = lambda p: xs[min(len(xs) - 1, int(p * len(xs)))]
    return {"median": statistics.median(xs), "q25": q(0.25), "q75": q(0.75)}


def describe_trigger(rows, elig_h, rng):
    rate = [r["fires"] / h for r, h in zip(rows, elig_h)]
    pooled = lambda rs: (sum(r["hit"] for r in rs) / sum(r["fires"] for r in rs)) if sum(r["fires"] for r in rs) else float("nan")
    per_prec = [r["hit"] / r["fires"] for r in rows if r["fires"] >= MIN_FIRES_PER_WRITER]
    leads = [x for r in rows for x in r["lead"]]
    stalls = sum(r["stalls"] for r in rows)
    return {
        "rate_per_hour": quart(rate) | {"median_ci95": boot(rate, statistics.median, rng),
                                        "mean": statistics.mean(rate)},
        "fires_total": sum(r["fires"] for r in rows),
        "reach_stall_pooled": pooled(rows), "reach_stall_pooled_ci95": boot(rows, pooled, rng),
        "reach_stall_per_writer_median": statistics.median(per_prec) if per_prec else None,
        "writers_with_ge5_fires": len(per_prec),
        "stalls_caught_before_t0": (sum(r["caught"] for r in rows) / stalls) if stalls else None,
        "lead_to_t0_ms": quart(leads),
    }


# ---------- 副: 案 A の追試 ----------

def sig_at(w, u):
    a, b = w.snap(u), w.snap(u - D_WIN)
    if a is None or b is None:
        return None
    return {"d_p_stuck": a[1] - b[1], "d_ewma_x": a[2] - b[2], "p_stuck": a[1], "ewma_x": a[2]}


def auc(pos, neg):
    if not pos or not neg:
        return None
    s = sum(1.0 if a > b else 0.5 if a == b else 0.0 for a in pos for b in neg)
    return s / (len(pos) * len(neg))


def pair_counts(rows, sig):
    """書き手の中の対の (勝ち, 対の数)。"""
    pos = [v[sig] for v, _ in rows]
    neg = [c[sig] for _, cs in rows for c in cs]
    s = sum(1.0 if a > b else 0.5 if a == b else 0.0 for a in pos for b in neg)
    return s, len(pos) * len(neg)


def replication(writers, seed, counts):
    rng = random.Random(seed)
    data = []
    for w in writers:
        anchors = {}
        for s0, t0, _, _ in w.onsets:
            counts["onsets_total"] += 1
            i = bisect.bisect_right(w.text, t0) - 1
            if i < 0:
                counts["onsets_no_text_before"] += 1
                continue
            u = w.text[i]
            nxt = w.text[i + 1] if i + 1 < len(w.text) else w.t_end
            if nxt - u < STALL_GAP:
                counts["onsets_gap_lt_30s"] += 1
                continue
            if u in anchors:
                counts["onsets_duplicate_anchor"] += 1
                continue
            anchors[u] = (s0, "boundary" if w.text_cat[i] in BOUNDARY else "mid")

        def in_stall(t):
            return any(s0 - D_WIN <= t <= t0 for s0, t0, _, _ in w.onsets)
        cands = {"boundary": [], "mid": []}
        for i in range(len(w.text) - 1):
            u, v = w.text[i], w.text[i + 1]
            if CTRL_LO <= v - u < CTRL_HI and u - w.t_start >= WARM:
                if in_stall(u):
                    counts["controls_dropped_inside_stall"] += 1
                    continue
                cands["boundary" if w.text_cat[i] in BOUNDARY else "mid"].append(u)
        rows = []
        for u in sorted(anchors):
            s0, cls = anchors[u]
            counts["stall_anchors"] += 1
            if u >= s0:
                counts["anchor_inside_stall"] += 1
                continue
            if u - w.t_start < WARM:
                counts["stall_ineligible"] += 1
                continue
            v = sig_at(w, u)
            if v is None:
                counts["stall_ineligible"] += 1
                continue
            pool = cands[cls][:]
            rng.shuffle(pool)
            ctrl = []
            for cu in pool:
                cv = sig_at(w, cu)
                if cv is not None:
                    ctrl.append(cv)
                    if len(ctrl) == N_MATCHED:
                        break
            if not ctrl:
                counts["stall_no_control"] += 1
                continue
            rows.append((v, ctrl))
        data.append(rows)
    return data


def boot_median(vals, rng):
    n = len(vals)
    meds = sorted(statistics.median(vals[rng.randrange(n)] for _ in range(n)) for _ in range(N_BOOT))
    q = lambda p: meds[min(N_BOOT - 1, int(p * N_BOOT))]
    return {"median": statistics.median(vals), "ci95": [q(0.025), q(0.975)], "ci975": [q(0.0125), q(0.9875)], "n_writers": n}


def boot_pooled(pc, rng):
    n = len(pc)
    ratio = lambda xs: sum(a for a, _ in xs) / sum(b for _, b in xs)
    vals = sorted(ratio([pc[rng.randrange(n)] for _ in range(n)]) for _ in range(N_BOOT))
    q = lambda p: vals[min(N_BOOT - 1, int(p * N_BOOT))]
    return {"pooled_auc": ratio(pc), "ci95": [q(0.025), q(0.975)], "ci975": [q(0.0125), q(0.9875)], "n_writers": n}


def main():
    wd, seed, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    check_only = "--check-only" in sys.argv
    os.makedirs(out, exist_ok=True)
    paths = sorted(glob.glob(os.path.join(wd, "w*.json")))
    summary = json.load(open(os.path.join(wd, "build_summary.json")))
    if len(paths) != summary["members"] or summary["status"].get("ok") != summary["members"]:
        sys.exit("中間の数が build_summary と合わない（欠けた書き手のまま照合しない）")
    writers = []
    chk = {"writers": 0, "onsets_replay": 0, "onsets_replicated": 0, "writers_onset_mismatch": 0,
           "theta_mismatch": 0}
    for p in paths:
        w = Writer(p)
        chk["writers"] += 1
        rep = replicate_onsets(w.ca, w.theta)
        chk["onsets_replay"] += len(w.onsets)
        chk["onsets_replicated"] += len(rep)
        if rep != [list(o) for o in w.onsets]:
            chk["writers_onset_mismatch"] += 1
        for s0, _, th, n in w.onsets:
            if w.theta.at(s0) != (th, n):
                chk["theta_mismatch"] += 1
        writers.append(w)
    json.dump(chk, open(os.path.join(out, "checks.json"), "w"), indent=1)
    print(json.dumps(chk))
    if chk["writers_onset_mismatch"] or chk["theta_mismatch"]:
        sys.exit("θ または onset の再現が replay と合わない（K4）。止める")
    if check_only:
        return

    # ---- 主 ----
    elig = [w for w in writers if w.theta.defined_from is not None and w.t_end - w.theta.defined_from >= ELIG_MIN_MS]
    elig_h = [(w.t_end - w.theta.defined_from) / 3_600_000 for w in elig]
    main_res = {"writers_total": len(writers), "writers_eligible": len(elig),
                "writers_excluded_short_eligible_time": len(writers) - len(elig),
                "eligible_minutes": quart([h * 60 for h in elig_h]),
                "stalls_total": sum(len(w.onsets) for w in elig),
                "stalls_per_hour": quart([len(w.onsets) / h for w, h in zip(elig, elig_h)]),
                "theta_ms_at_end": quart([w.theta.step[-1] for w in elig]),
                "triggers": {}}
    trig = {}
    for k in KS:
        trig["clock_a_k%s" % k] = lambda w, k=k: fires_clock_a(w, k=k)
        trig["clock_b_k%s" % k] = lambda w, k=k: fires_clock_b(w, k)
    trig["current_p_stuck_060_30s"] = fires_current
    for name, fn in trig.items():
        rows = [summarize_fires(w, fn(w)) for w in elig]
        main_res["triggers"][name] = describe_trigger(rows, elig_h, random.Random(seed + 10))
    json.dump(main_res, open(os.path.join(out, "main.json"), "w"), indent=1)

    curve = {"clock_a_k": {}, "clock_a_abs_ms": {}}
    for k in CURVE_K:
        rows = [summarize_fires(w, fires_clock_a(w, k=k)) for w in elig]
        r = describe_trigger(rows, elig_h, random.Random(seed + 20))
        curve["clock_a_k"][str(k)] = {"rate_median": r["rate_per_hour"]["median"], "reach_stall_pooled": r["reach_stall_pooled"]}
    for a in CURVE_ABS:
        rows = [summarize_fires(w, fires_clock_a(w, abs_ms=a)) for w in elig]
        r = describe_trigger(rows, elig_h, random.Random(seed + 20))
        curve["clock_a_abs_ms"][str(a)] = {"rate_median": r["rate_per_hour"]["median"], "reach_stall_pooled": r["reach_stall_pooled"]}
    json.dump(curve, open(os.path.join(out, "curve.json"), "w"), indent=1)

    # ---- 副 ----
    counts = {k: 0 for k in ("onsets_total", "onsets_no_text_before", "onsets_gap_lt_30s", "onsets_duplicate_anchor",
                             "stall_anchors", "anchor_inside_stall", "stall_ineligible", "stall_no_control",
                             "controls_dropped_inside_stall")}
    data = replication(writers, seed, counts)
    used = [rows for rows in data if len(rows) >= MIN_STALL]
    counts["writers_with_rows"] = sum(1 for r in data if r)
    counts["writers_used_default"] = len(used)
    counts["stalls_used_default"] = sum(len(r) for r in used)
    counts["unit"] = "per_writer" if len(used) >= MIN_WRITERS else "pooled_within_writer"
    json.dump(counts, open(os.path.join(out, "replication_counts.json"), "w"), indent=1)
    res = {"unit": counts["unit"]}
    for sig in SIGNALS:
        r = {}
        if used:
            per = [auc([v[sig] for v, _ in rows], [c[sig] for _, cs in rows for c in cs]) for rows in used]
            r["per_writer"] = boot_median(per, random.Random(seed + 1)) | {"share_gt_05": sum(a > 0.5 for a in per) / len(per)}
        allrows = [rows for rows in data if rows]
        r["pooled_within_writer"] = boot_pooled([pair_counts(rows, sig) for rows in allrows], random.Random(seed + 1))
        res[sig] = r
    json.dump(res, open(os.path.join(out, "replication_auc.json"), "w"), indent=1)

    desc = {}
    src = used if counts["unit"] == "per_writer" else [rows for rows in data if rows]
    for sig in ("d_p_stuck", "d_ewma_x"):
        desc[sig] = {}
        for q in (0.8, 0.9, 0.95):
            hits, fires = [], []
            for rows in src:
                cv = sorted(c[sig] for _, cs in rows for c in cs)
                th = cv[min(len(cv) - 1, int(q * len(cv)))]
                hits.append(sum(1 for v, _ in rows if v[sig] > th) / len(rows))
                fires.append(sum(1 for c in cv if c > th) / len(cv))
            desc[sig][str(q)] = {"stall_hit_rate_median": statistics.median(hits),
                                 "control_exceed_rate_median": statistics.median(fires)}
    json.dump(desc, open(os.path.join(out, "replication_describe.json"), "w"), indent=1)
    print(json.dumps({"main_writers": len(elig), "replication": counts}))


if __name__ == "__main__":
    main()
