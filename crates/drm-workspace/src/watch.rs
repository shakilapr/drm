//! `drm dev` watch skeleton ([build-orchestration.md](../../docs/algorithms/toolchain/build-orchestration.md)):
//! two data event categories (DRM declaration / application source) plus the
//! `DeclarationError` and `RescanRequired` control signals, debounce-coalesced
//! batches, reload-before-dispatch (I-BO5), pending-dirty retention across
//! invalid declaration batches (I-BO6), event-loss rescan recovery (I-BO7), and
//! path-only source ownership (I-BO8). Watch events are invalidation hints, not
//! truth. REQ: REQ-TOOL-016, REQ-TOOL-066, REQ-TOOL-067, REQ-TOOL-014,
//! REQ-TOOL-015.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::model::{ModelError, NodeName, WorkspaceModel};

/// Debounce window: coalescing only, never a correctness mechanism (I-BO4).
/// REQ: REQ-TOOL-015.
pub const DEBOUNCE_MS: u64 = 200;

/// A watched event dispatched to the driver. REQ: REQ-TOOL-016.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Declaration batch (all declaration paths), after the model reloaded.
    DrmDeclarationChanged(Vec<PathBuf>),
    /// An application source file changed; `node` owns it by path (I-BO8).
    AppSourceChanged {
        /// Owning node.
        node: NodeName,
        /// Changed source path.
        path: PathBuf,
    },
    /// The batch's declarations failed to reload; the last valid model is kept
    /// and the batch's source events are retained as pending (I-BO6).
    DeclarationError(Vec<ModelError>),
    /// Watch-queue loss/error: reconstruct state by rescan (I-BO7).
    RescanRequired,
}

/// The two data event categories. REQ: REQ-TOOL-016.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// A DRM declaration (`.drm` schema or `drm.toml` manifest/topology).
    Declaration,
    /// An application source file owned by a node.
    Source,
}

/// Classifies a path by name/extension only — file content is never decoded
/// (I-BO8). `None` for paths in neither category. REQ: REQ-TOOL-016.
#[must_use]
pub fn classify(path: &Path) -> Option<EventClass> {
    if path.file_name().and_then(std::ffi::OsStr::to_str) == Some("drm.toml")
        || path.extension().and_then(std::ffi::OsStr::to_str) == Some("drm")
    {
        return Some(EventClass::Declaration);
    }
    let ext = path.extension().and_then(std::ffi::OsStr::to_str)?;
    if matches!(
        ext,
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "hxx" | "rs" | "py" | "pyi" | "go"
    ) {
        Some(EventClass::Source)
    } else {
        None
    }
}

/// Control signal accompanying a batch: normal events or watch-queue loss.
/// REQ: REQ-TOOL-067.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchControl {
    /// A debounced batch of filesystem events.
    Normal,
    /// Watcher overflow/error: event loss detected (I-BO7).
    Overflow,
}

/// Live watch state: the last valid model, the path-only source index, pending
/// dirty source paths (I-BO6), and the loss flag (I-BO7). REQ: REQ-TOOL-016,
/// REQ-TOOL-066, REQ-TOOL-067.
#[derive(Debug, Clone)]
pub struct WatchState {
    /// Last successfully loaded model (never stale for dispatch, I-BO5).
    pub model: WorkspaceModel,
    /// Node → source root directory (path-only ownership, I-BO8).
    pub node_roots: BTreeMap<NodeName, PathBuf>,
    /// Source paths awaiting a valid model (I-BO6).
    pub pending_source: BTreeSet<PathBuf>,
    /// Loss detected; rescan before trusting further events (I-BO7).
    pub pending_rescan: bool,
}

/// Owner of `path` by directory containment against `node_roots`; the node must
/// also exist in `model` (I-BO8, path only — content never read).
/// REQ: REQ-TOOL-014.
#[must_use]
pub fn owner_of(
    path: &Path,
    model: &WorkspaceModel,
    node_roots: &BTreeMap<NodeName, PathBuf>,
) -> Option<NodeName> {
    node_roots.iter().find_map(|(node, root)| {
        (path.starts_with(root) && model.nodes.contains_key(node)).then(|| node.clone())
    })
}

