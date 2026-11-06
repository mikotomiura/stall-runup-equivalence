#!/usr/bin/env bash
# Positive controls for tools/check_release.py: each planted file must turn the check red (exit 1), and the clean
# copy must stay green (exit 0). The planted strings are built at run time so that no file of the release holds them.
# Usage: control_check_release.sh [DATA_DIR]   (with DATA_DIR, also plants one id taken from the holdout list)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; ROOT="$(cd "$HERE/.." && pwd)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
rsync -a --exclude .git --exclude target --exclude __pycache__ "$ROOT/" "$TMP/clean/"
run() { python3 -I "$TMP/clean/tools/check_release.py" --root "$1" ${2:+--private "$2"} >/dev/null; }
fail=0
expect() { # expect <0|1> <label> <dir> [data_dir]
  set +e; run "$3" "${4:-}"; rc=$?; set -e
  if [ "$rc" = "$1" ]; then echo "ok   $2 (exit $rc)"; else echo "FAIL $2 (exit $rc, expected $1)"; fail=1; fi
}
expect 0 "clean copy" "$TMP/clean"
plant() { rm -rf "$TMP/p"; cp -R "$TMP/clean" "$TMP/p"; }
plant; python3 -I -c 'import sys; open(sys.argv[1],"w").write("writer " + "S" + "123" + " stalled\n")' "$TMP/p/x.md"
expect 1 "CS1 2019 id in text" "$TMP/p"
plant; python3 -I -c 'import sys; open(sys.argv[1],"w").write("Stu" + "dent" + "42\n")' "$TMP/p/x.md"
expect 1 "CS1 2021 id in text" "$TMP/p"
plant; python3 -I -c 'import json,sys; json.dump({"u%05d" % i: 0.5 for i in range(25)}, open(sys.argv[1],"w"))' "$TMP/p/x.json"
expect 1 "per-unit object" "$TMP/p"
plant; python3 -I -c 'import json,sys; json.dump(["id%d" % i for i in range(60)], open(sys.argv[1],"w"))' "$TMP/p/x.json"
expect 1 "array of ids" "$TMP/p"
plant; printf 'a,b\n1,2\n' > "$TMP/p/x.csv"
expect 1 "raw data file" "$TMP/p"
plant; python3 -I -c 'import sys; open(sys.argv[1],"w").write("-----BEGIN " + "PRIVATE KEY-----\n")' "$TMP/p/x.txt"
expect 1 "private key" "$TMP/p"
if [ -n "${1:-}" ]; then
  expect 0 "clean copy, private lists" "$TMP/clean" "$1"
  # one CoAuthor holdout id (long enough to be searched as text), and the SHA-256 of one CS1 holdout id
  plant; python3 -I -c '
import hashlib, json, sys
h = json.load(open(sys.argv[1] + "/landscape/holdout.json"))["coauthor"][0]
c = json.load(open(sys.argv[1] + "/cs1/holdout.json"))["2019"][0]
open(sys.argv[2] + "/x.md", "w").write("see " + h + "\n")
open(sys.argv[2] + "/y.md", "w").write(hashlib.sha256(c.encode()).hexdigest()[:30] + "\n")' "$1" "$TMP/p"
  expect 1 "holdout id and truncated sha256" "$TMP/p" "$1"
fi
exit "$fail"
