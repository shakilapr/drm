//! Build orchestration ([build-orchestration.md](../../docs/algorithms/toolchain/build-orchestration.md)):
//! `drm build` as a high-level orchestrator — parse → `WorkspaceModel` → handle
//! assignment → deployment artifacts → codegen → `drm verify` → native adapter
//! invocation (I-BO1). DRM never parses application sources; the native adapter
//! decides object freshness (I-BO2). Adapters receive canonical include paths
//! and the deployment-metadata path through environment hooks so they never
//! hard-code the `.drm` layout. `drm info` is a pure, read-only projection of
//! the model plus the verify summary. REQ: REQ-TOOL-012, REQ-TOOL-011,
//! REQ-TOOL-014, REQ-TOOL-045, REQ-TOOL-047.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use drm_types::Status;

use crate::codegen;
use crate::deploy;
use crate::model::handles::HandleTable;
use crate::model::{self, ModelError, NodeEntry, NodeName, TreeInput, WorkspaceModel};
use crate::verify::{VerifyLiteFinding, verify_lite};

/// Generation root under the workspace (`.drm/gen`). REQ: REQ-TOOL-029.
pub const GEN_DIR_REL: &str = ".drm/gen";
/// Deployment-artifact root under the workspace (`.drm/deploy`).
/// REQ: REQ-TOOL-012.
pub const DEPLOY_DIR_REL: &str = ".drm/deploy";
/// Environment variable carrying the `:`-joined generated include paths.
/// REQ: REQ-TOOL-011.
pub const GEN_INCLUDE_ENV: &str = "DRM_GEN_INCLUDE";
/// Environment variable carrying the deployment-metadata path.
/// REQ: REQ-TOOL-011.
pub const DEPLOY_METADATA_ENV: &str = "DRM_DEPLOY_METADATA";
/// Path-list separator for [`GEN_INCLUDE_ENV`] (platform native).
/// REQ: REQ-TOOL-011.
pub const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// Declared node implementation language. REQ: REQ-TOOL-047.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// C (C11).
    C,
    /// C++ (C++20).
    Cpp,
    /// Rust.
    Rust,
    /// Python.
    Python,
}

impl Language {
    /// Parses a manifest language tag (`c`, `cpp`/`c++`, `rust`/`rs`,
    /// `python`/`py`). REQ: REQ-TOOL-047.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "c" => Some(Self::C),
            "cpp" | "c++" => Some(Self::Cpp),
            "rust" | "rs" => Some(Self::Rust),
            "python" | "py" => Some(Self::Python),
            _ => None,
        }
    }

    /// Canonical manifest tag. REQ: REQ-TOOL-047.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }

    /// Generated-bindings subdirectory under `.drm/gen/` (matches
    /// [`crate::codegen::Lang`]). REQ: REQ-TOOL-029.
    #[must_use]
    pub const fn gen_dir(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

/// Native build adapter declared in a node manifest `[build]` table
/// (architecture.md §DRM as Orchestrator). REQ: REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildAdapter {
    /// CMake/Ninja build of `dir` targeting `target`.
    Cmake {
        /// Build directory relative to the workspace root.
        dir: PathBuf,
        /// `CMake` target name.
        target: String,
    },
    /// Cargo build of `package`.
    Cargo {
        /// Cargo package name.
        package: String,
    },
    /// Explicit custom command (`[build] command = [...]`), verbatim.
    Command(Vec<String>),
}

/// A `drm build` request: workspace root plus an optional target subset.
/// REQ: REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// Workspace root directory.
    pub workspace: PathBuf,
    /// Optional node subset; `None` builds every declared node.
    pub targets: Option<BTreeSet<NodeName>>,
}

/// One orchestration step, emitted in order (I-BO1 sequence probe).
/// REQ: REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildStep {
    /// Declaration discovery from the filesystem.
    Parse,
    /// `WorkspaceModel` construction.
    Model,
    /// Stable handle assignment.
    Handles,
    /// Deployment-artifact generation (handle tables).
    Deploy,
    /// Codegen plan + execute.
    Codegen,
    /// `drm verify`.
    Verify,
    /// Native adapter invocation for `node`.
    Adapter(NodeName),
}

