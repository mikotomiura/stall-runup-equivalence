# 環境

- macOS（Darwin 27.0.0・Apple Silicon）・Python 3.14.4・numpy 2.5.2（区間の計算だけ）。
- gse-replay: `~/gse-public-data/klicke/bin/gse-replay`（SHA-256 `26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3`・K6 と CS1 の俯瞰と同じ）。HMM_VERSION 0.5.0。停滞の定義 `src-tauri/prereg/p1-episode.json` 1.2.0（frozen）。
- 前の実験のスクリプト（`build_cs1.py`・`analyze_cs1.py`・`analyze_item2.py`・`convert.py`・`add_feat.py`）は書き換えずに読み込む。SHA-256 は `results/provenance.json`。
