"""全セッションの DownEvent の値の種類と件数、category ごとの件数、読めなかった数を数える（K3・値ごとの件数だけ）。

使い方: python3 -I vocabulary.py <WritingTask.zip> <out.json>
1 字の英字・数字・非 ASCII は値を畳む。それ以外の値は名前と件数を出す（キーの名前で、本文ではない）。
"""
import json
import os
import sys
import zipfile
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import convert_klicke as ck  # noqa: E402

zp, out = sys.argv[1], sys.argv[2]
vocab, cats, errs, enc = Counter(), Counter(), Counter(), Counter()
n_files = 0
with zipfile.ZipFile(zp) as z:
    members = sorted(m for m in z.namelist() if "keystrokelogs/csv/" in m and m.endswith(".csv"))
    for m in members:
        n_files += 1
        try:
            ev, e = ck.read_events(z.read(m))
        except ck.LogError as x:
            errs[str(x)] += 1
            continue
        enc[e] += 1
        for _, _, name in ev:
            c = "mouse" if ck.is_mouse(name) else ck.category(name)
            cats[c] += 1
            if c in ("alphabetic", "numeric"):
                vocab["<1 字の%s>" % c] += 1
            elif len(name) == 1 and not name.isascii():
                vocab["<1 字の非 ASCII>"] += 1  # 打った文字の断片を値のまま出さない（レビュー指摘）
            else:
                vocab[name] += 1
json.dump({"files": n_files, "read_errors": errs, "encoding": enc, "category_counts": cats,
           "values_other": {k: v for k, v in vocab.most_common() if ck.category(k) == "other" and not ck.is_mouse(k) and not k.startswith("<")},
           "values_all_non_alnum": dict(vocab.most_common())}, open(out, "w"), ensure_ascii=False, indent=1)
