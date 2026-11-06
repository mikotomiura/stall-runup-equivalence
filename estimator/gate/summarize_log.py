"""Counts the lines of a compare-replay.sh log: calls, and mismatches of stdout / exit code / stderr, per mode.

Exit code: 0 = at least one call and no mismatch / 1 = a mismatch or an empty log."""
import collections
import json
import sys

calls = collections.Counter()
bad = collections.Counter()
for line in open(sys.argv[1], encoding="utf-8"):
    s_out, s_exit, s_err, rest = line.split(" ", 3)
    mode = rest[rest.index("[") + 1:rest.rindex("]")]
    exit_code = rest[rest.rindex("]") + 2:].split(" ")[0]
    calls[(mode, exit_code)] += 1
    for name, s in (("stdout", s_out), ("exit", s_exit), ("stderr", s_err)):
        if s != "match":
            bad[(mode, name)] += 1
print(json.dumps({"calls": sum(calls.values()),
                  "calls_by_mode_and_exit": {"%s exit=%s" % k: v for k, v in sorted(calls.items())},
                  "mismatches": {"%s %s" % k: v for k, v in sorted(bad.items())}}, ensure_ascii=False, indent=1))
sys.exit(1 if bad or not calls else 0)
