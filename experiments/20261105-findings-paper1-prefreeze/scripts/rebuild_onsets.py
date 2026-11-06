"""感度（事前登録の P8）のために、CS1 の中間の課題ごとに replay --features だけを、停滞の定義の設定を変えて回し直す。

使い方:
  python3 -I rebuild_onsets.py <work_in> <work_out> <gse-replay> <jobs> [--set key=value ...]
  python3 -I rebuild_onsets.py --selftest
中間の u*.json の keys と feat から、build_cs1.py と同じ key 行と feat 行の NDJSON を作り直し、
--features（--set を渡す）の onset と観測区間で onsets・observed_intervals を置き換える。ほかの項目
（keys・feat・blocks・events）はそのまま写す。推定の行（feat）は停滞の定義に依らないので回し直さない。
feat 行は p_stuck・ewma_x だけを持つ（中間はほかの量を残していない）。停滞の始まりと観測区間は押下と
feat 行の時刻だけで決まるので、--set なしで回したときに元の onsets・observed_intervals と全件一致する
ことを陽性対照として確かめる（--set なしで走らせ、mismatch が 0 であることを見る）。
出力は repo の外。キーの文字は元から中間に無い。
"""
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
CS1_DIR = os.path.join(HERE, "../../20261104-cs1-landscape/scripts")
sys.path.insert(0, CS1_DIR)
import build_cs1 as bc  # noqa: E402  key_rows・write_ndjson・ADD_FEAT

FEAT_KEYS = ["f1", "f3", "f4", "f5", "f6", "p_flow", "p_inc", "p_stuck", "raw_x", "raw_y",
             "ewma_x", "ewma_y", "obs", "alpha", "keys", "bs_keys"]


def feat_rows(feat):
    out = []
    for t, ps, ex in feat:
        r = {k: 0.0 for k in FEAT_KEYS}
        r.update(timestamp=t, p_stuck=ps, ewma_x=ex)
        out.append(r)
    return out


def one(args):
    p_in, p_out, replay, sets = args
    d = json.load(open(p_in))
    keys = [tuple(k) for k in d["keys"]]
    with tempfile.TemporaryDirectory() as td:
        nd, fp, wf = (os.path.join(td, n) for n in ("u.ndjson", "u_feat.json", "u_withfeat.ndjson"))
        bc.write_ndjson(nd, keys[0][0], bc.key_rows(keys))
        json.dump(feat_rows(d["feat"]), open(fp, "w"))
        subprocess.run([sys.executable, "-I", bc.ADD_FEAT, nd, fp, wf], check=True)
        cmd = [replay, wf, "--features"] + sum((["--set", s] for s in sets), [])
        ft = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
        if ft.returncode != 0:
            return {"status": "features_failed", "stderr": ft.stderr.decode(errors="replace")[:300]}
        e = json.loads(ft.stdout)
    onsets = [[x["stall_onset"]["s_ms"], x["stall_onset"]["t0_ms"], x["stall_onset"]["theta_person_ms"],
               x["stall_onset"]["theta_sample_n"]] for x in e["rows"] if x["stall_onset"].get("is_onset")]
    iv = [[v["a_ms"], v["b_ms"]] for v in e["provenance"]["observed_intervals"]]
    same = onsets == d["onsets"] and iv == d["observed_intervals"]
    rec = dict(d)
    rec.update(onsets=onsets, observed_intervals=iv, replay_stderr_bytes=len(ft.stderr),
               config_overrides=e.get("config", {}).get("overrides"))
    with open(p_out + ".tmp", "w") as f:
        json.dump(rec, f, separators=(",", ":"))
    os.replace(p_out + ".tmp", p_out)
    return {"status": "ok", "same_as_input": same, "onsets": len(onsets), "onsets_in": len(d["onsets"]),
            "stderr_bytes": len(ft.stderr), "overrides": e.get("config", {}).get("overrides")}


def main(argv):
    w_in, w_out, replay, jobs = argv[:4]
    rest = argv[4:]
    sets = [rest[i + 1] for i in range(len(rest)) if rest[i] == "--set"]
    os.makedirs(w_out, exist_ok=True)
    if any(n.startswith("u") and n.endswith(".json") for n in os.listdir(w_out)):
        sys.exit("work_out に前の中間がある。空の場所で作り直す")
    names = sorted(n for n in os.listdir(w_in) if n.startswith("u") and n[1:-5].isdigit() and n.endswith(".json"))
    tasks = [(os.path.join(w_in, n), os.path.join(w_out, n), replay, sets) for n in names]
    summ = {"sets": sets, "units": len(tasks), "ok": 0, "failed": 0, "same_as_input": 0,
            "onsets_out": 0, "onsets_in": 0, "stderr_nonempty": 0, "overrides": None}
    with ProcessPoolExecutor(int(jobs)) as ex:
        for r in ex.map(one, tasks, chunksize=4):
            if r["status"] != "ok":
                summ["failed"] += 1
                continue
            summ["ok"] += 1
            summ["same_as_input"] += r["same_as_input"]
            summ["onsets_out"] += r["onsets"]
            summ["onsets_in"] += r["onsets_in"]
            summ["stderr_nonempty"] += r["stderr_bytes"] != 0
            summ["overrides"] = r["overrides"]
    for n in ("units.json", "writers.json"):
        with open(os.path.join(w_in, n)) as f, open(os.path.join(w_out, n), "w") as g:
            g.write(f.read())
    json.dump(summ, open(os.path.join(w_out, "rebuild_summary.json"), "w"), indent=1)
    print(json.dumps(summ))
    if summ["failed"]:
        sys.exit("回し直しに失敗した課題がある")


def selftest():
    r = feat_rows([[1000, 0.2, 0.3]])
    assert r[0]["timestamp"] == 1000 and r[0]["p_stuck"] == 0.2 and r[0]["ewma_x"] == 0.3 and r[0]["f1"] == 0.0
    assert set(FEAT_KEYS) <= set(r[0])
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        main(sys.argv[1:])
