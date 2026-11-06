"""取り置きの変換の照合（逸脱 1・2026-11-06）。

元の照合「変換で読んだ書き手の数 = 取り置きの一覧の人数」は、生の記録に課題を作る行（キーの押下と出来事）を
1 つも持たない書き手がいると落ちる（取り置きの 1 人は X-Checkpoint の 1 行だけだった）。そこで次の 2 つを確かめる。
1. 中間に読まれた書き手が全員、取り置きの一覧にいる。
2. 読まれなかった取り置きの書き手は、生の記録に課題を作る行が 0 件である（変換と同じ写像 map_2019 で count 以外の行が無い）。
   つまり、読み飛ばしの誤りで黙って落ちた書き手はいない。
使い方: python3 -I check_holdout_writers.py <keystrokes.csv> <holdout.json> <中間の dir> <出力の json>
書き手の ID は出さない（数だけ）。
"""
import csv
import hashlib
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "20261104-cs1-landscape", "scripts"))
from build_cs1 import map_2019  # noqa: E402  変換と同じ写像


def main(raw, holdout_path, work, out):
    hold = [str(s) for s in json.load(open(holdout_path))["2019"]]
    read = set(json.load(open(os.path.join(work, "writers.json"))).values())
    hh = {hashlib.sha256(s.encode()).hexdigest(): s for s in hold}
    outside = [h for h in read if h not in hh]
    absent = {hh[h] for h in hh if h not in read}
    unit_rows = {s: 0 for s in absent}
    other_rows = {s: 0 for s in absent}
    csv.field_size_limit(10**9)
    with open(raw, newline="") as f:
        for row in csv.DictReader(f):
            sid = row["SubjectID"]
            if sid in unit_rows:
                kind, _ = map_2019(row)
                if kind == "count":
                    other_rows[sid] += 1
                else:
                    unit_rows[sid] += 1
    res = {"holdout": len(hold), "read": len(read), "read_outside_holdout": len(outside),
           "not_read": len(absent), "not_read_with_unit_rows": sum(1 for s in absent if unit_rows[s] > 0),
           "not_read_rows_without_units": sum(other_rows.values())}
    json.dump(res, open(out, "w"), indent=1)
    print(json.dumps(res))
    ok = res["read_outside_holdout"] == 0 and res["not_read_with_unit_rows"] == 0 and res["read"] + res["not_read"] == res["holdout"]
    sys.exit(0 if ok else "取り置きの書き手の照合が通らない（逸脱 1 の照合）")


if __name__ == "__main__":
    main(*sys.argv[1:5])