/// Outcome of a successful `drm build`. REQ: REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    /// Number of expected generated outputs.
    pub generated: usize,
    /// Number of units rendered this run.
    pub dirty: usize,
    /// Number of topics covered by the verify pass.
    pub verified: usize,
    /// Nodes whose adapter ran, sorted.
    pub built: Vec<NodeName>,
    /// Handle table after assignment (for lockfile persistence).
    pub handles: HandleTable,
}

/// Build failure. REQ: REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// Declaration discovery or filesystem failure.
    Io(String),
    /// Structural model failure.
    Model(Vec<ModelError>),
    /// Handle-space exhaustion.
    Handle(Status),
    /// Deployment-artifact write failure.
    Deploy(Status),
    /// Codegen failure.
    Codegen(Status),
    /// `drm verify` reported negatives; adapters are never invoked.
    Verify(Vec<VerifyLiteFinding>),
    /// A native adapter could not be resolved or failed.
    Adapter {
        /// Offending node.
        node: NodeName,
        /// Human description.
        message: String,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "{m}"),
            Self::Model(errs) => write!(f, "workspace model invalid ({} error(s))", errs.len()),
            Self::Handle(s) => write!(f, "handle assignment failed ({s:?})"),
            Self::Deploy(s) => write!(f, "deployment artifact generation failed ({s:?})"),
            Self::Codegen(s) => write!(f, "codegen failed ({s:?})"),
            Self::Verify(findings) => {
                write!(
                    f,
                    "verify failed ({} finding(s)); adapters not invoked",
                    findings.len()
                )
            }
            Self::Adapter { node, message } => write!(f, "adapter for node `{node}`: {message}"),
        }
    }
}

/// A concrete native-adapter invocation with DRM's environment hooks resolved.
/// REQ: REQ-TOOL-011.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInvocation {
    /// Node the adapter builds.
    pub node: NodeName,
    /// Executable name.
    pub program: String,
    /// Argument vector (excluding the program).
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Environment additions (`DRM_GEN_INCLUDE`, `DRM_DEPLOY_METADATA`).
    pub env: Vec<(String, String)>,
}

/// Executes native adapters; injectable so tests never spawn processes.
/// REQ: REQ-TOOL-012.
pub trait AdapterRunner {
    /// Runs one adapter invocation.
    ///
    /// # Errors
    /// A [`Status`] describing the spawn/exit failure.
    fn invoke(&mut self, invocation: &AdapterInvocation) -> Result<(), Status>;
}

/// Real [`AdapterRunner`] that spawns the native build tool.
/// REQ: REQ-TOOL-012.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpawnRunner;

impl AdapterRunner for SpawnRunner {
    fn invoke(&mut self, invocation: &AdapterInvocation) -> Result<(), Status> {
        let status = Command::new(&invocation.program)
            .args(&invocation.args)
            .envs(invocation.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&invocation.cwd)
            .status()
            .map_err(|_| Status::NotFound)?;
        if status.success() {
            Ok(())
        } else {
            Err(Status::InvalidConfig)
        }
    }
}

/// Canonical include paths handed to adapters (`$WS/.drm/gen/c|cpp`).
/// REQ: REQ-TOOL-011.
#[must_use]
pub fn include_paths(workspace: &Path) -> Vec<PathBuf> {
    vec![
        workspace.join(GEN_DIR_REL).join(Language::C.gen_dir()),
        workspace.join(GEN_DIR_REL).join(Language::Cpp.gen_dir()),
    ]
}

/// Deployment-metadata path handed to adapters (`.drm/deploy/deployment.json`).
/// REQ: REQ-TOOL-011.
#[must_use]
pub fn deployment_metadata_path(workspace: &Path) -> PathBuf {
    workspace.join(DEPLOY_DIR_REL).join("deployment.json")
}

/// Environment additions shared by every adapter. REQ: REQ-TOOL-011.
#[must_use]
pub fn adapter_env(workspace: &Path) -> Vec<(String, String)> {
    let includes = include_paths(workspace)
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(&PATH_SEP.to_string());
    vec![
        (GEN_INCLUDE_ENV.to_string(), includes),
        (
            DEPLOY_METADATA_ENV.to_string(),
            deployment_metadata_path(workspace).display().to_string(),
        ),
    ]
}

