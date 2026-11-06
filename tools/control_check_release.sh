#!/usr/bin/env bash
# Positive controls for tools/check_release.py. The clean copy must pass (exit 0). Each planted file must make the
# check exit 1 AND print a FOUND line naming the expected detector (an exit 1 from a crash does not count). A missing
# root and an empty root must exit 2. The planted strings are built at run time, so no file of the release holds them.
# Usage: control_check_release.sh [DATA_DIR]   (with DATA_DIR, also plants an id and a hash prefix from the lists)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"; ROOT="$(cd "$HERE/.." && pwd -P)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
# a plain copy of the committed files (no .git), so that the checks see exactly what was committed
git -C "$ROOT" archive --format=tar HEAD | (mkdir -p "$TMP/clean" && tar -x -C "$TMP/clean")
CHK="$TMP/clean/tools/check_release.py"
fail=0
# expect <exit> <label> <dir> <pattern in output or -> [data_dir]
expect() {
  local want="$1" label="$2" dir="$3" pat="$4" data="${5:-}" out rc
  set +e; out="$(python3 -I "$CHK" --root "$dir" ${data:+--private "$data"} 2>&1)"; rc=$?; set -e
  if [ "$rc" != "$want" ]; then echo "FAIL $label (exit $rc, expected $want)"; fail=1; return; fi
  if [ "$pat" != "-" ] && ! printf '%s\n' "$out" | grep -q -- "$pat"; then
    echo "FAIL $label (exit $rc but no line matching: $pat)"; fail=1; return
  fi
  echo "ok   $label (exit $rc)"
}
plant() { rm -rf "$TMP/p"; cp -R "$TMP/clean" "$TMP/p"; }
py() { python3 -I -c "$1" "$TMP/p"; }
expect 0 "clean copy" "$TMP/clean" "0 findings"
expect 2 "missing root" "$TMP/nonexistent" -
mkdir "$TMP/empty"; expect 2 "empty root" "$TMP/empty" -
plant; py 'import sys; open(sys.argv[1]+"/x.md","w").write("writer " + "S" + "123" + " stalled\n")'
expect 1 "CS1 2019 id" "$TMP/p" "FOUND x.md: cs1-2019-subject"
plant; py 'import sys; open(sys.argv[1]+"/x.md","w").write("Stu" + "dent" + "42\n")'
expect 1 "CS1 2021 id" "$TMP/p" "FOUND x.md: cs1-2021-subject"
plant; py 'import sys; open(sys.argv[1]+"/x.md","w").write("worker " + "A" + "1B2C3D4E5F6G7H" + "\n")'
expect 1 "CoAuthor worker id" "$TMP/p" "FOUND x.md: coauthor-worker"
plant; py 'import json,sys; json.dump({"w%d" % i: 0.5 for i in range(35)}, open(sys.argv[1]+"/x.json","w"))'
expect 1 "object with many keys" "$TMP/p" "FOUND x.json: object with 35 keys"
plant; py 'import json,sys; json.dump([0.5]*40, open(sys.argv[1]+"/x.json","w"))'
expect 1 "array of numbers" "$TMP/p" "FOUND x.json: array of 40 numbers"
plant; py 'import json,sys; json.dump(["id%d" % i for i in range(40)], open(sys.argv[1]+"/x.json","w"))'
expect 1 "array of strings" "$TMP/p" "FOUND x.json: array of 40 strings"
plant; py 'import json,sys; json.dump([{"a": i} for i in range(40)], open(sys.argv[1]+"/x.json","w"))'
expect 1 "array of objects" "$TMP/p" "FOUND x.json: array of 40 objects"
plant; py 'import sys; open(sys.argv[1]+"/x.md","w").write("".join("| %d | 0.5 |\n" % i for i in range(35)))'
expect 1 "Markdown table" "$TMP/p" "FOUND x.md: table of 35 rows"
plant; printf 'a,b\n1,2\n' > "$TMP/p/x.csv"
expect 1 "raw data file" "$TMP/p" "FOUND x.csv: raw data extension"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("a" * (3 * 1024 * 1024))'
expect 1 "large file" "$TMP/p" "FOUND x.txt: file over 2 MB"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","wb").write("text".encode("utf-16"))'
expect 1 "not UTF-8" "$TMP/p" "FOUND x.txt: not UTF-8"
plant; ln -s /etc/hosts "$TMP/p/x.txt"
expect 1 "symbolic link" "$TMP/p" "FOUND x.txt: symbolic link"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("-----BEGIN " + "PRIVATE KEY-----\n")'
expect 1 "private key" "$TMP/p" "FOUND x.txt: private-key"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("AGE-SEC" + "RET-KEY-1" + "Q" * 50 + "\n")'
expect 1 "age key" "$TMP/p" "FOUND x.txt: age-key"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("gh" + "p_" + "a" * 36 + "\n")'
expect 1 "GitHub token" "$TMP/p" "FOUND x.txt: github-token"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("AK" + "IA" + "A" * 16 + "\n")'
expect 1 "AWS key" "$TMP/p" "FOUND x.txt: aws-key"
plant; py 'import sys; open(sys.argv[1]+"/x.txt","w").write("sk" + "-" + "a" * 40 + "\n")'
expect 1 "API token" "$TMP/p" "FOUND x.txt: api-token"
plant; py 'import sys; open(sys.argv[1]+"/x.md","w").write("/Us" + "ers/someone/data\n")'
expect 1 "local path" "$TMP/p" "FOUND x.md: local-path"
# git: an uncommitted change and a committer with a local host name
plant; (cd "$TMP/p" && git init -q && git add -A && git -c user.name=t -c user.email=t@example.org commit -q -m init) >/dev/null
expect 0 "clean git copy" "$TMP/p" "0 findings"
echo x >> "$TMP/p/README.md"
expect 1 "uncommitted change" "$TMP/p" "uncommitted changes"
(cd "$TMP/p" && git checkout -q -- README.md && GIT_COMMITTER_EMAIL="t@host.lo""cal" git commit -q --allow-empty -m x)
expect 1 "committer with a local host" "$TMP/p" "FOUND (git log): local-host"
if [ -n "${1:-}" ]; then
  expect 0 "clean copy, private lists" "$TMP/clean" "0 findings" "$1"
  plant; python3 -I -c '
import json, sys
h = json.load(open(sys.argv[1] + "/landscape/holdout.json"))["coauthor"][0]
open(sys.argv[2] + "/x.md", "w").write("see " + h + "\n")' "$1" "$TMP/p"
  expect 1 "holdout id" "$TMP/p" "FOUND x.md: listed id" "$1"
  plant; python3 -I -c '
import hashlib, json, sys
c = json.load(open(sys.argv[1] + "/cs1/holdout.json"))["2019"][0]
open(sys.argv[2] + "/x.md", "w").write("hash " + hashlib.sha256(c.encode()).hexdigest()[:10] + "\n")' "$1" "$TMP/p"
  expect 1 "hash prefix of a holdout id" "$TMP/p" "FOUND x.md: sha256" "$1"
fi
exit "$fail"
