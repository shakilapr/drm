//! Channel-level metadata, attachment validation, and per-channel runtime state
//! (docs/algorithms/runtime/channel-metadata.md).
//! REQ: REQ-CORE-002 (validate once at attach), REQ-CORE-014/015/016/017 (descriptor),
//! REQ-CORE-019 (epoch), REQ-CORE-026 (first-valid watermark), REQ-CORE-093/094 (CCB).

pub mod ccb;
