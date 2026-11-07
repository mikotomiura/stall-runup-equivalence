# Summary of the pre-computation conditions documents

This file is an English guide to the conditions documents that the authors committed before each computation in this project. The Japanese originals are authoritative; this summary is only a reader's aid and adds nothing to them. Numbers and thresholds are copied from the originals; where a term is unclear, the original term is kept in parentheses. Commit times (UTC) are taken from `gse-commits.tsv`, a table built from the authors' private commit history (column `first_commit_utc`). Sections are in the order: the CS1 overview, the pre-freeze computations (with one deviation record), the exploration after the holdout was opened, the round-3 checks, and the four upstream experiments on which the computations depend.

Common vocabulary. "Stall" (停滞) is a long net-output stop for the writer, not a judgement that the writer is stuck. `theta` is a writer's own 0.99-quantile of inter-key intervals, undefined below 100 intervals. An "onset" is a stall start produced by the replay tool (`gse-replay --features`) using `theta` and "condition A" (net output of the 30 s window below 1). `p_stuck` and `ewma_x` are the two output signals of the keystroke estimator; `d_p_stuck` and `d_ewma_x` are their 30 s changes. A "control" is a pause of 2 s to under 30 s that did not become a stall. AUC is the probability that a stall-side value exceeds a control-side value (ties 0.5). The data directory lives outside the repository; only aggregate values are committed.

---

## experiments/20261104-cs1-landscape/conditions.md

First commit: 2026-11-03T23:46:28Z.

Purpose. A descriptive overview (no tests, no decision thresholds) applying the earlier overview's items L1-L6 to the CS1 keystroke dataset (introductory programming course, English, Python). Not data from the authors' own participants.

Input. CS1 2019 (Dataverse v5.2) and 2021 (v7.0) `keystrokes.csv`, each stopped unless its SHA-256 matches (2019: `b54d8cf5...7955`; 2021: `950a587b...22b6`). Columns read: event id, subject, assignment, event type, edit type, client timestamp, key names (2021 also `X-Action`, `X-Attention`). Inserted text is used by length only; deleted text, run input/output, source locations, and clipboard contents are not read. Single-character keys are read in memory only to assign a category and are never written. Order: timestamp, then event id.

Rules fixed before looking at counts.
- Unit: writer = `SubjectID`; task = (subject, assignment). A task is cut into work blocks wherever adjacent events are 1,800,000 ms (30 min) or more apart (same value as the P1 idle-trim setting). Blocks are converted and replayed separately, merged per task, and passed once through `--features`, so `theta` grows within a task. Blocks with fewer than 2 presses are dropped (count reported).
- Conversion tables for 2019 (browser IDE, no completion, no navigation keys) and 2021 (PyCharm, completion and auto-closing brackets): single-character insertions get their character category; space/return map to `space`/`enter`; delete maps to `backspace`; undo/redo to `other`; edits without a key name are counted as `auto-edit` (2019) and multi-character insertions as `multi-insert` (2021) and kept out of the keystroke series; paste, cut, drag, run, submit and task/editor switches go to an event table. In 2021, `X-Attention` lock-out and ON/OFF rows are read as a prompt and its answer (an inference).
- Holdout: 30% of writers per version, drawn with `random.Random(SEED)` (2019) and `SEED + 1` (2021), SEED 20261104, from the sorted subject list. The list is kept outside the repository; only the count and its SHA-256 are published; the run stops if any holdout writer appears in the input.
- Definitions: press intervals within blocks only; the 2021 "near a prompt" split (prompt to answer, or 60 s if no answer within 10 min) is used for the headline with totals also shown; writers with under 10 min observation are excluded (count reported); per-writer values are summarised by median and quartiles.
- Items: L1/L2 as in the earlier overview; L3 events near stalls (non-character keys, navigation, run, switch, paste); L4 receipt events (`paste`, and `multi-insert` in 2021); L5 task-level variation for writers with 5 or more tasks having defined `theta` and at least 5 min observation; L6 task thirds (stalls per hour, share with `p_stuck >= 0.60`, median `ewma_x`, share of time with `theta`); descriptive unit counts.

Output. Aggregate descriptions only.

