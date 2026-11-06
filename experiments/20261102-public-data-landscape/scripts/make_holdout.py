"""取っておく書き手を決める（conditions.md C2）。値を読む前に回す。

使い方: python3 -I make_holdout.py <klicke_n> <seed> <out_holdout.json> <metadata.csv>...
出力: <out_holdout.json>（repo の外）。標準出力には人数と一覧の SHA-256 だけを出す。
"""
import csv
import hashlib
import json
import random
import sys

FRAC = 0.3


def main():
    n_k, seed, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    workers = set()
    for p in sys.argv[4:]:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            workers.add(r["worker_id"])
    workers = sorted(workers)
    k_hold = sorted(random.Random(seed).sample(range(n_k), round(FRAC * n_k)))
    c_hold = sorted(random.Random(seed + 1).sample(workers, round(FRAC * len(workers))))
    rec = {"klicke": k_hold, "coauthor": c_hold}
    blob = json.dumps(rec, sort_keys=True).encode()
    with open(out, "w") as f:
        f.write(blob.decode())
    print(json.dumps({"klicke_n": n_k, "klicke_holdout": len(k_hold), "coauthor_writers": len(workers),
                      "coauthor_holdout": len(c_hold), "sha256": hashlib.sha256(blob).hexdigest()}))


if __name__ == "__main__":
    main()
