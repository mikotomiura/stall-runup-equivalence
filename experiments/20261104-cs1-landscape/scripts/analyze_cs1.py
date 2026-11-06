"""conditions.md D5〜D6（L1〜L6 と単位の記述）の記述の集計。検定はしない。

使い方:
  python3 -I analyze_cs1.py <work2019> <work2021> <holdout.json> <keystrokes_2019.csv> <keystrokes_2021.csv> <out_dir>
  python3 -I analyze_cs1.py --selftest
出力は集計値だけ（書き手・課題の ID も番号も出さない）: <out_dir>/cs1_landscape.json・counts.json・posthoc.json（事後・判定に使わない）。
元の俯瞰の analyze_landscape.py の Session と集計をそのまま使い、CS1 で変えるところ（D5）だけを上書きする。
"""
import bisect
import copy
import csv
from collections import Counter
import glob
import hashlib
import json
import tempfile
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../../20261102-public-data-landscape/scripts"))
import analyze_landscape as al  # noqa: E402

H = al.H
PROMPT_NO_ANSWER = 10 * 60_000  # D5
PROMPT_FALLBACK = 60_000
DAY = 86_400_000
L3_EVENTS = ("run", "switch", "paste")


def prompt_cover(events):
    """[(t, name)] → 問いの区間（併合済み）。D5。"""
    pr = sorted(t for t, n in events if n == "prompt")
    ans = sorted(t for t, n in events if n == "prompt-answer")
    out = []
    for t in pr:
        j = bisect.bisect_left(ans, t)
        end = ans[j] if j < len(ans) and ans[j] - t <= PROMPT_NO_ANSWER else t + PROMPT_FALLBACK
        out.append([t, end])
    return al.merge(out)


class Unit(al.Session):
    """1 課題。作業のまとまりの中だけで間隔を取り、問いの区間を「AI の近く」の代わりに使う。"""

    def __init__(self, d):
        ev = d["events"]
        kinds = {n: sorted(t for t, m in ev if m == n) for n in ("paste", "multi-insert")}
        super().__init__(d, receipts=kinds["paste"] + kinds["multi-insert"], ai=(), receipt_kinds=kinds)
        self.blocks = [tuple(b) for b in d["blocks"]]
        self.ev = {n: sorted(t for t, m in ev if m == n) for n in L3_EVENTS}
        self.ai_cover = prompt_cover(ev)
        self.n_prompt = sum(1 for _, n in ev if n == "prompt")
        self.ws_times = sorted(t for t, n in ev if n == "posthoc-ws")  # 事後の印（主の集計は読まない）
        self.first_event, self.last_event = d["span"]

    def span_ms(self):
        return sum(b - a for a, b in self.blocks)

    def normal_span_ms(self):
        return self.span_ms() - al.overlap_len([list(b) for b in self.blocks], self.ai_cover)

    def near_ai(self, a, b):
        i = bisect.bisect_right([c[0] for c in self.ai_cover], b)
        return i > 0 and self.ai_cover[i - 1][1] >= a

    def gaps(self):
        out = []
        for a, b in self.blocks:
            lo, hi = bisect.bisect_left(self.press, a), bisect.bisect_right(self.press, b)
            seg = self.press[lo:hi]
            out += list(zip(seg, seg[1:]))
        return out


def writer_l3(units, normal):
    rec = []
    for s in units:
        for st, t0, r, end in s.stalls():
            if normal and s.near_ai(st - al.PRE_WIN, end):
                continue
            pre = (st - al.PRE_WIN, t0)
            row = {}
            series = [("nontext", s.nontext), ("nav", s.nav), ("delete", s.delete),
                      ("other_nontext", s.other_nontext)] + [(n, s.ev[n]) for n in L3_EVENTS]
            for name, ts in series:
                row["pre_" + name] = al.count_in(ts, *pre) > 0
                row["post_" + name] = al.count_in(ts, t0, end, lo_open=True, hi_open=r is not None) > 0
            rec.append(row)
    return rec


