"""登録の外の探索（付録 F）の CoAuthor の見た書き手（conditions.md X6）。判定に使わない。

使い方: python -I explore_coauthor.py <coauthor_work_obs> <coauthor_work> <writer_onsets_dir> <出力の json> [--gates-only]
- 書き手ごとにセッションを詰めてつなぎ、つないだ時刻の上で θ と条件 A と onset を作る（付録 A4）。
- 確認のスクリプトには書き手 1 人 = セッションの並び（cs1=False）として渡す（研究者の判断）。
- AI の提案まわりの出来事の除外は stall_anchors と control_ok を包んで差し替える（凍結したファイルは書き換えない）。
- 門 G1（セッションだけの再現 = replay）と G2（つないだ replay --features の onset = 橋渡しの onset）が
  1 件でも外れたら計算しない。集計の値だけを書く。
"""
import bisect
import glob
import json
import os
import random
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import explore as E  # noqa: E402

C = E.C
k6 = C.k6
GAP_MS, DWELL = 2_500, 100
AI_PRE, AI_POST = 60_000, 60_000
EXPECT_SESSIONS, EXPECT_WRITERS = 1_157, 43  # conditions.md X6（中間の build_summary と sessions.json の件数）
EXTEND_TAIL =os.environ.get("GSE_X7_MUTATE_NO_TAIL") != "1"  # 変異（門 G2 が赤になることの確かめ）だけで外す


def is_ai(name):
    return name.startswith("suggestion") or name == "api-insert"


class ShiftTheta:
    """書き手のつないだ時刻の θ を、セッションの元の時刻で引く。"""

    def __init__(self, theta, off):
        self.theta, self.off = theta, off

    def at(self, t):
        return self.theta.at(t + self.off)


def session_unit(d, theta, onsets):
    w = object.__new__(k6.Writer)
    w.keys = d["keys"]
    w.press = [k[0] for k in w.keys]
    w.cats = [k[1] for k in w.keys]
    w.feat = d["feat"]
    w.feat_t = [f[0] for f in w.feat]
    w.onsets = onsets
    w.intervals = [tuple(iv) for iv in d["observed_intervals"]]
    w.theta = theta
    w.ca = k6.CondA(w.press, w.cats, w.intervals)
    w.text = [t for t, c in w.keys if c in k6.TEXT]
    w.text_cat = [c for t, c in w.keys if c in k6.TEXT]
    w.t_start, w.t_end = w.press[0], w.press[-1]
    w.blocks = [[w.t_start, w.t_end]]
    w.block_starts = [w.t_start]
    w.ev = []
    w.ai = sorted(t for t, n in d["events"] if is_ai(n))
    w.w_ms = k6.W_MS
    w.stderr_bytes = d["replay_stderr_bytes"]
    return w


def ai_in(w, a, b):
    i = bisect.bisect_left(w.ai, a)
    return i < len(w.ai) and w.ai[i] <= b


def bridge(sessions):
    """sessions: [(番号, d)]。つないだ時刻の θ と onset を作り、セッションへ戻す。"""
    order = sorted(sessions, key=lambda kd: min(kd[1]["keys"][0][0], kd[1]["feat"][0][0] if kd[1]["feat"] else kd[1]["keys"][0][0]))
    if EXTEND_TAIL:  # X7: つなぐと 2.5 秒の間隔は idle_trim の長い穴に当たらず、最後以外のセッションの終わりは削られない
        order = [(k, dict(d, observed_intervals=d["observed_intervals"][:-1]
                          + [[d["observed_intervals"][-1][0], max(d["observed_intervals"][-1][1], d["feat"][-1][0])]]))
                 if j < len(order) - 1 and d["feat"] and d["observed_intervals"] else (k, d)
                 for j, (k, d) in enumerate(order)]
    press, cats, ivs, offs, spans, cursor = [], [], [], {}, [], None
    for k, d in order:
        t0 = min(d["keys"][0][0], d["feat"][0][0]) if d["feat"] else d["keys"][0][0]
        off = 0 if cursor is None else cursor + GAP_MS - t0
        offs[k] = off
        press += [t + off for t, _ in d["keys"]]
        cats += [c for _, c in d["keys"]]
        ivs += [(a + off, b + off) for a, b in d["observed_intervals"]]
        spans += [(a + off, b + off, k) for a, b in d["observed_intervals"]]
        last = max([d["keys"][-1][0] + DWELL] + ([d["feat"][-1][0]] if d["feat"] else []))
        cursor = last + off
    theta = k6.Theta(press, ivs)
    ca = k6.CondA(press, cats, ivs)
    per = {k: [] for k, _ in sessions}
    unassigned = 0
    for s0, t0, th, n in k6.replicate_onsets(ca, theta):
        hit = next((k for a, b, k in spans if a <= s0 <= b), None)
        if hit is None:
            unassigned += 1
            continue
        per[hit].append([s0 - offs[hit], t0 - offs[hit], th, n])
    return theta, offs, per, unassigned


