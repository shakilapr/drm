//! Canonical C ABI crate (codebase.md 2.1 layer `runtime`): `extern "C"` exports
//! over drm-core; `bindings/c/include/drm/drm.h` is the single source of truth.
//! Fail-stop: a Rust panic here terminates the node process (no `Internal` status).
//! REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes), REQ-BIND-002
//! (explicit close; per-handle thread-safety).
//!
//! Unsafe is permitted here (codebase.md 3.2); exports land W2+ per
//! `docs/algorithms/bindings/c-abi-surface.md`.

#[cfg(test)]
mod tests {
    /// REQ: REQ-BIND-007 — placeholder compile smoke for the W1 skeleton.
    #[test]
    fn skeleton_compiles() {
        let s = drm_types::Status::Busy;
        assert_eq!(s.code(), -11);
    }
}
