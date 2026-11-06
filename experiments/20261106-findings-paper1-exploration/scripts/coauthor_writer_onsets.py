"""門 G2 の中間（conditions.md X6）: 見た書き手のセッションだけを zip から読み、付録 A4 の作り方
（convert → replay → add_feat → concat_writer → replay --features）で書き手ごとにつないだ onset を作る。

使い方: python3 -I coauthor_writer_onsets.py <coauthor.zip> <gse-replay> <coauthor_work> <out_dir> <jobs> <metadata.csv>...
- 読むセッションは <coauthor_work>/sessions.json にある番号だけ（取っておいた書き手は中間を作る段で除かれている）。
  holdout.json は開かない。番号の規則は build_obs.py と同じ（メタデータのセッション ID の昇順の通し番号）。
- 出力は書き手ごとに <out_dir>/w<書き手の通し番号>.json:
  {"onsets": [[セッションの番号, s0, t0, θ, 標本の数], ...]（元の時刻）, "unassigned": n, "stderr_bytes": n}
  中間の jsonl・NDJSON・replay の出力は書き手ごとに消す。本文を書き出さない。
"""
import csv
import hashlib
import json
import os
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
SHA_ZIP = "8d7380ca3811c671e774e43bfb9f5d3265e2a5d425788a066a612831aa1e418b"
SHA_REPLAY = "26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3"
SHA_META = ("66fd2ca541999412de4050a85c17274f02eea04424e83906b697f4cd824c65bf",   # metadata_creative.csv
            "cfadf031b21c07951f648b02422b6703850359c7ff6a78a8ef513624c8e4c0d1")   # metadata_argumentative.csv
ADD_FEAT = os.path.join(ITEM1, "add_feat.py")
CONCAT = os.path.join(ITEM1, "concat_writer.py")


def one_writer(args):
    zp, replay, wid, members, out_dir = args
    stderr = 0
    with tempfile.TemporaryDirectory() as td:
        files, spans_local = [], {}
        for k, member in members:
            src = os.path.join(td, "%s.jsonl" % k)
            with zipfile.ZipFile(zp) as z, open(src, "wb") as f:
                f.write(z.read(member))
            nd, ev_p = os.path.join(td, "%s.ndjson" % k), os.path.join(td, "%s_ev.json" % k)
            ca.convert(src, nd, ev_p, DWELL)
            os.remove(src)
            os.remove(ev_p)
            feat_p = os.path.join(td, "%s_feat.json" % k)
            with open(feat_p, "w") as f:
                r = subprocess.run([replay, nd], stdout=f, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
            stderr += len(r.stderr)
            if r.returncode != 0:
                return wid, {"error": "replay_failed"}
            wf = os.path.join(td, "%s.withfeat.ndjson" % k)
            subprocess.run([sys.executable, "-I", ADD_FEAT, nd, feat_p, wf], check=True)
            os.remove(nd)
            os.remove(feat_p)
            rows = [json.loads(line) for line in open(wf, encoding="utf-8")]
            ts = [x["t"] for x in rows if x["type"] in ("key", "feat")]
            if ts:
                spans_local[k] = (min(ts), max(ts))
                files.append(wf)
        cat, offs_p = os.path.join(td, "writer.ndjson"), os.path.join(td, "offsets.json")
        subprocess.run([sys.executable, "-I", CONCAT, cat, offs_p] + files, check=True, stdin=subprocess.DEVNULL)
        offs = json.load(open(offs_p))
        ft = subprocess.run([replay, cat, "--features"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            stdin=subprocess.DEVNULL)
        stderr += len(ft.stderr)
        if ft.returncode != 0:
            return wid, {"error": "features_failed"}
        d = json.loads(ft.stdout)
    spans = sorted((a + offs[k], b + offs[k], k) for k, (a, b) in spans_local.items() if k in offs)
    out, unassigned = [], 0
    for x in d["rows"]:
        so = x["stall_onset"]
        if not so.get("is_onset"):
            continue
        s0 = so["s_ms"]
        hit = next((k for a, b, k in spans if a <= s0 <= b), None)
        if hit is None:
            unassigned += 1
            continue
        off = offs[hit]
        out.append([hit, s0 - off, so["t0_ms"] - off, so["theta_person_ms"], so["theta_sample_n"]])
    rec = {"onsets": out, "unassigned": unassigned, "stderr_bytes": stderr}
    p = os.path.join(out_dir, "w%s.json" % wid)
    with open(p + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(p + ".tmp", p)
    return wid, {"onsets": len(out), "unassigned": unassigned, "stderr_bytes": stderr}


def main():
    zp, replay, old, out_dir, jobs = sys.argv[1:6]
    metas = sys.argv[6:]
    # 通し番号の規則（メタデータのセッション ID ∩ zip の昇順）が中間を作った時と同じ集合の上にあることを、
    # 読む前に入力の SHA-256 で確かめる（experiments/20261103-coauthor-per-person/run.sh と同じ値）。
    # 違えば番号が別のセッション（取っておいた書き手を含みうる）を指すので止める。
    for p, want in [(zp, SHA_ZIP), (replay, SHA_REPLAY)] + list(zip(metas, SHA_META)):
        if hashlib.sha256(open(p, "rb").read()).hexdigest() != want:
            sys.exit("入力の SHA-256 が中間を作った時と違う: " + os.path.basename(p))
    if len(metas) != len(SHA_META):
        sys.exit("メタデータは creative・argumentative の 2 本をこの順で渡す")
    keep = json.load(open(os.path.join(old, "sessions.json")))  # 番号 -> 書き手の通し番号
    sid2w = {}
    for p in metas:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            sid2w[r["session_id"]] = r["worker_id"]
    with zipfile.ZipFile(zp) as z:
        by_sid = {os.path.basename(m)[:-len(".jsonl")]: m for m in z.namelist() if m.endswith(".jsonl")}
    sids = sorted(set(by_sid) & set(sid2w))
    os.makedirs(out_dir, exist_ok=True)
    if any(n.startswith("w") and n.endswith(".json") for n in os.listdir(out_dir)):
        sys.exit("out_dir に前の中間がある。空の場所で作り直す")
    by_w = {}
    for k in sorted(keep):
        by_w.setdefault(keep[k], []).append((k, by_sid[sids[int(k)]]))
    tasks = [(zp, replay, wid, by_w[wid], out_dir) for wid in sorted(by_w)]
    summary = {"writers": len(tasks), "sessions": sum(len(v) for v in by_w.values()), "status": {}}
    with ProcessPoolExecutor(int(jobs)) as ex:
        for wid, res in ex.map(one_writer, tasks):
            key = res.get("error", "ok")
            summary["status"][key] = summary["status"].get(key, 0) + 1
            if key == "ok":
                for f in ("onsets", "unassigned", "stderr_bytes"):
                    summary[f] = summary.get(f, 0) + res[f]
    json.dump(summary, open(os.path.join(out_dir, "build_summary.json"), "w"), indent=1)
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
