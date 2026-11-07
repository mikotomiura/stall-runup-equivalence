# gse-estimator: the keystroke state estimator and gse-replay, as used in the paper

This crate is the keystroke state estimator of GSE (Generative Struggle Engine; `HMM_VERSION` 0.5.0) and the
`gse-replay` command that runs it offline, cut out of GSE commit `24767c7e` so that the paper's computations can be
rerun. It holds only what the paper ran:

- `gse-replay <session.ndjson>`: replays a session through the estimator and writes its per-second output
  (`FeatSnapshot` rows, including the probability of the Stuck state `p_stuck` and the smoothed friction axis `ewma_x`)
  as a JSON array to stdout.
- `gse-replay <session.ndjson> --features [--set <key>=<value> ...] [--feature-config <file>] [--pole <name>]`: writes
  the offline feature rows, the stall onsets and the observed intervals, under the stall definition in
  `prereg/p1-episode.json` (version 1.2.0) unless overridden.

The input hooks, the session logger, the interventions and the data-collection modes of GSE are not part of it. The
other flags of GSE's gse-replay (`--metrics`, `--segments`, `--sweep` and so on) are not recognised here: they are
rejected as extra arguments (exit code 2, as an unknown argument is in GSE), with a shorter usage text.

## Build

Rust 1.88 or later.

```
cargo build --release --locked
cargo test --release --locked
```

The binary is `target/release/gse-replay`. `Cargo.lock` pins every dependency to the version GSE used.

## What was copied and what was changed

`provenance/manifest.json` lists every source file with its SHA-256 and its relation to the GSE source:

| kind | files | relation |
|---|---|---|
| file | `analysis/` engine, features, intervention, key_gate, loop_step, heads/writing, heads/coding, feat_offline/* (9), `prereg/p1-episode.json` | byte-identical (their unit tests are included and run) |
| lock | `Cargo.lock` | every package in it appears in GSE's `Cargo.lock` with the same version, source and checksum |
| trimmed | `analysis/` anonymize, metrics, heads/mod | the upstream file up to its `#[cfg(test)]` module (those tests need modules this crate leaves out) |
| excerpt | `src/logger.rs` | `TaskKind` and its routing to a domain head: upstream lines 68 to 128, between the excerpt markers |
| cut | `src/bin/replay.rs` | the other modes of gse-replay removed; the full change is `provenance/replay.rs.diff` |
| new | `src/lib.rs`, `src/analysis/mod.rs`, `Cargo.toml` | module declarations and this crate's manifest |

`python3 provenance/check_provenance.py --self` checks the files against the recorded SHA-256 values and that no
file under `src/` or `prereg/` is missing from the manifest; `--upstream <gse-checkout>` checks each relation against
the GSE source itself. The crate's
version is kept at `0.1.40`, the GSE version it was cut from, because `--features` writes it into
`provenance.tool_version`.

## Equivalence gate

The paper's numbers were computed with the GSE build of `gse-replay` whose SHA-256 is
`26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3`. `gate/` checks that this crate's binary gives the
same output on every input the paper's pipelines feed it:

- `gate/compare-replay.sh` stands in for `gse-replay`. It runs the reference binary and this crate's binary with the
  same arguments and input, logs whether stdout, the exit code and stderr are byte-identical, and returns the
  reference's output so that the pipeline proceeds exactly as before.
- `gate/run-gate.sh <part>` runs the paper's own pipeline scripts (unchanged, from the GSE repository's
  `experiments/`) with that stand-in, then byte-compares the intermediates they write with the ones the paper used.
  It exits with 1 if any call or intermediate differs, if an intermediate the paper used was not rebuilt, or if any
  step fails (a failure both binaries share would otherwise compare as a match).

The evidence that this crate's binary is equivalent is the per-call comparison (stdout, exit code, stderr). Because the
stand-in returns the reference's output, the rebuilt intermediates come from the reference binary; their byte
identity with the paper's intermediates shows that the pipelines and the data were the ones the paper used, so that
the calls compared are the calls the paper made.

Result (2026-11-06, seen writers only; the held-out writers of CS1 2019 were not read):

| data | calls (no-flag / `--features`) | calls whose stdout, exit code or stderr differ | pipeline intermediates identical to the paper's |
|---|---|---|---|
| CS1 2019 | 2,644 / 1,079 | 0 | 1,078 of 1,078 tasks |
| CS1 2019, `--set` variants (q=0.995 with warmup_count=200; w_ms=15000; w_ms=60000) | 0 / 4,304 | 0 | 3 x 1,076 of 1,076 tasks |
| CoAuthor | 1,157 / 1,200 | 0 | 43 of 43 writers |
| KLiCKe | 4,992 / 4,992 | 0 | 4,992 of 4,992 |

The one CS1 call that exits with code 1 is the task the paper also excluded (its time span is too long for
`--features`); both binaries exit the same way with the same message. A first CS1 run with 7 parallel jobs
logged 9 differing `--features` calls, one of them a crash of the reference binary itself on a task it had
completed in the paper's run; free disk space was near zero then (it fell to 1.3 GiB even with 4 jobs). The
cause of those differences was not established. The rerun with 4 jobs, reported above, had none. The counts are in `gate/results/`.

A positive control confirms the gate can fail: a copy of this crate with one change (the initial state probabilities
`[0.80, 0.15, 0.05]` to `[0.79, 0.16, 0.05]`, or the rank of the stall threshold quantile rounded instead of taken as
the ceiling) was run on the first 20 tasks of the CS1 2019 data. The first change made 37 of 38 no-flag outputs differ,
the second 14 of 19 `--features` outputs.

The gate covers the calls the paper's pipelines make: no flag, `--features`, and `--features` with the three `--set`
variants above. `--feature-config` and `--pole` were not used by the paper and were not part of the gate (their code
is unchanged from GSE; see `provenance/replay.rs.diff`).

The gate needs the public datasets and the paper's intermediates, which are not redistributed here.

## License

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT license (`LICENSE-MIT`), at
your option.
