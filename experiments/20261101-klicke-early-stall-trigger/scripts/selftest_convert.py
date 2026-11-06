"""convert_klicke.py の自己検査（合成の CSV・人のデータを使わない）。

使い方: python3 -I selftest_convert.py  → 全部通れば "PASS" を出して 0 で終わる。
確かめること: マウスを key 行にしない／エスケープを戻す／種類の対応／離しは記録の実時刻／
出力に TextChange の文字列もキーの名前も残らない／cp1252 の記録も読める／見出しが違えば LogError。
"""
import csv
import io
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import convert_klicke as ck  # noqa: E402

HEADER = ["", "DownEventID", "UpEventID", "DownTime", "UpTime", "ActionTime", "DownEvent", "UpEvent",
          "CursorPosition", "PauseTime", "WordCount", "TextChange", "Activity"]
SECRET = "zqxsecretword"


def row(i, d, u, ev, tc="NoChange", act="Nonproduction"):
    return [str(i), str(i), str(i), str(d), str(u), str(u - d), ev, ev, "0", "0", "0", tc, act]


def to_csv(rows):
    buf = io.StringIO()
    w = csv.writer(buf, quoting=csv.QUOTE_ALL, lineterminator="\n")
    w.writerow(HEADER)
    w.writerows(rows)
    return buf.getvalue()


def build():
    rows = [row(1, 1000, 1080, "Leftclick"),
            row(2, 1200, 1290, "T", SECRET, "Input"),
            row(3, 1400, 1460, "Shift"),
            row(4, 1500, 1570, "\\\"", "\\\"", "Input"),  # 押したキーが "（エスケープ）
            row(5, 1700, 1750, "Space", " ", "Input"),
            row(6, 1900, 1990, ".", ".", "Input"),
            row(7, 2100, 2150, "Backspace", "", "Remove/Cut"),
            row(8, 2300, 2340, "ArrowLeft"),
            row(9, 2500, 2560, "AudioVolumeUp"),
            row(10, 2700, 2790, "7", "7", "Input"),
            row(11, 2900, 2950, "\\\\", "\\\\", "Input")]  # バックスラッシュ
    return to_csv(rows).encode("utf-8")


def main():
    fails = []
    ev, enc = ck.read_events(build())
    keys, mouse = ck.to_keys(ev)
    if mouse != 1:
        fails.append("mouse not excluded")
    cats = [k[2] for k in keys]
    want = ["alphabetic", "modifier", "punctuation", "space", "sentence_punctuation", "backspace", "navigation",
            "other", "numeric", "punctuation"]
    if cats != want:
        fails.append("categories %r" % cats)
    if keys[0][1] - keys[0][0] != 90:
        fails.append("release time not taken from UpTime")
    with tempfile.TemporaryDirectory() as td:
        p = os.path.join(td, "o.ndjson")
        ck.write_ndjson(keys, p)
        text = open(p).read()
        rows = [json.loads(x) for x in text.splitlines()]
    if SECRET in text or "Leftclick" in text or "AudioVolumeUp" in text:
        fails.append("content leaked to output")
    if any(set(r) - {"type", "t", "category", "press", "session_start", "hmm_version"} for r in rows):
        fails.append("unexpected keys in output rows")
    if sum(1 for r in rows if r["type"] == "key") != 2 * len(keys):
        fails.append("press/release count")
    # cp1252
    b = to_csv([row(1, 10, 20, "\xe9")]).encode("cp1252")
    ev2, enc2 = ck.read_events(b)
    if enc2 != "cp1252" or ck.category(ev2[0][2]) != "other":
        fails.append("cp1252")
    # 見出しが違えば止める
    try:
        ck.read_events(b"a,b\n1,2\n")
        fails.append("bad header accepted")
    except ck.LogError:
        pass
    print("PASS" if not fails else "FAIL: " + "; ".join(fails))
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