/// Resolves the native adapter for `node` into a concrete invocation with the
/// DRM environment hooks applied. REQ: REQ-TOOL-011, REQ-TOOL-012.
///
/// # Errors
/// [`BuildError::Adapter`] when the node declares no `[build]` adapter.
pub fn invocation_for(
    node: &str,
    entry: &NodeEntry,
    workspace: &Path,
) -> Result<AdapterInvocation, BuildError> {
    let adapter = entry.build.as_ref().ok_or_else(|| BuildError::Adapter {
        node: node.to_string(),
        message: "no `[build]` adapter configured (REQ-TOOL-012)".to_string(),
    })?;
    // Absolute paths so the adapter's cwd and the hook env agree.
    let workspace = std::path::absolute(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let (program, args) = match adapter {
        BuildAdapter::Cmake { dir, target } => (
            "cmake".to_string(),
            vec![
                "--build".to_string(),
                workspace.join(dir).display().to_string(),
                "--target".to_string(),
                target.clone(),
            ],
        ),
        BuildAdapter::Cargo { package } => (
            "cargo".to_string(),
            vec![
                "build".to_string(),
                "--package".to_string(),
                package.clone(),
            ],
        ),
        BuildAdapter::Command(argv) => {
            let Some((program, rest)) = argv.split_first() else {
                return Err(BuildError::Adapter {
                    node: node.to_string(),
                    message: "`build.command` is empty".to_string(),
                });
            };
            (program.clone(), rest.to_vec())
        }
    };
    Ok(AdapterInvocation {
        node: node.to_string(),
        program,
        args,
        cwd: workspace.clone(),
        env: adapter_env(&workspace),
    })
}

/// Builds the workspace tree input by walking `root` (packages → topics/nodes,
/// plus the root topology manifest). REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] naming the offending path.
pub fn discover_tree(root: &Path) -> Result<TreeInput, BuildError> {
    let packages_dir = root.join("packages");
    let mut packages = Vec::new();
    for pkg in sorted_dirs(&packages_dir)? {
        let name = file_name(&pkg)?;
        let mut topics = Vec::new();
        let topics_dir = pkg.join("topics");
        for file in files_with_ext(&topics_dir, "drm")? {
            let relative = topic_path(&topics_dir, &file)?;
            topics.push((relative, read(&file)?));
        }
        packages.push(model::PackageInput { name, topics });
    }
    let mut nodes = Vec::new();
    for pkg in sorted_dirs(&packages_dir)? {
        for node_dir in sorted_dirs(&pkg.join("nodes"))? {
            let manifest = node_dir.join("drm.toml");
            if manifest.is_file() {
                nodes.push((file_name(&node_dir)?, read(&manifest)?));
            }
        }
    }
    let topology = match std::fs::read(root.join("drm.toml")) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(BuildError::Io(format!(
                "read `{}` ({e})",
                root.join("drm.toml").display()
            )));
        }
    };
    Ok(TreeInput {
        packages,
        nodes,
        topology,
    })
}