Status. Description. Stated limits: English programming coursework at one US university; work blocks are an estimate; `p_stuck`/`ewma_x` are not calibrated on this population; the 2021 `paste` cannot separate outside pastes from in-file cut-and-paste.

---

## experiments/20261105-findings-paper1-prefreeze/conditions.md

First commit: 2026-11-05T06:10:30Z.

Purpose. Two pre-freeze computations on the writers already seen (CS1 2019 minus the 146 holdout writers): (1) the basis for the margin of the equivalence band, (2) a preliminary value for the precision rule. Explicitly not a hypothesis test.

Input and stop conditions. CS1 2019 seen writers, rebuilt with the same `build_cs1.py` and `gse-replay` (hash-checked); all build-summary counts must match the overview; 319 writers must remain after the 10-minute exclusion; `theta` and onsets reproduced in Python must match replay for every task and replay stderr must be empty. KLiCKe (4,992 writers, one session each) with the earlier K6 checks (pairwise AUC must reproduce 0.51474 for `d_p_stuck` and 0.52121 for `d_ewma_x` to 4 decimals). CS1 holdout, CS1 2021, and CoAuthor are not used.

Rules fixed beforehand.
- Stall: for each onset (s, t0), u is the last character input at or before t0; G(u) is the gap to the next character input (or the task's last press). Excluded: G(u) under 30 s, repeated u, u >= s, u within 60 s of the block start, and missing estimate rows at u or u - 30 s.
- Control: a pause of 2 s to under 30 s from the same writer, excluding the same kinds and any pause inside a stall interval [s - 30 s, t0].
- Matching: same delimiter class (whether the last press is `enter`) and same stratum (a `run`/`paste`/`switch` anywhere in the window gives "event present"; the primary stratum is "no event"); up to 5 controls per stall, drawn without replacement, SEED 20261105.
- Signals: `d_p_stuck(u)` and `d_ewma_x(u)` with the latest estimate row at or before u and at or before u - 30 s; non-numeric rows are dropped and counted.
- Computation 1 (width conversion): for each dataset and signal, find by bisection the constant added to stall values that makes the within-writer pooled AUC equal 0.55 (and 0.50). Per-writer threshold is the 90th percentile of that writer's control candidates (index `int(0.9 n)`; writers with fewer than 10 candidates dropped). Report hit rate, false-alarm rate, their difference, and firings per observed hour, with the share of firings that are stall pauses. An analytic reference for two equal-variance normals at AUC 0.55 is also given.
- Computation 2 (precision): per-writer win sums and pair counts; 146 of 319 writers drawn 1,000 times without replacement (SEED 20261105 + 1); for each, the draft's interval (resample the 146 writers 2,000 times, 2.5-97.5 percentiles) and its width; report the median and 5/95 percentiles of the width and the share of draws with width at most 0.10, plus tie share and zero-signal share (instrument degeneracy).
- Not reported: any real-signal AUC, interval, or direction on CS1 seen writers (not written to files), per-writer values.

Status. Rule-setting computation; not a hypothesis test. The margin 0.55 was set after seeing earlier KLiCKe and CoAuthor results; any change after this computation must be decided by the researcher and recorded as a deviation.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-2.md

First commit: 2026-11-05T07:04:11Z.

Purpose. A dry run of the confirmatory analysis script `scripts/confirm_p1.py` (a transcription of draft v4's rules) on the 319 seen writers: check that it runs end to end on holdout-shaped input; output direction-free quantities; widen the margin conversion; check that H3 can recover a known effect.

Rules. Environment pinned in `requirements-confirm.txt` (Python 3.14.4, numpy 2.5.2, scikit-learn 1.7.2, scipy 1.18.1, joblib 1.6.0, threadpoolctl 3.7.0). SEED 20261106 with fixed offsets (+1 H1 interval, +2 H3 split and gradient boosting, +3 H2 interval, +4 H3 resampling, +5 negative-control draw, +6 H2 negatives, +7 precision forecast); KLiCKe control draw keeps SEED 20261101.
- H1: pairs per writer x delimiter x stratum; ties within epsilon = 0.001. H2: window end w from block start + 60 s in 5 s steps; windows overlapping the stall-to-resumption span excluded; positive if a stall u lies in (w, w + 60 s]; realignment by elapsed-time decile (primary) and decile x press-count quintile. H3: 16 features, two learners, writer-grouped 5-fold, 200 resamples, and a negative control from H2 positives and negatives. Margin conversion at AUC 0.50-0.70 by an additive shift and by a quantile (Lehmann) shift. Precision forecast: 137 writers drawn 500 times.
- Recovery check: add a constant to the per-minute character-press difference so that feature alone has AUC 0.58; both learners' interval lower bounds must exceed 0.55 (50 resamples).

Not written when `mode=seen`: point estimates, interval positions, branches, 0.5 differences, or R for H1/H2/H3 on seen CS1 writers; only the KLiCKe H1 is reported. Stop on any failed reproduction check; smoke runs write outside the experiment directory and are not used.

Status. Dry run; description only.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-3.md

First commit: 2026-11-05T09:12:15Z (section G5 appended before the re-run; last commit 2026-11-05T10:41:10Z).

Purpose. Rerun the dry run after fixing the script (draft v5), with all sensitivity analyses (P8). Same output restrictions as computation 2.

Rules changed. (1) H3 uses 18 features: nine quantities (press count; 10/50/90th percentiles of intervals; counts of intervals of at least 2 s and at least 10 s; backspace share; mean burst length; character presses per minute) and their difference from the previous 30 s; missing values stay NaN (gradient boosting) or are median-imputed with a missing indicator inside the training split (logistic regression). (2) Controls must have condition A false at u (net output in (u - W, u] at least 1). (3) H1 branches are given at epsilon 0.001 and 0; support needs both signals in branch 1 at both; degeneracy is decided by the epsilon-0.001 tie share (over 50%). (4) H2 support: unaligned AUC 95% lower bound above 0.55 and the lower bound of R at least 0.5; lower bound above 0.55 with R's upper bound below 0.5 is "rise not explained by phase of work"; R straddling 0.5 is "undetermined". (5) More than 5% failed resamples makes a learner "undetermined". (6) In H3 the same control is counted once per writer x delimiter x stratum. (7) All intervals are percentile intervals (`int(0.025 k)`, `min(k - 1, int(0.975 k))`). (8) A synthetic self-test covers stall selection, control exclusions, H2 positives, degeneracy, duplicates, and `theta` across tasks.

Sensitivities (not used for decisions). H1: control pause upper bound 10 s / 60 s; whitespace-or-symbol delimiter; stratum window to the pause end; no condition A on controls. H2: lookahead 30 / 120 s; positives by t0; older exclusion. H1 and H2: signal-difference window 15 / 60 s; `theta` accumulated across tasks. H3: drop older-missing rows; burst cut at 1x `theta`; a gradient-boosting grid chosen by nested writer-grouped 3-fold CV (50 resamples). Replay is rerun for q = 0.995 (warm-up 200 = ceil(1/(1 - q)), both overridden) and W = 15 and 60 s, with a positive control that an unmodified rerun reproduces the original onsets.

G5 (before the re-run, after a two-system code review): sensitivity control draws use the primary's single random stream; the smoke-run repetition count also applies to the primary H3; grid-search failures are not swallowed; results are written to a temporary file then renamed; the holdout script refuses smoke runs and stops unless the start record is committed; the shell procedure no longer pipes its start-record check (a SIGPIPE under `pipefail` made an existing match look absent, reproduced by measurement), records interrupts, and runs the self-test first.

Status. Dry run; description only.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-4.md

First commit: 2026-11-05T11:44:10Z (sections H4, H5 appended before re-running; last commit 2026-11-05T14:02:45Z).

Purpose. Rerun with two rule changes (draft v6 round), estimate with synthetic signals how often equivalence would wrongly be declared at the band edge, and rehearse the holdout procedure in a disposable clone.

Rules. (1) Condition A is also imposed on stalls (order: first onset per u, u < s, condition A); the earlier version and a version with none are sensitivities. (2) H2's main exclusion covers u-to-resumption for every pause of 30 s or more; the excluded count is reported. (3) If R is non-numeric in over 5% of resamples, R is undetermined. Precision forecast and calibration use 138 = 146 x 319 / 338 writers.
- Edge calibration (SEED + 8): add a constant to seen-writer stall values so the pooled AUC (epsilon 0.001) is 0.45, 0.50, 0.55 per signal; draw 138 writers without replacement 500 times; apply the confirmatory interval (2,000 resamples) and five-branch rule, also at epsilon 0. Report the branch-1 share and the share with insufficient precision; at a target on the band edge the branch-1 share estimates the false-equivalence rate.
- Rehearsal: in a disposable independent `git clone`, 40 randomly drawn seen writers (SEED 20261107) act as a pretend holdout; run start commit, conversion, sensitivity intermediates, analysis and completion commit. Only pass/fail, time and writer-count checks are recorded; the clone is then deleted.
- Newly reported, direction-free: stalls removed by condition A; floor (`p_stuck` under 0.02) and ceiling (over 0.98) shares; the H3 negative-control pass/fail; calibration shares.

H4 (after the first run showed the H3 unaligned-window control failing on a thick estimate floor): the H3 gate becomes recovery of an embedded known size (single-feature AUC 0.58) by both learners above lower bound 0.55 (200 resamples, 5% failure cap), also on the holdout; the unaligned-window control becomes descriptive; an "off-floor pairs" H1 sensitivity (both `p_stuck` >= 0.02) is added.

H5 (after a second two-system review): gate and calibration check that the synthetic signal reached the target AUC (more than 0.002 off means "cannot calibrate"); uniform [0, 1) jitter (SEED + 9) breaks ties among the integer-valued gate features; calibration uses the production branch function; floor shares pair control ids with writer numbers; the holdout procedure stops if any commit contains a results file.

Status. Dry run and calibration; description only.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-5.md

First commit: 2026-11-05T15:22:24Z.

Purpose. Rerun after the round-5 fixes and rehearse a two-stage holdout procedure.

Rules. H3 is downgraded to a description (no branch written on seen data). The holdout procedure has two stages: `start` verifies and commits a start record then stops; `compute` fetches the start record through an OSF URL and its SHA-256 and computes; the script's gate requires `start.json` and `compute.json` at HEAD with equal SHA-256. The decision is fixed at the first completed run. New sensitivities: off-floor pairs by `p_stuck` at u - 30 s (primary) or at u; calibration with replacement as well as without; H2 negatives before non-stall long pauses removed; controls drawn within the same task only; writer-equal weights; a description of events inside the pause. After a code review the holdout gate also requires: HEAD is the start commit, no completion record, no results or interruption file, and no other start or interruption record with the same `start.json` on any branch.

Rehearsal. As in computation 4 (40 writers, SEED 20261107), plus checks that a wrong SHA-256 stops stage 2, skipping stage 1 stops, a repeat after completion stops, an interrupted stage 2 commits an interruption record that blocks a reason-less stage 1, and a stage 1 given a reason moves the earlier run aside. Only pass/fail, time, and writer counts are recorded.

Status. Dry run; description only. Values from computation 4 expected unchanged except the gate, calibration, and new sensitivities; differences are recorded.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-6.md

First commit: 2026-11-05T16:27:33Z (section J3 appended; last commit 2026-11-05T16:53:56Z).

Purpose. Add to the boundary calibration the share of draws in which both signals are in branch 1 in the same 138-writer draw; rerun on the 319 seen writers.

Rules. The random stream is re-seeded from SEED + 8 per version and target so draws agree across signals; the run stops if they differ, and the joint share is not reported if either signal misses its target AUC. Output only `boundary_calibration.joint_both_signals`: targets 0.45/0.50/0.55 x draw type, share equivalent for both signals at epsilon 0.001 and at both epsilons. Three mutations of the self-test all failed as intended.

J3 (defect found by the rehearsal). A completion-commit subject `"...（$OUT_REL）"` was read by bash 3.2 as a variable name including the multibyte character and aborted with `unbound variable`; it is now `${OUT_REL}`. Because the ERR trap does not fire on such an abort, no interruption record was committed; it moved to an EXIT trap, a missing completion mark forces exit code 1, and the scripts are searched mechanically for such variables (0 found). The rehearsal restarts.

Status. Dry run; description only.

---

## experiments/20261105-findings-paper1-prefreeze/conditions-7.md

First commit: 2026-11-05T18:11:18Z.

Purpose. After round-6 comments, fix two things in the boundary calibration and rerun on the 319 seen writers.

Rules. (1) In each calibration draw, the intervals for both epsilons and both signals use the same resampling seed, as production (`h1_summary`, seed + 1) does; before this, the "both epsilons" share came out smaller than production. Per-signal shares and the computation-6 joint shares may change. (2) Add the least favourable configuration of the intersection null: `boundary_calibration.joint_one_at_bound_other_at_050` (one signal at the band edge 0.45 or 0.55, the other at 0.50; share judged equivalent for both in the same draw, by draw type), combined from existing draws; the draw of the i-th 138 writers is the same across targets and signals and the run stops if not. Production computation and decision rules are unchanged. The self-test passes; a synthetic dataset (160 writers x 20 draws) shows the new output and the cross-target draw check passes; common keys outside calibration are compared with computation 6. Nothing direction-bearing is output.

Status. Dry run; description only.

---

## experiments/20261105-findings-paper1-prefreeze/deviation-1.md

First commit: 2026-11-06T01:22:44Z. The original has an English governing section; this is a short restatement.

What happened. Stage 2 of the holdout run (start record https://osf.io/hfvy7, `start.json` SHA-256 `53ef6f9c...20dd`) stopped at the conversion step before any statistic was computed: the registered check that the number of writers read equals the holdout list size (146) found 145.

Cause. One holdout writer has a single raw row (`File.Edit` with `X-Checkpoint`) and no press or event that creates an assignment unit, so no unit is read; this writer would be excluded anyway (observation under 10 minutes). No decision value, AUC, or per-writer value was computed or seen; only that writer's row count and event kinds were inspected.

Change. The check is replaced by `scripts/check_holdout_writers.py`, requiring (1) every writer read is in the holdout list, (2) every holdout writer not read has zero unit-creating rows under the conversion's own mapping (`map_2019` of `build_cs1.py`), (3) read + not read = 146; it writes counts only to `results_holdout/holdout_writers_check.json`. Negative controls: removing a read writer from a copy, or adding a writer outside the holdout, both fail the check. Nothing else changed apart from adding this note and the checker to the manifest.

Procedure. The researcher decided the change after the stop was reported. A new manifest is registered in a separate OSF registration of the same project with this note and the frozen files; the run restarts from stage 1 with a reason (previous outputs moved aside and committed), the new start record is registered, then stage 2 runs. The decision uses the first completed run, which is this restarted run. The paper reports the interrupted run, this deviation, and the order of registrations.

Status. Deviation record, written before the restarted run.

---

## experiments/20261106-findings-paper1-exploration/conditions.md

First commit: 2026-11-06T02:26:32Z (sections X4 onward appended before their computations).

Overall. Explorations outside the registration (Appendix F of the preregistration, https://osf.io/mrkxt; deviation 1 https://osf.io/fqgcp), run after the holdout run. All are reported as "exploration outside the registration", never use the word "supported" even if a branch is applied, and are not used for decisions. Aggregates only. The confirmatory script `confirm_p1.py` (frozen manifest version) is called unchanged; wrappers replace functions rather than editing it. Base SEED 20261126; derived seeds follow the script.

- X1-X3. H1-type (`h1_writer`, `h1_summary`: two signals x epsilon 0.001 and 0, intervals, branch, counts) and H2-type (`h2_writer`, `h2_summary`) outputs for KLiCKe (all 4,992 writers, one block each, `cs1=False`, no exclusions) and CS1 2021 seen writers (31; 13 holdout writers skipped; writers under 10 min excluded); no sensitivities; KLiCKe values sit beside, not in place of, the appendix values. Output `results/explore.json`. CoAuthor is X6.
- X4. The older 2021 intermediate lacked newer fields; it is rebuilt with the frozen `build_cs1.py` skipping the 13 holdout writers.
- X5 Seed spread (description). On the holdout intermediate, vary only the control-draw seed over 20261106 + 100 + k (k = 0-19); report min, max, SD of the pooled pair AUC against the main value; no intervals.
- X6 CoAuthor seen writers (43 writers, 1,157 sessions, release 100 ms after press; the 18 held-out writers were removed when the intermediate was built and `holdout.json` is not opened). Stall starts: a writer's sessions are joined with 2,500 ms gaps, `theta` and condition A computed on the joined clock, onsets mapped back. Each session is one block for the script (a researcher's decision departing from the appendix wording, because estimator rows are already per session). Gates: G1 per-session Python `theta`/onsets equal stored replay onsets; G2 the bridge equals the Appendix-A4 pipeline (convert, replay, add features, concatenate, replay `--features`; replay hash `26cae610...33bb3`) for every onset. AI-suggestion events (names starting `suggestion`, `api-insert`) exclude a stall if in [u - 60 s, u] and a control if in [u - 60 s, u + 60 s]; H2 negatives get none. Result `results/explore_coauthor.json`; never counted as equivalence evidence.
- X7 (after G2 stopped). G1 passed (1,519 onsets); G2 failed for 5 of 43 writers (12 onsets; 1,853 vs 1,865): replay drops no-input estimate rows before a gap of 60,000 ms or more and at file end, which differs between per-session and joined runs. Fix: reset the last observed interval end of each non-final session to its last estimate row; the corrected bridge matched all 1,865. The old bridge is kept as a mutation that turns the gate red (5 writers, 12 onsets).
- X8 Pause position. Categories by the character at pause start: after a newline (`enter`); after a token boundary (`space`, `tab`, `punctuation`, `sentence_punctuation`); inside a token (`alphabetic`, `numeric`). On the holdout intermediate: (a) shares by stratum x category for stalls and control candidates (gate: counts equal the confirmatory ones; control candidates 71,220); (b) H1-type with three-category matching. `results/explore_position.json`.
- X9 Counts before the draft. (a) Mapping of edit events: raw 2019 rows of the 146 holdout writers only, split under the frozen `map_2019` plus insert/delete lengths (insertions with a key of length 1, 2 or more, or whitespace-only; insertions without a key; deletions with a key of length 1 or 2 or more; deletions without a key; undo/redo; paste; cut; drag; unmapped), as shares of `File.Edit` rows and of inserted characters (gate: counts equal the build summary's `row_kinds`). (b) Instrument behaviour for the 140 holdout writers with 10 min or more: floor/ceiling shares, mean, SD, 10/50/90th percentiles of both signals, the 30 s difference distribution and exact-zero share, per-writer floor-share quantiles, and signal values at u - 30 s, u, u + 10, u + 20, u + 29 s over the 4,554 main-stratum stalls (gate: the four H1 points equal confirmatory values within 1e-12). (c) Effective number of writers and top-10% pair share under three-category matching. `results/explore_predraft.json`.
- X10 Positive control. Question: can the equivalence be told apart from an estimator that does not react at all? On seen writers only (1,078 tasks, 319 writers) only estimate rows are rebuilt (replay without `--features`). In (u - 30 s, u) of each stall: S (slower), k in {1.5, 2, 3}: presses moved to u - k(u - t) and those earlier than u - 30 s / k dropped; C (corrections added), r in {0.1, 0.25, 0.5}: each adjacent interval of 3 ms or more is chosen with probability r and a backspace (at 1/3) plus a same-kind press (at 2/3) inserted. Control windows are not changed. Output per condition: H1-type, share of stall estimates changed, number of control estimates changed. Pre-set reading: report per signal the smallest step whose interval lower bound exceeds 0.55 or upper bound is below 0.45; if none, write that a 30 s difference does not respond to changes of this size and that the equivalence cannot be separated from instrument properties; even if some do, do not write that the equivalence shows sensitivity. Gates: unchanged rebuild equals stored estimates for all tasks; counts and baseline pairs, tie share, interval width match confirmatory. Mutations: moving one press by 1 ms turns the first gate red; a no-op change leaves AUC at baseline. `results/explore_positive_control.json`.
- X11 Comparison table. On the 140 holdout writers, the four estimator quantities (`d_p_stuck`, `d_ewma_x`, `p_stuck`, `ewma_x`) and the two learners on the same pairs with the same statistic (`paired_auc`); difference intervals use the confirmatory out-of-fold resampling (200 resamples). Gate: recomputed learner points and intervals equal confirmatory values; estimator epsilon-0.001 points equal H1 (1e-12). `results/explore_comparison.json`.
- X12 (a) Distribution of onset minus u and of G(u) (10/25/50/75/90th percentiles, mean) and shares of G(u) at least 60, 120, 300 s over the 4,554 stalls; (b) counts of candidate onsets with no later character input or with the next input in another block; (c) effective number of writers per calibration draw (138 writers, 500 draws, same random stream), without rerunning branches. `results/explore_length_calibration.json`.
- X13. X10 plus smaller steps (S k in {1.02, 1.05, 1.1, 1.2}; C r in {0.02, 0.05}), with X10's steps rerun (gate: equal to X10), and a fixed-control version where drawn-control values come from baseline rows (gate: equals baseline for no change). Same reading rule, plus the AUC near S x 1.02 (the observed difference) as description. `results/explore_positive_control_v2.json`.
- X14. Control lower bound `CTRL_LO` (2,000 ms) replaced by 5,000 and 10,000 ms for the run only; H1-type output (gate: the 2,000 ms run reproduces confirmatory points to 1e-12). `results/explore_ctrl_lo.json`.
- X15. Each of the 18 learner features used alone as a score (`paired_auc`; rows with non-numeric values dropped per feature and counted); points and counts only (gate: estimator AUCs equal X11). `results/explore_learner_features.json`.

Status. Exploration outside the registration (holdout already opened); not used for decisions.

---

## experiments/20261107-findings-paper1-round3-checks/conditions.md

First commit: 2026-11-06T16:23:35Z.

Purpose. Two explorations outside the registration (X16 and X17), run after the holdout was opened; not used for decisions. Aggregates only, to `results/explore_round3.json`.

Common. The holdout intermediate is read with `load_cs1` (mode=holdout) from the frozen `confirm_p1.py`; the holdout procedure is not run. Rows come from `h1_writer` in the main setting with the control-draw seed 20261106; a copy in the script adds the time s (when condition A became true) to each stall row, using the same order and random numbers. Gate G0: copy rows (delimiter, stratum, signal, control id) equal `h1_writer`'s for all rows and counts equal the confirmatory `h1_counts`.

X16 Floor and ceiling of the scaling function phi. The five internal quantities f1, f3, f4, f5, f6 are not in the intermediate, so each holdout task is replayed again per block with the same input construction as `build_cs1.py` (replay hash `26cae610...33bb3`, no arguments). Gate G1: the concatenated (timestamp, `p_stuck`, `ewma_x`) equals the stored estimate rows for every task and row (exact float equality). Gate G2: the supplement's formula phi(f, beta) = min(1, max(0, (f - beta) / 2 beta)) with beta = 150 ms, 0.06, 2.0, 0.08 for f1, f3, f5, f6, combined as x = 0.30 phi3 + 0.25 phi6 + 0.25 phi1 + 0.20 phi5 (clipped to [0, 1]), equals replay's `raw_x` within 1e-9 on every row. Counted for main-stratum stall rows and drawn controls (each control id once per delimiter x stratum): the share of phi1, phi3, phi5, phi6 equal to 0 and to 1 at u and at u - 30 s, the share of the 30 s change exactly 0, and the share where all four changes are 0, separately for stalls and controls. Pre-set reading: if any component is pinned (0 or 1) in at least half of stall or control rows at u - 30 s or u, the limitations will say that part of the equivalence may come from floor and ceiling of the scale; if every component is under 1/4, it will say the equivalence is not explained by pinning; in between, only the shares are given.

X17 Stall definition and the amount of preceding typing. Stalls with s - u exactly 30 s (condition A became true only when u's press left the window) are S30; those with s - u under 30 s are S<30. For each subset (with the controls those rows drew), as in X15, report AUC (ties not epsilon-adjusted, main stratum) of presses in the 30 s before u and of character presses per minute, with the two signals' 30 s changes alongside; points and counts only. Gate G3: with all stalls kept, the values equal the X15 values within 1e-12. Pre-set reading: S30 AUC of press count within 0.02 of 0.5 is written as "the learner's 'fewer keystrokes' can be explained by how the stall is defined"; at or below 0.46, "not explained by the definition alone"; in between values only; with fewer than 200 S30 stalls, counts only.

Status. Exploration outside the registration.

---

## Upstream experiments

Only the parts that affect the paper's computations (data transformation, input construction, holdout construction) are summarised. Data are kept outside the repository; only aggregates are committed.

### experiments/20261031-coauthor-pre-request-signal (conditions.md, conditions_round2.md, data.md)

First commit: conditions.md 2026-10-31T13:41:17Z; conditions_round2.md 2026-10-31T15:42:50Z.

CoAuthor v1.0 (zip SHA-256 `8d7380ca...e418b`; writers from the site's two metadata sheets, hashes in `data.md`) is converted by `convert.py`: user single-character inserts get a character category, deletes `backspace`, cursor moves `navigation`; API-origin inserts, `suggestion-*` operations, and inserts of 2+ characters are left out (counted). No releases are logged, so a release is synthesised at min(press + 100 ms, next press - 1 ms); replay uses default beta, `hmm_version` 0.5.0, one session at a time. Round 2 adds 30 s signal changes and builds stall onsets from a writer's sessions joined with 2,500 ms gaps and mapped back.

### experiments/20261101-klicke-early-stall-trigger (conditions.md, data.md)

First commit: 2026-11-01T08:10:19Z.

KLiCKe `WritingTask.zip` (SHA-256 `ffa4d753...8555d`); only `DownTime`, `UpTime`, `DownEvent` of each keystroke CSV are read, one file at a time. One row is one press with real release time; mouse clicks are not key rows; key names map to categories (alphabetic, numeric, space, enter, tab, `sentence_punctuation` for `.` `,`, `punctuation`, `backspace`, `delete`, modifier, navigation, escape, function, other; table fixed before counting). Sessions under 200 presses are dropped. Pipeline: replay, add-features, replay `--features`; one session per writer. `theta` and condition A are recomputed in Python and must equal replay's onsets exactly. Its pre-stall signal rules (u = last character input at or before t0, G(u) at least 30 s, controls 2 s to under 30 s matched by delimiter, up to 5) are reused.

### experiments/20261102-public-data-landscape (conditions.md, data.md)

First commit: 2026-11-01T15:19:20Z.

A descriptive overview of KLiCKe and CoAuthor (items L1-L6). CoAuthor's intermediate is built per session (not joined across sessions) with the same converter and replay hash `26cae610...33bb3`, so `theta` grows within a session as in KLiCKe. Holdout: 30% of writers per dataset (KLiCKe: `random.Random(SEED).sample` of indices 0-4,991; CoAuthor: of sorted `worker_id`, `SEED + 1`; SEED 20261102), list kept outside the repository, count and SHA-256 published, run stops if a held-out writer appears.

### experiments/20261103-coauthor-per-person (conditions.md; no data.md)

First commit: 2026-11-03T12:56:05Z.

A per-person learning-curve experiment, not part of the paper's tests. The paper uses only its input construction: `scripts/build_obs.py` rebuilds the CoAuthor intermediate (`coauthor_work_obs`) for the session numbers already in the earlier intermediate, without opening `holdout.json`; a check requires keys, onsets, observed intervals and events to equal the earlier intermediate exactly and the estimate-row columns to match; one mismatch stops the run.

---

## experiments/20261107-findings-paper1-round4-checks/conditions.md

First commit: 2026-11-07T01:22:40Z.

Purpose. An audit after the holdout was opened (exploration [E-H]; the decision is not changed) of how the 30-second change is formed. The signal at t is the latest estimate row at or before t minus the latest row at or before t - 30 s, with no maximum gap set; across a break in the estimate rows the nominal 30-second change could span a different time.

Rows. The same rows as X16 and X17: the holdout intermediates read with `confirm_p1.py` (`load_cs1`, mode holdout), the main setting with seed 20261106, and the gate that the rows equal those of `h1_writer` and the counts equal those of `confirm_holdout.json`.

What is counted. For each stall of the main stratum and each drawn control (once per cell), with a the time of the row at or before u and b the time of the row at or before u - 30 s: the lags u - a and (u - 30 s) - b (median, 99th percentile, maximum) and the share above 2 s; the share of a - b outside 28-32 s; the share with a and b in different observation intervals; and the number of rows meeting any of these ("off rows"), separately for stalls and controls.

Pre-set reading. If there are no off rows, state in the paper that both rows lie in one observation interval and 28-32 s apart. Otherwise, report the counts and shares, and recompute the point estimate of the main statistic (epsilon 0.001, both signals) without the off stalls and controls (a stall left without controls is dropped); no interval; the decision is not changed.
