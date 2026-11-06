"""replay の出力から analyze.py が読む項目だけを残す（ディスク節約・解析の値は変えない）。

使い方: python3 -I slim.py <sid>_feat.json <sid>_ft.json
feat: timestamp / p_stuck / ewma_x / f3 / f5 だけ。ft: rows の t_ms と stall_onset.is_onset だけ。
"""
import json
import sys

KEEP = ["timestamp", "p_stuck", "ewma_x", "f3", "f5"]
feat_p, ft_p = sys.argv[1], sys.argv[2]
feat = json.load(open(feat_p))
json.dump([{k: r[k] for k in KEEP} for r in feat], open(feat_p, "w"))
ft = json.load(open(ft_p))
json.dump({"rows": [{"t_ms": r["t_ms"], "stall_onset": {"is_onset": r["stall_onset"].get("is_onset")}} for r in ft["rows"]]},
          open(ft_p, "w"))
