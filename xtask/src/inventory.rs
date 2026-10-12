//! Public API inventory and freeze-prep verification (work-plan M24W1-E2-T1,
//! [c-abi-surface.md](../../docs/algorithms/bindings/c-abi-surface.md)
//! §Public API Inventory & Freeze). Enumerates the externally-visible surface —
//! the canonical C ABI (`drm.h`), the opaque handles, the status taxonomy, and
//! the public Rust items of the runtime/toolchain crates — and re-resolves each
//! entry against its source so a renamed or removed symbol is a gate failure,
//! never silent drift. Post-v1.0 surface is marked deferred and excluded from
//! the freeze set.
//!
//! REQ: REQ-QAL-021 (public API inventory + freeze candidates).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// Kind of public surface item.
/// REQ: REQ-QAL-021.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKind {
    /// `extern "C"` export declared in `drm.h`.
    CExport,
    /// Opaque handle type.
    Handle,
    /// Status-taxonomy code mirrored 1:1 to `int32_t`.
    StatusCode,
    /// Public Rust item in a workspace crate.
    RustPublic,
}

impl ApiKind {
    /// Short label used in the rendered report.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::CExport => "c-export",
            Self::Handle => "handle",
            Self::StatusCode => "status-code",
            Self::RustPublic => "rust-public",
        }
    }
}

/// Freeze disposition of a surface item at v1.0.
/// REQ: REQ-QAL-021.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Stable enough to lock for v1.0 (append-only, never renamed, I-A8).
    Freeze,
    /// Specified but scheduled after v1.0; excluded from the freeze set.
    Deferred,
}

/// One public API surface entry.
/// REQ: REQ-QAL-021.
pub struct ApiEntry {
    /// Symbol or item name as it appears in its owner.
    pub symbol: &'static str,
    /// Surface kind.
    pub kind: ApiKind,
    /// Owner artifact (repository-relative path) that must contain `symbol`.
    pub owner: &'static str,
    /// Registry anchor.
    pub req: &'static str,
    /// Freeze disposition.
    pub disposition: Disposition,
}

/// The canonical C ABI header (single source of truth, ADR 0002).
/// REQ: REQ-QAL-021.
pub const HEADER: &str = "bindings/c/include/drm/drm.h";

/// The C-ABI algorithm doc, owner of the deferred surface.
/// REQ: REQ-QAL-021.
pub const C_ABI_DOC: &str = "docs/algorithms/bindings/c-abi-surface.md";

/// Convenience constructor for the inventory table.
/// REQ: REQ-QAL-021.
const fn api(
    symbol: &'static str,
    kind: ApiKind,
    owner: &'static str,
    req: &'static str,
    disposition: Disposition,
) -> ApiEntry {
    ApiEntry {
        symbol,
        kind,
        owner,
        req,
        disposition,
    }
}

