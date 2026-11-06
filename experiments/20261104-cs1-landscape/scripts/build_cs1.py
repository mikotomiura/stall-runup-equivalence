"""CS1 の keystrokes.csv を課題ごとに変換 → replay → replay --features し、解析が読む項目だけを残す（conditions.md D1〜D3）。

使い方:
  python3 -I build_cs1.py <year:2019|2021> <keystrokes.csv> <gse-replay> <holdout.json> <work_dir> <jobs> [--limit N]
  python3 -I build_cs1.py --selftest
出力（repo の外）: <work_dir>/u<番号>.json（番号は (SubjectID, AssignmentID) の昇順の通し番号）と
<work_dir>/units.json（番号 → 書き手の通し番号。ID は書かない）・writers.json（書き手の通し番号 →
SubjectID の SHA-256。解析が取っておく書き手と照らす）・build_summary.json。各 u*.json は
  keys:   [[t_press, category], ...]
  feat:   [[t, p_stuck, ewma_x], ...]
  onsets: [[s_ms, t0_ms, theta_person_ms, theta_sample_n], ...]
  observed_intervals: [[a_ms, b_ms], ...]
  blocks: [[first_press, last_press], ...]（作業のまとまり）
  events: [[t, name], ...]（D3 の出来事の表。時刻と名前だけ。残したまとまりの出来事だけ）
キーの文字・挿入と削除の文字・貼り付けた文字は書き出さない。取っておく書き手の行は読み飛ばす。
中間の NDJSON と replay の出力は課題ごとに消す。
"""
import csv
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ITEM1 = os.path.join(HERE, "../../20261031-coauthor-pre-request-signal/scripts")
sys.path.insert(0, ITEM1)
import convert as ca  # noqa: E402  char_category と HMM_VERSION をそのまま使う

ADD_FEAT = os.path.join(ITEM1, "add_feat.py")
DWELL = 100
BLOCK_GAP = 1_800_000  # D2
NAV_2021 = {"Left", "Right", "Up", "Down", "Home", "End", "PageUp", "PageDown"}
KEY_2021 = {"Backspace": "backspace", "Delete": "delete", "Enter": "enter", "Tab": "tab", "Escape": "escape",
            "Paste": "other", "Copy": "other", "Cut": "other", "Undefined": "other"}


def nanv(v):
    return v in ("", "nan", "NaN")


def map_2019(row):
    """1 行 → ("key", category) / ("ev", name) / ("count", name)。D3 の表（2019 年）。"""
    et, edit, ks = row["EventType"], row["EditType"], row["X-Keystroke"]
    if et == "File.Edit":
        if edit in ("Insert", "Delete", "Undo", "Redo") and nanv(ks):
            return ("ev", "auto-edit")
        if edit == "Insert":
            if ks == "space":
                return ("key", "space")
            if ks == "return":
                return ("key", "enter")
            if len(ks) == 1:
                return ("key", ca.char_category(ks))
            return ("count", "insert-unmapped")
        if edit == "Delete":
            return ("key", "backspace") if ks == "delete" else ("count", "delete-unmapped")
        if edit in ("Undo", "Redo"):
            return ("key", "other")
        if edit == "Paste":
            return ("ev", "paste")
        if edit == "Cut":
            return ("ev", "cut")
        if edit == "Drag":
            return ("ev", "drag")
        return ("count", "edit-" + (edit or "none"))
    if et == "Run.Program":
        return ("ev", "run")
    if et == "Submit":
        return ("ev", "submit")
    if et == "X-SwitchTask":
        return ("ev", "switch")
    return ("count", "type-" + et)


