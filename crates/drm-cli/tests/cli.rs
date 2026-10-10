//! End-to-end tests: the built `drm` binary is invoked via
//! `std::process::Command` against throwaway fixture workspaces. Covers the
//! compact gate (I-H4/REQ-TOOL-038), dense sorted compaction output
//! (REQ-TOOL-080), the read-only ids report, and the `assign` round-trip verify
//! (REQ-TOOL-013, REQ-TOOL-047).
//!
//! REQ: REQ-TOOL-038, REQ-TOOL-080, REQ-TOOL-037, REQ-TOOL-047.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Creates a unique throwaway workspace root under the system temp directory.
fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("drm-cli-it-{}-{n}-{tag}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap_or_else(|e| unreachable!("temp dir: {e}"));
    root
}

/// Writes `body` to `path`, creating parent directories.
fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| unreachable!("mkdir {parent:?}: {e}"));
    }
    std::fs::write(path, body).unwrap_or_else(|e| unreachable!("write {path:?}: {e}"));
}

/// Reads `path` as UTF-8 text.
fn read(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| unreachable!("read {path:?}: {e}"));
    String::from_utf8(bytes).unwrap_or_else(|e| unreachable!("{e}"))
}

/// Runs the `drm` binary against `workspace` with `args`.
fn drm(workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drm"))
        .arg("--workspace")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap_or_else(|e| unreachable!("run drm: {e}"))
}

/// Captures stdout text.
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Captures stderr text.
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A workspace with two topics and two nodes (a publisher of `control/target`
/// and a node with empty contracts), plus an optional lockfile.
fn fixture(tag: &str, lockfile: Option<&str>) -> PathBuf {
    let root = temp_root(tag);
    write(&root.join("packages/control/topics/target.drm"), "v: u32\n");
    write(
        &root.join("packages/planning/topics/trajectory.drm"),
        "v: u32\n",
    );
    write(
        &root.join("packages/control/nodes/controller/drm.toml"),
        "name = \"controller\"\npublishes = []\nconsumes = []\n",
    );
    write(
        &root.join("packages/planning/nodes/planner/drm.toml"),
        "name = \"planner\"\npublishes = [\"control/target\"]\nconsumes = []\n",
    );
    if let Some(body) = lockfile {
        write(&root.join(".drm/ids.json"), body);
    }
    root
}

/// REQ: REQ-TOOL-080, REQ-TOOL-037 — the report is sorted by topic path and
/// lists reservations with their reason.
#[test]
fn ids_report_is_sorted() {
    let root = fixture(
        "report",
        Some(
            "{\n  \"format_version\": 1,\n  \"next_free\": 3,\n  \"assignments\": {\n    \
             \"planning/trajectory\": 1,\n    \"control/target\": 0\n  },\n  \"reserved\": {\n    \
             \"2\": { \"reason\": \"deleted\" }\n  }\n}\n",
        ),
    );
    let out = drm(&root, &["ids"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    let ctrl = text
        .find("control/target = 0")
        .unwrap_or_else(|| unreachable!("missing control/target line: {text}"));
    let traj = text
        .find("planning/trajectory = 1")
        .unwrap_or_else(|| unreachable!("missing planning/trajectory line: {text}"));
    assert!(ctrl < traj, "report not sorted by topic path: {text}");
    assert!(text.contains("2 = deleted (tombstone)"), "got: {text}");
    assert!(text.contains("next_free: 3"), "got: {text}");
}

/// REQ: REQ-TOOL-080 — a missing lockfile is a friendly, non-zero error.
#[test]
fn ids_missing_lockfile_errors() {
    let root = fixture("missing", None);
    let out = drm(&root, &["ids"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("cannot read"),
        "got: {}",
        stderr(&out)
    );
}

/// REQ: REQ-TOOL-038 — `ids compact` without `--yes` refuses and leaves the
/// lockfile byte-identical (I-H4).
#[test]
fn compact_refuses_without_yes() {
    let lockfile = "{\n  \"format_version\": 1,\n  \"next_free\": 4,\n  \"assignments\": {\n    \
                    \"control/target\": 3\n  },\n  \"reserved\": {\n    \"2\": { \"reason\": \
                    \"deleted\" }\n  }\n}\n";
    let root = fixture("refuse", Some(lockfile));
    let path = root.join(".drm/ids.json");
    let before = read(&path);
    let out = drm(&root, &["ids", "compact"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("without --yes"),
        "got: {}",
        stderr(&out)
    );
    assert_eq!(read(&path), before, "lockfile must be untouched on refusal");
}

/// REQ: REQ-TOOL-038, REQ-TOOL-080 — `ids compact --yes` re-packs live
/// assignments dense in sorted order, drops reservations, and writes the
/// canonical lockfile (`format_version` first).
#[test]
fn compact_with_yes_is_dense_and_sorted() {
    let root = fixture(
        "compact",
        Some(
            "{\n  \"format_version\": 1,\n  \"next_free\": 8,\n  \"assignments\": {\n    \
             \"c/d\": 1,\n    \"a/b\": 3\n  },\n  \"reserved\": {\n    \"2\": { \"reason\": \
             \"merge_collision\" },\n    \"7\": { \"reason\": \"deleted\" }\n  }\n}\n",
        ),
    );
    let out = drm(&root, &["ids", "compact", "--yes"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let expected = "{\n  \"format_version\": 1,\n  \"next_free\": 2,\n  \"assignments\": {\n    \
                    \"a/b\": 0,\n    \"c/d\": 1\n  },\n  \"reserved\": {}\n}\n";
    assert_eq!(read(&root.join(".drm/ids.json")), expected);
    assert!(
        stdout(&out).contains("new deployment identity"),
        "guidance missing"
    );
}

/// REQ: REQ-TOOL-013, REQ-TOOL-047 — `assign control/target --consume controller`
/// edits the manifest and the subsequent verify exits 0 (green).
#[test]
fn assign_roundtrip_verify_green() {
    let root = fixture("assign", None);
    let out = drm(
        &root,
        &["assign", "control/target", "--consume", "controller"],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("verify: green"),
        "got: {}",
        stdout(&out)
    );
    let manifest = read(&root.join("packages/control/nodes/controller/drm.toml"));
    assert!(
        manifest.contains("control/target"),
        "manifest not mutated: {manifest}"
    );
}

/// REQ: REQ-TOOL-047 — an undeclared node is an actionable, non-zero error and
/// no manifest is written.
#[test]
fn assign_undeclared_node_errors() {
    let root = fixture("badnode", None);
    let out = drm(&root, &["assign", "control/target", "--consume", "ghost"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("undeclared node `ghost`"),
        "got: {}",
        stderr(&out)
    );
}

/// REQ: REQ-TOOL-013 — an undeclared topic is an actionable, non-zero error.
#[test]
fn assign_undeclared_topic_errors() {
    let root = fixture("badtopic", None);
    let out = drm(&root, &["assign", "nope/x", "--publish", "planner"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("undeclared topic `nope/x`"),
        "got: {}",
        stderr(&out)
    );
}
