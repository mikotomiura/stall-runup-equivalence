#!/usr/bin/env python3
"""Check that the release holds no raw data, no per-writer identifiers and no secrets.

Usage:
  python3 -I tools/check_release.py [--root DIR] [--private DATA_DIR]

Without --private, the check is structural and can be run by anyone:
  - no raw data files (by extension) and no file over 2 MB;
  - no identifier of the shapes the public datasets use for writers (CS1 2019 "S" and three digits, CS1 2021
    "Student" and digits, CoAuthor worker ids), in any text file;
  - no JSON array of 50 or more strings and no JSON object with 20 or more keys of a per-unit shape
    (the shapes of the intermediate files: "u" or "w" and digits);
  - no secret of common shapes (private keys, age secret keys, GitHub and cloud tokens).
With --private (for the authors, who hold the data), it also searches every file for each writer identifier of the
holdout and skip lists and for the SHA-256 (full and as truncated in the intermediates) of each identifier.

Exit code: 0 = nothing found / 1 = something found / 2 = usage error.
"""
import hashlib
import json
import os
import re
import sys

RAW_EXT = {".csv", ".tsv", ".zip", ".gz", ".tar", ".ndjson", ".jsonl", ".parquet", ".pkl", ".npy", ".npz", ".xlsx",
           ".db", ".sqlite", ".age"}
ALLOWED_TSV = {"gse-commits.tsv", "gse-source.tsv"}  # provenance tables written by the assembly
MAX_BYTES = 2 * 1024 * 1024
SKIP_DIRS = {".git", "target", "__pycache__", "node_modules"}
# Identifier shapes, assembled at run time so that this file does not match itself.
_D = r"\d"
ID_PATTERNS = {
    "cs1-2019-subject": re.compile(r"(?<![A-Za-z0-9_])S" + _D + "{3}(?![A-Za-z0-9_])"),
    "cs1-2021-subject": re.compile(r"(?<![A-Za-z0-9_])Stu" + "dent" + _D + r"+(?![A-Za-z0-9_])"),
    "coauthor-worker": re.compile(r"(?<![A-Za-z0-9_])A[0-9A-Z]{11,20}(?![A-Za-z0-9_])"),
}
UNIT_KEY = re.compile(r"^[uw]" + _D + r"+$")
SECRET_PATTERNS = {
    "private-key": re.compile("-----BEGIN [A-Z ]*PRIV" + "ATE KEY-----"),
    "age-secret": re.compile("AGE-SEC" + "RET-KEY-1[0-9A-Z]{20,}"),
    "github-token": re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,}\b"),
    "aws-key": re.compile(r"\bAK" + r"IA[0-9A-Z]{16}\b"),
    "openai-like": re.compile(r"\bsk-[A-Za-z0-9_-]{30,}\b"),
}


def files(root):
    for d, ds, fs in os.walk(root):
        ds[:] = sorted(x for x in ds if x not in SKIP_DIRS)
        for f in sorted(fs):
            yield os.path.join(d, f)


def read_text(p):
    with open(p, "rb") as fh:
        b = fh.read()
    try:
        return b.decode("utf-8")
    except UnicodeDecodeError:
        return None


def json_findings(obj, path=""):
    out = []
    if isinstance(obj, dict):
        unit = [k for k in obj if UNIT_KEY.match(str(k))]
        if len(unit) >= 20:
            out.append("object with %d per-unit keys at %s" % (len(unit), path or "/"))
        for k, v in obj.items():
            out += json_findings(v, path + "/" + str(k))
    elif isinstance(obj, list):
        strs = [x for x in obj if isinstance(x, str)]
        if len(strs) >= 50:
            out.append("array of %d strings at %s" % (len(strs), path or "/"))
        for i, v in enumerate(obj):
            out += json_findings(v, "%s[%d]" % (path, i))
    return out


def private_needles(data_dir):
    """Writer identifiers of the holdout and skip lists, and their SHA-256 (full and truncated to 30)."""
    ids = set()
    for rel in ("cs1/holdout.json", "cs1/skip_seen_2019.json", "cs1/skip_holdout_2021.json",
                "cs1/rehearsal_holdout.json", "landscape/holdout.json", "extend/holdout.json"):
        p = os.path.join(data_dir, rel)
        if not os.path.exists(p):
            continue
        with open(p) as fh:
            j = json.load(fh)
        stack = [j]
        while stack:
            x = stack.pop()
            if isinstance(x, dict):
                stack += list(x.values())
            elif isinstance(x, list):
                stack += x
            elif isinstance(x, (str, int)):
                ids.add(str(x))
    if not ids:
        sys.exit("no holdout or skip list found under %s" % data_dir)
    needles = {}
    for i in ids:
        h = hashlib.sha256(i.encode()).hexdigest()
        needles[h] = "sha256 of a listed id"
        needles[h[:30]] = "truncated sha256 of a listed id"
        if len(i) >= 4 and not i.isdigit():  # short or numeric ids (KLiCKe) are matched by the structure checks
            needles[i] = "listed id"
    return needles


def main(argv):
    root, data_dir = ".", None
    while argv:
        a = argv.pop(0)
        if a == "--root" and argv:
            root = argv.pop(0)
        elif a == "--private" and argv:
            data_dir = argv.pop(0)
        else:
            print(__doc__, file=sys.stderr)
            return 2
    root = os.path.abspath(root)
    needles = private_needles(data_dir) if data_dir else {}
    tok = re.compile(r"[A-Za-z0-9_]+")
    found, n = [], 0
    for p in files(root):
        rel = os.path.relpath(p, root)
        n += 1
        ext = os.path.splitext(p)[1].lower()
        if ext in RAW_EXT and os.path.basename(p) not in ALLOWED_TSV:
            found.append((rel, "raw data extension " + ext))
        if os.path.getsize(p) > MAX_BYTES:
            found.append((rel, "file over 2 MB"))
        text = read_text(p)
        if text is None:
            continue
        for name, pat in list(ID_PATTERNS.items()) + list(SECRET_PATTERNS.items()):
            hits = pat.findall(text)
            if hits:
                found.append((rel, "%s x%d" % (name, len(hits))))
        if ext == ".json":
            try:
                for f in json_findings(json.loads(text)):
                    found.append((rel, f))
            except ValueError:
                found.append((rel, "unparsable JSON"))
        if needles:
            words = set(tok.findall(text))
            for w in words:
                for cut in (w, w[:30]):
                    if cut in needles:
                        found.append((rel, needles[cut]))
                        break
    for rel, what in found:
        print("FOUND %s: %s" % (rel, what))
    print("checked %d files%s; %d findings" % (n, " (with the private lists)" if needles else "", len(found)))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