def load(obs, old, ons_dir):
    keep = json.load(open(os.path.join(old, "sessions.json")))
    by_w = {}
    for p in sorted(glob.glob(os.path.join(obs, "s[0-9]*.json"))):
        k = os.path.basename(p)[1:-5]
        if k not in keep:
            sys.exit("sessions.json に無いセッションが中間にある")
        by_w.setdefault(keep[k], []).append((k, json.load(open(p))))
    chk = Counter(writers=len(by_w))
    # 門の網羅性: 中間のセッションの集合 = sessions.json の全件、G2 の書き手の集合 = 中間の書き手の集合
    chk["sessions_missing_from_work"] = len(set(keep) - {k for v in by_w.values() for k, _ in v})
    g2_w = {os.path.basename(p)[1:-5] for p in glob.glob(os.path.join(ons_dir, "w*.json"))}
    chk["g2_writer_set_mismatch"] = len(g2_w ^ {str(w) for w in by_w})
    writers = []
    for wid in sorted(by_w):
        sess = by_w[wid]
        for _, d in sess:  # G1: セッションだけの再現 = replay
            chk["sessions"] += 1
            chk["stderr_bytes"] += d["replay_stderr_bytes"]
            pr = [t for t, _ in d["keys"]]
            ivs = [tuple(iv) for iv in d["observed_intervals"]]
            rep = k6.replicate_onsets(k6.CondA(pr, [c for _, c in d["keys"]], ivs), k6.Theta(pr, ivs))
            chk["g1_session_onsets"] += len(d["onsets"])
            if rep != [list(o) for o in d["onsets"]]:
                chk["g1_mismatch_sessions"] += 1
        theta, offs, per, unassigned = bridge(sess)
        chk["bridge_unassigned"] += unassigned
        ref_p = os.path.join(ons_dir, "w%s.json" % wid)  # G2
        if not os.path.exists(ref_p):
            sys.exit("門が崩れた: G2 の中間に書き手が欠けている")
        ref = json.load(open(ref_p))
        chk["g2_unassigned"] += ref["unassigned"]
        chk["g2_stderr_bytes"] += ref["stderr_bytes"]
        mine = sorted([k] + o for k in per for o in per[k])
        theirs = sorted(ref["onsets"])
        chk["bridge_onsets"] += len(mine)
        chk["g2_onsets"] += len(theirs)
        if mine != theirs:
            chk["g2_mismatch_writers"] += 1
            chk["g2_mismatch_onsets"] += len(set(map(tuple, mine)) ^ set(map(tuple, theirs)))
        writers.append([session_unit(d, ShiftTheta(theta, offs[k]), per[k]) for k, d in sorted(sess)])
    return writers, dict(chk)


def gates_ok(chk):
    return (chk.get("g1_mismatch_sessions", 0) == 0 and chk.get("g2_mismatch_writers", 0) == 0
            and chk.get("bridge_unassigned", 0) == 0 and chk.get("g2_unassigned", 0) == 0
            and chk.get("g2_onsets", 0) > 0 and chk.get("g1_session_onsets", 0) > 0
            and chk.get("sessions_missing_from_work", 1) == 0 and chk.get("g2_writer_set_mismatch", 1) == 0
            and chk.get("sessions") == EXPECT_SESSIONS and chk.get("writers") == EXPECT_WRITERS)


def patch(cnt):
    orig_sa, orig_co = C.stall_anchors, C.control_ok

    def stall_anchors(w, cfg=None):
        return [a for a in orig_sa(w, cfg) if not ai_in(w, a[0] - AI_PRE, a[0])]

    def control_ok(w, i, cfg, cs1, cnt_=None):
        if not orig_co(w, i, cfg, cs1, cnt_):
            return False
        u = w.text[i]
        if ai_in(w, u - AI_PRE, u + AI_POST):
            cnt["control_excluded_ai"] += 1
            return False
        return True

    C.stall_anchors, C.control_ok = stall_anchors, control_ok
    return orig_sa, orig_co


def main(obs, old, ons_dir, out, gates_only=False):
    writers, chk = load(obs, old, ons_dir)
    res = {"seed": E.SEED, "note": "登録の外の探索（付録 F・X6）。判定に使わない。", "checks": chk}
    if not gates_ok(chk):
        sys.exit("門が崩れた: " + json.dumps(chk))
    if not gates_only:
        cnt = Counter()
        orig_sa, orig_co = patch(cnt)
        try:
            for units in writers:
                for w in units:
                    raw = orig_sa(w, C.MAIN)
                    cnt["stall_anchors_before_ai"] += len(raw)
                    cnt["stall_excluded_ai"] += sum(1 for a in raw if ai_in(w, a[0] - AI_PRE, a[0]))
            res["coauthor_seen"] = {"writers": len(writers), "sessions": sum(len(u) for u in writers)} \
                | E.run_set(writers, cs1=False)
        finally:
            C.stall_anchors, C.control_ok = orig_sa, orig_co
        res["coauthor_seen"]["ai_exclusion_counts"] = dict(cnt)
    res = E.strip_per_writer(res)
    with open(out + ".tmp", "w") as f:
        json.dump(res, f, indent=1, default=str)
    os.replace(out + ".tmp", out)
    print(json.dumps(chk))


if __name__ == "__main__":
    a = sys.argv[1:]
    main(*a[:4], gates_only="--gates-only" in a)
