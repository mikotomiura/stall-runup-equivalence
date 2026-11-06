"""書き手ごとのセッションの一覧を 1 行 1 書き手（"<wid> <sid> <sid> ..."）で出す。書き手の ID は repo に出さない（中間の置き場だけ）。"""
import csv
import sys

g = {}
for p in sys.argv[1:]:
    for r in csv.DictReader(open(p, encoding="utf-8")):
        g.setdefault(r["worker_id"], []).append(r["session_id"])
for w in sorted(g):
    print(w, *g[w])
