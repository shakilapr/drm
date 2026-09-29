//! Boundary router crate (codebase.md 2.1 layer `runtime`): framing, priority
//! lanes, traffic shaping. Transport adapters are the ONLY other OS-code home
//! besides `drm-core::shm` (codebase.md 3.1 R8). Bodies land W2+.
//! REQ: REQ-ROUTE-001 (imported topics are never re-exported back over the ingress link).

#![forbid(unsafe_code)]

/// REQ: REQ-ROUTE-001 — placeholder module; framing lands W2 (router-core.md).
pub mod framing {
    /// REQ: REQ-ROUTE-001 — placeholder type for the W1 skeleton build.
    #[derive(Debug)]
    pub struct FramePlaceholder;
}

#[cfg(test)]
mod tests {
    /// REQ: REQ-ROUTE-001 — placeholder compile smoke for the W1 skeleton.
    #[test]
    fn skeleton_compiles() {
        let _ = super::framing::FramePlaceholder;
    }
}
