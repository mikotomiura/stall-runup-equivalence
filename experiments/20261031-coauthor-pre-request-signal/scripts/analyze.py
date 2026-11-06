"""conditions.md C5〜C7 の標本・対照・AUC・区間を計算する。

使い方: python3 -I analyze.py <work_dir> <meta_creative.csv> <meta_argumentative.csv> <seed> <out_dir>
work_dir には convert.py と gse-replay の出力が <sid>.ndjson / <sid>_events.json / <sid>_feat.json / <sid>_ft.json で並ぶ。
出力: <out_dir>/counts.json（AUC の前に書く標本の数）・auc.json・trajectory.csv
"""
import bisect
import csv
import glob
import json
import os
import random
import statistics
import sys

SIGNALS = ["p_stuck", "ewma_x", "f3", "f5", "onset30"]
PRIMARY = ["p_stuck", "ewma_x"]
WARM = 60_000
AI_GUARD = 60_000
KEY_WIN = 30_000
N_MATCHED = 5
UNMATCHED_MULT = 5
MIN_REQ = 5
N_BOOT = 2000
TRAJ_S = 120


def load_writers(paths):
    m = {}
    for p in paths:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            m[r["session_id"]] = r["worker_id"]
    return m


class Session:
    def __init__(self, wd, sid):
        self.sid = sid
        ev = json.load(open(os.path.join(wd, f"{sid}_events.json")))
        self.t_start, self.t_end = ev["t_start"], ev["t_end"]
        self.req = sorted(e["t"] for e in ev["events"] if e["name"] == "suggestion-get")
        self.ai = sorted(e["t"] for e in ev["events"] if e["name"].startswith("suggestion") or e["name"] == "api-insert")
        self.press = []
        for line in open(os.path.join(wd, f"{sid}.ndjson")):
            r = json.loads(line)
            if r.get("type") == "key" and r["press"]:
                self.press.append(r["t"])
        self.press.sort()
        feat = json.load(open(os.path.join(wd, f"{sid}_feat.json")))
        feat.sort(key=lambda x: x["timestamp"])
        self.feat_t = [x["timestamp"] for x in feat]
        self.feat = feat
        ft = json.load(open(os.path.join(wd, f"{sid}_ft.json")))
        self.onsets = sorted(r["t_ms"] for r in ft["rows"] if r["stall_onset"].get("is_onset"))

    def any_in(self, arr, lo, hi):  # lo <= x < hi
        i = bisect.bisect_left(arr, lo)
        return i < len(arr) and arr[i] < hi

    def has_key(self, T):
        return self.any_in(self.press, T - KEY_WIN, T)

    def req_ok(self, T):
        return T - self.t_start >= WARM and not self.any_in(self.ai, T - AI_GUARD, T) and self.has_key(T)

    def ctrl_ok(self, T):
        return (T - self.t_start >= WARM and T <= self.t_end
                and not self.any_in(self.ai, T - AI_GUARD, T + AI_GUARD + 1) and self.has_key(T))

    def last_press_before(self, T):
        i = bisect.bisect_left(self.press, T)
        return self.press[i - 1] if i > 0 else None

    def values(self, T):
        i = bisect.bisect_right(self.feat_t, T)
        if i == 0:
            return None
        f = self.feat[i - 1]
        v = {k: f[k] for k in ["p_stuck", "ewma_x", "f3", "f5"]}
        v["onset30"] = 1.0 if self.any_in(self.onsets, T - KEY_WIN + 1, T + 1) else 0.0
        return v

    def traj(self, T, key):
        out = []
        for k in range(TRAJ_S + 1):
            i = bisect.bisect_right(self.feat_t, T - 1000 * k)
            out.append(self.feat[i - 1][key] if i > 0 else None)
        return out


def auc(pos, neg):
    if not pos or not neg:
        return None
    s = 0.0
    for a in pos:
        for b in neg:
            s += 1.0 if a > b else 0.5 if a == b else 0.0
    return s / (len(pos) * len(neg))


def boot_median(vals, rng):
    meds = []
    n = len(vals)
    for _ in range(N_BOOT):
        meds.append(statistics.median(vals[rng.randrange(n)] for _ in range(n)))
    meds.sort()
    q = lambda p: meds[min(N_BOOT - 1, int(p * N_BOOT))]
    return {"median": statistics.median(vals), "ci95": [q(0.025), q(0.975)], "ci975": [q(0.0125), q(0.9875)], "n_writers": n}