def summarize_l3(writers, normal, has_nav):
    l3w = [writer_l3(w, normal) for w in writers]
    names = ["nontext", "delete", "other_nontext"] + (["nav"] if has_nav else []) + list(L3_EVENTS)
    out = {}
    for side in ("pre", "post"):
        for nm in names:
            key = side + "_" + nm
            allrows = [r[key] for rows in l3w for r in rows]
            out[key] = {"pooled": al.pooled(sum(allrows), len(allrows)),
                        "writer_median": al.quart([sum(r[key] for r in rows) / len(rows) for rows in l3w
                                                   if len(rows) >= al.MIN_STALLS_FOR_MEDIAN])}
    return out


def unit_l6(s):
    """課題の観測時間を 3 等分する（観測区間をつなげた時間で切る・D6）。"""
    ivs = al.merge([list(iv) for iv in s.intervals])
    tot = sum(b - a for a, b in ivs)
    if tot <= 0:
        return None
    # 観測時間の 1/3・2/3 にあたる時刻
    cuts, acc, k = [], 0, 1
    for a, b in ivs:
        while k < 3 and acc + (b - a) >= tot * k / 3:
            cuts.append(a + (tot * k / 3 - acc))
            k += 1
        acc += b - a
    bounds = [ivs[0][0]] + cuts + [ivs[-1][1]]
    rows = s.feat_in_obs()
    dfrom = s.theta.defined_from
    row = {}
    for k in range(3):
        a, b = bounds[k], bounds[k + 1]
        ob = al.overlap_len(ivs, [[a, b]])
        n = sum(1 for o in s.onsets if a <= o[1] < b or (k == 2 and o[1] == b))
        fr = [r for r in rows if a <= r[0] < b or (k == 2 and r[0] == b)]
        tdef = 0 if dfrom is None or dfrom >= b else al.overlap_len(ivs, [[max(a, dfrom), b]])
        xv = [r[2] for r in fr if r[2] is not None and math.isfinite(r[2])]
        row[k] = {"stall_per_h": n / (ob / H) if ob > 0 else None,
                  "p_ge": sum(1 for r in fr if r[1] >= al.P_TRIG) / len(fr) if fr else None,
                  "x_median": statistics.median(xv) if xv else None,
                  "theta_defined_frac": tdef / ob if ob > 0 else None}
    return row


def summarize_l6(writers):
    per_writer = []
    for w in writers:
        rows = [r for r in (unit_l6(s) for s in w) if r]
        if not rows:
            continue
        per_writer.append({k: {m: (statistics.median(v) if v else None) for m, v in
                               ((m, [r[k][m] for r in rows if r[k][m] is not None])
                                for m in ("stall_per_h", "p_ge", "x_median", "theta_defined_frac"))}
                           for k in range(3)})
    out = {nm: {m: al.quart([w[k][m] for w in per_writer]) for m in
                ("stall_per_h", "p_ge", "x_median", "theta_defined_frac")}
           for k, nm in enumerate(("early", "middle", "late"))}
    out["writers"] = len(per_writer)
    return out


def describe_units(writers):
    """単位の記述（D6 の最後）。"""
    us = [s for w in writers for s in w]
    return {"writers": len(writers), "units": len(us),
            "units_per_writer": al.quart([len(w) for w in writers]),
            "unit_obs_h": al.quart([s.obs_ms() / H for s in us]),
            "unit_blocks": al.quart([len(s.blocks) for s in us]),
            "block_len_min": al.quart([(b - a) / 60_000 for s in us for a, b in s.blocks]),
            "unit_span_days": al.quart([(s.last_event - s.first_event) / DAY for s in us]),
            "writer_obs_h": al.quart([sum(s.obs_ms() for s in w) / H for w in writers]),
            "units_theta_defined_obs_ge_5min": sum(1 for s in us if s.theta.defined_from is not None
                                                   and s.obs_ms() >= al.L5_MIN_OBS),
            "prompts": sum(s.n_prompt for s in us)}