/// Every owned source root (conservative dirty set after a rescan, I-BO7).
/// REQ: REQ-TOOL-067.
#[must_use]
pub fn all_source_paths(state: &WatchState) -> BTreeSet<PathBuf> {
    state.node_roots.values().cloned().collect()
}

/// Nodes affected by a declaration change: nodes whose interface changed, and
/// nodes publishing/consuming a topic whose schema changed (I-BO3). Removed
/// nodes are included so their adapters stop. REQ: REQ-TOOL-016.
#[must_use]
pub fn affected_nodes(old: &WorkspaceModel, new: &WorkspaceModel) -> BTreeSet<NodeName> {
    let mut out = BTreeSet::new();
    for (name, entry) in &new.nodes {
        match old.nodes.get(name) {
            Some(previous) if previous == entry => {}
            _ => {
                out.insert(name.clone());
            }
        }
    }
    for name in old.nodes.keys() {
        if !new.nodes.contains_key(name) {
            out.insert(name.clone());
        }
    }
    for (topic, entry) in &new.topics {
        let changed = old
            .topics
            .get(topic)
            .is_none_or(|previous| previous.schema_hash != entry.schema_hash);
        if changed {
            for (name, node) in &new.nodes {
                if node.publishes.contains(topic) || node.consumes.contains(topic) {
                    out.insert(name.clone());
                }
            }
        }
    }
    out
}

/// Applies one debounced batch: on loss, emit `RescanRequired` and flag a rescan
/// (I-BO7); otherwise recover if flagged, reload declarations **before** mapping
/// source events (I-BO5), and retain source events as pending when the reload
/// fails (I-BO6). `reload` is the total-reload model rebuild.
/// REQ: REQ-TOOL-016, REQ-TOOL-066, REQ-TOOL-067.
pub fn apply_batch(
    state: &mut WatchState,
    batch: &[PathBuf],
    control: WatchControl,
    reload: &mut dyn FnMut() -> Result<WorkspaceModel, Vec<ModelError>>,
) -> Vec<WatchEvent> {
    let mut events = Vec::new();

    if control == WatchControl::Overflow {
        state.pending_rescan = true;
        events.push(WatchEvent::RescanRequired);
        return events;
    }

    if state.pending_rescan {
        match reload() {
            Ok(next) => {
                state.model = next;
                state.pending_source = all_source_paths(state);
                state.pending_rescan = false;
            }
            Err(errs) => events.push(WatchEvent::DeclarationError(errs)),
        }
    }

    let declarations: Vec<PathBuf> = batch
        .iter()
        .filter(|path| classify(path) == Some(EventClass::Declaration))
        .cloned()
        .collect();

    let mut reload_failed = false;
    if !declarations.is_empty() {
        match reload() {
            Ok(next) => {
                state.model = next;
                events.push(WatchEvent::DrmDeclarationChanged(declarations));
            }
            Err(errs) => {
                events.push(WatchEvent::DeclarationError(errs));
                reload_failed = true;
            }
        }
    }

    let mut sources: BTreeSet<PathBuf> = batch
        .iter()
        .filter(|path| classify(path) == Some(EventClass::Source))
        .cloned()
        .collect();
    sources.extend(std::mem::take(&mut state.pending_source));

    for path in sources {
        if reload_failed {
            // I-BO6: retain; dispatch only after a valid reload.
            state.pending_source.insert(path);
            continue;
        }
        if let Some(node) = owner_of(&path, &state.model, &state.node_roots) {
            events.push(WatchEvent::AppSourceChanged { node, path });
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{EventClass, classify};

    /// REQ: REQ-TOOL-016 — declarations and sources classify by name/extension;
    /// an unknown extension is neither category.
    #[test]
    fn classify_two_categories() {
        assert_eq!(
            classify(&PathBuf::from("a/b.drm")),
            Some(EventClass::Declaration)
        );
        assert_eq!(
            classify(&PathBuf::from("pkg/nodes/n/drm.toml")),
            Some(EventClass::Declaration)
        );
        assert_eq!(
            classify(&PathBuf::from("src/main.cpp")),
            Some(EventClass::Source)
        );
        assert_eq!(classify(&PathBuf::from("src/blob.bin")), None);
    }
}
