//! `xtask gen-check` (R6): regenerate the committed `.drm/gen` tree from the
//! workspace declarations and fail on any difference — a hand-edited, missing,
//! or stale generated file. Regeneration is the diff: the command renders into a
//! scratch tree and compares bytes, so it cannot be fooled by the fingerprint
//! database. REQ: REQ-TOOL-024, REQ-TOOL-025, REQ-TOOL-030.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use drm_workspace::codegen::{existing_generated, regen_diff};
use drm_workspace::model::{PackageInput, TreeInput, build};

/// Workspace the gate checks when no `--workspace` is given: the Phase 0
/// integration fixture, which declares at least one topic and one node and ships
/// its generated tree. Checking the repository root instead — which declares no
/// DRM packages — is the vacuous green this command now refuses.
/// REQ: REQ-TOOL-024.
const DEFAULT_WORKSPACE: &str = "tests/integration/primitive";

/// REQ: REQ-TOOL-024 — `xtask gen-check [--workspace <path>]`: hand-edit and
/// staleness detection over the committed generated tree.
///
/// # Errors
/// Malformed flags, or any failure [`check`] reports.
pub fn run(flags: &[String]) -> Result<()> {
    let (root, ts_only) = parse_workspace(flags)?;
    if ts_only {
        return ts_check(&root);
    }
    check(&root)
}

/// Strict generation check over `root`.
///
/// A check that inspects nothing must not report success, so this refuses three
/// vacuous states: no `packages/` directory, a workspace declaring no topics, a
/// workspace declaring no nodes, and a workspace shipping no generated artifacts.
/// Only then does it compare the committed tree against a fresh render.
/// REQ: REQ-TOOL-024, REQ-TOOL-045.
///
/// # Errors
/// Any vacuous-fixture condition, an invalid workspace model, or any offending
/// generated file (mismatched, missing, or orphaned).
pub fn check(root: &Path) -> Result<()> {
    ts_check(root)?;
    if !root.join("packages").is_dir() {
        bail!(
            "gen-check: no `packages/` under `{}` — checking nothing is a failure, not a pass",
            root.display()
        );
    }
    let model = model_at(root)?;
    if model.topics.is_empty() {
        bail!(
            "gen-check: `{}` declares no topics — a fixture that verifies nothing must fail",
            root.display()
        );
    }
    if model.nodes.is_empty() {
        bail!(
            "gen-check: `{}` declares no nodes — a fixture that verifies nothing must fail",
            root.display()
        );
    }
    let present =
        existing_generated(root).map_err(|e| anyhow!("gen-check: walk generated tree: {e:?}"))?;
    if present.is_empty() {
        bail!(
            "gen-check: `{}` ships no generated artifacts, but declares {} topic(s) and {} node(s) — expected output is absent",
            root.display(),
            model.topics.len(),
            model.nodes.len()
        );
    }
    let diff = regen_diff(&model, root).map_err(|e| anyhow!("gen-check: regen-diff: {e:?}"))?;
    if diff.is_clean() {
        println!(
            "gen-check: clean ({} topics, {} nodes, {} generated files)",
            model.topics.len(),
            model.nodes.len(),
            present.len()
        );
        return Ok(());
    }
    for path in &diff.mismatched {
        eprintln!("gen-check: hand-edited or stale: {}", path.display());
    }
    for path in &diff.missing {
        eprintln!("gen-check: missing generated file: {}", path.display());
    }
    for path in &diff.orphaned {
        eprintln!("gen-check: stale generated file: {}", path.display());
    }
    bail!(
        "gen-check: {} offending generated file(s)",
        diff.offenders().len()
    );
}

/// TS protocol codegen (R6), checked independently of the `.drm/gen` tree: a
/// workspace that declares no `studio-protocol` package has no TS surface.
/// REQ: REQ-TOOL-024.
///
/// # Errors
/// A missing or hand-edited committed TS module.
pub fn ts_check(root: &Path) -> Result<()> {
    crate::studio_ts::check(root)
}

/// Parses `--workspace <path>` (default: [`DEFAULT_WORKSPACE`]) and `--ts-only`
/// (verify only the generated TypeScript surface). REQ: REQ-TOOL-024.
///
/// # Errors
/// Unknown flags or a missing path argument.
fn parse_workspace(flags: &[String]) -> Result<(PathBuf, bool)> {
    let mut root = PathBuf::from(DEFAULT_WORKSPACE);
    let mut ts_only = false;
    let mut i = 0;
    while i < flags.len() {
        match flags.get(i).map(String::as_str) {
            Some("--ts-only") => ts_only = true,
            Some("--workspace") => {
                let path = flags
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("gen-check: --workspace needs a path"))?;
                root = PathBuf::from(path);
                i += 1;
            }
            other => bail!("gen-check flags: {other:?} (expected --workspace <path>, --ts-only)"),
        }
        i += 1;
    }
    Ok((root, ts_only))
}

