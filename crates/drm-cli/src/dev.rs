//! `drm dev` — watch skeleton: an initial build followed by a debounce-polled
//! loop that classifies changes into the two data categories, reloads the model
//! before dispatch (I-BO5), retains pending dirty sources across invalid
//! declaration batches (I-BO6), and invokes only the owning node's adapter on a
//! source change (I-BO2). A native event watcher replaces the poller without
//! changing the dispatch core (`drm_workspace::watch`). REQ: REQ-TOOL-016,
//! REQ-TOOL-014, REQ-TOOL-015, REQ-TOOL-011.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use drm_workspace::model::{ModelError, NodeName, WorkspaceModel};
use drm_workspace::orchestrator::{self, AdapterRunner, SpawnRunner};
use drm_workspace::watch::{self, WatchControl, WatchEvent, WatchState};

use crate::error::CliError;
use crate::{build, ws};

/// Runs `drm dev` against `root`; `once` stops after the initial build.
/// REQ: REQ-TOOL-016.
///
/// # Errors
/// [`CliError`] from the initial build or any subsequent declaration reload.
pub fn run(root: &Path, once: bool) -> Result<(), CliError> {
    build::run(root, &[], false)?;
    if once {
        return Ok(());
    }

    let mut state = load_state(root)?;
    let mut baseline = snapshot(root);
    let mut runner = SpawnRunner;
    loop {
        std::thread::sleep(Duration::from_millis(watch::DEBOUNCE_MS));
        let next = snapshot(root);
        let changed = diff(&baseline, &next);
        baseline = next;
        if changed.is_empty() {
            continue;
        }
        let mut reload = || -> Result<WorkspaceModel, Vec<ModelError>> {
            ws::discover(root)
                .map_err(|_| Vec::new())
                .and_then(|discovered| ws::build_model(&discovered))
        };
        let events = watch::apply_batch(&mut state, &changed, WatchControl::Normal, &mut reload);
        dispatch(root, &mut state, &events, &mut runner)?;
    }
}

/// Builds the initial watch state (model + path-only source index).
/// REQ: REQ-TOOL-016.
///
/// # Errors
/// [`CliError::Io`] on discovery failure, [`CliError::Manifest`] on a
/// structurally invalid workspace.
fn load_state(root: &Path) -> Result<WatchState, CliError> {
    let discovered = ws::discover(root)?;
    let model = ws::build_model(&discovered).map_err(|errs| {
        CliError::Manifest(format!("workspace model invalid ({} error(s))", errs.len()))
    })?;
    let node_roots = discovered
        .nodes
        .iter()
        .filter_map(|(node, manifest)| {
            manifest
                .parent()
                .map(|dir| (node.clone(), dir.to_path_buf()))
        })
        .collect();
    Ok(WatchState {
        model,
        node_roots,
        pending_source: BTreeSet::new(),
        pending_rescan: false,
    })
}

/// Dispatches watch events: declaration changes regenerate artifacts and rebuild
/// affected nodes; source changes invoke only the owning node's adapter (I-BO2).
/// REQ: REQ-TOOL-016, REQ-TOOL-014.
///
/// # Errors
/// [`CliError`] when a declaration rebuild or adapter invocation fails.
fn dispatch(
    root: &Path,
    state: &mut WatchState,
    events: &[WatchEvent],
    runner: &mut dyn AdapterRunner,
) -> Result<(), CliError> {
    let mut declaration = false;
    for event in events {
        match event {
            WatchEvent::DrmDeclarationChanged(paths) => {
                println!("dev: declaration change ({} path(s))", paths.len());
                declaration = true;
            }
            WatchEvent::AppSourceChanged { node, path } => {
                invoke_source(root, state, node, path, runner)?;
            }
            WatchEvent::DeclarationError(errs) => {
                println!(
                    "dev: declaration error ({}); pending dirty retained",
                    errs.len()
                );
            }
            WatchEvent::RescanRequired => {
                println!("dev: rescan required (watch-queue loss)");
                declaration = true;
            }
        }
    }
    if declaration {
        build::run(root, &[], false)?;
        *state = load_state(root)?;
    }
    Ok(())
}

/// Invokes the adapter of the node owning `path` exactly once (I-BO2).
/// REQ: REQ-TOOL-014.
///
/// # Errors
/// [`CliError::Io`] when the node has no adapter or the adapter fails.
fn invoke_source(
    root: &Path,
    state: &WatchState,
    node: &NodeName,
    path: &Path,
    runner: &mut dyn AdapterRunner,
) -> Result<(), CliError> {
    let Some(entry) = state.model.nodes.get(node) else {
        return Ok(());
    };
    let invocation = orchestrator::invocation_for(node, entry, root)
        .map_err(|e| CliError::Io(format!("{e}")))?;
    println!("dev: source change {} -> adapter `{node}`", path.display());
    runner
        .invoke(&invocation)
        .map_err(|s| CliError::Io(format!("adapter for node `{node}`: {s:?}")))
}

/// Snapshot of `(path, mtime)` for every declaration/source file under `root`.
/// Unreadable entries are skipped (best-effort poller). REQ: REQ-TOOL-015.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if watch::classify(&path).is_some()
                && let Ok(meta) = path.metadata()
                && let Ok(mtime) = meta.modified()
            {
                out.insert(path, mtime);
            }
        }
    }
    out
}

/// Changed paths between two snapshots, sorted (final state, I-BO4).
/// REQ: REQ-TOOL-015.
#[must_use]
fn diff(old: &BTreeMap<PathBuf, SystemTime>, new: &BTreeMap<PathBuf, SystemTime>) -> Vec<PathBuf> {
    let mut changed: Vec<PathBuf> = new
        .iter()
        .filter(|(path, mtime)| old.get(*path) != Some(*mtime))
        .map(|(path, _)| path.clone())
        .collect();
    for path in old.keys() {
        if !new.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed.sort();
    changed
}
