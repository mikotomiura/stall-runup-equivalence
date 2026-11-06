#!/usr/bin/env python3
"""Check the files of this release against the manifest of verified files registered on OSF.

The preregistration (https://osf.io/mrkxt) holds the SHA-256 of the first manifest of the verified files (19 entries,
9a1a7f14...28da9b; attached to that registration). The deviation (https://osf.io/fqgcp) added two entries, and the
second start record (https://osf.io/ajg2x) and the results were computed under the new manifest
experiments/20261105-findings-paper1-prefreeze/confirm-manifest.sha256 (21 entries, c821ae03...f6b0; its SHA-256 is
recorded in results_holdout/start.json). The manifest lists files under the paths of the authors' development
repository. This release keeps those paths for
the experiment scripts. The definition of a stall (src-tauri/prereg/p1-episode.json) is checked against the
byte-identical copy that the estimator reads (estimator/prereg/p1-episode.json). Six entries are documents of the
development repository that are not part of this release; they are reported as "not released". Three of them are the
preregistration text in Japanese and English and the check of the English version, which are attached to the OSF
registration under other names.

Usage: python3 -I tools/check_manifest.py [--root DIR]
Exit code: 0 = every released entry matches and the manifest has the registered SHA-256 / 1 = a mismatch or a
missing released entry / 2 = usage error.
"""
import hashlib
import os
import sys

MANIFEST = "experiments/20261105-findings-paper1-prefreeze/confirm-manifest.sha256"
MANIFEST_SHA256 = "c821ae034a05111ff37d572e1a15d7292d6cc4757f90529d594868559088f6b0"  # second start record
MOVED = {"src-tauri/prereg/p1-episode.json": "estimator/prereg/p1-episode.json"}
NOT_RELEASED = {
    "docs/preregistration-p1-episode.md": "document of the development repository",
    "docs/adr/ADR-030-stall-onset-and-decision-episode.md": "document of the development repository",
    ".steering/20261105-public-data-findings-paper/paper-write/prereg-draft.md":
        "preregistration (Japanese); on OSF as preregistration-paper1-v10-ja.md",
    ".steering/20261105-public-data-findings-paper/paper-write/prereg-draft.en.md":
        "preregistration (English); on OSF as preregistration-paper1-v10-en.md",
    ".steering/20261105-public-data-findings-paper/paper-write/prereg-en-check.md":
        "check of the English version; on OSF as english-version-check.md",
    ".steering/20261105-public-data-findings-paper/paper-write/check_prereg_en.py":
        "script of that check (development repository)",
}


def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for b in iter(lambda: fh.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def main(argv):
    root = "."
    if argv[:1] == ["--root"] and len(argv) == 2:
        root = argv[1]
    elif argv:
        print(__doc__, file=sys.stderr)
        return 2
    m = os.path.join(root, MANIFEST)
    bad = 0
    got = sha(m)
    print("%s %s" % ("ok  " if got == MANIFEST_SHA256 else "FAIL", "manifest SHA-256 is the value of the second start record"))
    bad += got != MANIFEST_SHA256
    seen = set()
    with open(m) as fh:
        for line in fh:
            want, path = line.rstrip("\n").split("  ", 1)
            seen.add(path)
            if path in NOT_RELEASED:
                print("--   %s: not released (%s)" % (path, NOT_RELEASED[path]))
                continue
            local = MOVED.get(path, path)
            p = os.path.join(root, local)
            if not os.path.exists(p):
                print("FAIL %s: missing" % local)
                bad += 1
            elif sha(p) != want:
                print("FAIL %s: SHA-256 differs" % local)
                bad += 1
            else:
                print("ok   %s%s" % (local, "" if local == path else " (registered as %s)" % path))
    unknown = (set(NOT_RELEASED) | set(MOVED)) - seen
    if unknown:
        print("FAIL entries named here but not in the manifest: %s" % sorted(unknown))
        bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
