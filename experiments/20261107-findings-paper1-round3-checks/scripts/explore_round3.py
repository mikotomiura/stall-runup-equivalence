"""X16（尺度づけ φ の床と天井）と X17（停滞の定義と直前の打鍵の量）。登録の外の探索・取り置き・判定に使わない。

条件は ../conditions.md（計算の前にコミット）。集計の値だけを書く。
使い方: python -I explore_round3.py <取り置きの中間> <holdout.json> <confirm_holdout.json> <explore_learner_features.json>
        <gse-replay> <作業の dir（repo の外）> <出力の json> [jobs]
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
import build_cs1 as bc  # noqa: E402
import confirm_p1 as C  # noqa: E402

SEED = 20261106
TOL = 1e-12
BETA = {"f1": 150.0, "f3": 0.06, "f5": 2.0, "f6": 0.08}
PHI = ("f1", "f3", "f5", "f6")
LEVEL_COLS = {"level_presses": 0, "level_char_presses_per_min": 8}
REPLAY_SHA = "26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3"


def phi(f, b):
    return min(1.0, max(0.0, (f - b) / (2 * b)))


def x_of(r):
    p = {k: phi(r[k], BETA[k]) for k in PHI}
    return min(1.0, max(0.0, 0.30 * p["f3"] + 0.25 * p["f6"] + 0.25 * p["f1"] + 0.20 * p["f5"]))


# ---------------- 行（h1_writer の写し・s を添える） ----------------

def h1_writer_s(units, rng, cnt, cfg):
    """C.h1_writer と同じ順で同じ乱数を使い、停滞の行ごとに (k, u, s0) を添える（feats は常に計算する）。"""
    stalls, cands = [], {}
    for k, w in enumerate(units):
        anchors = C.stall_anchors(w, cfg)
        cnt["stall_anchors"] += len(anchors)
        if cfg["stall_condA"]:
            cnt["stall_excluded_condA_true"] += len(C.stall_anchors(w, dict(cfg, stall_condA=False))) - len(anchors)
        for u, s0, nxt, i, _ in anchors:
            bs = C.pf.block_start(w, u)
            if bs is None or u - bs < C.WARM:
                cnt["stall_warm"] += 1
                continue
            sv = C.sig4(w, u, cfg["d_win"])
            if sv is None:
                cnt["stall_no_signal"] += 1
                continue
            cell = (C.align_of(w, i, cfg, True), C.layer_of(w, u, nxt, cfg["layer"]))
            sv |= C.pause_info(w, u, nxt, True)
            stalls.append(((k, u), sv, cell, C.feats18(w, u, None), s0))
        for i in range(len(w.text) - 1):
            if not C.control_ok(w, i, cfg, True, cnt):
                continue
            u, v = w.text[i], w.text[i + 1]
            sv = C.sig4(w, u, cfg["d_win"])
            if sv is None:
                cnt["control_no_signal"] += 1
                continue
            cell = (C.align_of(w, i, cfg, True), C.layer_of(w, u, v, cfg["layer"]))
            sv |= C.pause_info(w, u, v, True)
            cands.setdefault(cell, []).append(((k, u), sv, C.feats18(w, u, None)))
            cnt["control_candidates"] += 1
    rows, meta = [], []
    for sid, sv, cell, fv, s0 in stalls:
        pool = cands.get(cell, [])[:]
        rng.shuffle(pool)
        ctrl = pool[:C.N_MATCHED]
        if not ctrl:
            cnt["stall_no_control"] += 1
            continue
        cnt["stall_rows_%s_%s" % cell[:2]] += 1
        rows.append((cell, sv, fv, ctrl))
        meta.append((sid[0], sid[1], s0))
    return rows, meta


def rows_with_s(writers):
    cnt = Counter()
    rng = random.Random(SEED)
    out = [h1_writer_s(u, rng, cnt, C.MAIN) for u in writers]
    return [r for r, _ in out], [m for _, m in out], dict(cnt)


def gate_g0(writers, rows, conf_counts, cnt):
    cnt0 = Counter()
    rng = random.Random(SEED)
    ref = [C.h1_writer(u, rng, cnt0, C.MAIN, feats=True) for u in writers]
    same = json.dumps(ref, default=list) == json.dumps(rows, default=list)
    return {"rows_equal_h1_writer": same, "counts_equal_confirm": cnt == conf_counts, "counts_equal_h1_writer": dict(cnt0) == cnt}


# ---------------- replay の量（X16） ----------------

def unit_paths(work, writers):
    """load_cs1 と同じ順で、各単位の中間のパスを並べる（観測 10 分未満で除いた書き手は飛ばす）。"""
    uw = json.load(open(os.path.join(work, "units.json")))
    by_w = {}
    for p in sorted(glob.glob(os.path.join(work, "u[0-9]*.json"))):
        by_w.setdefault(uw[os.path.basename(p)[1:-5]], []).append(p)
    out, wi = [], 0
    for wid in sorted(by_w):
        if wi < len(writers) and len(by_w[wid]) == len(writers[wi]) and all(
                json.load(open(p))["keys"] == w.keys for p, w in zip(by_w[wid], writers[wi])):
            out.append(by_w[wid])
            wi += 1
    if wi != len(writers):
        sys.exit("中間のパスと書き手の対応が取れない")
    return out


def replay_unit(args):
    p, replay, tmp = args
    d = json.load(open(p))
    keys = [tuple(k) for k in d["keys"]]
    rows = []
    with tempfile.TemporaryDirectory(dir=tmp) as td:
        nd = os.path.join(td, "b.ndjson")
        for a, b in d["blocks"]:
            presses = [k for k in keys if a <= k[0] <= b]
            bc.write_ndjson(nd, presses[0][0], bc.key_rows(presses))
            r = subprocess.run([replay, nd], stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
            if r.returncode != 0:
                return p, None
            rows += [{k: x[k] for k in ("timestamp", "p_stuck", "ewma_x", "raw_x") + PHI} for x in json.loads(r.stdout)]
    same = [[x["timestamp"], x["p_stuck"], x["ewma_x"]] for x in rows] == d["feat"]
    return p, (rows if same else "mismatch")


def x16(writers, rows, meta, paths, replay, tmp, jobs):
    flat = [p for ws in paths for p in ws]
    with ProcessPoolExecutor(jobs) as ex:
        res = dict(ex.map(replay_unit, [(p, replay, tmp) for p in flat]))
    bad = [p for p, r in res.items() if not isinstance(r, list)]
    g1 = {"units": len(flat), "units_feat_identical": len(flat) - len(bad)}
    if bad:
        sys.exit("X16 の門 G1 が外れた: " + json.dumps(g1))
    worst = max(abs(x_of(r) - r["raw_x"]) for rs in res.values() for r in rs)
    g2 = {"max_abs_x_minus_raw_x": worst}
    if worst > 1e-9:
        sys.exit("X16 の門 G2 が外れた: " + json.dumps(g2))

    def at(wi, k, t):
        w = writers[wi][k]
        i = bisect.bisect_right(w.feat_t, t)
        return res[paths[wi][k]][i - 1]

    tally = {g: Counter() for g in ("stall", "control")}
    n = Counter()
    for wi, (wr, wm) in enumerate(zip(rows, meta)):
        seen = set()
        for (cell, _, _, ctrl), (k, u, _) in zip(wr, wm):
            if cell[1] != "none":
                continue
            items = [("stall", k, u)] + [("control", cid[0], cid[1]) for cid, _, _ in ctrl
                                         if (cell, cid) not in seen and not seen.add((cell, cid))]
            for g, kk, uu in items:
                a, b = at(wi, kk, uu), at(wi, kk, uu - 30_000)
                n[g] += 1
                unchanged_all = True
                for f in PHI:
                    pa, pb = phi(a[f], BETA[f]), phi(b[f], BETA[f])
                    tally[g]["%s_zero_at_u" % f] += pa == 0.0
                    tally[g]["%s_one_at_u" % f] += pa == 1.0
                    tally[g]["%s_zero_at_u-30s" % f] += pb == 0.0
                    tally[g]["%s_one_at_u-30s" % f] += pb == 1.0
                    tally[g]["%s_change_zero" % f] += pa == pb
                    unchanged_all &= pa == pb
                tally[g]["all_four_change_zero"] += unchanged_all
    shares = {g: {k: v / n[g] for k, v in sorted(tally[g].items())} for g in tally}
    sat = max(max(shares[g]["%s_zero_at_%s" % (f, t)] + shares[g]["%s_one_at_%s" % (f, t)]
                  for f in PHI for t in ("u", "u-30s")) for g in shares)
    reading = ("half_or_more_saturated" if sat >= 0.5 else "all_below_quarter" if sat < 0.25 else "between")
    return {"gate_g1": g1, "gate_g2": g2, "rows": dict(n), "shares": shares, "max_saturated_share": sat,
            "reading_rule": reading}


# ---------------- X17 ----------------

def auc_block(rows):
    X, y, wt, g, cells, cids, miss = C.ml_rows(rows)
    keys = [(g[i], cells[i]) for i in range(len(y))]
    out = {"rows": int(len(y)), "stalls": int(y.sum())}
    for name, j in LEVEL_COLS.items():
        ok = np.isfinite(X[:, j])
        out[name] = C.paired_auc(X[ok, j], y[ok], [kk for kk, m in zip(keys, ok) if m], [c for c, m in zip(cids, ok) if m])
    sig = {s: [] for s in C.SIGS}
    for r in rows:
        for c, sv, fv, ctrl in r:
            if c[1] != "none":
                continue
            for s in sig:
                sig[s].append(sv[s])
            for _, cs, _ in ctrl:
                for s in sig:
                    sig[s].append(cs[s])
    for s in sig:
        out[s] = C.paired_auc(np.asarray(sig[s]), y, keys, cids)
    return out


def x17(rows, meta, x15):
    full = auc_block(rows)
    g3 = {k: abs(full[k] - x15["features"][k]["paired_auc_eps0"]) <= TOL for k in LEVEL_COLS}
    if not all(g3.values()):
        sys.exit("X17 の門 G3 が外れた: " + json.dumps(g3))
    s30 = [[r for r, m in zip(wr, wm) if m[2] - m[1] == 30_000] for wr, wm in zip(rows, meta)]
    lt30 = [[r for r, m in zip(wr, wm) if m[2] - m[1] < 30_000] for wr, wm in zip(rows, meta)]
    other = sum(1 for wm in meta for m in wm if m[2] - m[1] > 30_000)
    a30, alt = auc_block(s30), auc_block(lt30)
    v = a30["level_presses"]
    reading = ("too_few" if a30["stalls"] < 200 else "explainable_by_definition" if abs(v - 0.5) <= 0.02
               else "not_explained_by_definition_alone" if v <= 0.46 else "between")
    return {"gate_g3": g3, "all": full, "s_minus_u_eq_30s": a30, "s_minus_u_lt_30s": alt,
            "stall_rows_s_minus_u_gt_30s": other, "reading_rule": reading}


def main():
    hw, hold_p, conf_p, x15_p, replay, tmp, out_p = sys.argv[1:8]
    jobs = int(sys.argv[8]) if len(sys.argv) > 8 else 4
    import hashlib
    if hashlib.sha256(open(replay, "rb").read()).hexdigest() != REPLAY_SHA:
        sys.exit("gse-replay が論文の版でない")
    if subprocess.run(["git", "-C", tmp, "rev-parse"], capture_output=True).returncode == 0:
        sys.exit("作業の dir は repo の外に置く")
    writers, chk = C.load_cs1(hw, hold_p, "holdout")
    rows, meta, cnt = rows_with_s(writers)
    conf = json.load(open(conf_p))
    g0 = gate_g0(writers, rows, conf["h1_counts"], cnt)
    if not all(g0.values()):
        sys.exit("門 G0 が外れた: " + json.dumps(g0))
    paths = unit_paths(hw, writers)
    out = {"checks_load": chk, "gate_g0": g0,
           "x17": x17(rows, meta, json.load(open(x15_p))),
           "x16": x16(writers, rows, meta, paths, replay, tmp, jobs)}
    json.dump(out, open(out_p, "w"), indent=1)
    print(json.dumps({"g0": g0, "x16_reading": out["x16"]["reading_rule"], "x17_reading": out["x17"]["reading_rule"]}))


if __name__ == "__main__":
    main()
