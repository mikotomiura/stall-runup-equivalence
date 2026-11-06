"""CoAuthor part of the equivalence gate (see ../README.md, "Equivalence gate").

Usage: python3 -I coauthor_gate.py <repo_root> <coauthor.zip> <metadata_creative.csv> <metadata_argumentative.csv>
                                   <sessions.json> <replay> <out_dir> <jobs>

Runs, with <replay> standing in for gse-replay (normally gate/compare-replay.sh):
  1. the writer-level path of experiments/20261106-findings-paper1-exploration/scripts/coauthor_writer_onsets.py
     (its `one_writer`, unchanged: convert -> replay -> add_feat -> concat_writer -> replay --features), and
  2. the session-level `replay <session.ndjson> --features` of experiments/20261031-coauthor-pre-request-signal/run.sh,
for the seen writers only (the session numbers in <sessions.json>; the holdout list is not opened).
`coauthor_writer_onsets.main` is not called because it refuses any replay other than the reference binary
(SHA-256 check); the input selection below repeats its rule (metadata session IDs ∩ zip, ascending) and its
SHA-256 checks of the zip and the two metadata files.
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


def session_features(args):
    zp, member, replay = args
    import convert as ca  # noqa: E402  (on sys.path in the worker, set below)
    with tempfile.TemporaryDirectory() as td:
        src, nd, ev = (os.path.join(td, n) for n in ("s.jsonl", "s.ndjson", "s_ev.json"))
        with zipfile.ZipFile(zp) as z, open(src, "wb") as f:
            f.write(z.read(member))
        ca.convert(src, nd, ev, 100)
        r = subprocess.run([replay, nd, "--features"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           stdin=subprocess.DEVNULL)
    return r.returncode


def main():
    repo, zp, meta_c, meta_a, sessions_p, replay, out_dir, jobs = sys.argv[1:9]
    exp = os.path.join(repo, "experiments/20261106-findings-paper1-exploration/scripts")
    item1 = os.path.join(repo, "experiments/20261031-coauthor-pre-request-signal/scripts")
    sys.path[:0] = [exp, item1]
    # spawned workers receive this sys.path from multiprocessing (its preparation data)
    import coauthor_writer_onsets as cw  # noqa: E402
    for p, want in [(zp, cw.SHA_ZIP), (meta_c, cw.SHA_META[0]), (meta_a, cw.SHA_META[1])]:
        if hashlib.sha256(open(p, "rb").read()).hexdigest() != want:
            sys.exit("input SHA-256 differs from the one the intermediate was built from: " + os.path.basename(p))
    keep = json.load(open(sessions_p))
    sid2w = {}
    for p in (meta_c, meta_a):
        for r in csv.DictReader(open(p, encoding="utf-8")):
            sid2w[r["session_id"]] = r["worker_id"]
    with zipfile.ZipFile(zp) as z:
        by_sid = {os.path.basename(m)[:-len(".jsonl")]: m for m in z.namelist() if m.endswith(".jsonl")}
    sids = sorted(set(by_sid) & set(sid2w))
    by_w = {}
    for k in sorted(keep):
        by_w.setdefault(keep[k], []).append((k, by_sid[sids[int(k)]]))
    os.makedirs(out_dir, exist_ok=True)
    if os.listdir(out_dir):
        sys.exit("out_dir is not empty")
    tasks = [(zp, replay, wid, by_w[wid], out_dir) for wid in sorted(by_w)]
    status = {}
    with ProcessPoolExecutor(int(jobs)) as ex:
        for _, res in ex.map(cw.one_writer, tasks, chunksize=1):
            key = "error:" + res["error"] if "error" in res else "ok"
            status[key] = status.get(key, 0) + 1
        members = [m for wid in sorted(by_w) for _, m in by_w[wid]]
        rcs = list(ex.map(session_features, [(zp, m, replay) for m in members], chunksize=4))
    summary = {"writers": len(tasks), "sessions": len(members), "writer_status": status,
               "session_features_exit_codes": {str(c): rcs.count(c) for c in sorted(set(rcs))}}
    json.dump(summary, open(os.path.join(out_dir, "coauthor_gate_summary.json"), "w"), indent=1)
    print(json.dumps(summary))
    # a failure both binaries share would still compare as a match, so any failure fails this step
    if set(status) != {"ok"} or set(rcs) != {0}:
        sys.exit("a writer or a session failed (see coauthor_gate_summary.json)")


if __name__ == "__main__":
    main()