/// The declared public surface. Freeze candidates are the v1.0 block; deferred
/// entries are specified for after v1.0 and are never frozen here.
/// REQ: REQ-QAL-021.
#[rustfmt::skip]
pub const INVENTORY: &[ApiEntry] = &[
    // ---- Canonical C ABI exports (drm.h) ----
    api("drm_abi_version", ApiKind::CExport, HEADER, "REQ-BIND-018", Disposition::Freeze),
    api("drm_context_open", ApiKind::CExport, HEADER, "REQ-BIND-021", Disposition::Freeze),
    api("drm_context_close", ApiKind::CExport, HEADER, "REQ-BIND-002", Disposition::Freeze),
    api("drm_publisher_open", ApiKind::CExport, HEADER, "REQ-BIND-008", Disposition::Freeze),
    api("drm_publisher_close", ApiKind::CExport, HEADER, "REQ-BIND-002", Disposition::Freeze),
    api("drm_consumer_open", ApiKind::CExport, HEADER, "REQ-BIND-010", Disposition::Freeze),
    api("drm_consumer_close", ApiKind::CExport, HEADER, "REQ-BIND-002", Disposition::Freeze),
    api("drm_publish_copy", ApiKind::CExport, HEADER, "REQ-BIND-006", Disposition::Freeze),
    api("drm_read_latest", ApiKind::CExport, HEADER, "REQ-BIND-009", Disposition::Freeze),
    api("drm_read_next", ApiKind::CExport, HEADER, "REQ-BIND-019", Disposition::Freeze),
    api("drm_borrow_latest", ApiKind::CExport, HEADER, "REQ-BIND-001", Disposition::Freeze),
    api("drm_borrow_release", ApiKind::CExport, HEADER, "REQ-BIND-001", Disposition::Freeze),
    // ---- Opaque handles (drm.h) ----
    api("drm_context", ApiKind::Handle, HEADER, "REQ-BIND-004", Disposition::Freeze),
    api("drm_publisher", ApiKind::Handle, HEADER, "REQ-BIND-004", Disposition::Freeze),
    api("drm_consumer", ApiKind::Handle, HEADER, "REQ-BIND-004", Disposition::Freeze),
    // ---- Status taxonomy (drm.h, append-only) ----
    api("DRM_OK", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_INVALID_ARG", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_NOT_FOUND", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_ALREADY_EXISTS", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_PERMISSION", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_QUEUE_FULL", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_GAP", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_STALE", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_WRONG_ABI", ApiKind::StatusCode, HEADER, "REQ-BIND-017", Disposition::Freeze),
    api("DRM_ERR_POOL_EXHAUSTED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_RETRY", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_BUSY", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_NO_SAMPLE", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_INVALID_BORROW", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_GENERATION_EXHAUSTED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_INVALID_DATA", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_INVALID_CONFIG", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_NOT_READY", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_LOCK_FAILED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_SEQUENCE_EXHAUSTED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_EPOCH_EXHAUSTED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_RING_STAMP_EXHAUSTED", ApiKind::StatusCode, HEADER, "REQ-BIND-007", Disposition::Freeze),
    api("DRM_ERR_NOT_IMPLEMENTED", ApiKind::StatusCode, HEADER, "REQ-BIND-017", Disposition::Freeze),
    // ---- Public Rust items ----
    api("pub enum Status", ApiKind::RustPublic, "crates/drm-types/src/lib.rs", "REQ-CORE-024", Disposition::Freeze),
    api("pub struct Channel", ApiKind::RustPublic, "crates/drm-core/src/channel/mod.rs", "REQ-CORE-021", Disposition::Freeze),
    api("pub struct HistoryRing", ApiKind::RustPublic, "crates/drm-core/src/ring/mod.rs", "REQ-CORE-066", Disposition::Freeze),
    api("pub enum OverflowPolicy", ApiKind::RustPublic, "crates/drm-core/src/channel/overflow.rs", "REQ-CORE-039", Disposition::Freeze),
    api("pub fn acquire", ApiKind::RustPublic, "crates/drm-core/src/hazard/mod.rs", "REQ-CORE-030", Disposition::Freeze),
    api("pub trait TransportAdapter", ApiKind::RustPublic, "crates/drm-router/src/transport/mod.rs", "REQ-ROUTE-009", Disposition::Freeze),
    api("pub struct WorkspaceModel", ApiKind::RustPublic, "crates/drm-workspace/src/model/mod.rs", "REQ-TOOL-003", Disposition::Freeze),
    // ---- Deferred (post-v1.0): specified, not yet implemented ----
    api("drm_write_acquire", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-019", Disposition::Deferred),
    api("drm_write_commit", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-019", Disposition::Deferred),
    api("drm_write_abort", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-019", Disposition::Deferred),
    api("drm_copy_latest", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-009", Disposition::Deferred),
    api("drm_copy_next", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-019", Disposition::Deferred),
    api("drm_borrow_next", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-001", Disposition::Deferred),
    api("drm_log", ApiKind::CExport, C_ABI_DOC, "REQ-BIND-023", Disposition::Deferred),
];

/// Resolution outcome for one inventory entry.
/// REQ: REQ-QAL-021.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryOutcome {
    /// Symbol / item name.
    pub symbol: String,
    /// Surface kind.
    pub kind: ApiKind,
    /// Freeze disposition.
    pub disposition: Disposition,
    /// Registry anchor.
    pub req: &'static str,
    /// Whether the owner artifact contains the symbol.
    pub resolved: bool,
}

/// Inventory verification report.
/// REQ: REQ-QAL-021.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryReport {
    /// One outcome per declared entry.
    pub entries: Vec<EntryOutcome>,
}

impl InventoryReport {
    /// Total declared entries.
    #[must_use]
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// Entries whose owner contains the symbol.
    #[must_use]
    pub fn resolved(&self) -> usize {
        self.entries.iter().filter(|e| e.resolved).count()
    }

    /// Gate: every declared entry resolves against its owner.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.entries.iter().all(|e| e.resolved)
    }

    /// Renders the report (unresolved entries plus the summary).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            if !entry.resolved {
                let _ = writeln!(
                    out,
                    "inventory | UNRESOLVED | {} | {} | {}",
                    entry.kind.label(),
                    entry.symbol,
                    entry.req
                );
            }
        }
        let _ = writeln!(
            out,
            "API_INVENTORY = {}/{} resolved ({} freeze candidates, {} deferred)",
            self.resolved(),
            self.total(),
            freeze_candidates().len(),
            deferred().len()
        );
        out
    }
}

/// Verifies every declared inventory entry against its owner artifact under
/// `root`. Freeze entries are checked against their implementation source;
/// deferred entries against the C-ABI algorithm doc that specifies them.
/// REQ: REQ-QAL-021.
///
/// # Errors
/// [`anyhow::Error`] when an owner artifact cannot be read.
pub fn verify(root: &Path) -> Result<InventoryReport> {
    let mut entries = Vec::with_capacity(INVENTORY.len());
    for entry in INVENTORY {
        let owner = root.join(entry.owner);
        let text =
            std::fs::read_to_string(&owner).with_context(|| format!("read {}", owner.display()))?;
        entries.push(EntryOutcome {
            symbol: entry.symbol.to_string(),
            kind: entry.kind,
            disposition: entry.disposition,
            req: entry.req,
            resolved: text.contains(entry.symbol),
        });
    }
    Ok(InventoryReport { entries })
}

/// Freeze-candidate symbols (the v1.0 block), in declaration order.
/// REQ: REQ-QAL-021.
#[must_use]
pub fn freeze_candidates() -> Vec<&'static str> {
    INVENTORY
        .iter()
        .filter(|e| e.disposition == Disposition::Freeze)
        .map(|e| e.symbol)
        .collect()
}

