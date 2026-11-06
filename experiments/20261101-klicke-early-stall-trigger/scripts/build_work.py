"""KLiCKe の全セッションを 1 本ずつ変換 → replay → add_feat → replay --features し、解析が読む項目だけを残す（K3）。

使い方: python3 -I build_work.py <WritingTask.zip> <gse-replay> <add_feat.py> <work_dir> [jobs] [limit]
出力: <work_dir>/w<番号>.json（番号は zip の中の名前の昇順の通し番号。書き手の ID は書かない）と
<work_dir>/build_summary.json。各 w*.json は
  keys:   [[t_press, category], ...]（押下だけ・時刻順）
  feat:   [[t, p_stuck, ewma_x], ...]
  onsets: [[s_ms, t0_ms, theta_person_ms, theta_sample_n], ...]
  observed_intervals: [[a_ms, b_ms], ...]（replay の封筒の値・idle_trim の後）
中間の NDJSON と replay の出力は 1 本ごとに消す。
"""
import json
import os
import subprocess
import sys
import tempfile
import zipfile
from collections import Counter
from concurrent.futures import ProcessPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import convert_klicke as ck  # noqa: E402

MIN_PRESSES = 200  # K3


def one(args):
    zp, member, idx, replay, add_feat, work = args
    out = os.path.join(work, "w%05d.json" % idx)
    with zipfile.ZipFile(zp) as z:
        raw = z.read(member)
    try:
        ev, enc = ck.read_events(raw)
    except ck.LogError as e:
        return {"status": "unreadable:" + str(e)}
    keys, mouse = ck.to_keys(ev)
    if len(keys) < MIN_PRESSES:
        return {"status": "few_presses", "mouse": mouse}
    with tempfile.TemporaryDirectory(dir=work) as td:
        nd = os.path.join(td, "s.ndjson")
        ck.write_ndjson(keys, nd)
        feat_p = os.path.join(td, "feat.json")
        with open(feat_p, "w") as f:
            r = subprocess.run([replay, nd], stdout=f, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
        if r.returncode != 0:
            return {"status": "replay_failed", "stderr_bytes": len(r.stderr)}
        wf = os.path.join(td, "withfeat.ndjson")
        subprocess.run([sys.executable, "-I", add_feat, nd, feat_p, wf], check=True)
        ft = subprocess.run([replay, wf, "--features"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            stdin=subprocess.DEVNULL)
        if ft.returncode != 0:
            return {"status": "features_failed", "stderr_bytes": len(ft.stderr)}
        feat = json.load(open(feat_p))
        d = json.loads(ft.stdout)
    onsets = [[r["stall_onset"]["s_ms"], r["stall_onset"]["t0_ms"], r["stall_onset"]["theta_person_ms"],
               r["stall_onset"]["theta_sample_n"]] for r in d["rows"] if r["stall_onset"].get("is_onset")]
    rec = {"keys": [[k[0], k[2]] for k in keys],
           "feat": [[x["timestamp"], x["p_stuck"], x["ewma_x"]] for x in feat],
           "onsets": onsets,
           # replay が刈り込み（idle_trim）の後に作った観測区間（解析はこれを使う・K4）
           "observed_intervals": [[iv["a_ms"], iv["b_ms"]] for iv in d["provenance"]["observed_intervals"]],
           "idle_trim_dropped": d["provenance"]["idle_trim"]["dropped_feat_rows"],
           "input_gap_trim_dropped": d["provenance"]["input_gap_trim"]["dropped_feat_rows"],
           "replay_stderr_bytes": len(r.stderr) + len(ft.stderr)}
    with open(out + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(out + ".tmp", out)
    return {"status": "ok", "encoding": enc, "mouse": mouse, "onsets": len(onsets)}


def main():
    zp, replay, add_feat, work = sys.argv[1:5]
    jobs = int(sys.argv[5]) if len(sys.argv) > 5 else 4
    limit = int(sys.argv[6]) if len(sys.argv) > 6 else None
    os.makedirs(work, exist_ok=True)
    if any(n.startswith("w") and n.endswith(".json") for n in os.listdir(work)):
        sys.exit("work_dir に前の中間がある。空の場所で作り直す（中間を使い回さない）")
    with zipfile.ZipFile(zp) as z:
        members = sorted(m for m in z.namelist() if "keystrokelogs/csv/" in m and m.endswith(".csv"))
    if limit:
        members = members[:limit]
    tasks = [(zp, m, i, replay, add_feat, work) for i, m in enumerate(members)]
    status = Counter()
    with ProcessPoolExecutor(jobs) as ex:
        for res in ex.map(one, tasks, chunksize=4):
            status[res["status"]] += 1
    json.dump({"members": len(members), "status": status}, open(os.path.join(work, "build_summary.json"), "w"),
              indent=1)
    print(json.dumps(status))
    if status["ok"] != len(members):
        sys.exit("ok でないセッションがある: %s" % json.dumps(status))


if __name__ == "__main__":
    main()