def map_2021(row):
    """1 行 → 同上。D3 の表（2021 年）。X-Paste・X-Copy の X-Metadata は読まない。"""
    et = row["EventType"]
    if et == "X-Keystroke":
        md = row["X-Metadata"]
        return ("key", ca.char_category(md)) if len(md) == 1 else ("count", "keystroke-unmapped")
    if et == "X-Action":
        md = row["X-Metadata"]
        if md in NAV_2021:
            return ("key", "navigation")
        if md in KEY_2021:
            return ("key", KEY_2021[md])
        if md == "EditorChange":
            return ("ev", "switch")
        return ("ev", "ide-action")
    if et == "File.Edit":
        # 長さだけで決める（"nan" ちょうどの挿入は 2019・2021 とも 0 件だったが、欠損の印と取り違えないため nanv は当てない）
        if row["EditType"] == "Insert" and len(row["InsertText"]) >= 2:
            return ("ev", "multi-insert")
        return ("count", "file-edit-other")
    if et == "X-Paste":
        return ("ev", "paste")
    if et == "X-Copy":
        return ("ev", "copy")
    if et == "Run.Program":
        return ("ev", "run") if row["X-Metadata"] == "Start" else ("count", "run-other")
    if et == "X-ReplayAction":
        return ("count", "replay-action")
    if et == "X-Attention":
        md = row["X-Metadata"]
        if md == "Lockout":
            return ("ev", "prompt")
        if md in ("ON", "OFF"):
            return ("ev", "prompt-answer")
        return ("count", "attention-other")
    return ("count", "type-" + et)


def read_units(year, path, hold):
    """{(subject, assignment): [(t, eid, kind, value), ...]}・Counter。取っておく書き手は読み飛ばす。"""
    csv.field_size_limit(10**9)
    fn = map_2019 if year == "2019" else map_2021
    units = defaultdict(list)
    cnt = Counter()
    held = set()
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            sid = row["SubjectID"]
            if sid in hold:
                held.add(sid)
                continue
            kind, val = fn(row)
            cnt[kind + ":" + val if kind != "key" else "key"] += 1
            if kind == "ev" and val == "multi-insert":
                # 事後（レビューの後・判定に使わない）: 空白と改行だけの挿入（Enter の自動インデントの候補）。数だけ
                it = row["InsertText"]
                if it.strip() == "":
                    cnt["posthoc:multi-insert-whitespace-only" + ("-with-newline" if "\n" in it else "")] += 1
            if kind == "count":
                continue
            t, eid = int(float(row["ClientTimestamp"])), int(row["EventID"])
            units[(sid, row["AssignmentID"])].append((t, eid, kind, val))
            if kind == "ev" and val == "multi-insert" and row["InsertText"].strip() == "":
                # 事後の印（判定に使わない）: 同じ時刻に「空白だけの挿入」の印を足す。主の集計は読まない
                units[(sid, row["AssignmentID"])].append((t, eid, "ev", "posthoc-ws"))
    return units, cnt, len(held)


def split_blocks(items):
    """時刻で並べた出来事を、隣接差が BLOCK_GAP 以上のところで切る（D2）。"""
    blocks, cur = [], []
    for it in items:
        if cur and it[0] - cur[-1][0] >= BLOCK_GAP:
            blocks.append(cur)
            cur = []
        cur.append(it)
    if cur:
        blocks.append(cur)
    return blocks


def key_rows(presses):
    """[(t, cat)] → key 行（離しは押下 + DWELL と次の押下の 1 ms 前の早いほう・convert.py と同じ）。"""
    out = []
    for i, (t, cat) in enumerate(presses):
        rel = t + DWELL
        if i + 1 < len(presses):
            rel = min(rel, max(t, presses[i + 1][0] - 1))
        out.append({"type": "key", "t": t, "category": cat, "press": True})
        out.append({"type": "key", "t": rel, "category": cat, "press": False})
    return out


def write_ndjson(path, t0, rows):
    with open(path, "w", encoding="utf-8") as f:
        f.write(json.dumps({"type": "meta", "session_start": t0, "hmm_version": ca.HMM_VERSION}) + "\n")
        f.write(json.dumps({"type": "session_start", "t": t0}) + "\n")
        for r in rows:
            f.write(json.dumps(r) + "\n")


