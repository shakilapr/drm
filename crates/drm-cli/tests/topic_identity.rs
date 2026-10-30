//! Topic identity: arbitrary depth, declared case, and the collision that case
//! preservation makes possible.
//!
//! `docs/algorithms/toolchain/schema-parser.md` pinned two-segment lowercase
//! paths (`REQ-TOOL-056`) for two independent reasons: identity stability, and
//! Windows' case-insensitive filesystem. ROS 2 names are `/`-rooted and have
//! arbitrary depth, so the depth half of the rule cannot survive a bridge. The
//! case half must — but as an explicit, named error rather than by forbidding
//! uppercase everywhere.
//!
//! REQ: REQ-TOOL-045, REQ-TOOL-056.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A throwaway workspace root.
fn ws(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("drm-topic-id-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("packages/p/topics")).expect("mkdir");
    root
}

/// Writes a topic file, creating parent directories.
fn topic(root: &Path, rel: &str, body: &str) {
    let path = root.join("packages/p/topics").join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, body).expect("write");
}

/// The built topic keys, read back from the generated deployment plan.
fn built_topics(root: &Path) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_drm"))
        .args(["--workspace", &root.display().to_string(), "build"])
        .output()
        .expect("run drm");
    let text =
        std::fs::read_to_string(root.join(".drm/deploy/deployment.json")).unwrap_or_else(|_| {
            panic!(
                "no deployment.json; stdout={} stderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
    // The plan is `{"topics": {"<path>": {...}, ...}}`. Keys are extracted from
    // the `topics` object only: a top-level scan picks up `fingerprint_expectations`
    // and `topics` themselves.
    let body = text
        .split("\"topics\": {")
        .nth(1)
        .expect("deployment.json has no topics object");
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix('"')?;
            let (key, _) = rest.split_once("\": {")?;
            Some(key.to_string())
        })
        .collect()
}

/// A nested ROS-shaped path survives discovery and reaches the deployed plan.
///
/// REQ: REQ-TOOL-045 — the identity a user wrote is the identity that ships.
#[test]
fn nested_topic_reaches_the_deployment() {
    let root = ws("nested");
    topic(&root, "robot1/sensors/lidar/front.drm", "v: f32\n");

    let keys = built_topics(&root);
    assert!(
        keys.contains(&"p/robot1/sensors/lidar/front".to_string()),
        "nested topic missing from the plan; keys = {keys:?}"
    );
}

/// Uppercase is preserved end to end, not silently lowercased.
///
/// REQ: REQ-TOOL-056 — identity must not change under a builder that has no
/// business renaming a user's topic.
#[test]
fn uppercase_topic_is_preserved() {
    let root = ws("case");
    topic(&root, "CmdVel.drm", "v: f32\n");

    let keys = built_topics(&root);
    assert!(
        keys.contains(&"p/CmdVel".to_string()),
        "uppercase was altered; keys = {keys:?}"
    );
}

/// Two topics differing only by case are a build error naming both files.
///
/// This is the property the two-segment-lowercase rule silently provided: on a
/// case-insensitive filesystem (Windows) these two files are one file with one
/// identity, so letting both exist would make a workspace that builds on Linux
/// and merges on Windows. Preserving case is only safe with this check.
///
/// REQ: REQ-TOOL-056.
#[test]
fn case_collision_is_refused() {
    let root = ws("collision");
    topic(&root, "CmdVel.drm", "v: f32\n");
    topic(&root, "cmdvel.drm", "v: f32\n");

    let out = Command::new(env!("CARGO_BIN_EXE_drm"))
        .args(["--workspace", &root.display().to_string(), "build"])
        .output()
        .expect("run drm");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let both = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "a case collision must fail the build; output = {both}"
    );
    assert!(
        both.contains("CmdVel") && both.contains("cmdvel"),
        "the error must name both colliding topics; output = {both}"
    );
}
