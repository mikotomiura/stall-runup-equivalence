"""conditions_round2.md R1〜R4 の標本・対照・AUC・区間を計算する。

使い方: python3 -I analyze2.py <work_dir> <meta_creative.csv> <meta_argumentative.csv> <seed> <out_dir>
work_dir: <sid>.ndjson（key 行つき）/ <sid>_events.json / <sid>_feat.json（slim 済み）/
          writers/<wid>_ft.json（slim 済み・つないだ時刻）/ writers/<wid>_offsets.json
出力: counts.json（AUC の前に書く）・auc.json・onsets_by_writer.json（書き手は番号のみ）
"""
import bisect
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import analyze as A  # noqa: E402  1 回目の読み込み・AUC・ブートストラップを使い回す

BOUNDARY = {"space", "enter", "sentence_punctuation"}
D_WIN = 30_000
P_LONG = 2_000
N_MATCHED = 5
MIN_L, MIN_A = 3, 5
SIGNALS = ["d_p_stuck", "d_ewma_x", "onset30"]


class S2(A.Session):
    def __init__(self, wd, sid):
        self.sid = sid
        ev = json.load(open(os.path.join(wd, f"{sid}_events.json")))
        self.t_start, self.t_end = ev["t_start"], ev["t_end"]
        self.req = sorted(e["t"] for e in ev["events"] if e["name"] == "suggestion-get")
        self.ai = sorted(e["t"] for e in ev["events"] if e["name"].startswith("suggestion") or e["name"] == "api-insert")
        ks = []
        for line in open(os.path.join(wd, f"{sid}.ndjson")):
            r = json.loads(line)
            if r.get("type") == "key" and r["press"]:
                ks.append((r["t"], r["category"]))
        ks.sort(key=lambda x: x[0])  # 安定ソート: 同時刻は行順を保つ（レビュー指摘）
        self.press = [t for t, _ in ks]
        self.cat = [c for _, c in ks]
        feat = sorted(json.load(open(os.path.join(wd, f"{sid}_feat.json"))), key=lambda x: x["timestamp"])
        self.feat_t = [x["timestamp"] for x in feat]
        self.feat = feat
        self.onsets = []

    def snap(self, T):
        i = bisect.bisect_right(self.feat_t, T)
        return self.feat[i - 1] if i > 0 else None

    def cls_at(self, t_press):
        # 同時刻の打鍵が複数あるときは最後のもの（last_press_before と同じ打鍵・レビュー指摘）
        i = bisect.bisect_right(self.press, t_press) - 1
        return "boundary" if self.cat[i] in BOUNDARY else "mid"

    def values2(self, T):
        a, b = self.snap(T), self.snap(T - D_WIN)
        if a is None or b is None:
            return None
        return {"d_p_stuck": a["p_stuck"] - b["p_stuck"], "d_ewma_x": a["ewma_x"] - b["ewma_x"],
                "onset30": 1.0 if self.any_in(self.onsets, T - D_WIN + 1, T + 1) else 0.0}


