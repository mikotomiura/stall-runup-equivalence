"""推定器の陽性対照（conditions.md X10・登録の外の探索・見た書き手・判定に使わない）。

停滞の手前の窓 (u − 30 秒, u) の打鍵に既知の変化（S: 間隔を k 倍・C: 訂正の対を割合 r で足す）を入れ、
replay（--features なし）で推定の行だけを作り直して、主の層の H1 の型の AUC が帯の外へ動くかを見る。
打鍵の列・onset・区切り・出来事は中間のまま（停滞と対照の選び方は変えない）。
門: G1 変化なしの作り直し = 中間の feat（全課題）／G2 件数と書き手ごとの対の数が基準と同じ／
    G3 基準の対の数・同値の割合・区間の幅 = confirm_seen.json。
集計の値だけを書く。書き手ごとの値は出さない。
使い方: python -I explore_positive_control.py <見た書き手の中間> <holdout.json> <gse-replay> <confirm_seen.json>
        <出力の json> [--mutate-m1]
"""
import bisect
import glob
import json
import os
import random
import subprocess
import sys
import tempfile
from collections import Counter
from concurrent.futures import ProcessPoolExecutor

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261104-cs1-landscape", "scripts"))
import confirm_p1 as C  # noqa: E402
import build_cs1 as B  # noqa: E402

SEED = 20261106
WIN = 30_000
CONDS = [("base", None), ("noop", None), ("S", 1.5), ("S", 2.0), ("S", 3.0), ("C", 0.1), ("C", 0.25), ("C", 0.5)]
# X13（判断 36）: 小さい側の段と、対照を基準の値に固定した版
CONDS_V2 = [("base", None), ("noop", None), ("S", 1.02), ("S", 1.05), ("S", 1.1), ("S", 1.2), ("S", 1.5), ("S", 2.0),
            ("S", 3.0), ("C", 0.02), ("C", 0.05), ("C", 0.1), ("C", 0.25), ("C", 0.5)]


def fixed_rows(base_rows, rows):
    """停滞の値は rows から、引かれた対照の値は base_rows から取った行（対照の組と引きは同じ・G2 で確かめた後に呼ぶ）。"""
    out = []
    for rb, rn in zip(base_rows, rows):
        out.append([(cn, sn, fn, ctb) for (cb, sb, fb, ctb), (cn, sn, fn, ctn) in zip(rb, rn)])
    return out


def cname(kind, x):
    return kind if x is None else "%s_%g" % (kind, x)


