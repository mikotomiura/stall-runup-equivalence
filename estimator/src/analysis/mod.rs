//! The platform-independent analysis modules that `gse-replay` needs (no-flag path and `--features`).
//! The GSE original (`src-tauri/src/analysis/mod.rs`) also declares calibration, cohort, invitation,
//! portrait, pointer_bin and probe_scheduler modules, which are not part of this crate.

pub mod anonymize;
pub mod engine;
pub mod feat_offline;
pub mod features;
pub mod heads;
pub mod intervention;
pub mod key_gate;
pub mod loop_step;
pub mod metrics;
