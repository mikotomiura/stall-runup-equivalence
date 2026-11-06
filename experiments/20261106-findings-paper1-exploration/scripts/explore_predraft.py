"""草稿の前に数える量（conditions.md X9・登録の外の探索・取り置き・判定に使わない）。

(a) 編集の出来事の写し方（生データの取り置きの書き手の File.Edit 行・文字は長さだけ）。
    門: map_2019 の分け方の数が取り置きの中間の build_summary.json の row_kinds と全て一致する。
(b) 計器の全時間の挙動と u の後の値。門: 主の設定の H1 の点推定が確認の値と一致・主の層の停滞 4,554 行。
(c) 位置の 3 区分の実効の書き手。門: 点推定が explore_position.json と一致・主の重みが確認の重みと一致。
集計の値だけを書く。書き手ごとの値は出さない。
使い方: python -I explore_predraft.py <keystrokes.csv> <取り置きの中間> <holdout.json> <confirm_holdout.json>
        <explore_position.json> <出力の json>
"""
import bisect
import csv
import json
import math
import os
import sys
from collections import Counter

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261104-cs1-landscape", "scripts"))
sys.path.insert(0, HERE)
import confirm_p1 as C  # noqa: E402
import build_cs1 as B  # noqa: E402
import explore_position as XP  # noqa: E402

SEED = 20261106
OFFSETS = (-30_000, 0, 10_000, 20_000, 29_000)
TOL = 1e-12


# ---------------- (a) ----------------

def nlen(v):
    return 0 if B.nanv(v) else len(v)


def edit_class(row):
    """File.Edit の行の区分（map_2019 の分け方に長さを重ねる）。"""
    edit, ks = row["EditType"], row["X-Keystroke"]
    keyed = not B.nanv(ks)
    if edit == "Insert":
        n = nlen(row["InsertText"])
        it = "" if B.nanv(row["InsertText"]) else row["InsertText"]
        size = "len1" if n == 1 else ("len0" if n == 0 else ("len2plus_ws" if it.strip() == "" else "len2plus_other"))
        return ("insert_keyed_" if keyed else "insert_auto_") + size
    if edit == "Delete":
        n = nlen(row["DeleteText"])
        size = "len1" if n == 1 else ("len0" if n == 0 else "len2plus")
        return ("delete_keyed_" if keyed else "delete_auto_") + size
    if edit in ("Undo", "Redo"):
        return "undo_redo_" + ("keyed" if keyed else "auto")
    return {"Paste": "paste", "Cut": "cut", "Drag": "drag"}.get(edit, "other_" + (edit or "none"))


def part_a(csv_p, hold_p, work):
    hold = set(json.load(open(hold_p))["2019"])
    csv.field_size_limit(10**9)
    kinds, cls, chars, chars_by = Counter(), Counter(), Counter(), Counter()
    with open(csv_p, newline="") as f:
        for row in csv.DictReader(f):
            if row["SubjectID"] not in hold:
                continue
            kind, val = B.map_2019(row)
            kinds[kind + ":" + val if kind != "key" else "key"] += 1
            if row["EventType"] != "File.Edit":
                continue
            c = edit_class(row)
            cls[c] += 1
            if row["EditType"] in ("Insert", "Paste"):
                chars["multi" if c.startswith("insert_") and "len2plus" in c or c == "paste" else "single"] += nlen(row["InsertText"])
                chars_by[c] += nlen(row["InsertText"])
    expect = json.load(open(os.path.join(work, "build_summary.json")))["row_kinds"]
    gate = {"pass": dict(kinds) == expect, "mismatch": {k: (kinds.get(k), expect.get(k)) for k in set(kinds) | set(expect)
                                                        if kinds.get(k) != expect.get(k)}}
    tot = sum(cls.values())
    multi_ins = sum(v for k, v in cls.items() if k.startswith("insert_") and "len2plus" in k) + cls["paste"]
    range_del = cls["delete_keyed_len2plus"] + cls["delete_auto_len2plus"] + cls["cut"]
    out = {"gate": gate, "file_edit_rows": tot, "counts": dict(sorted(cls.items())),
           "shares": {k: v / tot for k, v in sorted(cls.items())},
           "multi_char_insert_incl_paste_share": multi_ins / tot, "range_delete_incl_cut_share": range_del / tot,
           "inserted_chars": dict(chars), "inserted_chars_by_class": dict(sorted(chars_by.items())),
           "inserted_chars_multi_share": chars["multi"] / (chars["multi"] + chars["single"])
           if chars["multi"] + chars["single"] else None}
    return out


