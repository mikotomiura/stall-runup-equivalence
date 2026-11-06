"""事後の確かめ（conditions.md に無い・判定に使わない）。集計値だけを出す。

使い方: python3 -I posthoc.py <klicke_work.tar.gz> <klicke_side.json> <coauthor_work_dir> <holdout.json> <out.json> <coauthor.zip> <metadata.csv>...
1. KLiCKe の L4 で「受領が再開より前」の停滞と「離席候補の後 5 分に受領」の途切れが、同じ貼り付けを数えているか。
2. CoAuthor のふつうの区間の長さが観測時間・セッションの長さに占める割合。
3. L6 の停滞の頻度を、書き手の中央値ではなく全員を合わせた比（停滞の数の和 / 時間の和）でも出す。
"""
import json
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import analyze_landscape as al  # noqa: E402

tar_p, side_p, cwork, hold_p, out, czip = sys.argv[1:7]
metas = sys.argv[7:]
hold = json.load(open(hold_p))
kw, _ = al.load_klicke(tar_p, side_p, set(hold["klicke"]))
cw = al.load_coauthor(cwork, al.coauthor_hold_idx(czip, metas, set(hold["coauthor"])))
kw = [w for w in kw if sum(s.obs_ms() for s in w) >= al.MIN_WRITER_OBS]
cw = [w for w in cw if sum(s.obs_ms() for s in w) >= al.MIN_WRITER_OBS]
res = {}
# 1
both = st_only = away_only = 0
st_paste, away_paste = set(), set()
for w in kw:
    s = w[0]
    for st, t0, r, end in s.stalls():
        for p in s.receipts:
            if t0 < p <= end:
                st_paste.add((id(s), p))
    for a, b in s.gaps():
        if b - a >= al.AWAY:
            for p in s.receipts:
                if b <= p <= b + al.RECEIPT_WIN:
                    away_paste.add((id(s), p))
res["klicke_l4_overlap"] = {"pastes_counted_by_stall": len(st_paste), "pastes_counted_by_away": len(away_paste),
                            "shared": len(st_paste & away_paste)}
# 2
obs = sum(s.obs_ms() for w in cw for s in w)
nobs = sum(s.normal_obs_ms() for w in cw for s in w)
span = sum(s.span_ms() for w in cw for s in w)
nspan = sum(s.normal_span_ms() for w in cw for s in w)
res["coauthor_normal_share"] = {"obs_h": round(obs / al.H, 2), "normal_obs_h": round(nobs / al.H, 2),
                                "obs_share": round(nobs / obs, 4), "span_h": round(span / al.H, 2),
                                "normal_span_h": round(nspan / al.H, 2), "span_share": round(nspan / span, 4)}
# 3
def pooled_l6(writers):
    num, den = [0, 0, 0], [0.0, 0.0, 0.0]
    for w in writers:
        for s in w:
            L = s.span_ms()
            if L <= 0:
                continue
            cuts = [s.t_start + L * k / 3 for k in range(4)]
            ivs = al.merge([list(iv) for iv in s.intervals])
            for k in range(3):
                a, b = cuts[k], cuts[k + 1]
                den[k] += al.overlap_len(ivs, [[a, b]])
                num[k] += sum(1 for o in s.onsets if a <= o[1] < b or (k == 2 and o[1] == b))
    return {nm: {"stalls": num[k], "obs_h": round(den[k] / al.H, 2), "per_h": round(num[k] / (den[k] / al.H), 3)}
            for k, nm in enumerate(("early", "middle", "late"))}
res["l6_pooled_stall_per_h"] = {"klicke": pooled_l6(kw), "coauthor": pooled_l6(cw)}
json.dump(res, open(out, "w"), indent=1)
print(json.dumps(res))
