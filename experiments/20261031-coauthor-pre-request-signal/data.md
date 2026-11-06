# 入力データ

- CoAuthor v1.0（Lee, Liang & Yang, CHI 2022）。https://coauthor.stanford.edu の "Download writing sessions"（Google Drive id 1C9FCCsyY-5I7mcBHi-__R7lxHkGX_-9Q）。2026-10-31 取得。
  - `coauthor.zip` SHA-256 `8d7380ca3811c671e774e43bfb9f5d3265e2a5d425788a066a612831aa1e418b`（jsonl 1,447 本）
- メタデータ（同サイトの Google Sheets・CSV で書き出し）:
  - 創作 gid=1870708729 → `metadata_creative.csv` SHA-256 `66fd2ca541999412de4050a85c17274f02eea04424e83906b697f4cd824c65bf`（829 行）
  - 論述 gid=320516663 → `metadata_argumentative.csv` SHA-256 `cfadf031b21c07951f648b02422b6703850359c7ff6a78a8ef513624c8e4c0d1`（614 行）
  - アンケートの 2 タブは使わず、手元にも残していない。
- 置き場: `~/gse-public-data/coauthor/raw/`（repo の外・再配布しない・判断 2）。
