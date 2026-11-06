#!/usr/bin/env python3
"""Check that the release holds no raw data, no per-writer identifiers or tables, no secrets and no local paths.

Usage:
  python3 -I tools/check_release.py [--root DIR] [--private DATA_DIR]

The checks are structural: they look for shapes, not for meaning, so a per-writer value written as prose would pass.
Without --private, anyone can run them:
  - files: no raw data by extension, no file over 2 MB, and every file must be UTF-8 text (a file that cannot be read
    as text is reported, since its content cannot be checked);
  - identifiers of the shapes the public datasets use for writers (CS1 2019 "S" and three digits, CS1 2021 "Student"
    and digits, CoAuthor worker ids), in any file;
  - tables large enough to be per writer: a JSON array of 30 or more numbers, strings or objects, a JSON object with
    30 or more keys, and a Markdown or TeX table of 30 or more rows;
  - secrets of common shapes (private keys, age secret keys, GitHub, Slack, Google, AWS and API tokens) and local
    paths (/Users/, /home/, C:\\, host names ending in .local);
  - in a git repository: the files are the committed ones (no uncommitted change), and the author, committer and
    message of every commit pass the identifier, secret and local-path checks.
With --private (for the authors, who hold the data), it also searches for each writer identifier of the holdout and
skip lists and for the SHA-256 of each identifier (full, or any prefix of 8 or more hex digits).

Exit code: 0 = nothing found / 1 = something found / 2 = usage error, missing root or nothing to check.
"""
import hashlib
import json
import os
import re
import subprocess
import sys

RAW_EXT = {".csv", ".tsv", ".zip", ".gz", ".bz2", ".xz", ".7z", ".zst", ".tar", ".ndjson", ".jsonl", ".parquet",
           ".feather", ".arrow", ".h5", ".hdf5", ".pkl", ".npy", ".npz", ".xls", ".xlsx", ".db", ".sqlite", ".age",
           ".rds", ".sav", ".dta"}
ALLOWED_RAW = {"gse-commits.tsv", "gse-source.tsv"}  # provenance tables at the root, written by the assembly
MAX_BYTES = 2 * 1024 * 1024
BIG = 30
PRIVATE_LISTS = ("cs1/holdout.json", "cs1/skip_seen_2019.json", "cs1/skip_holdout_2021.json",
                 "cs1/rehearsal_holdout.json", "landscape/holdout.json", "extend/holdout.json")
# Shapes are assembled at run time so that this file does not match itself.
_D = r"\d"
ID_PATTERNS = {
    "cs1-2019-subject": re.compile(r"(?<![A-Za-z0-9_])S" + _D + "{3}(?![A-Za-z0-9_])"),
    "cs1-2021-subject": re.compile(r"(?<![A-Za-z0-9_])Stu" + "dent" + _D + r"+(?![A-Za-z0-9_])"),
    "coauthor-worker": re.compile(r"(?<![A-Za-z0-9_])A[0-9A-Z]{11,20}(?![A-Za-z0-9_])"),
}
SECRET_PATTERNS = {
    "private-key": re.compile("-----BEGIN [A-Z ]*PRIV" + "ATE KEY-----"),
    "age-key": re.compile("AGE-SEC" + "RET-KEY-1[0-9A-Z]{20,}"),
    "github-token": re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{30,}|github_" + r"pat_[A-Za-z0-9_]{30,})\b"),
    "slack-token": re.compile(r"\bxo" + r"x[abprs]-[A-Za-z0-9-]{10,}"),
    "google-key": re.compile(r"\bAI" + r"za[0-9A-Za-z_-]{35}\b"),
    "aws-key": re.compile(r"\bAK" + r"IA[0-9A-Z]{16}\b"),
    "api-token": re.compile(r"\bsk-[A-Za-z0-9_-]{30,}\b"),
}
LOCAL_PATTERNS = {
    "local-path": re.compile(r"/Us" + r"ers/[A-Za-z0-9._-]+|/ho" + r"me/[a-z][a-z0-9._-]+/|[A-Z]:\\" + r"Users"),
    "local-host": re.compile(r"@[A-Za-z0-9-]+\.lo" + r"cal\b"),
}
TABLE_ROW = re.compile(r"^\s*\|.*\|\s*$|\\\\\s*$")