def perturb(keys, anchors, kind, x, tag):
    """keys: [[t, cat]]（時刻順）。anchors: u の昇順。窓 (u − 30 秒, u) の押下を変えた新しい列を返す。"""
    if kind in ("base", "noop"):
        return [list(k) for k in keys]
    press = [k[0] for k in keys]
    out, j = [], 0
    for u in anchors:
        lo = bisect.bisect_right(press, u - WIN)  # 窓の最初（u − 30 秒 より後）
        hi = bisect.bisect_left(press, u)         # u の押下は動かさない
        out += [list(k) for k in keys[j:lo]]
        win = keys[lo:hi]
        if kind == "S":
            for t, c in win:
                if t > u - WIN / x:
                    out.append([int(round(u - x * (u - t))), c])
        else:  # C
            rng = random.Random("x10:%d:%s:%d:%g" % (SEED, tag, u, x))
            seq = win + ([keys[hi]] if hi < len(keys) else [])
            seq = [k for k in seq if k[0] >= u - WIN]
            for a, b in zip(seq, seq[1:]):
                out.append(list(a))
                if b[0] - a[0] >= 3 and rng.random() < x:
                    gap = b[0] - a[0]
                    out.append([a[0] + gap // 3, "backspace"])
                    out.append([a[0] + 2 * gap // 3, b[1]])
            if seq and seq[-1][0] < u:
                out.append(list(seq[-1]))
        j = hi
    out += [list(k) for k in keys[j:]]
    out.sort(key=lambda k: k[0])  # 安定（同じ時刻の順は保つ）
    return out


def rebuild(args):
    path, anchors, kind, x, mutate, replay = args
    d = json.load(open(path))
    feat, allk = [], []
    for a, z in d["blocks"]:
        bk = [k for k in d["keys"] if a <= k[0] <= z]
        # 窓がまとまりの頭をまたぐ u（頭から 30 秒以内・主の行では区切りの頭から 60 秒の除外で落ちる）には入れない
        an = [u for u in anchors if a + WIN <= u <= z]
        bk = perturb(bk, an, kind, x, os.path.basename(path))
        allk += bk
    if mutate:  # 変異 m1: 押下を 10 個に 1 個落とす（1 ms ずらすだけでは量子化で推定の行が変わらなかった）
        allk = [k for i, k in enumerate(allk) if i % 10]
    with tempfile.TemporaryDirectory() as td:
        for a, z in d["blocks"]:
            pr = [(t, c) for t, c in allk if a <= t <= z]
            if len(pr) < 2:
                continue
            nd = os.path.join(td, "b.ndjson")
            B.write_ndjson(nd, pr[0][0], B.key_rows(pr))
            r = subprocess.run([replay, nd], capture_output=True, stdin=subprocess.DEVNULL)
            if r.returncode != 0 or r.stderr:
                return path, None
            feat += [[q["timestamp"], q["p_stuck"], q["ewma_x"]] for q in json.loads(r.stdout)]
    return path, feat


def seen_paths(work, hold_p):
    """load_cs1 と同じ順の課題のファイル（観測 10 分以上の書き手だけ）。"""
    uw = json.load(open(os.path.join(work, "units.json")))
    by_w = {}
    for p in sorted(glob.glob(os.path.join(work, "u[0-9]*.json"))):
        by_w.setdefault(uw[os.path.basename(p)[1:-5]], []).append(p)
    out = []
    for wid in sorted(by_w):
        ps = by_w[wid]
        obs = 0
        for p in ps:
            obs += sum(b - a for a, b in json.load(open(p))["observed_intervals"])
        if obs >= C.MIN_WRITER_OBS:
            out.append(ps)
    return out


ORIG_PAUSE_INFO = C.pause_info


def pause_info_u(w, u, end, cs1):
    r = ORIG_PAUSE_INFO(w, u, end, cs1)
    r["_wu"] = (id(w), u)
    return r


def window_desc(w, u):
    """(u − 30 秒, u] の文字の押下の数・押下の間隔の平均・backspace の割合（元の打鍵）。"""
    lo, hi = bisect.bisect_right(w.press, u - WIN), bisect.bisect_right(w.press, u)
    ts, cs = w.press[lo:hi], w.cats[lo:hi]
    n_text = sum(1 for c in cs if c in C.TEXT)
    iki = float(np.mean(np.diff(ts))) if len(ts) >= 2 else float("nan")
    bs = sum(1 for c in cs if c == "backspace") / len(cs) if cs else float("nan")
    return n_text, iki, bs


def main():
    argv = sys.argv[1:]
    mutate = "--mutate-m1" in argv
    v2 = "--v2" in argv
    argv = [a for a in argv if a not in ("--mutate-m1", "--v2")]
    v1_p = None
    if v2:
        global CONDS
        CONDS = CONDS_V2
        work, hold_p, replay, seen_p, out_p, v1_p = argv
        argv = argv[:5]
    work, hold_p, replay, seen_p, out_p = argv
    seen = json.load(open(seen_p))
    writers, chk = C.load_cs1(work, hold_p, "seen")
    paths = seen_paths(work, hold_p)
    assert len(paths) == len(writers) == 319, (len(paths), len(writers))
    for ps, units in zip(paths, writers):
        assert len(ps) == len(units)
        for p, w in zip(ps, units):
            d = json.load(open(p))
            assert d["keys"][0][0] == w.press[0] and len(d["keys"]) == len(w.press) and d["feat"] == w.feat
    flat = [(p, w) for ps, units in zip(paths, writers) for p, w in zip(ps, units)]
    anchors = {p: sorted({a[0] for a in C.stall_anchors(w, C.MAIN)}) for p, w in flat}
    orig_feat = {p: w.feat for p, w in flat}
    C.pause_info = pause_info_u
    res = {"checks_load": chk, "units": len(flat), "anchors": sum(len(v) for v in anchors.values()),
           "conditions": {}, "gates": {}}
    base_pairs = None
    for kind, x in CONDS:
        name = cname(kind, x)
        tasks = [(p, anchors[p], kind, x, mutate and kind == "base" and i == 0, replay) for i, (p, _) in enumerate(flat)]
        with ProcessPoolExecutor(int(os.environ.get("JOBS", "8"))) as ex:
            new = dict(ex.map(rebuild, tasks, chunksize=4))
        if any(v is None for v in new.values()):
            sys.exit("replay が失敗した: " + name)
        if kind == "base":
            mism = sum(1 for p in new if new[p] != orig_feat[p])
            res["gates"]["G1_rebuild_equal_units"] = len(flat) - mism
            res["gates"]["G1_mismatch_units"] = mism
            if mism:
                json.dump(res, open(out_p, "w"), indent=1)
                sys.exit("G1 が外れた: %d 課題" % mism)
        for p, w in flat:
            w.feat = new[p]
            w.feat_t = [f[0] for f in w.feat]
        rows, summ = C.h1_run(writers, C.MAIN, SEED, hide=True)
        summ["H1"] = C.h1_summary(rows, SEED, "none", False)
        pairs = [C.h1_pairs(r, "d_p_stuck")[1] for r in rows]
        if base_pairs is None:
            base_pairs = pairs
            base_rows = rows
        g2 = summ["counts"] == seen["h1_counts"] and pairs == base_pairs
        res["gates"]["G2_" + name] = g2
        if not g2:
            json.dump(res, open(out_p, "w"), indent=1)
            sys.exit("G2 が外れた: " + name)
        # u の値が基準から変わった停滞の行と、値が変わった対照
        ch_s = ch_c = n_s = n_c = 0
        for rb, rn in zip(base_rows, rows):
            for (cb, sb, _, ctb), (cn, sn, _, ctn) in zip(rb, rn):
                if cb[1] != "none":
                    continue
                n_s += 1
                ch_s += (sb["p_stuck"], sb["ewma_x"]) != (sn["p_stuck"], sn["ewma_x"])
                for (_, xb, _), (_, xn, _) in zip(ctb, ctn):
                    n_c += 1
                    ch_c += (xb["p_stuck"], xb["ewma_x"]) != (xn["p_stuck"], xn["ewma_x"])
        e = {"stall_rows_main_layer": n_s, "stall_rows_u_value_changed_share": ch_s / n_s if n_s else None,
             "control_draws_main_layer": n_c, "control_draws_value_changed": ch_c}
        for s in C.SIGS + C.LEVELS:
            h = summ["H1"][s]
            e[s] = {k: {"pooled_auc": h[k]["pooled_auc"], "ci95": h[k]["ci95"], "tie_share": h[k]["tie_share"],
                        "branch_form": h[k].get("branch")} for k in ("eps_0.001", "eps_0")}
        if v2:
            fr = fixed_rows(base_rows, rows)
            fs = C.h1_summary(fr, SEED, "none", False, C.SIGS + C.LEVELS, light=True)
            e["controls_fixed"] = {s: {k: {"pooled_auc": fs[s][k]["pooled_auc"], "ci95": fs[s][k]["ci95"]}
                                       for k in ("eps_0.001", "eps_0")} for s in C.SIGS + C.LEVELS}
        res["conditions"][name] = e
        if kind == "base":
            g3 = all(summ["H1"][s][k][f] == seen["H1"][s][k][f] for s in C.SIGS + C.LEVELS
                     for k in ("eps_0.001", "eps_0") for f in ("pairs", "tie_share", "ci_width"))
            res["gates"]["G3_seen_pairs_ties_width_equal"] = g3
            if not g3:
                json.dump(res, open(out_p, "w"), indent=1)
                sys.exit("G3 が外れた")
            # 変化の大きさを自然の差と並べる（元の打鍵の窓）
            ws = {id(w): w for _, w in flat}
            st, ct = [], {}
            for wi, r in enumerate(rows):
                for cell, sv, _, ctrl in r:
                    if cell[1] != "none":
                        continue
                    st.append(window_desc(ws[sv["_wu"][0]], sv["_wu"][1]))
                    for cid, cs, _ in ctrl:
                        ct[(wi, cid)] = window_desc(ws[cs["_wu"][0]], cs["_wu"][1])
            ct = list(ct.values())
            med = lambda v, j: float(np.nanmedian([x[j] for x in v]))  # noqa: E731
            res["natural_window_description"] = {
                g: {"n": len(v), "median_text_presses": med(v, 0), "median_mean_interval_ms": med(v, 1),
                    "median_backspace_share": med(v, 2), "mean_backspace_share": float(np.nanmean([x[2] for x in v]))}
                for g, v in (("stall", st), ("control_drawn_unique", ct))}
    # 読み（条件の規則）: 帯を越えた最も小さい段
    res["first_level_outside_band"] = {}
    for s in C.SIGS:
        for kind in ("S", "C"):
            hit = None
            for k2, x in CONDS:
                if k2 != kind:
                    continue
                ci = res["conditions"][cname(k2, x)][s]["eps_0.001"]["ci95"]
                if ci and (ci[0] > C.BAND_HI or ci[1] < C.BAND_LO):
                    hit = x
                    break
            res["first_level_outside_band"]["%s_%s" % (s, kind)] = hit
    res["noop_equals_base"] = res["conditions"]["noop"] == res["conditions"]["base"] or \
        all(res["conditions"]["noop"][s] == res["conditions"]["base"][s] for s in C.SIGS + C.LEVELS)
    res["noop_changed_share"] = res["conditions"]["noop"]["stall_rows_u_value_changed_share"]
    with open(out_p + ".tmp", "w") as f:
        json.dump(res, f, indent=1)
    os.replace(out_p + ".tmp", out_p)
    if v2:
        v1 = json.load(open(v1_p))["conditions"]
        res["gates"]["G4_v1_conditions_equal"] = all(
            res["conditions"][c][s] == v1[c][s] for c in v1 for s in C.SIGS + C.LEVELS)
        b = res["conditions"]["base"]
        res["gates"]["G5_fixed_equals_base_at_base"] = all(
            b["controls_fixed"][s][k]["pooled_auc"] == b[s][k]["pooled_auc"] for s in C.SIGS for k in ("eps_0.001", "eps_0"))
        res["first_level_outside_band_controls_fixed"] = {}
        for s in C.SIGS:
            for kind in ("S", "C"):
                hit = None
                for k2, x in CONDS:
                    if k2 != kind:
                        continue
                    ci = res["conditions"][cname(k2, x)]["controls_fixed"][s]["eps_0.001"]["ci95"]
                    if ci and (ci[0] > C.BAND_HI or ci[1] < C.BAND_LO):
                        hit = x
                        break
                res["first_level_outside_band_controls_fixed"]["%s_%s" % (s, kind)] = hit
        with open(out_p + ".tmp", "w") as f:
            json.dump(res, f, indent=1)
        os.replace(out_p + ".tmp", out_p)
        if not (res["gates"]["G4_v1_conditions_equal"] and res["gates"]["G5_fixed_equals_base_at_base"]):
            sys.exit("G4 か G5 が外れた")
    print(json.dumps({"gates": res["gates"], "first_level_outside_band": res["first_level_outside_band"]}))


if __name__ == "__main__":
    main()
