"""KLiCKe の CSV から、クリックと貼り付けの時刻だけを取る（conditions.md C1）。

使い方: python3 -I klicke_side.py <WritingTask.zip> <holdout.json> <out_side.json>
- 読む列は DownTime・DownEvent・Activity の 3 つ。TextChange・CursorPosition は読まない。
- 時刻は T2 の変換と同じ（T_BASE + round(DownTime)）。書き手の番号は zip の中の名前の昇順の通し番号。
- 取っておく書き手（holdout）は読まずに飛ばす。
出力は {"<番号>": {"clicks": [...], "pastes": [...]}}（repo の外）。標準出力には件数だけ。
"""
import csv
import io
import json
import os
import sys
import zipfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                "../../20261101-klicke-early-stall-trigger/scripts"))
import convert_klicke as ck  # noqa: E402


def side_of(raw):
    s, _ = ck.decode(raw)
    reader = csv.reader(io.StringIO(s, newline=""), strict=True)
    header = next(reader, None)
    if header != ck.EXPECTED_HEADER:
        raise ck.LogError("header differs")
    clicks, pastes = [], []
    for rec in reader:
        t = ck.T_BASE + int(round(float(rec[3])))
        if ck.is_mouse(ck.unescape(rec[6])):
            clicks.append(t)
        if rec[12] == "Paste":
            pastes.append(t)
    return sorted(clicks), sorted(pastes)


def main():
    zp, hold_p, out = sys.argv[1:4]
    hold = set(json.load(open(hold_p))["klicke"])
    out_d, n_click, n_paste = {}, 0, 0
    with zipfile.ZipFile(zp) as z:
        members = sorted(m for m in z.namelist() if "keystrokelogs/csv/" in m and m.endswith(".csv"))
        for i, m in enumerate(members):
            if i in hold:
                continue
            c, p = side_of(z.read(m))
            out_d["%05d" % i] = {"clicks": c, "pastes": p}
            n_click += len(c)
            n_paste += len(p)
    json.dump(out_d, open(out, "w"), separators=(",", ":"))
    print(json.dumps({"members": len(members), "written": len(out_d), "clicks": n_click, "pastes": n_paste}))


if __name__ == "__main__":
    main()
