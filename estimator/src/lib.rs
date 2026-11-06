//! The GSE cognitive-state estimator (HMM_VERSION 0.5.0) and the offline feature pipeline used by
//! `gse-replay`, cut out of GSE commit 24767c7e. See `README.md` and `provenance/manifest.json` for what
//! was copied and what was left out (input hooks, the session logger, interventions and the O1 collection mode).

pub mod analysis;
pub mod logger;