def load(work):
    uw = json.load(open(os.path.join(work, "units.json")))
    by_w = {}
    for p in sorted(glob.glob(os.path.join(work, "u*.json"))):
        k = os.path.basename(p)[1:-5]
        if k not in uw:
            continue
        by_w.setdefault(uw[k], []).append(Unit(json.load(open(p))))
    return [by_w[w] for w in sorted(by_w)]


def check_build(work, hold_ids, hold_path):
    """D4 の照合（レビューの後に、働く形へ直した）。止める条件:
    - 中間の書き手（writers.json の SubjectID の SHA-256）に、取っておく書き手が 1 人でもいる
    - build が使った取っておく一覧が、いま渡された一覧と違う（SHA-256）
    - 部分実行（--limit）の中間である
    - 中間が同じ build の世代でない（units.json・u*.json・build の状態の数が合わない・2 巡目のレビューの後）"""
    wj = json.load(open(os.path.join(work, "writers.json")))
    wh = set(wj.values())
    hh = {hashlib.sha256(h.encode()).hexdigest() for h in hold_ids}
    if wh & hh:
        sys.exit("取っておく書き手が中間に入っている")
    bs = json.load(open(os.path.join(work, "build_summary.json")))
    if bs["holdout_sha256"] != hashlib.sha256(open(hold_path, "rb").read()).hexdigest():
        sys.exit("中間を作ったときの取っておく一覧が違う")
    if bs["attempted"] != bs["units"]:
        sys.exit("部分実行の中間は正式の集計に使わない")
    if not set(bs["status"]) <= {"ok", "few_presses", "span_too_long"}:
        sys.exit("build に許していない状態の課題がある")
    uw = json.load(open(os.path.join(work, "units.json")))
    n_files = len(glob.glob(os.path.join(work, "u[0-9]*.json")))
    if not (n_files == len(uw) == bs["status"].get("ok", 0)):
        sys.exit("中間の課題の数が build の状態と合わない（別の世代の中間が混ざっている）")
    if not {str(v) for v in uw.values()} <= set(wj):
        sys.exit("units.json の書き手が writers.json に無い")
    return len(wh)


def n_not_held(path, hold_ids):
    """CSV の書き手のうち、取っておく一覧に無い人数（標本の表の数）。"""
    csv.field_size_limit(10**9)
    with open(path, newline="") as f:
        r = csv.reader(f)
        i = next(r).index("SubjectID")
        subj = {row[i] for row in r}
    return len(subj - hold_ids)


def posthoc_l4_nonws(writers):
    """事後（レビューの後・判定に使わない）: 空白と改行だけの複数の文字の挿入を受領から外した L4。"""
    alt = []
    for w in writers:
        aw = []
        for s in w:
            u = copy.copy(s)
            nonws = sorted((Counter(s.receipt_kinds["multi-insert"]) - Counter(s.ws_times)).elements())
            u.receipt_kinds = {"paste": s.receipt_kinds["paste"], "multi-insert-nonws": nonws}
            u.receipts = sorted(s.receipt_kinds["paste"] + nonws)
            aw.append(u)
        alt.append(aw)
    return al.summarize_l4(alt)


def posthoc_resume(writers):
    """事後（レビューの後・判定に使わない）: 再開が t0 と別の作業のまとまりにある停滞の数。
    t の属するまとまり = 始まりが t 以下の最後のまとまり。"""
    n = cross = unresumed = 0
    for w in writers:
        for s in w:
            starts = [b[0] for b in s.blocks]
            for _, t0, r, _ in s.stalls():
                n += 1
                if r is None:
                    unresumed += 1
                elif bisect.bisect_right(starts, r) != bisect.bisect_right(starts, t0):
                    cross += 1
    return {"stalls": n, "resume_in_other_block": cross, "unresumed": unresumed}


def summarize_year(writers, has_prompt, has_nav):
    res = {}
    for label, normal in (("normal", True), ("all", False)) if has_prompt else (("all", False),):
        r = al.summarize(writers, normal=normal, has_click=False)
        r["L3"] = summarize_l3(writers, normal, has_nav)
        res[label] = r
    res["all"]["L4"] = al.summarize_l4(writers)
    res["all"]["L5"] = al.summarize_l5(writers)
    res["all"]["L6"] = summarize_l6(writers)
    return res