# ---------------- (b) ----------------

def finite(x):
    return x is not None and isinstance(x, (int, float)) and math.isfinite(x)


def qs(v):
    v = np.asarray(v, dtype=float)
    return {"n": int(len(v)), "mean": float(v.mean()), "sd": float(v.std(ddof=1)),
            **{"q%02d" % int(q * 100): float(np.quantile(v, q)) for q in (0.1, 0.5, 0.9)}}


def fresh(w, T):
    r = w.snap(T)
    if r is None or not (T - r[0] < 1000) or not (finite(r[1]) and finite(r[2])):
        return None
    return r


def part_b(writers, conf):
    ps, ex, dps, dex, wfloor = [], [], [], [], []
    for units in writers:
        n = fl = 0
        for w in units:
            ft = w.feat_t
            for t, p, e in w.feat:
                if not (finite(p) and finite(e)):
                    continue
                ps.append(p)
                ex.append(e)
                n += 1
                fl += p < C.FLOOR
                j = bisect.bisect_right(ft, t - 30_000)
                if j > 0:
                    b = w.feat[j - 1]
                    if finite(b[1]) and finite(b[2]):
                        dps.append(p - b[1])
                        dex.append(e - b[2])
        if n:
            wfloor.append(fl / n)
    ps, ex, dps, dex = map(np.asarray, (ps, ex, dps, dex))
    full = {"rows": int(len(ps)), "p_stuck": qs(ps), "ewma_x": qs(ex),
            "p_stuck_floor_share": float((ps < C.FLOOR).mean()), "p_stuck_ceiling_share": float((ps > 0.98).mean()),
            "d_p_stuck": qs(dps) | {"zero_share": float((dps == 0).mean())},
            "d_ewma_x": qs(dex) | {"zero_share": float((dex == 0).mean())},
            "writer_floor_share_quantiles": {"q%02d" % int(q * 100): float(np.quantile(wfloor, q)) for q in (0.1, 0.5, 0.9)},
            "writers": len(wfloor)}
    # 陽性対照と停滞の行
    rows, msum = C.h1_run(writers, C.MAIN, SEED, hide=False)
    rep = {s + "_" + e: (msum["H1"][s][e]["pooled_auc"], conf["H1"][s][e]["pooled_auc"]) for s in C.SIGS for e in ("eps_0.001", "eps_0")}
    n_none = sum(1 for r in rows for cell, *_ in r if cell[1] == "none")
    gate = {"auc_reproduced": all(abs(a - b) < TOL for a, b in rep.values()), "stall_rows_none": n_none,
            "pass": all(abs(a - b) < TOL for a, b in rep.values()) and n_none == 4554}
    # u の後: 停滞の行の (課題の番号, u) から単位を引く
    after = {o: {"p": [], "e": [], "missing": 0} for o in OFFSETS}
    for units, r in zip(writers, rows):
        for cell, sv, _, _ in r:
            if cell[1] != "none":
                continue
            k, u = sv_id(r, sv)
            w = units[k]
            for o in OFFSETS:
                x = fresh(w, u + o)
                if x is None:
                    after[o]["missing"] += 1
                    continue
                after[o]["p"].append(x[1])
                after[o]["e"].append(x[2])
    aft = {}
    for o, d in after.items():
        p, e = np.asarray(d["p"]), np.asarray(d["e"])
        aft["%+d_s" % (o // 1000)] = {"n": int(len(p)), "missing": d["missing"],
                                       "p_stuck_mean": float(p.mean()), "p_stuck_median": float(np.median(p)),
                                       "p_stuck_floor_share": float((p < C.FLOOR).mean()),
                                       "ewma_x_mean": float(e.mean()), "ewma_x_median": float(np.median(e))}
    return {"gate": gate, "main_values": rep, "full_time": full, "stall_after_u": aft}, rows


def sv_id(r, sv):
    raise RuntimeError("part_b_wrapped から呼ぶ")


# ---------------- (c) ----------------

def part_c(writers, conf, pos_res, main_rows):
    C.align_of = XP.align_wrap
    try:
        rows, summ = C.h1_run(writers, C.cfg_of(align="pos3"), SEED, hide=False)
    finally:
        C.align_of = XP._orig_align
    prev = pos_res["pos3_matched"]["H1"]
    rep = {s + "_" + e: (summ["H1"][s][e]["pooled_auc"], prev[s][e]["pooled_auc"]) for s in C.SIGS for e in ("eps_0.001", "eps_0")}
    wmain = C.weight_concentration([C.h1_pairs(r, "d_p_stuck", "none")[1] for r in main_rows])
    cw = conf["H1"]["d_p_stuck"]["weights"]
    main_ok = all(abs(wmain[k] - cw[k]) < 1e-9 for k in cw)
    wpos = C.weight_concentration([C.h1_pairs(r, "d_p_stuck", "none")[1] for r in rows])
    return {"gate": {"auc_reproduced": all(abs(a - b) < TOL for a, b in rep.values()), "main_weights_reproduced": main_ok,
                     "pass": all(abs(a - b) < TOL for a, b in rep.values()) and main_ok},
            "pos3_weights": wpos, "pos3_pairs": summ["H1"]["d_p_stuck"]["eps_0.001"]["pairs"],
            "pos3_ci_width": {s + "_" + e: summ["H1"][s][e]["ci_width"] for s in C.SIGS for e in ("eps_0.001", "eps_0")}}


def main(csv_p, work, hold_p, conf_p, pos_p, out):
    conf = json.load(open(conf_p))
    res = {"a_edit_mapping": part_a(csv_p, hold_p, work)}
    writers, chk = C.load_cs1(work, hold_p, "holdout")
    res["checks"] = chk
    # 停滞の行の id を取るため、h1_writer が作る sv に (k, u) を紐づける
    orig_pause = C.pause_info

    def pause_wrap(w, u, end, cs1):
        d = orig_pause(w, u, end, cs1)
        d["_wu"] = (id(w), u)
        return d
    C.pause_info = pause_wrap
    try:
        b, main_rows = part_b_wrapped(writers, conf)
    finally:
        C.pause_info = orig_pause
    res["b_instrument"] = b
    res["c_pos3_effective_writers"] = part_c(writers, conf, json.load(open(pos_p)), main_rows)
    gates = {k: res[k]["gate"]["pass"] for k in ("a_edit_mapping", "b_instrument", "c_pos3_effective_writers")}
    for k, ok in gates.items():
        if not ok:
            res[k] = {"gate": res[k]["gate"], "withheld": True}
    res["gates"] = gates
    json.dump(res, open(out, "w"), indent=1, ensure_ascii=False)
    print(json.dumps(gates))
    if not all(gates.values()):
        sys.exit("門で止めた: " + json.dumps(gates))


def part_b_wrapped(writers, conf):
    """part_b の u の後を、sv に紐づけた (id(w), u) から引く版。"""
    global sv_id
    wmap = {}
    for units in writers:
        for k, w in enumerate(units):
            wmap[id(w)] = k

    def sid(r, sv):
        wid, u = sv["_wu"]
        return wmap[wid], u
    sv_id = sid
    return part_b(writers, conf)


if __name__ == "__main__":
    main(*sys.argv[1:7])
