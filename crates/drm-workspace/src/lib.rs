//! Toolchain crate (codebase.md 2.1 layer `toolchain`): parser, `WorkspaceModel`,
//! verifier, codegen, topology. Never depends on drm-core — toolchain and runtime
//! evolve independently (codebase.md 2.2 R3). Bodies land W2+.
//! REQ: REQ-TOOL-013 (manifest is the single source of truth).

#![forbid(unsafe_code)]

/// REQ: REQ-TOOL-013 — placeholder module; parser lands W2 (schema-parser.md).
pub mod model {
    /// REQ: REQ-TOOL-013 — placeholder type for the W1 skeleton build.
    #[derive(Debug)]
    pub struct WorkspaceModelPlaceholder;
}

#[cfg(test)]
mod tests {
    /// REQ: REQ-TOOL-013 — placeholder compile smoke for the W1 skeleton.
    #[test]
    fn skeleton_compiles() {
        let _ = super::model::WorkspaceModelPlaceholder;
    }
}