/// Builds the workspace model under `root` (the check's input).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// Unreadable inputs or a structurally invalid workspace.
pub(crate) fn model_at(root: &Path) -> Result<drm_workspace::model::WorkspaceModel> {
    let tree = discover(root)?;
    build(&tree).map_err(|errs| anyhow!("gen-check: workspace model invalid: {errs:?}"))
}

/// Walks `root` into a [`TreeInput`]: `packages/<pkg>/topics/<stem>.drm`,
/// `packages/<pkg>/nodes/<node>/drm.toml`, and an optional `drm.toml`.
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// Any IO failure, named with the offending path.
fn discover(root: &Path) -> Result<TreeInput> {
    let mut packages = Vec::new();
    for pkg in sorted_dirs(&root.join("packages"))? {
        // `packages/` also holds non-DRM frontend packages (e.g.
        // `packages/studio-protocol`); only directories declaring `topics/` or
        // `nodes/` are DRM workspace packages. REQ: REQ-TOOL-045.
        if !pkg.join("topics").is_dir() && !pkg.join("nodes").is_dir() {
            continue;
        }
        let name = file_name(&pkg)?;
        let mut topics = Vec::new();
        for file in files_with_ext(&pkg.join("topics"), "drm")? {
            let stem = file_stem(&file)?;
            let bytes =
                fs::read(&file).with_context(|| format!("gen-check: read `{}`", file.display()))?;
            topics.push((stem, bytes));
        }
        packages.push(PackageInput { name, topics });
    }

    let mut nodes = Vec::new();
    for pkg in sorted_dirs(&root.join("packages"))? {
        for node_dir in sorted_dirs(&pkg.join("nodes"))? {
            let manifest = node_dir.join("drm.toml");
            if manifest.is_file() {
                let bytes = fs::read(&manifest)
                    .with_context(|| format!("gen-check: read `{}`", manifest.display()))?;
                nodes.push((file_name(&node_dir)?, bytes));
            }
        }
    }

    let topology = fs::read(root.join("drm.toml")).ok();
    Ok(TreeInput {
        packages,
        nodes,
        topology,
    })
}

/// Sorted immediate subdirectories of `dir` (empty when absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// An unreadable existing directory.
fn sorted_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(entries(dir)?.into_iter().filter(|p| p.is_dir()).collect())
}

/// Sorted files in `dir` with the given extension (empty when absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// An unreadable existing directory.
fn files_with_ext(dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|p| p.is_file() && p.extension().and_then(std::ffi::OsStr::to_str) == Some(ext))
        .collect())
}

/// Sorted directory entries (empty when the directory is absent).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// An unreadable existing directory.
fn entries(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let rd = fs::read_dir(dir).with_context(|| format!("gen-check: read `{}`", dir.display()))?;
    for entry in rd {
        let entry = entry.with_context(|| format!("gen-check: read `{}`", dir.display()))?;
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

/// UTF-8 file name of a path. REQ: REQ-TOOL-045.
///
/// # Errors
/// A non-UTF-8 file name.
fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("gen-check: non-UTF-8 path `{}`", path.display()))
}

