#!/usr/bin/env bash
# Run every check of the release: the manifest, the estimator's tests and provenance, and the release check.
# Usage: tools/verify.sh [DATA_DIR]   (DATA_DIR, held only by the authors, adds the checks against the private lists)
# Needs: python3, cargo (Rust 1.85 or later). Exit code 0 only if every check passes.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
fail=0
step() { echo "== $1"; shift; if "$@"; then :; else echo "FAILED: $*"; fail=1; fi; }
step "manifest of the verified files" python3 -I tools/check_manifest.py
step "estimator tests" bash -c 'cd estimator && cargo test --locked --quiet'
step "estimator provenance (self)" bash -c 'cd estimator && python3 -I provenance/check_provenance.py --self'
step "release check" python3 -I tools/check_release.py ${1:+--private "$1"}
step "release check, positive controls" bash tools/control_check_release.sh ${1:+"$1"}
[ "$fail" -eq 0 ] && echo "all checks passed" || echo "some checks failed"
exit "$fail"
