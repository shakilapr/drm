//! `drm build --verify` full-verifier gate (issue 8): the default build keeps
//! the V-SW `verify_lite` slice, the opt-in strict path runs the whole
//! catalogue, and failures name their check id + REQ citation.
//! REQ: REQ-TOOL-012, REQ-TOOL-056, REQ-TOOL-057, REQ-TOOL-060.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Creates a unique throwaway workspace root under the system temp directory.
fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("drm-strict-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// Writes `body` to `path`, creating parents.
fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| unreachable!("mkdir {parent:?}: {e}"));
    }
    std::fs::write(path, body).unwrap_or_else(|e| unreachable!("write {path:?}: {e}"));
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

/// One-package, one-compute workspace: `publisher` publishes `topic` on `c1`.
/// `body` is the topic source; `topology` optional (defaults to a single
/// compute with the node placed on it).
fn workspace(tag: &str, body: &str) -> PathBuf {
    let root = temp_root(tag);
    write(&root.join("packages/pkg/topics/topic.drm"), body);
    write(
        &root.join("packages/pkg/nodes/publisher/drm.toml"),
        "name = \"publisher\"\npublishes = [\"pkg/topic\"]\n[build]\ncommand = [\"true\"]\n",
    );
    write(
        &root.join("drm.toml"),
        "[computes]\nc1 = true\n\n[placement]\npublisher = \"c1\"\n",
    );
    root
}

/// REQ: REQ-TOOL-012, REQ-TOOL-056 — an uppercase topic path builds by default
/// (additive: no regression) and fails `--verify`, naming `V-ID` and
/// `REQ-TOOL-056`.
#[test]
fn uppercase_topic_needs_strict_flag() {
    let root = workspace("uppercase", "v: u32\n");
    write(
        &root.join("packages/Robot/topics/CmdVel.drm"),
        "v: u32\n",
    );
    write(
        &root.join("packages/Robot/nodes/robot/drm.toml"),
        "name = \"robot\"\npublishes = [\"Robot/CmdVel\"]\n[build]\ncommand = [\"true\"]\n",
    );

    let plain = drm(&root, &["build"]);
    let stdout = String::from_utf8_lossy(&plain.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&plain.stderr).into_owned();
    assert!(plain.status.success(), "default build regressed: {stderr}");
    assert!(
        stdout.contains("verify-lite [V-SW]"),
        "default pass must report verify-lite: {stdout}"
    );

    let strict = drm(&root, &["build", "--verify"]);
    assert!(!strict.status.success(), "strict build must fail");
    let stderr = String::from_utf8_lossy(&strict.stderr).into_owned();
    assert!(stderr.contains("V-ID"), "check id named: {stderr}");
    assert!(
        stderr.contains("REQ-TOOL-056"),
        "REQ citation named: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-057 — a payload pool below the `R+1` floor fails `--verify`
/// citing `V-MEM`.
#[test]
fn below_pool_floor_fails_strict() {
    let root = workspace("pool", "@history: 4\n@pool_size: 2\nv: u64\n");
    let strict = drm(&root, &["build", "--verify"]);
    assert!(!strict.status.success(), "strict build must fail");
    let stderr = String::from_utf8_lossy(&strict.stderr).into_owned();
    assert!(stderr.contains("V-MEM"), "check id named: {stderr}");
    assert!(stderr.contains("pool_slots"), "detail named: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-012 — a clean workspace passes `--verify` with exit 0 and
/// reports the full catalogue.
#[test]
fn clean_workspace_passes_strict() {
    let root = workspace("clean", "@history: 4\n@pool_size: 8\nv: u64\n");
    let strict = drm(&root, &["build", "--verify"]);
    let stdout = String::from_utf8_lossy(&strict.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&strict.stderr).into_owned();
    assert!(strict.status.success(), "strict build failed: {stderr}");
    assert!(
        stdout.contains("full verifier [V-SCH, V-SW, V-PLC, V-RT, V-REL, V-MEM, V-CMP, V-ID, V-SPAT, V-RPC, V-REC]"),
        "full catalogue reported: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
