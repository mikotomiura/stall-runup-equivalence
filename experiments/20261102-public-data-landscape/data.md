# 入力データ

- KLiCKe [V264]: `~/gse-public-data/klicke/raw/WritingTask.zip`（SHA-256 `ffa4d753…555d`・取得は T2 の `data.md`）と、T2 の中間 `~/gse-public-data/klicke/work.tar.gz`（SHA-256 は `results/provenance.json`）。zip からは `DownTime`・`DownEvent`・`Activity` の 3 列だけを読み、クリックと貼り付けの時刻を取った。本文・`TextChange`・採点・人口統計は読んでいない。
- CoAuthor [V348]: `~/gse-public-data/coauthor/raw/coauthor.zip`（SHA-256 `8d7380ca…418b`）とメタデータ 2 本（SHA-256 は項目 1 の `data.md`）。メタデータは `worker_id`・`session_id` だけを使った。
- CS1 [V243]: 使っていない。研究者の決定（2026-11-02）で、著者へ連絡してから使う（`.steering/20261102-gse-public-data-landscape/decisions.md` 判断 1）。
- 中間と取っておく書き手の一覧: `~/gse-public-data/landscape/`（repo の外・書き手の ID は repo に出さない）。
- 記録を外部の LLM（Codex を含む）に送っていない。
- replay: `~/gse-public-data/klicke/bin/gse-replay`（T2 と同じバイナリ・SHA-256 は `run.sh` が照合）。
