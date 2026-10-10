//! Workspace discovery + model build + verify-lite re-run. The `drm` facade
//! owns the filesystem walk (a facade responsibility); the model itself is
//! built and validated by [`drm_workspace`] (REQ-STUDIO-003: every mutation is
//! executed and validated by the Rust engine).
//!
//! Workspace layout (architecture.md §Workspace Layout and Node Declarations):
//!
//! ```text
//! <root>/
//!   drm.toml                          # optional topology manifest
//!   packages/<pkg>/topics/<stem>.drm  # topic <pkg>/<stem>
//!   packages/<pkg>/nodes/<node>/drm.toml
//! ```
//!
//! REQ: REQ-TOOL-045 (deterministic in-memory model), REQ-TOOL-047 (node
//! publish/consume contracts), REQ-TOOL-013 (manifest is the single source of
//! truth).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use drm_workspace::model::{ModelError, PackageInput, TreeInput, WorkspaceModel, build};
use drm_workspace::verify::{VerifyLiteFinding, verify_lite};

use crate::error::CliError;

/// A discovered workspace: the model inputs plus the on-disk indexes the
/// `assign` command needs. REQ: REQ-TOOL-047.
#[derive(Debug, Clone, Default)]
pub struct Discovered {
    /// Per-package topic inputs, sorted by package then stem (I-D1).
    pub packages: Vec<PackageInput>,
    /// Node directory name → manifest path (`packages/*/nodes/<node>/drm.toml`).
    pub nodes: BTreeMap<String, PathBuf>,
    /// Node manifest inputs `(node dir stem, bytes)`.
    pub node_inputs: Vec<(String, Vec<u8>)>,
    /// Every declared canonical topic path.
    pub topics: BTreeSet<String>,
    /// Root topology manifest bytes, when `<root>/drm.toml` exists.
    pub topology: Option<Vec<u8>>,
}

impl Discovered {
    /// Builds the [`TreeInput`] for a model rebuild from this view.
    /// REQ: REQ-TOOL-045.
    #[must_use]
    pub fn tree_input(&self) -> TreeInput {
        TreeInput {
            packages: self.packages.clone(),
            nodes: self.node_inputs.clone(),
            topology: self.topology.clone(),
        }
    }
}

/// Walks `root` into a [`Discovered`] view.
/// REQ: REQ-TOOL-045, REQ-TOOL-047.
///
/// # Errors
/// [`CliError::Io`] when `packages/` is missing (friendly "not a DRM
/// workspace" message) or an entry is unreadable; any IO failure names the path.
pub fn discover(root: &Path) -> Result<Discovered, CliError> {
    let packages_dir = root.join("packages");
    if !packages_dir.is_dir() {
        return Err(CliError::Io(format!(
            "`{}` is not a DRM workspace: missing `packages/` directory",
            root.display()
        )));
    }

    let mut out = Discovered::default();
    for pkg in subdirs(&packages_dir)? {
        let pkg_name = file_name(&pkg)?;
        let mut topics: Vec<(String, Vec<u8>)> = Vec::new();
        for topic_file in files_with_ext(&pkg.join("topics"), "drm")? {
            let stem = stem(&topic_file)?;
            let bytes = fs::read(&topic_file).map_err(|e| {
                CliError::Io(format!("cannot read `{}` ({e})", topic_file.display()))
            })?;
            out.topics.insert(format!("{pkg_name}/{stem}"));
            topics.push((stem, bytes));
        }
        topics.sort_by(|a, b| a.0.cmp(&b.0));
        out.packages.push(PackageInput {
            name: pkg_name,
            topics,
        });
    }
    out.packages.sort_by(|a, b| a.name.cmp(&b.name));

    let mut node_inputs: Vec<(String, Vec<u8>)> = Vec::new();
    for pkg in subdirs(&packages_dir)? {
        for node_dir in subdirs(&pkg.join("nodes"))? {
            let manifest = node_dir.join("drm.toml");
            if !manifest.is_file() {
                continue;
            }
            let node = file_name(&node_dir)?;
            let bytes = fs::read(&manifest)
                .map_err(|e| CliError::Io(format!("cannot read `{}` ({e})", manifest.display())))?;
            out.nodes.insert(node.clone(), manifest);
            node_inputs.push((node, bytes));
        }
    }
    node_inputs.sort_by(|a, b| a.0.cmp(&b.0));
    out.node_inputs = node_inputs;

    let topology = root.join("drm.toml");
    if topology.is_file() {
        out.topology =
            Some(fs::read(&topology).map_err(|e| {
                CliError::Io(format!("cannot read `{}` ({e})", topology.display()))
            })?);
    }
    Ok(out)
}

