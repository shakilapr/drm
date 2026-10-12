//! Perf fixture node: controller (publisher + consumer).
//!
//! Smoke source at the same fidelity as `examples/mixed_robot`: the manifest's
//! build command asserts the generated include tree and deployment metadata
//! exist; the publish/consume loops for `control/{command,telemetry,pose}` and
//! `sensor/imu` are driven by `cargo xtask perf-matrix`.
//!
//! REQ: REQ-QAL-011 (latency baseline), REQ-QAL-012 (in-bench invariants).

fn main() {}
