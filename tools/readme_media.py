#!/usr/bin/env python3
"""Draw the README figure of the holdout AUCs, and check its numbers against the paper's supplement table.

Usage:
  python3 -I tools/readme_media.py            writes docs/media/holdout-auc.svg
  python3 -I tools/readme_media.py --check    checks that every value of the figure, rounded as printed, equals the
                                              row of the same name in Table 5, label tab:sens (paper/appendix/supplement.tex), and
                                              that the SVG on disk is the one this script draws
Reads only experiments/20261105-findings-paper1-prefreeze/results_holdout/confirm_holdout.json (aggregate values).
No number of the paper comes from this script. Exit code: 0 = written or checked / 1 = a mismatch.
"""
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RESULT = os.path.join(ROOT, "experiments/20261105-findings-paper1-prefreeze/results_holdout/confirm_holdout.json")
TABLE = os.path.join(ROOT, "paper/appendix/supplement.tex")
OUT = os.path.join(ROOT, "docs/media/holdout-auc.svg")
SIGNALS = (("d_p_stuck", "Change in the Stuck-state probability"), ("d_ewma_x", "Change in the smoothed friction axis"))
# (label in Table S, path to the H1 block in the result, statistic)
ROWS = [
    ("Main", ("H1",), "pooled_auc"),
    ("Control pauses up to 10 s", ("sensitivity", "ctrl_hi_10s", "H1"), "pooled_auc"),
    ("Control pauses up to 60 s", ("sensitivity", "ctrl_hi_60s", "H1"), "pooled_auc"),
    ("Boundary type: whitespace or symbol", ("sensitivity", "align_sep_sym", "H1"), "pooled_auc"),
    ("Stratum window to the end of the pause", ("sensitivity", "layer_until_resume", "H1"), "pooled_auc"),
    ("No condition A on stalls", ("sensitivity", "stall_without_condA", "H1"), "pooled_auc"),
    ("Condition A on neither side", ("sensitivity", "neither_condA", "H1"), "pooled_auc"),
    ("Controls within the assignment", ("sensitivity", "ctrl_within_task", "H1"), "pooled_auc"),
    ("Writer-equal weights", ("sensitivity", "h1_writer_equal_weight", "H1"), "writer_mean_auc"),
    ("Not at floor, start point $u - 30$ s", ("sensitivity", "h1_off_floor_start", "H1"), "pooled_auc"),
    ("Not at floor, end point $u$", ("sensitivity", "h1_off_floor_end", "H1"), "pooled_auc"),
    ("Signal change over 15 s", ("sensitivity", "signal_diff_window_15s", "H1"), "pooled_auc"),
    ("Signal change over 60 s", ("sensitivity", "signal_diff_window_60s", "H1"), "pooled_auc"),
    ("$\\theta$ accumulated across assignments", ("sensitivity", "theta_cross_task", "H1"), "pooled_auc"),
    ("Stall quantile $q = 0.995$ (warm-up 200)", ("sensitivity", "replay_q995", "H1"), "pooled_auc"),
    ("Condition A window $W = 15$ s", ("sensitivity", "replay_w15", "H1"), "pooled_auc"),
    ("Condition A window $W = 60$ s", ("sensitivity", "replay_w60", "H1"), "pooled_auc"),
    ("Event stratum (described)", ("H1_event_layer_described",), "pooled_auc"),
]
BAND = (0.45, 0.55)
XMIN, XMAX = 0.42, 0.58


def plain(label):
    return (label.replace("$u - 30$", "u − 30").replace("$u$", "u").replace("$\\theta$", "θ")
            .replace("$q = 0.995$", "q = 0.995").replace("$W = 15$", "W = 15").replace("$W = 60$", "W = 60"))


def values():
    with open(RESULT) as fh:
        j = json.load(fh)
    out = []
    for label, path, stat in ROWS:
        node = j
        for k in path:
            node = node[k]
        out.append((label, [(node[s]["eps_0.001"][stat], node[s]["eps_0.001"]["ci95"]) for s, _ in SIGNALS]))
    return out


