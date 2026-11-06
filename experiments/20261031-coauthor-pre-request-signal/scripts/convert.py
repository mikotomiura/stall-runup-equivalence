"""CoAuthor の 1 セッション (jsonl) を、本文を含まない GSE の NDJSON と出来事の表に変換する。

- 出力 NDJSON は meta / session_start / key 行だけ。key 行は時刻とキーの種類 (category) だけを持つ。
  文字そのもの・文書のスナップショット・提案の文面は書き出さない。
- 打鍵として数えるのは eventSource == "user" の text-insert (1 文字) / text-delete / cursor-* だけ。
  API 起源の挿入 (受け入れた提案の本文) と suggestion-* の操作は打鍵の列に入れない。
- CoAuthor は離し (key up) を記録しないので、押下の DWELL_MS 後に離しを合成する (conditions.md C3)。
- 出来事の表 (events.json) は、提案の要求 (suggestion-get) と提案まわりの出来事の時刻だけを持つ。

使い方: python3 -I convert.py <in.jsonl> <out.ndjson> <out_events.json> [dwell_ms]
"""
import ast
import json
import sys

HMM_VERSION = "0.5.0"  # meta 行の値。replay は meta の hmm_version を刻印に使うだけ (conditions.md C3)
SENTENCE_PUNCT = set(".,")
PUNCT = set(";:'\"!?-_=+()[]{}<>/\\|`~@#$%^&*")


def delta_ops(d):
    if not d:
        return []
    if isinstance(d, str):
        d = ast.literal_eval(d)
    return d.get("ops", [])


def char_category(c):
    if c.isalpha():
        return "alphabetic"
    if c.isdigit():
        return "numeric"
    if c == " ":
        return "space"
    if c == "\n":
        return "enter"
    if c == "\t":
        return "tab"
    if c in SENTENCE_PUNCT:
        return "sentence_punctuation"
    if c in PUNCT:
        return "punctuation"
    return "other"


def convert(src, dst, dst_events, dwell_ms):
    rows = [json.loads(line) for line in open(src, encoding="utf-8")]
    rows.sort(key=lambda e: (int(e["eventTimestamp"]), int(e["eventNum"])))
    t0 = int(rows[0]["eventTimestamp"])
    keys = []  # (t, category)
    events = []  # (t, name, source)
    stats = {"user_insert_1": 0, "user_insert_multi_dropped": 0, "user_delete": 0,
             "user_cursor": 0, "api_insert_excluded": 0, "suggestion_events": 0}
    for e in rows:
        t = int(e["eventTimestamp"])
        name, src_ = e["eventName"], e["eventSource"]
        if name.startswith("suggestion"):
            events.append({"t": t, "name": name, "source": src_})
            stats["suggestion_events"] += 1
            continue
        if src_ != "user":
            if name == "text-insert":
                stats["api_insert_excluded"] += 1
                events.append({"t": t, "name": "api-insert", "source": src_})
            continue
        if name == "text-insert":
            s = "".join(o["insert"] for o in delta_ops(e["textDelta"]) if isinstance(o.get("insert"), str))
            if len(s) == 1:
                keys.append((t, char_category(s)))
                stats["user_insert_1"] += 1
            elif len(s) > 1:
                stats["user_insert_multi_dropped"] += 1
                events.append({"t": t, "name": "multi-insert", "source": src_})
        elif name == "text-delete":
            keys.append((t, "backspace"))
            stats["user_delete"] += 1
        elif name.startswith("cursor-"):
            keys.append((t, "navigation"))
            stats["user_cursor"] += 1
    with open(dst, "w", encoding="utf-8") as f:
        f.write(json.dumps({"type": "meta", "session_start": t0, "hmm_version": HMM_VERSION}) + "\n")
        f.write(json.dumps({"type": "session_start", "t": t0}) + "\n")
        for i, (t, cat) in enumerate(keys):
            # 押下の時刻は動かさない。離しは「押下 + dwell」と「次の押下の 1 ms 前」の早いほう (C3)
            rel_t = t + dwell_ms
            if i + 1 < len(keys):
                rel_t = min(rel_t, max(t, keys[i + 1][0] - 1))
            f.write(json.dumps({"type": "key", "t": t, "category": cat, "press": True}) + "\n")
            f.write(json.dumps({"type": "key", "t": rel_t, "category": cat, "press": False}) + "\n")
    t_end = int(rows[-1]["eventTimestamp"])
    with open(dst_events, "w", encoding="utf-8") as f:
        json.dump({"t_start": t0, "t_end": t_end, "events": events, "stats": stats}, f)
    return stats


if __name__ == "__main__":
    dwell = int(sys.argv[4]) if len(sys.argv) > 4 else 100
    print(json.dumps(convert(sys.argv[1], sys.argv[2], sys.argv[3], dwell)))
