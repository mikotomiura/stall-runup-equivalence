"""CoAuthor を 1 セッションずつ変換 → replay → add_feat → replay --features し、解析が読む項目だけを残す（C3）。

使い方: python3 -I build_coauthor.py <coauthor.zip> <gse-replay> <holdout.json> <work_dir> <jobs> <metadata.csv>... [--limit N]
出力（repo の外）: <work_dir>/s<番号>.json（番号はメタデータのセッション ID の昇順の通し番号）と
<work_dir>/sessions.json（番号 → 書き手の通し番号。書き手の ID は書かない）・build_summary.json。各 s*.json は
  keys:   [[t_press, category], ...]
  feat:   [[t, p_stuck, ewma_x], ...]
  onsets: [[s_ms, t0_ms, theta_person_ms, theta_sample_n], ...]
  observed_intervals: [[a_ms, b_ms], ...]
  events: [[t, name], ...]（提案まわり・API の挿入・複数の文字の挿入の時刻と名前だけ）
取っておく書き手のセッションは読まない。中間の jsonl・NDJSON・replay の出力は 1 本ごとに消す。
"""
import csv
import json
import os
import subprocess
import sys
import tempfile
import zipfile
from collections import Counter
from concurrent.futures import ProcessPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ITEM1 = os.path.join(HERE, "../../20261031-coauthor-pre-request-signal/scripts")
sys.path.insert(0, ITEM1)
import convert as ca  # noqa: E402

DWELL = 100
ADD_FEAT = os.path.join(ITEM1, "add_feat.py")


def one(args):
    zp, member, idx, replay, work = args
    out = os.path.join(work, "s%05d.json" % idx)
    with tempfile.TemporaryDirectory(dir=work) as td:
        src = os.path.join(td, "s.jsonl")
        with zipfile.ZipFile(zp) as z, open(src, "wb") as f:
            f.write(z.read(member))
        nd, ev_p = os.path.join(td, "s.ndjson"), os.path.join(td, "ev.json")
        stats = ca.convert(src, nd, ev_p, DWELL)
        os.remove(src)
        keys = [json.loads(line) for line in open(nd, encoding="utf-8")]
        presses = [[r["t"], r["category"]] for r in keys if r["type"] == "key" and r["press"]]
        if len(presses) < 2:
            return {"status": "few_presses"}
        feat_p = os.path.join(td, "feat.json")
        with open(feat_p, "w") as f:
            r = subprocess.run([replay, nd], stdout=f, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
        if r.returncode != 0:
            return {"status": "replay_failed"}
        wf = os.path.join(td, "withfeat.ndjson")
        subprocess.run([sys.executable, "-I", ADD_FEAT, nd, feat_p, wf], check=True)
        ft = subprocess.run([replay, wf, "--features"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            stdin=subprocess.DEVNULL)
        if ft.returncode != 0:
            return {"status": "features_failed"}
        feat = json.load(open(feat_p))
        d = json.loads(ft.stdout)
        evs = json.load(open(ev_p))["events"]
    onsets = [[x["stall_onset"]["s_ms"], x["stall_onset"]["t0_ms"], x["stall_onset"]["theta_person_ms"],
               x["stall_onset"]["theta_sample_n"]] for x in d["rows"] if x["stall_onset"].get("is_onset")]
    rec = {"keys": presses,
           "feat": [[x["timestamp"], x["p_stuck"], x["ewma_x"]] for x in feat],
           "onsets": onsets,
           "observed_intervals": [[iv["a_ms"], iv["b_ms"]] for iv in d["provenance"]["observed_intervals"]],
           "events": [[e["t"], e["name"]] for e in evs],
           "replay_stderr_bytes": len(r.stderr) + len(ft.stderr)}
    with open(out + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(out + ".tmp", out)
    return {"status": "ok", "onsets": len(onsets), "multi_dropped": stats["user_insert_multi_dropped"]}


def main():
    argv = sys.argv[1:]
    limit = None
    if "--limit" in argv:
        i = argv.index("--limit")
        limit = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    zp, replay, hold_p, work, jobs = argv[:5]
    metas = argv[5:]
    hold = set(json.load(open(hold_p))["coauthor"])
    sid2w = {}
    for p in metas:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            sid2w[r["session_id"]] = r["worker_id"]
    os.makedirs(work, exist_ok=True)
    if any(n.startswith("s") and n.endswith(".json") for n in os.listdir(work)):
        sys.exit("work_dir に前の中間がある。空の場所で作り直す")
    with zipfile.ZipFile(zp) as z:
        by_sid = {os.path.basename(m)[:-len(".jsonl")]: m for m in z.namelist() if m.endswith(".jsonl")}
    sids = sorted(set(by_sid) & set(sid2w))
    no_meta = len(set(by_sid) - set(sid2w))
    workers = sorted(set(sid2w[s] for s in sids))
    w_idx = {w: i for i, w in enumerate(workers)}
    tasks, sessions, held = [], {}, 0
    for i, s in enumerate(sids):
        if sid2w[s] in hold:
            held += 1
            continue
        tasks.append((zp, by_sid[s], i, replay, work))
        sessions["%05d" % i] = w_idx[sid2w[s]]
    if limit:
        tasks = tasks[:limit]
        sessions = {("%05d" % t[2]): sessions["%05d" % t[2]] for t in tasks}
    status = Counter()
    with ProcessPoolExecutor(int(jobs)) as ex:
        for res in ex.map(one, tasks, chunksize=4):
            status[res["status"]] += 1
    ok = {k: v for k, v in sessions.items() if os.path.exists(os.path.join(work, "s%s.json" % k))}
    json.dump(ok, open(os.path.join(work, "sessions.json"), "w"))
    summ = {"jsonl": len(by_sid), "no_metadata": no_meta, "sessions_with_metadata": len(sids),
            "held_out_sessions": held, "attempted": len(tasks), "status": status}
    json.dump(summ, open(os.path.join(work, "build_summary.json"), "w"), indent=1)
    print(json.dumps(summ))
    if status["ok"] + status["few_presses"] != len(tasks):
        sys.exit("失敗したセッションがある")


if __name__ == "__main__":
    main()
