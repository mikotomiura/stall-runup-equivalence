"""変異の結果を読み、名指しの対照が赤になったか（落ちたのではなく passed が false か）を確かめて selftest.json にまとめる。

使い方: python3 -I check_mutations.py <mutations_dir> <selftest.json>
返り値: 0 = すべての変異が名指しの対照を赤にした、3 = 当たらない変異がある。
"""
import json
import os
import sys

NAMED = {"m1": [("m1.json", "P-EM4")], "m2": [("m2.json", "P-EM3")],
         "m3": [("m3_pd2.json", None), ("m3_em1.json", "P-EM1")],
         "m4": [("m4.json", "P-L2")], "m5": [("m5.json", "P-L2")], "m6": [("m6.json", "P-S")]}


def main():
    d, st = sys.argv[1:3]
    res = {}
    for m, items in NAMED.items():
        red = []
        for f, ctl in items:
            p = os.path.join(d, f)
            if not os.path.exists(p):
                red.append(dict(file=f, control=ctl, red=False, note="出力が無い（落ちた）"))
                continue
            j = json.load(open(p))
            node = j if ctl is None else j.get(ctl, {})
            red.append(dict(file=f, control=ctl or "P-D2", red=node.get("passed") is False or node.get("passed") == 0))
        res[m] = dict(controls=red, hit=all(r["red"] for r in red))
    S = json.load(open(st))
    S["mutations"] = res
    json.dump(S, open(st, "w"), indent=1)
    print(json.dumps({m: r["hit"] for m, r in res.items()}))
    return 0 if all(r["hit"] for r in res.values()) else 3


if __name__ == "__main__":
    sys.exit(main())
