#!/usr/bin/env python3
"""Run the confirmation (confirm_p1.py, unchanged) on the holdout outside the authors' one-time procedure.

confirm_p1.py refuses a holdout run that does not come from the second stage of run-confirm-holdout.sh (its
holdout_gate checks the git history and the OSF start record, so that the authors could run it only once). This
wrapper imports the frozen script, replaces only that gate with a no-op, and calls the same run() with the same
arguments. The computation is unchanged; the result is a rerun, not a confirmation.

Usage: python3 -I tools/rerun_confirm.py <holdout intermediates> <holdout.json> <seed> <out-dir>
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "experiments", "20261105-findings-paper1-prefreeze", "scripts"))
import confirm_p1  # noqa: E402

if __name__ == "__main__":
    if len(sys.argv) != 5:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    confirm_p1.holdout_gate = lambda out, smoke: None
    confirm_p1.run("holdout", sys.argv[1:])
