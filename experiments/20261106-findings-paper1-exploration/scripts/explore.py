"""登録の外の探索（論文 1 の事前登録の付録 F・conditions.md）。判定に使わない。

確認のスクリプト confirm_p1.py の関数をそのまま呼び、KLiCKe の全員と CS1 2021 年版の見た 31 人で
H1 の型と H2 の型の値を出す。集計の値だけを書く（書き手ごとの値は出さない）。
使い方: python -I explore.py <klicke の work> <cs1 2021 の work> <holdout.json> <出力の json> [klicke|cs1_2021|all]
"""
import glob
import hashlib
import json
import os
import random
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402

SEED = 20261126


def load_cs1_2021(work, hold_p):
    """confirm_p1.load_cs1 と同じ作りで年だけを 2021 にした版。中間に取り置きの書き手がいれば止める。"""
    hold = set(json.load(open(hold_p))["2021"])
    hh = {hashlib.sha256(h.encode()).hexdigest() for h in hold}
    wh = json.load(open(os.path.join(work, "writers.json")))
    uw = json.load(open(os.path.join(work, "units.json")))
    present = {wh[str(v)] for v in uw.values()}
    if present & hh:
        sys.exit("CS1 2021 年版の見た書き手の中間に取り置きの書き手がいる")
    by_w = {}
    for p in sorted(glob.glob(os.path.join(work, "u[0-9]*.json"))):
        by_w.setdefault(uw[os.path.basename(p)[1:-5]], []).append(p)
    chk = {"units": 0, "onset_mismatch_units": 0, "theta_mismatch": 0, "writers_with_units": len(by_w), "writers_excluded_obs_lt_10min": 0}
    writers = []
    for wid in sorted(by_w):
        units = []
        for p in by_w[wid]:
            d = json.load(open(p))
            w = C.pf.make_unit(d)
            w.blocks = d["blocks"]
            w.w_ms = C.k6.W_MS
            units.append(w)
            chk["units"] += 1
            if C.k6.replicate_onsets(w.ca, w.theta) != [list(o) for o in w.onsets]:
                chk["onset_mismatch_units"] += 1
            chk["theta_mismatch"] += sum(1 for s0, _, th, n in w.onsets if w.theta.at(s0) != (th, n))
        if sum(sum(b - a for a, b in w.intervals) for w in units) < C.MIN_WRITER_OBS:
            chk["writers_excluded_obs_lt_10min"] += 1
            continue
        writers.append(units)
    if chk["onset_mismatch_units"] or chk["theta_mismatch"]:
        sys.exit("照合が崩れた: " + json.dumps(chk))
    chk["writers_eligible"] = len(writers)
    return writers, chk


def run_set(writers, cs1):
    cnt = Counter()
    rng = random.Random(SEED)
    rows = [C.h1_writer(u, rng, cnt, C.MAIN, cs1=cs1) for u in writers]
    h1 = C.h1_summary(rows, SEED, "none", False, C.SIGS)
    rng2 = random.Random(SEED + 6)
    h2w = [C.h2_writer(u, rng2, C.MAIN, cs1=cs1) for u in writers]
    h2 = C.h2_summary(h2w, SEED, False)
    return {"counts": dict(cnt), "H1": h1, "H2": h2}


def strip_per_writer(o):
    # 書き手ごとの AUC の分位は集計（分位）なので残す。リストや書き手の番号の対応は出さない
    if isinstance(o, dict):
        return {k: strip_per_writer(v) for k, v in o.items()}
    if isinstance(o, list) and len(o) > 4:
        raise ValueError("長いリストは出さない")
    return o


def main(kwork, cwork, hold_p, out, which="all"):
    res = {"seed": SEED, "note": "登録の外の探索（付録 F）。判定に使わない。"}
    if which in ("all", "cs1_2021"):
        ws, chk = load_cs1_2021(cwork, hold_p)
        res["cs1_2021_seen"] = {"checks": chk} | run_set(ws, cs1=True)
    if which in ("all", "klicke"):
        paths = sorted(glob.glob(os.path.join(kwork, "w*.json")))
        ws = [[C.k6.Writer(p)] for p in paths]
        res["klicke"] = {"writers": len(ws)} | run_set(ws, cs1=False)
    res = strip_per_writer(res)
    with open(out + ".tmp", "w") as f:
        json.dump(res, f, indent=1, default=str)
    os.replace(out + ".tmp", out)


if __name__ == "__main__":
    main(*sys.argv[1:6])