def svg(rows):
    w, left, right, top, rh = 900, 300, 30, 70, 26
    h = top + rh * len(rows) + 60
    pw = w - left - right

    def x(v):
        return left + (v - XMIN) / (XMAX - XMIN) * pw

    s = ['<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %d %d" role="img" '
         'aria-label="Holdout AUC of the two signals for the main and sensitivity analyses, all near 0.49, '
         'with the equivalence band from 0.45 to 0.55">' % (w, h, w, h),
         '<style>.bg{fill:#fcfcfb}.t{fill:#1b1b1a;font:13px system-ui,-apple-system,sans-serif}'
         '.m{fill:#5b5a55;font:12px system-ui,-apple-system,sans-serif}.g{stroke:#e4e3de}.band{fill:#ecebe6}'
         '.mid{stroke:#8a8983}.c1{fill:#2a78d6;stroke:#2a78d6}.c2{fill:#eb6834;stroke:#eb6834}'
         '@media (prefers-color-scheme: dark){.bg{fill:#1a1a19}.t{fill:#f0efec}.m{fill:#c3c2b7}.g{stroke:#33332f}'
         '.band{fill:#2a2a27}.mid{stroke:#77766f}.c1{fill:#3987e5;stroke:#3987e5}.c2{fill:#d95926;stroke:#d95926}}'
         '</style>',
         '<rect class="bg" width="%d" height="%d"/>' % (w, h)]
    y0, y1 = top - 8, top + rh * len(rows) + 4
    s.append('<rect class="band" x="%.1f" y="%d" width="%.1f" height="%d"/>' % (x(BAND[0]), y0, x(BAND[1]) - x(BAND[0]), y1 - y0))
    for t in (0.42, 0.45, 0.50, 0.55, 0.58):
        s.append('<line class="%s" x1="%.1f" y1="%d" x2="%.1f" y2="%d" stroke-width="1"%s/>'
                 % ("mid" if t == 0.5 else "g", x(t), y0, x(t), y1, ' stroke-dasharray="4 3"' if t == 0.5 else ""))
        s.append('<text class="m" x="%.1f" y="%d" text-anchor="middle">%.2f</text>' % (x(t), y1 + 18, t))
    s.append('<text class="m" x="%.1f" y="%d" text-anchor="middle">AUC (ε = 0.001) with 95%% interval; '
             'shaded: equivalence band [0.45, 0.55]</text>' % (left + pw / 2, y1 + 40))
    # legend
    s.append('<circle class="c1" cx="%d" cy="24" r="5"/><text class="t" x="%d" y="28">%s</text>' % (left, left + 12, SIGNALS[0][1]))
    s.append('<rect class="c2" x="%d" y="40" width="9" height="9"/><text class="t" x="%d" y="49">%s</text>'
             % (left - 4, left + 12, SIGNALS[1][1]))
    for i, (label, vals) in enumerate(rows):
        yc = top + rh * i + rh / 2
        s.append('<text class="t" x="%d" y="%.1f" text-anchor="end">%s</text>' % (left - 12, yc + 4, plain(label)))
        for k, (v, (lo, hi)) in enumerate(vals):
            yy = yc - 5 if k == 0 else yc + 5
            cls = "c%d" % (k + 1)
            s.append('<line class="%s" x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke-width="2"/>' % (cls, x(lo), yy, x(hi), yy))
            if k == 0:
                s.append('<circle class="%s" cx="%.1f" cy="%.1f" r="4.5"/>' % (cls, x(v), yy))
            else:
                s.append('<rect class="%s" x="%.1f" y="%.1f" width="8" height="8"/>' % (cls, x(v) - 4, yy - 4))
    s.append('</svg>')
    return "\n".join(s) + "\n"


def check(rows):
    tex = open(TABLE).read()
    bad = 0
    num = re.compile(r"(0\.\d+) \((0\.\d+)--(0\.\d+)\)")
    for label, vals in rows:
        line = next((l for l in tex.splitlines() if l.strip().startswith(label + " &")), None)
        if line is None:
            print("FAIL no row in Table 5: %s" % label)
            bad += 1
            continue
        printed = num.findall(line)
        for (v, (lo, hi)), p in zip(vals, printed):
            for got, shown in ((v, p[0]), (lo, p[1]), (hi, p[2])):
                if "%.*f" % (len(shown) - 2, got) != shown:
                    print("FAIL %s: %s printed, %.6f in the result" % (label, shown, got))
                    bad += 1
        if len(printed) != 2:
            print("FAIL %s: expected two values in Table 5" % label)
            bad += 1
    with open(OUT) as fh:
        if fh.read() != svg(rows):
            print("FAIL docs/media/holdout-auc.svg is not what this script draws")
            bad += 1
    print("checked %d rows against Table 5 (tab:sens); %d mismatches" % (len(rows), bad))
    return 1 if bad else 0


def main(argv):
    rows = values()
    if argv == ["--check"]:
        return check(rows)
    if argv:
        print(__doc__, file=sys.stderr)
        return 2
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as fh:
        fh.write(svg(rows))
    print("wrote", os.path.relpath(OUT, ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
