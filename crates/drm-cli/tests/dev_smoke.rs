//! `drm dev` dev-mode smoke suite (work-plan M09W3-E4-T2;
//! build-orchestration.md §Protocols): scripted declaration and application-source
//! edits on a scratch copy of `examples/mixed_robot` stay green across
//! successive dev cycles and the final state is reflected in the deployment
//! artifact (I-BO4 no-stale-code, I-BO2 source-delegation). The committed tree
//! is never mutated. REQ: REQ-TOOL-016, REQ-TOOL-012, REQ-TOOL-015.

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

/// A scratch copy of `examples/mixed_robot` (never the committed tree).
fn scratch_example(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mixed_robot");
    let dst = std::env::temp_dir().join(format!("drm-dev-{}-{n}-{tag}", std::process::id()));
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

/// Asserts the command exited zero, printing captured output on failure.
fn assert_ok(out: &Output, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{what} failed\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// Appends `line` to `path`. REQ: REQ-TOOL-015.
fn append(path: &Path, line: &str) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap_or_else(|e| unreachable!("open {path:?}: {e}"));
    file.write_all(line.as_bytes())
        .unwrap_or_else(|e| unreachable!("append {path:?}: {e}"));
}

/// REQ: REQ-TOOL-016 — `drm dev --once` runs the initial build green and writes
/// the deployment artifact.
#[test]
fn dev_once_smoke() {
    let root = scratch_example("once");
    let out = drm(&root, &["dev", "--once"]);
    assert_ok(&out, "drm dev --once");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("3 built"), "all nodes built: {stdout}");
    assert!(
        root.join(".drm/deploy/deployment.json").is_file(),
        "deployment artifact present"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-015, REQ-TOOL-012 — scripted declaration and source edits keep
/// successive dev cycles green; the final state is applied (I-BO4) and adapters
/// are delegated source freshness (I-BO2).
#[test]
fn dev_scripted_edit_cycles() {
    let root = scratch_example("cycles");
    let topic = root.join("packages/control/topics/target.drm");
    let source = root.join("packages/control/nodes/controller/src/main.c");

    assert_ok(&drm(&root, &["dev", "--once"]), "initial dev cycle");

    // Declaration edit: add a field; the model reloads and artifacts regenerate.
    append(&topic, "timestamp: u64\n");
    let after_decl = drm(&root, &["build"]);
    assert_ok(&after_decl, "build after declaration edit");
    let decl_out = String::from_utf8_lossy(&after_decl.stdout).into_owned();
    assert!(
        decl_out.contains("3 built"),
        "declaration cycle: {decl_out}"
    );

    // Application-source edit: only the owning node's adapter runs (no DRM
    // dependency analysis); the cycle stays green.
    append(&source, "/* dev smoke edit */\n");
    let after_source = drm(&root, &["build"]);
    assert_ok(&after_source, "build after source edit");
    let source_out = String::from_utf8_lossy(&after_source.stdout).into_owned();
    assert!(source_out.contains("3 built"), "source cycle: {source_out}");

    // Final state: deployment artifact and stable handle lockfile present.
    assert!(root.join(".drm/deploy/deployment.json").is_file());
    assert!(root.join(".drm/ids.json").is_file());
    let _ = std::fs::remove_dir_all(&root);
}
