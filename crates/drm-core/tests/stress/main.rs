//! Storm stress suites (`tests/stress/README.md`, work-plan M04W3-E1).
//! Excluded from the default gate; run with `cargo xtask test --stress`.
//! REQ: REQ-QAL-006.

mod chaos_kill_matrix;
mod hazard_storm;
mod reclaim_e2e;
