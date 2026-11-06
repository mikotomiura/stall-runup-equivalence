"""CoAuthor の中間を、HMM の観測の記号（obs）を足して作り直す（conditions.md の Q1）。

使い方: python3 -I build_obs.py <coauthor.zip> <gse-replay> <old_work> <new_work> <jobs> <metadata.csv>...
- 作るセッションは既存の中間（old_work）の sessions.json にある番号だけ。取っておいた書き手は既存の中間を
  作る段で除かれている。holdout.json は開かない。番号はメタデータのセッション ID の昇順の通し番号で、
  既存の中間（experiments/20261102-public-data-landscape/scripts/build_coauthor.py）と同じ規則。
- 手順は build_coauthor.one と同じ（convert → replay → add_feat → replay --features）。違いは feat を
  [[t, p_flow, p_inc, p_stuck, ewma_x, ewma_y, obs, alpha], ...] にすることだけ。
- 中間の jsonl・NDJSON・replay の出力は 1 本ごとに消す。本文を書き出さない。
"""
import csv
import json
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile
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
        ca.convert(src, nd, ev_p, DWELL)
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
           "feat": [[x["timestamp"], x["p_flow"], x["p_inc"], x["p_stuck"], x["ewma_x"], x["ewma_y"], x["obs"], x["alpha"]]
                    for x in feat],
           "onsets": onsets,
           "observed_intervals": [[iv["a_ms"], iv["b_ms"]] for iv in d["provenance"]["observed_intervals"]],
           "events": [[e["t"], e["name"]] for e in evs],
           "replay_stderr_bytes": len(r.stderr) + len(ft.stderr)}
    with open(out + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(out + ".tmp", out)
    return {"status": "ok"}


def main():
    zp, replay, old, new, jobs = sys.argv[1:6]
    metas = sys.argv[6:]
    keep = json.load(open(os.path.join(old, "sessions.json")))
    sid2w = {}
    for p in metas:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            sid2w[r["session_id"]] = r["worker_id"]
    with zipfile.ZipFile(zp) as z:
        by_sid = {os.path.basename(m)[:-len(".jsonl")]: m for m in z.namelist() if m.endswith(".jsonl")}
    sids = sorted(set(by_sid) & set(sid2w))
    os.makedirs(new, exist_ok=True)
    if any(n.startswith("s") and n.endswith(".json") for n in os.listdir(new)):
        sys.exit("new_work に前の中間がある。空の場所で作り直す")
    tasks = [(zp, by_sid[sids[int(k)]], int(k), replay, new) for k in sorted(keep)]
    status = {}
    with ProcessPoolExecutor(int(jobs)) as ex:
        for res in ex.map(one, tasks, chunksize=4):
            status[res["status"]] = status.get(res["status"], 0) + 1
    shutil.copy(os.path.join(old, "sessions.json"), os.path.join(new, "sessions.json"))
    json.dump({"attempted": len(tasks), "status": status}, open(os.path.join(new, "build_summary.json"), "w"), indent=1)
    print(json.dumps(status))
    if status.get("ok", 0) != len(tasks):
        sys.exit("作れなかったセッションがある")


if __name__ == "__main__":
    main()
