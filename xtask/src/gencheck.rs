//! `xtask gen-check` (R6): regenerate the committed `.drm/gen` tree from the
//! workspace declarations and fail on any difference — a hand-edited, missing,
//! or stale generated file. Regeneration is the diff: the command renders into a
//! scratch tree and compares bytes, so it cannot be fooled by the fingerprint
//! database. REQ: REQ-TOOL-024, REQ-TOOL-025, REQ-TOOL-030.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use drm_workspace::codegen::regen_diff;
use drm_workspace::model::{PackageInput, TreeInput, build};

/// REQ: REQ-TOOL-024 — `xtask gen-check [--workspace <path>]`: hand-edit
/// detection over the committed generated tree.
///
/// # Errors
/// Malformed flags, an unreadable workspace, a structurally invalid model, or
/// any offending generated file.
pub fn run(flags: &[String]) -> Result<()> {
    let root = parse_workspace(flags)?;
    if !root.join("packages").is_dir() {
        eprintln!(
            "gen-check: no `packages/` under `{}`; nothing to check",
            root.display()
        );
        return Ok(());
    }
    let model = model_at(&root)?;
    let diff = regen_diff(&model, &root).map_err(|e| anyhow!("gen-check: regen-diff: {e:?}"))?;
    if diff.is_clean() {
        println!(
            "gen-check: clean ({} topics, {} nodes)",
            model.topics.len(),
            model.nodes.len()
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

/// Parses the optional `--workspace <path>` flag (default: the current
/// directory). REQ: REQ-TOOL-024.
///
/// # Errors
/// Unknown flags or a missing path argument.
fn parse_workspace(flags: &[String]) -> Result<PathBuf> {
    match flags {
        [] => Ok(PathBuf::from(".")),
        [flag, path] if flag == "--workspace" => Ok(PathBuf::from(path)),
        other => bail!("gen-check flags: {other:?} (expected none or --workspace <path>)"),
    }
}

/// Builds the workspace model under `root` (the check's input).
/// REQ: REQ-TOOL-045.
///
/// # Errors
/// Unreadable inputs or a structurally invalid workspace.
fn model_at(root: &Path) -> Result<drm_workspace::model::WorkspaceModel> {
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
    /// deliberately hand-edited file is detected (red→green fixture).
    #[test]
    fn detects_hand_edited_generated_file() {
        let root = scratch("selftest");
        fs::write(
            root.join("packages/control/topics/target.drm"),
            "a: u8\nb: u64\n",
        )
        .unwrap_or_else(|e| unreachable!("write topic: {e}"));

        // Commit the generated tree exactly as `drm build` would.
        let model = model_at(&root).unwrap_or_else(|e| unreachable!("model: {e:?}"));
        let mut db = FingerprintDb::default();
        execute(&plan(&model, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("generate: {e:?}"));
        let clean = diff_at(&root);
        assert!(clean.is_clean(), "fresh tree clean: {clean:?}");

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

        // Discovery round-trips the workspace tree.
        let tree = discover(&root).unwrap_or_else(|e| unreachable!("discover: {e:?}"));
        assert_eq!(tree.packages.len(), 1);
        assert!(build(&tree).is_ok());
        let _ = fs::remove_dir_all(&root);
    }
}