def run(w19, w21, hold_p, csv19, csv21, out_dir):
    hold = json.load(open(hold_p))
    res, counts, posthoc = {}, {}, {}
    for year, work, path, has_prompt, has_nav in (("2019", w19, csv19, False, False),
                                                  ("2021", w21, csv21, True, True)):
        hold_ids = set(hold[year])
        n_kept = n_not_held(path, hold_ids)
        n_built = check_build(work, hold_ids, hold_p)
        if n_built != n_kept:
            sys.exit("中間の書き手の数が、取っておく書き手を除いた数と違う")
        ws = load(work)
        el = [w for w in ws if sum(s.obs_ms() for s in w) >= al.MIN_WRITER_OBS]
        counts[year] = {"writers_not_held": n_kept, "writers_loaded": len(ws), "units_loaded": sum(len(w) for w in ws),
                        "excluded_obs_lt_10min": len(ws) - len(el), "units_eligible": sum(len(w) for w in el)}
        res[year] = summarize_year(el, has_prompt, has_nav)
        res[year]["units"] = describe_units(el)
        posthoc[year] = {"resume": posthoc_resume(el),
                         "L4_nonws": posthoc_l4_nonws(el) if has_prompt else None,
                         "row_kinds_posthoc": {k: v for k, v in json.load(open(os.path.join(
                             work, "build_summary.json")))["row_kinds"].items() if k.startswith(("posthoc:", "ev:multi"))}}
    os.makedirs(out_dir, exist_ok=True)
    json.dump(res, open(os.path.join(out_dir, "cs1_landscape.json"), "w"), indent=1)
    json.dump(counts, open(os.path.join(out_dir, "counts.json"), "w"), indent=1)
    json.dump(posthoc, open(os.path.join(out_dir, "posthoc.json"), "w"), indent=1)
    print(json.dumps(counts))