/// `drm build` orchestration (I-BO1): parse → model → handles → deploy →
/// codegen → verify → adapters, in that order, no step skipped; a verify
/// failure aborts before any adapter runs. `observe` receives each step in
/// order so callers and tests can assert the sequence. REQ: REQ-TOOL-012,
/// REQ-TOOL-011.
///
/// # Errors
/// [`BuildError`] for any failing step.
#[allow(clippy::too_many_lines)]
pub fn build(
    req: &BuildRequest,
    prior: &HandleTable,
    runner: &mut dyn AdapterRunner,
    observe: &mut dyn FnMut(BuildStep),
) -> Result<BuildReport, BuildError> {
    observe(BuildStep::Parse);
    let tree = discover_tree(&req.workspace)?;

    observe(BuildStep::Model);
    let model = model::build(&tree).map_err(BuildError::Model)?;

    observe(BuildStep::Handles);
    let topics: BTreeSet<String> = model.topics.keys().cloned().collect();
    let (handles, _deltas) = prior.assign(&topics).map_err(BuildError::Handle)?;

    observe(BuildStep::Deploy);
    deploy::generate(&model, &handles, &req.workspace).map_err(BuildError::Deploy)?;

    observe(BuildStep::Codegen);
    let mut db = codegen::FingerprintDb::load(&req.workspace).map_err(BuildError::Codegen)?;
    let gen_plan = codegen::plan(&model, &db, &req.workspace);
    codegen::execute(&gen_plan, &mut db, &req.workspace).map_err(BuildError::Codegen)?;

    observe(BuildStep::Verify);
    let findings = verify_lite(&model);
    if !findings.is_empty() {
        return Err(BuildError::Verify(findings));
    }

    let targets = resolve_targets(req.targets.as_ref(), &model);
    let mut built = Vec::new();
    for node in targets {
        let Some(entry) = model.nodes.get(&node) else {
            return Err(BuildError::Io(format!("unknown target node `{node}`")));
        };
        let invocation = invocation_for(&node, entry, &req.workspace)?;
        observe(BuildStep::Adapter(node.clone()));
        runner
            .invoke(&invocation)
            .map_err(|s| BuildError::Adapter {
                node: node.clone(),
                message: format!("{s:?}"),
            })?;
        built.push(node);
    }

    Ok(BuildReport {
        generated: gen_plan.expected_outputs.len(),
        dirty: gen_plan.dirty_units.len(),
        verified: model.topics.len(),
        built,
        handles,
    })
}

/// Resolves the ordered target node list: the requested subset (validated) or
/// every declared node. REQ: REQ-TOOL-012.
#[must_use]
fn resolve_targets(
    requested: Option<&BTreeSet<NodeName>>,
    model: &WorkspaceModel,
) -> Vec<NodeName> {
    match requested {
        Some(set) => set.iter().cloned().collect(),
        None => model.nodes.keys().cloned().collect(),
    }
}

/// Read-only `drm info` projection: deterministic model render plus the verify
/// summary. No mutation, no codegen, no adapter invocation. REQ: REQ-TOOL-045,
/// REQ-TOOL-047.
#[must_use]
pub fn info(model: &WorkspaceModel, findings: &[VerifyLiteFinding]) -> String {
    let mut out = model::render_model(model);
    out.push_str("verify ");
    if findings.is_empty() {
        out.push_str("green\n");
    } else {
        let _ = writeln!(out, "{} finding(s)", findings.len());
        for finding in findings {
            out.push_str("  ");
            out.push_str(&finding.to_string());
            out.push('\n');
        }
    }
    out
}

/// Sorted immediate subdirectories of `dir` (empty when absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] when an existing directory cannot be read.
fn sorted_dirs(dir: &Path) -> Result<Vec<PathBuf>, BuildError> {
    Ok(entries(dir)?.into_iter().filter(|p| p.is_dir()).collect())
}

/// Sorted files in `dir` with the given extension (empty when absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] when an existing directory cannot be read.
fn files_with_ext(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, BuildError> {
    let mut out = Vec::new();
    collect_with_ext(dir, ext, &mut out)?;
    Ok(out)
}

/// Depth-first collection of `ext` files under `dir`.
///
/// Recursive because a topic path is `<package>/<relative/path>/<stem>`
/// (`architecture.md` §Workspace Layout): a ROS-shaped name such as
/// `robot/sensors/lidar/front` is a directory tree, not one file. The flat walk
/// this replaces silently dropped every nested topic — `drm build` reported
/// `0 generated` and exited 0 while the workspace plainly held files.
/// REQ: REQ-TOOL-045.
fn collect_with_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> Result<(), BuildError> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in entries(dir)? {
        if entry.is_dir() {
            collect_with_ext(&entry, ext, out)?;
        } else if entry.is_file()
            && entry.extension().and_then(std::ffi::OsStr::to_str) == Some(ext)
        {
            out.push(entry);
        }
    }
    Ok(())
}

/// Sorted directory entries (empty when the directory is absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] when an existing directory cannot be read.
fn entries(dir: &Path) -> Result<Vec<PathBuf>, BuildError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let rd = std::fs::read_dir(dir)
        .map_err(|e| BuildError::Io(format!("read `{}` ({e})", dir.display())))?;
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| BuildError::Io(format!("read `{}` ({e})", dir.display())))?;
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