def listing(root):
    """Files to check: in a git repository the tracked and the untracked-not-ignored files, otherwise all files."""
    inside = subprocess.run(["git", "-C", root, "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if inside.returncode == 0 and os.path.realpath(inside.stdout.strip()) == os.path.realpath(root):
        out = subprocess.run(["git", "-C", root, "ls-files", "-co", "--exclude-standard", "-z"],
                             capture_output=True, check=True).stdout
        return [p.decode() for p in out.split(b"\0") if p], True
    paths = []
    for d, ds, fs in os.walk(root):
        ds[:] = sorted(x for x in ds if not (d == root and x == ".git"))
        paths += [os.path.relpath(os.path.join(d, f), root) for f in sorted(fs)]
    return paths, False


def json_findings(obj, path=""):
    out = []
    if isinstance(obj, dict):
        if len(obj) >= BIG:
            out.append("object with %d keys at %s" % (len(obj), path or "/"))
        for k, v in obj.items():
            out += json_findings(v, path + "/" + str(k))
    elif isinstance(obj, list):
        for kind, n in (("numbers", sum(isinstance(x, (int, float)) and not isinstance(x, bool) for x in obj)),
                        ("strings", sum(isinstance(x, str) for x in obj)),
                        ("objects", sum(isinstance(x, (dict, list)) for x in obj))):
            if n >= BIG:
                out.append("array of %d %s at %s" % (n, kind, path or "/"))
        for i, v in enumerate(obj):
            out += json_findings(v, "%s[%d]" % (path, i))
    return out


def longest_table(text):
    best = run = 0
    for line in text.splitlines():
        run = run + 1 if TABLE_ROW.search(line) else 0
        best = max(best, run)
    return best


def private_needles(data_dir):
    """Writer identifiers of the holdout and skip lists, and the SHA-256 of each."""
    ids, missing = set(), []
    for rel in PRIVATE_LISTS:
        p = os.path.join(data_dir, rel)
        if not os.path.exists(p):
            missing.append(rel)
            continue
        with open(p) as fh:
            stack = [json.load(fh)]
        while stack:
            x = stack.pop()
            if isinstance(x, dict):
                stack += list(x.values())
            elif isinstance(x, list):
                stack += x
            elif isinstance(x, (str, int)):
                ids.add(str(x))
    if "cs1/holdout.json" in missing:
        sys.exit("cs1/holdout.json not found under %s" % data_dir)
    for rel in missing:
        print("note: private list not found, not searched: %s" % rel)
    words = {i for i in ids if len(i) >= 4 and not i.isdigit()}  # numeric ids are covered by the table checks
    hashes = {hashlib.sha256(i.encode()).hexdigest() for i in ids}
    return words, hashes


def text_findings(text, label):
    found = []
    for name, pat in list(ID_PATTERNS.items()) + list(SECRET_PATTERNS.items()) + list(LOCAL_PATTERNS.items()):
        n = len(pat.findall(text))
        if n:
            found.append((label, "%s x%d" % (name, n)))
    return found


def needle_findings(text, label, words, hashes):
    found = []
    if words and any(t in words for t in set(re.findall(r"[A-Za-z0-9_:.-]+", text))):
        found.append((label, "listed id"))
    if hashes:
        prefixes = {h[:8] for h in hashes}
        for t in set(re.findall(r"[0-9a-f]{8,64}", text)):
            if t[:8] in prefixes and any(h.startswith(t) for h in hashes):
                found.append((label, "sha256 (or prefix) of a listed id"))
                break
    return found


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
    if not os.path.isdir(root):
        print("root is not a directory: %s" % root, file=sys.stderr)
        return 2
    words, hashes = private_needles(data_dir) if data_dir else (set(), set())
    paths, in_git = listing(root)
    if not paths:
        print("nothing to check under %s" % root, file=sys.stderr)
        return 2
    found = []
    for rel in paths:
        p = os.path.join(root, rel)
        if os.path.islink(p):
            found.append((rel, "symbolic link"))
            continue
        ext = os.path.splitext(rel)[1].lower()
        if ext in RAW_EXT and rel not in ALLOWED_RAW:
            found.append((rel, "raw data extension " + ext))
        if os.path.getsize(p) > MAX_BYTES:
            found.append((rel, "file over 2 MB"))
        with open(p, "rb") as fh:
            b = fh.read()
        try:
            text = b.decode("utf-8")
        except UnicodeDecodeError:
            found.append((rel, "not UTF-8 text (content not checkable)"))
            continue
        found += text_findings(text, rel)
        found += needle_findings(text, rel, words, hashes)
        if ext == ".json":
            try:
                found += [(rel, f) for f in json_findings(json.loads(text))]
            except ValueError:
                found.append((rel, "unparsable JSON"))
        if ext in (".md", ".tex") and longest_table(text) >= BIG:
            found.append((rel, "table of %d rows" % longest_table(text)))
    if in_git:
        st = subprocess.run(["git", "-C", root, "status", "--porcelain"], capture_output=True, text=True, check=True)
        if st.stdout.strip():
            found.append(("(git)", "uncommitted changes: the checked files are not the committed ones"))
        log = subprocess.run(["git", "-C", root, "log", "--all", "--format=%an <%ae>%n%cn <%ce>%n%B%n----"],
                             capture_output=True, text=True, check=True).stdout
        found += [(l.replace("(log)", "(git log)"), w) for l, w in text_findings(log, "(log)")]
        found += needle_findings(log, "(git log)", words, hashes)
    for rel, what in found:
        print("FOUND %s: %s" % (rel, what))
    print("checked %d files%s%s; %d findings" % (len(paths), " and the git history" if in_git else "",
                                                 " (with the private lists)" if data_dir else "", len(found)))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