/// Deferred (post-v1.0) symbols, in declaration order.
/// REQ: REQ-QAL-021.
#[must_use]
pub fn deferred() -> Vec<&'static str> {
    INVENTORY
        .iter()
        .filter(|e| e.disposition == Disposition::Deferred)
        .map(|e| e.symbol)
        .collect()
}

/// REQ: REQ-QAL-021 — inventory self-tests over the live repository.
#[cfg(test)]
mod tests {
    use super::{C_ABI_DOC, Disposition, HEADER, INVENTORY, deferred, freeze_candidates, verify};

    /// Repository root fixture.
    /// REQ: REQ-QAL-021.
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            )
    }

    /// REQ: REQ-QAL-021 — the live repository resolves every inventory entry
    /// (`API_INVENTORY = 1`), and the freeze/deferred split is non-empty.
    #[test]
    fn repository_inventory_resolves() {
        let report = verify(&repo_root()).unwrap_or_else(|e| unreachable!("inventory: {e}"));
        assert!(
            report.is_complete(),
            "unresolved: {:?}",
            report
                .entries
                .iter()
                .filter(|e| !e.resolved)
                .map(|e| e.symbol.clone())
                .collect::<Vec<_>>()
        );
        assert!(!freeze_candidates().is_empty(), "freeze list non-empty");
        assert!(!deferred().is_empty(), "deferred list non-empty");
        assert_eq!(freeze_candidates().len() + deferred().len(), report.total());
    }

    /// REQ: REQ-QAL-021 — the documented inventory artifacts exist and the
    /// header/doc anchors are the declared paths.
    #[test]
    fn inventory_anchors_exist() {
        let root = repo_root();
        assert!(root.join(HEADER).is_file(), "drm.h present");
        assert!(root.join(C_ABI_DOC).is_file(), "c-abi-surface doc present");
        assert!(INVENTORY.iter().all(
            |e| e.disposition == Disposition::Freeze || e.disposition == Disposition::Deferred
        ));
    }

    /// REQ: REQ-QAL-021 — freeze candidates and deferred lists partition the
    /// inventory with no symbol in both.
    #[test]
    fn freeze_and_deferred_are_disjoint() {
        let frozen = freeze_candidates();
        let deferred = deferred();
        assert_eq!(frozen.len() + deferred.len(), INVENTORY.len());
        for symbol in &frozen {
            assert!(!deferred.contains(symbol), "{symbol} in both sets");
        }
    }

    /// REQ: REQ-QAL-021 — the documented inventory lists every declared symbol,
    /// so code and reviewable doc cannot drift apart.
    #[test]
    fn documented_inventory_covers_all_symbols() {
        let doc = repo_root().join("docs/spec/api-inventory.md");
        let text = std::fs::read_to_string(&doc).unwrap_or_else(|e| unreachable!("read doc: {e}"));
        for entry in INVENTORY {
            assert!(
                text.contains(entry.symbol),
                "api-inventory.md missing `{}`",
                entry.symbol
            );
        }
    }
}