/// UTF-8 file stem of a path. REQ: REQ-TOOL-045.
///
/// # Errors
/// A non-UTF-8 file stem.
fn file_stem(path: &Path) -> Result<String> {
    path.file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("gen-check: non-UTF-8 file stem `{}`", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use drm_workspace::codegen::{FingerprintDb, execute, plan, regen_diff};
    use drm_workspace::model::build;

    use super::{discover, model_at};

    /// Diffs the committed tree under `root` against a fresh render.
    fn diff_at(root: &Path) -> drm_workspace::codegen::RegenDiff {
        let model = model_at(root).unwrap_or_else(|e| unreachable!("model: {e:?}"));
        regen_diff(&model, root).unwrap_or_else(|e| unreachable!("regen-diff: {e:?}"))
    }

    /// Unique scratch workspace under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("drm-gencheck-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("packages/control/topics"))
            .unwrap_or_else(|e| unreachable!("scratch: {e}"));
        dir
    }

    /// REQ: REQ-TOOL-024 — the self-test: a freshly generated tree is clean, a
    /// deliberately hand-edited file is detected (red→green fixture), and the
    /// strict gate accepts the fixture only because it declares a topic *and* a
    /// node and ships the resulting artifacts.
    #[test]
    fn detects_hand_edited_generated_file() {
        let root = scratch("selftest");
        fs::write(
            root.join("packages/control/topics/target.drm"),
            "a: u8\nb: u64\n",
        )
        .unwrap_or_else(|e| unreachable!("write topic: {e}"));
        // A node is required for the fixture to be non-vacuous (REQ-TOOL-024).
        fs::create_dir_all(root.join("packages/control/nodes/controller"))
            .unwrap_or_else(|e| unreachable!("node dir: {e}"));
        fs::write(
            root.join("packages/control/nodes/controller/drm.toml"),
            "name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = []\n",
        )
        .unwrap_or_else(|e| unreachable!("write node: {e}"));

        // Commit the generated tree exactly as `drm build` would.
        let model = model_at(&root).unwrap_or_else(|e| unreachable!("model: {e:?}"));
        let mut db = FingerprintDb::default();
        execute(&plan(&model, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("generate: {e:?}"));
        let clean = diff_at(&root);
        assert!(clean.is_clean(), "fresh tree clean: {clean:?}");
        assert!(
            super::check(&root).is_ok(),
            "a generated fixture with a topic and a node must pass the strict gate"
        );

        // Hand-edit one generated artifact.
        let generated = root.join(".drm/gen/c/control/target/target.h");
        let mut body = fs::read(&generated).unwrap_or_else(|e| unreachable!("read: {e}"));
        body.extend_from_slice(b"\n/* hand edited */\n");
        fs::write(&generated, body).unwrap_or_else(|e| unreachable!("write: {e}"));

        let diff = diff_at(&root);
        assert!(!diff.is_clean(), "hand-edit must be detected");
        assert!(
            diff.mismatched.iter().any(|p| p.ends_with("target.h")),
            "edited path reported: {diff:?}"
        );
        assert!(
            super::check(&root).is_err(),
            "a hand-edited committed artifact must fail the strict gate"
        );

        // Discovery round-trips the workspace tree.
        let tree = discover(&root).unwrap_or_else(|e| unreachable!("discover: {e:?}"));
        assert_eq!(tree.packages.len(), 1);
        assert!(build(&tree).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024 — every state in which the gate would verify nothing
    /// must fail: absent `packages/`, no topics, no nodes, no generated
    /// artifacts. This is the vacuous-green defect the gate now refuses.
    #[test]
    fn vacuous_fixtures_fail() {
        // 1. No `packages/` directory at all.
        let none = scratch("vacuous-none");
        fs::remove_dir_all(none.join("packages")).unwrap_or_else(|e| unreachable!("rm: {e}"));
        assert!(
            super::check(&none).is_err(),
            "an absent packages/ directory must fail"
        );
        let _ = fs::remove_dir_all(&none);

        // 2. Topics but no nodes.
        let no_nodes = scratch("vacuous-nodes");
        fs::write(
            no_nodes.join("packages/control/topics/target.drm"),
            "a: u8\n",
        )
        .unwrap_or_else(|e| unreachable!("write: {e}"));
        let Err(err) = super::check(&no_nodes) else {
            unreachable!("a workspace without nodes must fail")
        };
        assert!(
            err.to_string().contains("no nodes"),
            "message names the cause: {err}"
        );
        let _ = fs::remove_dir_all(&no_nodes);

        // 3. Topics and a node, but no generated artifacts committed.
        let no_output = scratch("vacuous-output");
        fs::write(
            no_output.join("packages/control/topics/target.drm"),
            "a: u8\n",
        )
        .unwrap_or_else(|e| unreachable!("write: {e}"));
        fs::create_dir_all(no_output.join("packages/control/nodes/controller"))
            .unwrap_or_else(|e| unreachable!("node dir: {e}"));
        fs::write(
            no_output.join("packages/control/nodes/controller/drm.toml"),
            "name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = []\n",
        )
        .unwrap_or_else(|e| unreachable!("write node: {e}"));
        let Err(err) = super::check(&no_output) else {
            unreachable!("a fixture with no generated artifacts must fail")
        };
        assert!(
            err.to_string().contains("no generated artifacts"),
            "message names the cause: {err}"
        );
        let _ = fs::remove_dir_all(&no_output);

        // 4. The default workspace is the fixture, never the repository root.
        assert_eq!(super::DEFAULT_WORKSPACE, "tests/integration/primitive");
    }
}
