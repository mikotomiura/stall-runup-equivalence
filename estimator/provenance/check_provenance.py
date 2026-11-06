#!/usr/bin/env python3
"""Checks how each source file of this crate relates to GSE commit 24767c7e (Python standard library only).

  check_provenance.py --self                    every file listed in manifest.json matches its recorded SHA-256
  check_provenance.py --upstream <gse-checkout> the relation recorded in manifest.json holds against the GSE
                                                source at the recorded commit (read with `git show`)
  check_provenance.py --write <gse-checkout>    (re)writes manifest.json and the diffs under provenance/

Kinds:
  file     byte-identical to the upstream file.
  trimmed  the upstream file up to (not including) its first line `#[cfg(test)]`, with trailing blank lines
           removed; nothing else changed. Used where the upstream unit tests need modules this crate leaves out.
  excerpt  the lines between `// @excerpt-begin <id>` and `// @excerpt-end <id>` equal upstream lines
           [upstream_start, upstream_end] (1-based, inclusive).
  cut      written by removing code from the upstream file; the full change is provenance/<name>.diff
           (`diff -u` upstream -> this crate).
  lock     Cargo.lock: every package in it, except this crate itself, appears in the upstream Cargo.lock with the
           same name, version, source and checksum (cargo pruned the packages this crate does not use).
  new      written for this crate.
--self also fails when a file under src/ or prereg/ (or Cargo.toml / Cargo.lock) is not listed in manifest.json.
Exit code: 0 = all hold / 1 = a mismatch / 2 = usage error or unreadable input.
"""
import difflib
import hashlib
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
MANIFEST = os.path.join(HERE, "manifest.json")
UP = "src-tauri/"

ENTRIES = [  # (kind, path in this crate, upstream path, extra)
    *[("file", "src/analysis/%s.rs" % m, UP + "src/analysis/%s.rs" % m, None)
      for m in ("engine", "features", "intervention", "key_gate", "loop_step")],
    *[("file", "src/analysis/heads/%s.rs" % m, UP + "src/analysis/heads/%s.rs" % m, None) for m in ("writing", "coding")],
    *[("file", "src/analysis/feat_offline/%s.rs" % m, UP + "src/analysis/feat_offline/%s.rs" % m, None)
      for m in ("mod", "activity", "boundary", "dispersion", "idle_trim", "quality", "restart", "revision",
                "stall_onset")],
    ("file", "prereg/p1-episode.json", UP + "prereg/p1-episode.json", None),
    ("lock", "Cargo.lock", UP + "Cargo.lock", "pruned by cargo from the upstream src-tauri/Cargo.lock"),
    *[("trimmed", "src/analysis/%s.rs" % m, UP + "src/analysis/%s.rs" % m, None)
      for m in ("anonymize", "metrics", "heads/mod")],
    ("excerpt", "src/logger.rs", UP + "src/logger.rs", {"id": "task_kind", "upstream_start": 68, "upstream_end": 128}),
    ("cut", "src/bin/replay.rs", UP + "src/bin/replay.rs", None),
    ("new", "src/lib.rs", UP + "src/lib.rs", "declares only `analysis` and `logger`"),
    ("new", "src/analysis/mod.rs", UP + "src/analysis/mod.rs", "declares only the modules this crate keeps"),
    ("new", "Cargo.toml", UP + "Cargo.toml", "this crate's own manifest (version kept at 0.1.40, see the comment)"),
]


def sha(b):
    return hashlib.sha256(b).hexdigest()


def read(path):
    with open(os.path.join(ROOT, path), "rb") as f:
        return f.read()


def upstream(repo, commit, path):
    return subprocess.run(["git", "-C", repo, "show", "%s:%s" % (commit, path)], check=True,
                          stdout=subprocess.PIPE).stdout


def trim(b):
    lines = b.decode("utf-8").split("\n")
    i = lines.index("#[cfg(test)]")
    return ("\n".join(lines[:i]).rstrip("\n") + "\n").encode("utf-8")


def excerpt(b, eid):
    lines = b.decode("utf-8").split("\n")
    s, e = lines.index("// @excerpt-begin " + eid), lines.index("// @excerpt-end " + eid)
    return lines[s + 1:e]