def main():
    wd, mc, ma, seed, out = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5]
    os.makedirs(out, exist_ok=True)
    rng = random.Random(seed)
    writers = load_writers([mc, ma])
    by_w = {}
    no_writer = 0
    for p in sorted(glob.glob(os.path.join(wd, "*_events.json"))):
        sid = os.path.basename(p)[: -len("_events.json")]
        if sid not in writers:
            no_writer += 1
            continue
        by_w.setdefault(writers[sid], []).append(Session(wd, sid))

    counts = {"sessions_without_writer": no_writer, "writers_total": len(by_w), "requests_raw": 0,
              "requests_c5": 0, "requests_no_snapshot": 0, "requests_no_matched_control": 0, "writers_ge_min": 0}
    data = {}  # w -> {"req": [(sess,T,P)], "matched": [...], "unmatched": [...]}
    for w in sorted(by_w):
        sess = by_w[w]
        reqs = []
        for s in sess:
            counts["requests_raw"] += len(s.req)
            for T in s.req:
                if s.req_ok(T):
                    lp = s.last_press_before(T)
                    reqs.append((s, T, T - lp))
        counts["requests_c5"] += len(reqs)
        if len(reqs) < MIN_REQ:
            continue
        counts["writers_ge_min"] += 1
        gaps = []  # (session, start, gap)
        for s in sess:
            for a, b in zip(s.press, s.press[1:]):
                gaps.append((s, a, b - a))
        req_vals, matched_vals = [], []
        for s, T, P in reqs:
            v = s.values(T)
            if v is None:
                counts["requests_no_snapshot"] += 1
                continue
            cands = [(g[0], g[1] + P) for g in gaps if g[2] > P and g[0].ctrl_ok(g[1] + P)]
            if not cands:
                counts["requests_no_matched_control"] += 1
                continue
            pick = rng.sample(cands, min(N_MATCHED, len(cands)))
            mv = [(c[0], c[1], c[0].values(c[1])) for c in pick]
            mv = [m for m in mv if m[2] is not None]  # 時刻と値を組のまま除く（レビュー指摘）
            if not mv:
                counts["requests_no_matched_control"] += 1
                continue
            req_vals.append((s, T, v))
            matched_vals.extend(mv)
        unmatched_vals = []
        dur = [(s, s.t_start + WARM, s.t_end) for s in sess if s.t_end > s.t_start + WARM]
        tot = sum(b - a for _, a, b in dur)
        need = UNMATCHED_MULT * len(reqs)
        tries = 0
        while len(unmatched_vals) < need and tot > 0 and tries < 200 * need:
            tries += 1
            x = rng.random() * tot
            hit = None
            for s, a, b in dur:
                if x < b - a:
                    hit = (s, int(a + x))
                    break
                x -= b - a
            if hit is None:  # 浮動小数の端（x == tot）。レビュー指摘
                continue
            s, T = hit
            if s.ctrl_ok(T):
                v = s.values(T)
                if v is not None:
                    unmatched_vals.append((s, T, v))
        data[w] = {"req": req_vals, "matched": matched_vals, "unmatched": unmatched_vals}
    counts["writers_used"] = sum(1 for w in data if len(data[w]["req"]) >= MIN_REQ)
    counts["requests_used"] = sum(len(d["req"]) for w, d in data.items() if len(d["req"]) >= MIN_REQ)
    counts["matched_controls"] = sum(len(d["matched"]) for w, d in data.items() if len(d["req"]) >= MIN_REQ)
    counts["unmatched_controls"] = sum(len(d["unmatched"]) for w, d in data.items() if len(d["req"]) >= MIN_REQ)
    counts["onsets_in_req_windows"] = sum(int(v["onset30"]) for d in data.values() for _, _, v in d["req"])
    counts["onsets_in_matched_windows"] = sum(int(v["onset30"]) for d in data.values() for _, _, v in d["matched"])
    json.dump(counts, open(os.path.join(out, "counts.json"), "w"), indent=1)

    res = {}
    used = [w for w in sorted(data) if len(data[w]["req"]) >= MIN_REQ]
    for ctrl in ["matched", "unmatched"]:
        res[ctrl] = {}
        for sig in SIGNALS:
            per = []
            for w in used:
                a = auc([v[sig] for _, _, v in data[w]["req"]], [v[sig] for _, _, v in data[w][ctrl]])
                if a is not None:
                    per.append(a)
            brng = random.Random(seed + 1)
            res[ctrl][sig] = boot_median(per, brng) if per else None
            if per:
                res[ctrl][sig]["share_gt_05"] = sum(1 for a in per if a > 0.5) / len(per)
    json.dump(res, open(os.path.join(out, "auc.json"), "w"), indent=1)

    with open(os.path.join(out, "trajectory.csv"), "w", newline="") as f:
        wr = csv.writer(f)
        wr.writerow(["signal", "group", "sec_before", "mean_of_writer_means", "n_writers"])
        for sig in PRIMARY:
            for grp in ["req", "matched"]:
                for k in range(TRAJ_S + 1):
                    wm = []
                    for w in used:
                        vals = [s.traj(T, sig)[k] for s, T, _ in data[w][grp]]
                        vals = [x for x in vals if x is not None]
                        if vals:
                            wm.append(sum(vals) / len(vals))
                    wr.writerow([sig, grp, k, round(sum(wm) / len(wm), 5) if wm else "", len(wm)])
    print(json.dumps(counts))


if __name__ == "__main__":
    main()