/// Builds the [`WorkspaceModel`] for a discovered view.
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// The collected [`ModelError`]s when the tree is structurally invalid.
pub fn build_model(discovered: &Discovered) -> Result<WorkspaceModel, Vec<ModelError>> {
    build(&discovered.tree_input())
}

/// Re-runs the V-SW verify-lite checks over a discovered workspace.
/// REQ: REQ-TOOL-047, REQ-TOOL-036.
///
/// # Errors
/// The collected [`ModelError`]s when the tree cannot even be built.
pub fn verify(discovered: &Discovered) -> Result<Vec<VerifyLiteFinding>, Vec<ModelError>> {
    let model = build_model(discovered)?;
    Ok(verify_lite(&model))
}

/// Immediate subdirectories of `dir`, sorted by path (empty when absent).
/// REQ: REQ-TOOL-045.
fn subdirs(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = read_entries(dir)?;
    entries.retain(|p| p.is_dir());
    Ok(entries)
}

/// Files in `dir` whose extension equals `ext`, sorted by path (empty when the
/// directory is absent). REQ: REQ-TOOL-045.
fn files_with_ext(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, CliError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = read_entries(dir)?;
    entries.retain(|p| p.is_file() && p.extension().and_then(std::ffi::OsStr::to_str) == Some(ext));
    Ok(entries)
}

/// Sorted entries of `dir`. REQ: REQ-TOOL-045.
fn read_entries(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    let rd = fs::read_dir(dir)
        .map_err(|e| CliError::Io(format!("cannot read `{}` ({e})", dir.display())))?;
    let mut out = Vec::new();
    for entry in rd {
        let entry =
            entry.map_err(|e| CliError::Io(format!("cannot read `{}` ({e})", dir.display())))?;
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

/// UTF-8 file name of a path. REQ: REQ-TOOL-045.
fn file_name(path: &Path) -> Result<String, CliError> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_string)
        .ok_or_else(|| CliError::Io(format!("non-UTF-8 path `{}`", path.display())))
}

/// UTF-8 file stem of a path. REQ: REQ-TOOL-045.
fn stem(path: &Path) -> Result<String, CliError> {
    path.file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_string)
        .ok_or_else(|| CliError::Io(format!("non-UTF-8 file stem `{}`", path.display())))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Builds a throwaway workspace tree under the system temp directory.
    fn temp_root(tag: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("drm-cli-ws-{}-{n}-{tag}", std::process::id()));
        std::fs::create_dir_all(root.join("packages/control/topics"))
            .unwrap_or_else(|e| unreachable!("temp dir: {e}"));
        root
    }

    fn write(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).unwrap_or_else(|e| unreachable!("write {path:?}: {e}"));
    }

    /// REQ: REQ-TOOL-047 — discovery maps `packages/<pkg>/topics/<stem>.drm` to
    /// topic `<pkg>/<stem>` and finds node manifests by directory name.
    #[test]
    fn discover_maps_topics_and_nodes() {
        let root = temp_root("discover");
        write(&root.join("packages/control/topics/target.drm"), "v: u32\n");
        let node = root.join("packages/control/nodes/controller");
        std::fs::create_dir_all(&node).unwrap_or_else(|e| unreachable!("mkdir: {e}"));
        write(
            &node.join("drm.toml"),
            "name = \"controller\"\nconsumes = []\n",
        );
        let d = super::discover(&root).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(d.topics.contains("control/target"));
        assert!(d.nodes.contains_key("controller"));
        let model = super::build_model(&d).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert!(model.nodes.contains_key("controller"));
    }

    /// REQ: REQ-TOOL-045 — a missing `packages/` is a friendly, named error.
    #[test]
    fn missing_packages_is_friendly() {
        let root = std::env::temp_dir().join(format!("drm-cli-ws-missing-{}", std::process::id()));
        assert!(super::discover(&root).is_err());
    }
}