def one(args):
    idx, items, replay, work = args
    out = os.path.join(work, "u%05d.json" % idx)
    items.sort(key=lambda x: (x[0], x[1]))
    blocks_raw = split_blocks(items)
    all_keys, all_feat, blocks, few, kept_ev = [], [], [], 0, []
    with tempfile.TemporaryDirectory(dir=work) as td:
        for b in blocks_raw:
            presses = [(t, v) for t, _, k, v in b if k == "key"]
            if len(presses) < 2:
                few += 1
                continue
            nd, fp = os.path.join(td, "b.ndjson"), os.path.join(td, "b_feat.json")
            write_ndjson(nd, presses[0][0], key_rows(presses))
            with open(fp, "w") as f:
                r = subprocess.run([replay, nd], stdout=f, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
            if r.returncode != 0:
                return {"status": "replay_failed"}
            all_feat += json.load(open(fp))
            all_keys += presses
            kept_ev += [[t, v] for t, _, k, v in b if k == "ev"]  # 捨てたまとまりの出来事は入れない（レビューの後）
            blocks.append([presses[0][0], presses[-1][0]])
            os.remove(nd)
            os.remove(fp)
        if len(all_keys) < 2:
            return {"status": "few_presses", "blocks_few": few}
        nd, fp, wf = (os.path.join(td, n) for n in ("u.ndjson", "u_feat.json", "u_withfeat.ndjson"))
        write_ndjson(nd, all_keys[0][0], key_rows(all_keys))
        # まとまりの間で離しが次のまとまりへ食い込まない（差は 30 分以上）ので、1 本に並べた key 行はまとまりごとと同じ
        json.dump(all_feat, open(fp, "w"))
        subprocess.run([sys.executable, "-I", ADD_FEAT, nd, fp, wf], check=True)
        ft = subprocess.run([replay, wf, "--features"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            stdin=subprocess.DEVNULL)
        if ft.returncode != 0:
            # replay の --features は時間の幅に上限がある（ADR-030 D9・100 万 tick ≈ 11.6 日）。何日にもまたがる
            # 課題は上限に当たる。条件（D2）は想定していなかったので、除いて数を報告する（notes の逸脱）。
            if b"time span too large" in ft.stderr:
                return {"status": "span_too_long",
                        "span_days": (all_keys[-1][0] - all_keys[0][0]) / 86_400_000}
            return {"status": "features_failed"}
        d = json.loads(ft.stdout)
    onsets = [[x["stall_onset"]["s_ms"], x["stall_onset"]["t0_ms"], x["stall_onset"]["theta_person_ms"],
               x["stall_onset"]["theta_sample_n"]] for x in d["rows"] if x["stall_onset"].get("is_onset")]
    rec = {"keys": [list(k) for k in all_keys],
           "feat": [[x["timestamp"], x["p_stuck"], x["ewma_x"]] for x in all_feat],
           "onsets": onsets,
           "observed_intervals": [[iv["a_ms"], iv["b_ms"]] for iv in d["provenance"]["observed_intervals"]],
           "blocks": blocks,
           "events": kept_ev,
           "span": [items[0][0], items[-1][0]],  # 課題の span（D6・最初から最後の出来事まで・捨てたまとまりも含む）
           "replay_stderr_bytes": len(ft.stderr)}
    with open(out + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(out + ".tmp", out)
    return {"status": "ok", "blocks": len(blocks), "blocks_few": few, "onsets": len(onsets)}


def main():
    argv = sys.argv[1:]
    limit = None
    if "--limit" in argv:
        i = argv.index("--limit")
        limit = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    year, path, replay, hold_p, work, jobs = argv
    hold = set(json.load(open(hold_p))[year])
    os.makedirs(work, exist_ok=True)
    if any(n.startswith("u") and n.endswith(".json") for n in os.listdir(work)):
        sys.exit("work_dir に前の中間がある。空の場所で作り直す")
    units, cnt, held = read_units(year, path, hold)
    keys = sorted(units)
    subj = sorted({s for s, _ in keys})
    s_idx = {s: i for i, s in enumerate(subj)}
    tasks = [(i, units[k], replay, work) for i, k in enumerate(keys)]
    unit_w = {"%05d" % i: s_idx[k[0]] for i, k in enumerate(keys)}
    if limit:
        tasks = tasks[:limit]
    status, blocks, few, too_long = Counter(), 0, 0, []
    with ProcessPoolExecutor(int(jobs)) as ex:
        for res in ex.map(one, tasks, chunksize=1):
            status[res["status"]] += 1
            blocks += res.get("blocks", 0)
            few += res.get("blocks_few", 0)
            if "span_days" in res:
                too_long.append(round(res["span_days"], 1))
    ok = {k: v for k, v in unit_w.items() if os.path.exists(os.path.join(work, "u%s.json" % k))}
    json.dump(ok, open(os.path.join(work, "units.json"), "w"))
    # 書き手の通し番号 → SubjectID の SHA-256（repo の外）。解析が取っておく書き手と照らす（D4・レビューの後）
    json.dump({str(s_idx[s]): hashlib.sha256(s.encode()).hexdigest() for s in subj},
              open(os.path.join(work, "writers.json"), "w"))
    summ = {"year": year, "writers_read": len(subj), "holdout_writers_skipped": held, "units": len(keys),
            "attempted": len(tasks), "holdout_sha256": hashlib.sha256(open(hold_p, "rb").read()).hexdigest(),
            "status": status, "blocks_kept": blocks, "blocks_few_presses": few,
            "span_too_long_days": sorted(too_long), "row_kinds": dict(sorted(cnt.items()))}
    json.dump(summ, open(os.path.join(work, "build_summary.json"), "w"), indent=1)
    print(json.dumps({k: v for k, v in summ.items() if k != "row_kinds"}))
    if status["ok"] + status["few_presses"] + status["span_too_long"] != len(tasks):
        sys.exit("失敗した課題がある")


def selftest():
    r = lambda **kw: {"EventType": "File.Edit", "EditType": "", "X-Keystroke": "nan", "X-Metadata": "nan",  # noqa: E731
                      "InsertText": "", **kw}
    assert map_2019(r(EditType="Insert", **{"X-Keystroke": "a"})) == ("key", "alphabetic")
    assert map_2019(r(EditType="Insert", **{"X-Keystroke": "space"})) == ("key", "space")
    assert map_2019(r(EditType="Insert", **{"X-Keystroke": "return"})) == ("key", "enter")
    assert map_2019(r(EditType="Delete", **{"X-Keystroke": "delete"})) == ("key", "backspace")
    assert map_2019(r(EditType="Insert")) == ("ev", "auto-edit")
    assert map_2019(r(EditType="Paste")) == ("ev", "paste")
    assert map_2019(r(EditType="Undo", **{"X-Keystroke": "z"})) == ("key", "other")
    assert map_2019(r(EventType="X-SwitchTask")) == ("ev", "switch")
    assert map_2019(r(EditType="X-Checkpoint"))[0] == "count"
    assert map_2021(r(EventType="X-Keystroke", **{"X-Metadata": "("})) == ("key", "punctuation")
    assert map_2021(r(EventType="X-Action", **{"X-Metadata": "Up"})) == ("key", "navigation")
    assert map_2021(r(EventType="X-Action", **{"X-Metadata": "Backspace"})) == ("key", "backspace")
    assert map_2021(r(EventType="X-Action", **{"X-Metadata": "EditorChange"})) == ("ev", "switch")
    assert map_2021(r(EventType="X-Action", **{"X-Metadata": "Comment Line"})) == ("ev", "ide-action")
    assert map_2021(r(EditType="Insert", InsertText="()")) == ("ev", "multi-insert")
    assert map_2021(r(EditType="Insert", InsertText="a"))[0] == "count"
    assert map_2021(r(EditType="Insert", InsertText="nan")) == ("ev", "multi-insert")
    assert map_2021(r(EventType="X-Paste", **{"X-Metadata": "should-not-matter"})) == ("ev", "paste")
    assert map_2021(r(EventType="Run.Program", **{"X-Metadata": "Start"})) == ("ev", "run")
    assert map_2021(r(EventType="Run.Program", **{"X-Metadata": "0"}))[0] == "count"
    assert map_2021(r(EventType="X-Attention", **{"X-Metadata": "Lockout"})) == ("ev", "prompt")
    assert map_2021(r(EventType="X-Attention", **{"X-Metadata": "OFF"})) == ("ev", "prompt-answer")
    it = [(0, 1, "key", "a"), (1_799_999, 2, "key", "a"), (3_599_999, 3, "key", "a"), (3_600_000, 4, "ev", "run")]
    b = split_blocks(it)
    assert [len(x) for x in b] == [2, 2], b  # 1,799,999 ms は同じまとまり・1,800,000 ms で切る
    kr = key_rows([(0, "a"), (50, "a"), (1000, "a")])
    assert [x["t"] for x in kr] == [0, 49, 50, 150, 1000, 1100]
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        main()