def selftest():
    assert prompt_cover([(100, "prompt"), (5_000, "prompt-answer")]) == [[100, 5_000]]
    assert prompt_cover([(100, "prompt"), (100 + 11 * 60_000, "prompt-answer")]) == [[100, 60_100]]
    press = [i * 1000 for i in range(200)] + [259_000 + i * 1000 for i in range(50)] + \
        [10_000_000 + i * 1000 for i in range(30)]
    d = {"keys": [[t, "alphabetic"] for t in press], "feat": [[t, 0.7, 0.1] for t in press],
         "onsets": [[229_000, 235_000, 1000, 199]],
         "observed_intervals": [[0, 199_000], [259_000, 308_000], [10_000_000, 10_029_000]],
         "blocks": [[0, 308_000], [10_000_000, 10_029_000]],
         "events": [[230_000, "run"], [240_000, "prompt"], [250_000, "prompt-answer"], [259_000, "paste"]],
         "span": [0, 10_029_000]}
    u = Unit(d)
    assert u.span_ms() == 308_000 + 29_000
    g = u.gaps()
    assert len(g) == len(press) - 2 and max(b - a for a, b in g) == 60_000  # まとまりの間の差は入らない
    assert u.near_ai(245_000, 246_000) and not u.near_ai(251_000, 252_000)
    assert u.normal_span_ms() == u.span_ms() - 10_000
    l3 = writer_l3([u], normal=False)
    # 後の窓は (t0, 再開) の開区間（元の L3 と同じ）。再開と同じ時刻の貼り付けは入らない
    assert l3[0]["pre_run"] and not l3[0]["post_run"] and not l3[0]["post_paste"]
    assert al.summarize_l4([[u]])["stall_receipt_before_resume"]["num"] == 1  # L4 は再開の時刻を含む
    assert writer_l3([u], normal=True) == []  # 停滞 [s − 30 秒, 再開] が問いの区間にかかる
    l6 = unit_l6(u)
    assert l6 is not None and sum(1 for k in range(3) if l6[k]["stall_per_h"]) == 1
    du = describe_units([[u]])
    assert du["units"] == 1 and du["unit_blocks"]["median"] == 2
    assert posthoc_resume([[u]]) == {"stalls": 1, "resume_in_other_block": 0, "unresumed": 0}
    # 陽性例: 最初のまとまりの終わり近くの停滞で、次の文字が次のまとまりにある
    # press[245:250] は 304,000〜308,000（最初のまとまりの終わり）。t0 = 303,500 の後の文字は次のまとまりにだけある
    d3 = dict(d, keys=[[t, "alphabetic"] for t in press[:245]] + [[t, "backspace"] for t in press[245:250]]
              + [[t, "alphabetic"] for t in press[250:]], onsets=[[300_000, 303_500, 1000, 199]])
    assert posthoc_resume([[Unit(d3)]])["resume_in_other_block"] == 1
    d2 = dict(d, events=d["events"] + [[259_000, "multi-insert"], [259_000, "posthoc-ws"], [260_000, "multi-insert"]])
    u2 = Unit(d2)
    assert len(u2.receipts) == 3  # 主の受領は印を読まない
    p4 = posthoc_l4_nonws([[u2]])
    assert set(p4["receipt_per_h_by_kind"]) == {"paste", "multi-insert-nonws"}
    obs_h = u2.obs_ms() / H
    # 259_000 の挿入は空白の印と対になって外れ、260_000 の 1 件だけが残る（paste は 1 件のまま）
    assert abs(p4["receipt_per_h_by_kind"]["multi-insert-nonws"]["median"] - 1 / obs_h) < 1e-3
    assert abs(p4["receipt_per_h_by_kind"]["paste"]["median"] - 1 / obs_h) < 1e-3
    d4 = dict(d, events=d["events"] + [[259_000, "multi-insert"], [259_000, "posthoc-ws"]])
    assert posthoc_l4_nonws([[Unit(d4)]])["receipt_per_h_by_kind"]["multi-insert-nonws"]["median"] == 0
    # D4 の照合: 陽性対照（取っておく書き手が混ざる・一覧が違う・部分実行）で止まり、正常で通る
    with tempfile.TemporaryDirectory() as td:
        hp = os.path.join(td, "hold.json")
        json.dump({"2019": ["B"]}, open(hp, "w"))
        hs = hashlib.sha256(open(hp, "rb").read()).hexdigest()
        sha = lambda x: hashlib.sha256(x.encode()).hexdigest()  # noqa: E731

        ok_st = {"ok": 2, "few_presses": 1}

        def put(writers, bs, units=None, files=("00000", "00001")):
            for n in glob.glob(os.path.join(td, "u[0-9]*.json")):
                os.remove(n)
            for k in files:
                open(os.path.join(td, "u%s.json" % k), "w").write("{}")
            json.dump(writers, open(os.path.join(td, "writers.json"), "w"))
            json.dump(bs, open(os.path.join(td, "build_summary.json"), "w"))
            json.dump(units if units is not None else {"00000": 0, "00001": 1},
                      open(os.path.join(td, "units.json"), "w"))
        good_w = {"0": sha("A"), "1": sha("C")}
        good_bs = {"holdout_sha256": hs, "attempted": 3, "units": 3, "status": ok_st}
        put(good_w, good_bs)
        assert check_build(td, {"B"}, hp) == 2
        for args in (({"0": sha("A"), "1": sha("B")}, good_bs),
                     (good_w, dict(good_bs, holdout_sha256="0" * 64)),
                     (good_w, dict(good_bs, attempted=2)),
                     (good_w, dict(good_bs, status={"ok": 2, "features_failed": 1})),
                     (good_w, good_bs, None, ("00000", "00001", "00002")),  # 古い世代の u*.json が残る
                     (good_w, good_bs, {"00000": 0, "00001": 5})):
            put(*args)
            try:
                check_build(td, {"B"}, hp)
                raise AssertionError("止まらなかった")
            except SystemExit:
                pass
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        run(*sys.argv[1:])
