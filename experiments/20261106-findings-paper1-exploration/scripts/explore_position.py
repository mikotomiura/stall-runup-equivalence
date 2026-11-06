"""休止の位置（conditions.md X8・登録の外の探索・判定に使わない）。

(a) 主の設定で、停滞と対照の候補を 層 × 位置の 3 区分で数え、全員分の割合を出す。
    門: 停滞の数（層 × 改行か否か）が確認の結果の stall_rows_* + stall_no_control に、
    対照の候補の数が control_candidates に一致する。外れたら (b) を回さない。
(b) align_of を包み（cfg の align が pos3 のときだけ 3 区分）、H1 の型を確認と同じ種で出す。
集計の値だけを書く。書き手ごとの値は出さない。
使い方: python -I explore_position.py <取り置きの中間> <holdout.json> <confirm_holdout.json> <出力の json>
"""
import json
import os
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1 as C  # noqa: E402

SEED = 20261106  # 確認と同じ
BOUND = {"space", "tab", "punctuation", "sentence_punctuation"}
INSIDE = {"alphabetic", "numeric"}


def pos3(c):
    if c == "enter":
        return "newline"
    if c in BOUND:
        return "boundary"
    if c in INSIDE:
        return "within_token"
    raise ValueError("区分の外の文字の種類: %r" % (c,))


_orig_align = C.align_of


def align_wrap(w, i, cfg, cs1):
    if cfg.get("align") == "pos3":
        return pos3(w.text_cat[i])
    return _orig_align(w, i, cfg, cs1)


def count_positions(writers):
    cfg = C.MAIN
    st, ct = Counter(), Counter()
    for units in writers:
        for w in units:
            for u, s0, nxt, i, _ in C.stall_anchors(w, cfg):
                bs = C.pf.block_start(w, u)
                if bs is None or u - bs < C.WARM:
                    continue
                if C.sig4(w, u, cfg["d_win"]) is None:
                    continue
                st[(C.layer_of(w, u, nxt, cfg["layer"]), pos3(w.text_cat[i]))] += 1
            for i in range(len(w.text) - 1):
                if not C.control_ok(w, i, cfg, True):
                    continue
                u, v = w.text[i], w.text[i + 1]
                if C.sig4(w, u, cfg["d_win"]) is None:
                    continue
                ct[(C.layer_of(w, u, v, cfg["layer"]), pos3(w.text_cat[i]))] += 1
    return st, ct


def gate(st, ct, conf):
    cn = conf["h1_counts"]
    exp_st = {}
    for k, v in cn.items():
        if k.startswith("stall_rows_"):
            a, layer = k[len("stall_rows_"):].rsplit("_", 1)
            exp_st[(layer, a)] = v
    got = Counter()
    for (layer, p), v in st.items():
        got[(layer, "newline" if p == "newline" else "other")] += v
    rows_ok = all(got[k] >= v for k, v in exp_st.items())
    gap = sum(got.values()) - sum(exp_st.values())
    stall_ok = rows_ok and gap == cn.get("stall_no_control", 0)
    ctrl_ok = sum(ct.values()) == cn["control_candidates"]
    return {"stall_counted": sum(got.values()), "stall_rows_plus_no_control": sum(exp_st.values()) + cn.get("stall_no_control", 0),
            "control_counted": sum(ct.values()), "control_candidates": cn["control_candidates"],
            "pass": bool(stall_ok and ctrl_ok)}


def shares(cnt):
    out = {}
    for layer in ("none", "event", "all"):
        sub = {p: sum(v for (l, q), v in cnt.items() if q == p and (layer == "all" or l == layer))
               for p in ("newline", "boundary", "within_token")}
        n = sum(sub.values())
        out[layer] = {"n": n, **{p: (sub[p] / n if n else None) for p in sub}}
    return out


def main(work, hold_p, conf_p, out):
    conf = json.load(open(conf_p))
    writers, chk = C.load_cs1(work, hold_p, "holdout")
    st, ct = count_positions(writers)
    g = gate(st, ct, conf)
    res = {"checks": chk, "gate": g, "stall_shares": shares(st), "control_shares": shares(ct)}
    if not g["pass"]:
        json.dump(res, open(out, "w"), indent=1, ensure_ascii=False)
        sys.exit("門で止めた: " + json.dumps(g, ensure_ascii=False))
    _, msum = C.h1_run(writers, C.MAIN, SEED, hide=False)
    rep = {s + "_" + e: (msum["H1"][s][e]["pooled_auc"], conf["H1"][s][e]["pooled_auc"]) for s in C.SIGS for e in ("eps_0.001", "eps_0")}
    res["main_reproduced"] = {"values": rep, "pass": all(abs(a - b) < 1e-12 for a, b in rep.values())}
    if not res["main_reproduced"]["pass"]:
        json.dump(res, open(out, "w"), indent=1, ensure_ascii=False)
        sys.exit("主の設定が確認の値を再現しない: " + json.dumps(rep))
    C.align_of = align_wrap
    try:
        rows, summ = C.h1_run(writers, C.cfg_of(align="pos3"), SEED, hide=False)
    finally:
        C.align_of = _orig_align
    res["pos3_matched"] = summ
    json.dump(res, open(out, "w"), indent=1, ensure_ascii=False)
    print(json.dumps({k: res[k] for k in ("gate", "stall_shares", "control_shares")}, indent=1, ensure_ascii=False))
    for s in C.SIGS:
        for e in ("eps_0.001", "eps_0"):
            x = summ["H1"][s][e]
            print(s, e, round(x["pooled_auc"], 4), [round(v, 4) for v in x["ci95"]], "branch", x["branch"])


if __name__ == "__main__":
    main(*sys.argv[1:5])