def main():
    wd, mc, ma, seed, out = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5]
    os.makedirs(out, exist_ok=True)
    rng = random.Random(seed)
    writers = A.load_writers([mc, ma])
    by_w = {}
    import glob
    for p in sorted(glob.glob(os.path.join(wd, "*_events.json"))):
        sid = os.path.basename(p)[: -len("_events.json")]
        if sid in writers:
            by_w.setdefault(writers[sid], []).append(S2(wd, sid))
    onset_stats = {}
    unassigned = [0]
    for w, sess in by_w.items():
        ft = json.load(open(os.path.join(wd, "writers", f"{w}_ft.json")))
        offs = json.load(open(os.path.join(wd, "writers", f"{w}_offsets.json")))
        on = [r["t_ms"] for r in ft["rows"] if r["stall_onset"].get("is_onset")]
        th = [r["stall_onset"].get("theta_person_ms") for r in ft["rows"] if r["stall_onset"].get("is_onset")]
        bysid = {s.sid: s for s in sess}
        # 区間は concat_writer が詰めた基準（最初と最後の key/feat 行）で作る（レビュー指摘・重ならない）
        spans = sorted((min(s.press[:1] + s.feat_t[:1]) + offs[s.sid], max(s.press[-1:] + s.feat_t[-1:]) + offs[s.sid], s.sid)
                       for s in sess if s.sid in offs and (s.press or s.feat_t))
        for t in on:
            for a, b, sid in spans:
                if a <= t <= b:
                    bysid[sid].onsets.append(t - offs[sid])
                    break
            else:
                unassigned[0] += 1
        for s in sess:
            s.onsets.sort()
        onset_stats[w] = {"onsets": len(on), "theta_median_ms": sorted(th)[len(th) // 2] if th else None}

    counts = {"writers_total": len(by_w), "onsets_unassigned": unassigned[0], "req_c5": 0, "req_L": 0, "req_A_no_value": 0,
              "req_A_no_control": 0, "req_L_no_control": 0}
    data = {"L": {}, "A": {}}
    for w in sorted(by_w):
        sess = by_w[w]
        gaps = [(s, s.press[i], s.press[i + 1] - s.press[i], s.cls_at(s.press[i]))
                for s in sess for i in range(len(s.press) - 1)]
        for s in sess:
            for T in s.req:
                if not s.req_ok(T):
                    continue
                counts["req_c5"] += 1
                lp = s.last_press_before(T)
                P, cls = T - lp, s.cls_at(lp)
                long_ = P >= P_LONG
                counts["req_L"] += int(long_)
                v = s.values2(T)
                if v is None:
                    counts["req_A_no_value"] += 1
                    continue
                cands = [(g[0], g[1] + P) for g in gaps
                         if g[2] > P and g[3] == cls and g[0].ctrl_ok(g[1] + P)]
                rng.shuffle(cands)
                pick = []
                for c in cands:  # 無作為の順に見て、値が取れる最初の N_MATCHED 件
                    cv = c[0].values2(c[1])
                    if cv is not None:
                        pick.append((c[0], c[1], cv))
                        if len(pick) == N_MATCHED:
                            break
                if not pick:
                    counts["req_A_no_control"] += 1
                    counts["req_L_no_control"] += int(long_)
                    continue
                for layer, ok in (("A", True), ("L", long_)):
                    if ok:
                        d = data[layer].setdefault(w, {"req": [], "ctrl": []})
                        d["req"].append(v)
                        d["ctrl"].extend(c[2] for c in pick)
                        d.setdefault("pairs", []).append((v, [c[2] for c in pick]))
    for layer, mn in (("L", MIN_L), ("A", MIN_A)):
        used = [w for w, d in data[layer].items() if len(d["req"]) >= mn]
        counts[f"req_{layer}_with_control"] = sum(len(d["req"]) for d in data[layer].values())
        counts[f"writers_{layer}"] = len(used)
        counts[f"writers_{layer}_with_any_onset"] = sum(
            1 for w in used if any(v["onset30"] for v in data[layer][w]["req"] + data[layer][w]["ctrl"]))
        counts[f"requests_{layer}"] = sum(len(data[layer][w]["req"]) for w in used)
        counts[f"controls_{layer}"] = sum(len(data[layer][w]["ctrl"]) for w in used)
        counts[f"onsets_req_{layer}"] = sum(int(v["onset30"]) for w in used for v in data[layer][w]["req"])
        counts[f"onsets_ctrl_{layer}"] = sum(int(v["onset30"]) for w in used for v in data[layer][w]["ctrl"])
    counts["onsets_total"] = sum(x["onsets"] for x in onset_stats.values())
    json.dump(counts, open(os.path.join(out, "counts.json"), "w"), indent=1)
    # 書き手の ID（MTurk の ID）は出さない。並びは ID の順で、番号だけを付ける
    json.dump([dict(writer=i + 1, **onset_stats[w]) for i, w in enumerate(sorted(onset_stats))],
              open(os.path.join(out, "onsets_by_writer.json"), "w"), indent=1)

    res = {}
    for layer, mn in (("L", MIN_L), ("A", MIN_A)):
        used = [w for w in sorted(data[layer]) if len(data[layer][w]["req"]) >= mn]
        res[layer] = {}
        for sig in SIGNALS:
            per = [A.auc([v[sig] for v in data[layer][w]["req"]], [v[sig] for v in data[layer][w]["ctrl"]]) for w in used]
            per = [a for a in per if a is not None]
            if per:
                r = A.boot_median(per, random.Random(seed + 1))
                r["share_gt_05"] = sum(1 for a in per if a > 0.5) / len(per)
                res[layer][sig] = r
            else:
                res[layer][sig] = None
    json.dump(res, open(os.path.join(out, "auc.json"), "w"), indent=1)
    # 感度（事後に足した・判定に使わない）: 要求ごとに自分の対照との勝率を出し、書き手の中で平均する
    res2 = {}
    for layer, mn in (("L", MIN_L), ("A", MIN_A)):
        used = [w for w in sorted(data[layer]) if len(data[layer][w]["req"]) >= mn]
        res2[layer] = {}
        for sig in SIGNALS:
            per = []
            for w in used:
                wins = [A.auc([v[sig]], [c[sig] for c in cs]) for v, cs in data[layer][w]["pairs"]]
                per.append(sum(wins) / len(wins))
            r = A.boot_median(per, random.Random(seed + 1))
            r["share_gt_05"] = sum(1 for a in per if a > 0.5) / len(per)
            res2[layer][sig] = r
    json.dump(res2, open(os.path.join(out, "auc_per_request.json"), "w"), indent=1)
    print(json.dumps(counts))


if __name__ == "__main__":
    main()
