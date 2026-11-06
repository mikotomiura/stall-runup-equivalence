# 環境

- macOS（Darwin 27.0.0・Apple Silicon）・python3（標準ライブラリだけ）。
- `gse-replay`: このブランチのコミット `182c8bdb`（ソースは origin/main `24767c7e` と同じ）から `cargo build --release --bin gse-replay` で作った。SHA-256 は `results/provenance.json`。lockfile は `src-tauri/Cargo.lock`。ディスクの空きのためビルドの中間は消し、バイナリを `~/gse-public-data/klicke/bin/` に写して `GSE_REPLAY` で渡した。
- HMM_VERSION 0.5.0（`src-tauri/src/analysis/engine.rs`）。停滞の始まりのパラメータは `src-tauri/prereg/p1-episode.json`（1.2.0・frozen）の既定。
