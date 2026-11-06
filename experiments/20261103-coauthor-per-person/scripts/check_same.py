"""作り直した中間が既存の中間と一致するかを確かめる（conditions.md の Q1）。値は出さず、件数だけを出す。

使い方: python3 -I check_same.py <old_work> <new_work>
feat は新しい中間の [t, p_stuck, ewma_x]（0・3・4 列）が既存の 3 列と完全一致（NaN は NaN 同士）、obs は 0〜25 の整数、
alpha は {0.3, 0.15, 0.18, 0.25} のどれか。ほかの項目は完全一致。
"""
import json
import math
import os
import sys


def same(a, b):
    if isinstance(a, float) and isinstance(b, float) and math.isnan(a) and math.isnan(b):
        return True
    return a == b


def main():
    old, new = sys.argv[1:3]
    so, sn = (json.load(open(os.path.join(d, "sessions.json"))) for d in (old, new))
    if so != sn:
        sys.exit("sessions.json が違う")
    bad, rows = 0, 0
    for k in sorted(so):
        a = json.load(open(os.path.join(old, "s%s.json" % k)))
        b = json.load(open(os.path.join(new, "s%s.json" % k)))
        ok = all(a[f] == b[f] for f in ("keys", "onsets", "observed_intervals", "events"))
        ok = ok and len(a["feat"]) == len(b["feat"])
        for ra, rb in zip(a["feat"], b["feat"]):
            rows += 1
            if not (len(rb) == 8 and all(same(x, y) for x, y in zip(ra, (rb[0], rb[3], rb[4])))
                    and isinstance(rb[6], int) and 0 <= rb[6] <= 25 and rb[7] in (0.3, 0.15, 0.18, 0.25)):
                ok = False
                break
        bad += not ok
    print(json.dumps({"sessions": len(so), "feat_rows": rows, "mismatched_sessions": bad}))
    if bad:
        sys.exit("一致しないセッションがある")


if __name__ == "__main__":
    main()
