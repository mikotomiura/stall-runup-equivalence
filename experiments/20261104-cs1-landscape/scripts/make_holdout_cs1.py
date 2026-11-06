"""取っておく書き手を決める（conditions.md D4）。値を読む前に回す。

使い方: python3 -I make_holdout_cs1.py <seed> <out_holdout.json> <keystrokes_2019.csv> <keystrokes_2021.csv>
読むのは SubjectID の列だけ。出力は repo の外。標準出力には人数と一覧の SHA-256 だけを出す。
"""
import csv
import hashlib
import json
import random
import sys

FRAC = 0.3


def subjects(path):
    csv.field_size_limit(10**9)
    with open(path, newline="") as f:
        r = csv.reader(f)
        i = next(r).index("SubjectID")
        return sorted({row[i] for row in r})


def main():
    seed, out, p19, p21 = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
    s19, s21 = subjects(p19), subjects(p21)
    rec = {"2019": sorted(random.Random(seed).sample(s19, round(FRAC * len(s19)))),
           "2021": sorted(random.Random(seed + 1).sample(s21, round(FRAC * len(s21))))}
    blob = json.dumps(rec, sort_keys=True).encode()
    with open(out, "w") as f:
        f.write(blob.decode())
    print(json.dumps({"writers_2019": len(s19), "holdout_2019": len(rec["2019"]),
                      "writers_2021": len(s21), "holdout_2021": len(rec["2021"]),
                      "sha256": hashlib.sha256(blob).hexdigest()}))


if __name__ == "__main__":
    main()