def lock_packages(b):
    """(name, version, source, checksum) of every [[package]] in a Cargo.lock."""
    out = set()
    for block in b.decode("utf-8").split("[[package]]")[1:]:
        f = {}
        for line in block.split("\n"):
            if " = " in line and not line.startswith(" "):
                k, v = line.split(" = ", 1)
                if k in ("name", "version", "source", "checksum"):
                    f[k] = v
        out.add(tuple(f.get(k) for k in ("name", "version", "source", "checksum")))
    return out


def listed_candidates():
    """Files that must be listed in manifest.json: src/**, prereg/**, Cargo.toml, Cargo.lock."""
    out = {"Cargo.toml", "Cargo.lock"}
    for top in ("src", "prereg"):
        for d, _, names in os.walk(os.path.join(ROOT, top)):
            out.update(os.path.relpath(os.path.join(d, n), ROOT) for n in names)
    return out


def check_upstream(repo, m):
    if not m["entries"]:
        raise ValueError("manifest.json has no entries")
    bad = 0
    for e in m["entries"]:
        if e["kind"] in ("new",) or e["upstream_path"] is None:
            continue
        up = upstream(repo, m["upstream_commit"], e["upstream_path"])
        local = read(e["path"])
        if e["kind"] == "file":
            ok = local == up
        elif e["kind"] == "trimmed":
            ok = local == trim(up)
        elif e["kind"] == "excerpt":
            want = up.decode("utf-8").split("\n")[e["upstream_start"] - 1:e["upstream_end"]]
            ok = excerpt(local, e["excerpt_id"]) == want
        elif e["kind"] == "cut":
            ok = make_diff(up, local, e) == read(e["diff"])
        elif e["kind"] == "lock":
            mine = {p for p in lock_packages(local) if p[0] != "\"gse-estimator\""}
            ok = bool(mine) and mine <= lock_packages(up)
        else:
            ok = False
        print(("ok   " if ok else "FAIL ") + e["kind"] + " " + e["path"])
        bad += not ok
    return bad


def make_diff(up, local, e):
    d = difflib.unified_diff(up.decode("utf-8").splitlines(True), local.decode("utf-8").splitlines(True),
                             "upstream/" + e["upstream_path"], "estimator/" + e["path"])
    return "".join(d).encode("utf-8")


def write(repo):
    full = subprocess.run(["git", "-C", repo, "rev-parse", "24767c7e"], check=True,
                          stdout=subprocess.PIPE).stdout.decode().strip()
    out = []
    for kind, path, up_path, extra in ENTRIES:
        e = {"kind": kind, "path": path, "upstream_path": up_path, "sha256": sha(read(path))}
        if kind == "file" and up_path is None:
            e["kind"] = "new"
            e["reason"] = extra
        elif kind in ("new", "lock"):
            e["reason"] = extra
        elif kind == "excerpt":
            e.update(excerpt_id=extra["id"], upstream_start=extra["upstream_start"], upstream_end=extra["upstream_end"])
        elif kind == "cut":
            e["diff"] = "provenance/%s.diff" % os.path.basename(path)
            with open(os.path.join(ROOT, e["diff"]), "wb") as f:
                f.write(make_diff(upstream(repo, full, up_path), read(path), e))
        if up_path is not None:
            e["upstream_sha256"] = sha(upstream(repo, full, up_path))
        out.append(e)
    m = {"upstream_repository": "GSE (Generative Struggle Engine)", "upstream_commit": full, "entries": out}
    with open(MANIFEST, "w", encoding="utf-8") as f:
        json.dump(m, f, indent=1, ensure_ascii=False)
        f.write("\n")
    return m


def main():
    a = sys.argv[1:]
    try:
        if a == ["--self"]:
            m = json.load(open(MANIFEST, encoding="utf-8"))
            bad = 0
            for e in m["entries"]:
                ok = sha(read(e["path"])) == e["sha256"]
                print(("ok   " if ok else "FAIL ") + e["path"])
                bad += not ok
            for path in sorted(listed_candidates() - {e["path"] for e in m["entries"]}):
                print("FAIL unlisted " + path)
                bad += 1
            if not m["entries"]:
                return 2
            return 1 if bad else 0
        if len(a) == 2 and a[0] == "--upstream":
            m = json.load(open(MANIFEST, encoding="utf-8"))
            return 1 if check_upstream(a[1], m) else 0
        if len(a) == 2 and a[0] == "--write":
            m = write(a[1])
            return 1 if check_upstream(a[1], m) else 0
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as ex:
        print("error: %s" % ex, file=sys.stderr)
        return 2
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
