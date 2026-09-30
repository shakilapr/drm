//! Counter registration module: `GlobalAlloc` wrapper over `System` that
//! totals every allocator entry (work-plan M01W4-E1-T2; canon-codec I4).
//! Compiled only with the `test-count-allocs` feature.
//! REQ: REQ-CORE-082.

#[cfg(feature = "test-count-allocs")]
pub mod counter;
