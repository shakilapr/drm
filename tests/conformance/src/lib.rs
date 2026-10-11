//! Whole-system conformance harness library (work-plan M10W2-E4-T1): the
//! two-host router simulation and the route-derivation ↔ router-filtering e2e
//! live as integration tests in this package. The package is excluded from the
//! root workspace because it links the runtime (`drm-router`, `drm-core`) and
//! toolchain (`drm-workspace`) layers together, which per-crate layering
//! forbids (codebase.md §2.1); the whole-system suite is exactly where that
//! crossing is allowed.
//! REQ: REQ-ROUTE-022, REQ-ROUTE-031, REQ-QAL-008.

/// The two-host harness package version, so the crate has a public surface.
/// REQ: REQ-QAL-008.
pub const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    /// REQ: REQ-QAL-008 — the harness package reports its version.
    #[test]
    fn version_present() {
        assert!(!super::HARNESS_VERSION.is_empty());
    }
}
