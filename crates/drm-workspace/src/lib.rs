//! Toolchain crate (codebase.md 2.1 layer `toolchain`): parser, `WorkspaceModel`,
//! verifier, codegen, topology. Never depends on drm-core — toolchain and runtime
//! evolve independently (codebase.md 2.2 R3).
//! REQ: REQ-TOOL-013 (manifest is the single source of truth).

#![forbid(unsafe_code)]

pub mod canon;
pub mod model;
pub mod parser;
pub mod topology;

#[cfg(test)]
mod tests {
    /// REQ: REQ-TOOL-045 — empty tree builds an empty model (tools still run).
    #[test]
    fn empty_tree_builds() {
        let tree = super::model::TreeInput::default();
        let m = super::model::build(&tree).unwrap_or_else(|e| unreachable!("empty tree: {e:?}"));
        assert_eq!(m.topics.len(), 0);
        assert_eq!(m.packages.len(), 0);
    }
}