/// Reads a file's bytes. REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] naming the path.
fn read(path: &Path) -> Result<Vec<u8>, BuildError> {
    std::fs::read(path).map_err(|e| BuildError::Io(format!("read `{}` ({e})", path.display())))
}

/// UTF-8 file name of a path. REQ: REQ-TOOL-045.
///
/// # Errors
/// [`BuildError::Io`] for a non-UTF-8 name.
fn file_name(path: &Path) -> Result<String, BuildError> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_string)
        .ok_or_else(|| BuildError::Io(format!("non-UTF-8 path `{}`", path.display())))
}

/// The topic path of `file` relative to its `topics/` directory, without the
/// `.drm` extension, with `/` separators. REQ: REQ-TOOL-045, REQ-TOOL-056.
///
/// `topics/nested/deep/lidar.drm` -> `nested/deep/lidar`: identity keeps the
/// depth the user declared instead of collapsing to the file stem.
///
/// # Errors
/// [`BuildError::Io`] when the path is not under `topics_dir`, contains a
/// non-`Normal` component, or is not UTF-8.
fn topic_path(topics_dir: &Path, file: &Path) -> Result<String, BuildError> {
    let rel = file.strip_prefix(topics_dir).map_err(|_| {
        BuildError::Io(format!(
            "`{}` is not under `{}`",
            file.display(),
            topics_dir.display()
        ))
    })?;
    let mut segments: Vec<String> = Vec::new();
    for component in rel.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(BuildError::Io(format!(
                "unexpected path component in `{}`",
                file.display()
            )));
        };
        let text = part
            .to_str()
            .ok_or_else(|| BuildError::Io(format!("non-UTF-8 path `{}`", file.display())))?;
        segments.push(text.to_string());
    }
    let last = segments
        .pop()
        .ok_or_else(|| BuildError::Io(format!("`{}` has no file name", file.display())))?;
    let stem = last.strip_suffix(".drm").unwrap_or(&last).to_string();
    segments.push(stem);
    Ok(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Language, PATH_SEP, include_paths, invocation_for};
    use crate::model::NodeEntry;
    use crate::orchestrator::BuildAdapter;

    /// REQ: REQ-TOOL-047 — language tags round-trip through their canonical form.
    #[test]
    fn language_tags_round_trip() {
        for tag in ["c", "cpp", "rust", "python"] {
            let l = Language::from_tag(tag).unwrap_or_else(|| unreachable!("{tag}"));
            assert_eq!(l.as_str(), tag);
        }
        assert_eq!(Language::from_tag("c++"), Some(Language::Cpp));
        assert_eq!(Language::from_tag("go"), None);
    }

    /// REQ: REQ-TOOL-011 — include paths are the canonical c/cpp subtrees.
    #[test]
    fn include_paths_are_canonical() {
        let ws = PathBuf::from("/ws");
        let paths = include_paths(&ws);
        assert!(paths[0].ends_with(".drm/gen/c"));
        assert!(paths[1].ends_with(".drm/gen/cpp"));
        assert_eq!(PATH_SEP, if cfg!(windows) { ';' } else { ':' });
    }

    /// REQ: REQ-TOOL-012 — a cargo adapter resolves to `cargo build --package`.
    #[test]
    fn cargo_invocation() {
        let entry = NodeEntry {
            build: Some(BuildAdapter::Cargo {
                package: "driver".to_string(),
            }),
            ..NodeEntry::default()
        };
        let inv = invocation_for("driver", &entry, &PathBuf::from("/ws"))
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(inv.program, "cargo");
        assert_eq!(inv.args, ["build", "--package", "driver"]);
        assert!(inv.env.iter().any(|(k, _)| k == super::GEN_INCLUDE_ENV));
    }

    /// REQ: REQ-TOOL-012 — a node without `[build]` is a clear adapter error.
    #[test]
    fn missing_adapter_is_error() {
        let entry = NodeEntry::default();
        assert!(matches!(
            invocation_for("x", &entry, &PathBuf::from("/ws")),
            Err(super::BuildError::Adapter { .. })
        ));
    }
}
