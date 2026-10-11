//! Example-build integration test (work-plan M09W1-E2-T1; examples/ rule
//! codebase.md §2.4 R2): `drm build` is green on `examples/mixed_robot` (one
//! node per language: c + python + rust) and `drm info` prints a deterministic
//! model + verify summary. The example is copied to a scratch directory so the
//! committed tree is never mutated.
//! REQ: REQ-TOOL-012, REQ-TOOL-011, REQ-TOOL-045.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Recursively copies `src` into `dst`.
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|e| unreachable!("mkdir {dst:?}: {e}"));
    let entries = std::fs::read_dir(src).unwrap_or_else(|e| unreachable!("read {src:?}: {e}"));
    for entry in entries.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap_or_else(|e| unreachable!("copy {from:?}: {e}"));
        }
    }
}

/// A scratch copy of `examples/mixed_robot`.
fn scratch_example(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mixed_robot");
    let dst = std::env::temp_dir().join(format!("drm-example-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_tree(&src, &dst);
    dst
}

/// Runs the `drm` binary against `root` with `args`.
fn drm(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drm"))
        .arg("--workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap_or_else(|e| unreachable!("run drm: {e}"))
}

/// REQ: REQ-TOOL-012, REQ-TOOL-011 — `drm build` is green across all three
/// languages and writes the deployment artifact.
#[test]
fn example_build_green() {
    let root = scratch_example("build");
    let out = drm(&root, &["build"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "drm build failed: {stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("3 built"),
        "all three nodes built: {stdout}"
    );
    assert!(
        root.join(".drm/deploy/deployment.json").is_file(),
        "deployment artifact present"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-045 — `drm info` is deterministic and reports verify green.
#[test]
fn example_info_deterministic() {
    let root = scratch_example("info");
    let first = drm(&root, &["info"]);
    let second = drm(&root, &["info"]);
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout, "deterministic projection");
    let stdout = String::from_utf8_lossy(&first.stdout).into_owned();
    assert!(stdout.contains("verify green"), "got: {stdout}");
    assert!(stdout.contains("node controller"), "model render: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}
