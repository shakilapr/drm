//! Build-orchestration integration suite (work-plan M09W1-E2-T1;
//! build-orchestration.md §Test Matrix): exact orchestration sequence (I-BO1),
//! adapter matrix (cmake/cargo/command), adapter environment hooks, verify-aborts
//! behaviour, and the deterministic `drm info` projection.
//! REQ: REQ-TOOL-012, REQ-TOOL-011, REQ-TOOL-014, REQ-TOOL-045.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use drm_types::Status;
use drm_workspace::model::handles::HandleTable;
use drm_workspace::orchestrator::{
    AdapterInvocation, AdapterRunner, BuildAdapter, BuildError, BuildRequest, BuildStep, Language,
    build, info, invocation_for,
};
use drm_workspace::verify::verify_lite;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Records adapter invocations without spawning a process.
#[derive(Default)]
struct Recorder {
    calls: Vec<AdapterInvocation>,
}

impl AdapterRunner for Recorder {
    fn invoke(&mut self, invocation: &AdapterInvocation) -> Result<(), Status> {
        self.calls.push(invocation.clone());
        Ok(())
    }
}

/// Creates a unique throwaway workspace root under the system temp directory.
fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("drm-build-{}-{n}-{tag}", std::process::id()));
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

/// A workspace with two topics and two nodes: `controller` publishes
/// `control/target`; `planner` consumes it and publishes `planning/trajectory`.
/// Both declare a `[build]` adapter. REQ: REQ-TOOL-012.
fn fixture(tag: &str) -> PathBuf {
    let root = temp_root(tag);
    write(&root.join("packages/control/topics/target.drm"), "v: u32\n");
    write(
        &root.join("packages/planning/topics/trajectory.drm"),
        "v: u32\n",
    );
    write(
        &root.join("packages/control/nodes/controller/drm.toml"),
        "name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = []\nlanguage = \"c\"\n[build]\nsystem = \"cargo\"\npackage = \"controller\"\n",
    );
    write(
        &root.join("packages/planning/nodes/planner/drm.toml"),
        "name = \"planner\"\npublishes = [\"planning/trajectory\"]\nconsumes = [\"control/target\"]\nlanguage = \"python\"\n[build]\ncommand = [\"true\"]\n",
    );
    root
}

/// REQ: REQ-TOOL-012 — I-BO1: the step sequence is exact and adapters run last.
#[test]
fn build_order() {
    let root = fixture("order");
    let req = BuildRequest {
        workspace: root.clone(),
        targets: None,
    };
    let mut recorder = Recorder::default();
    let mut steps: Vec<BuildStep> = Vec::new();
    let report = build(&req, &HandleTable::default(), &mut recorder, &mut |step| {
        steps.push(step);
    })
    .unwrap_or_else(|e| unreachable!("build: {e}"));

    assert_eq!(
        steps,
        vec![
            BuildStep::Parse,
            BuildStep::Model,
            BuildStep::Handles,
            BuildStep::Deploy,
            BuildStep::Codegen,
            BuildStep::Verify,
            BuildStep::Adapter("controller".to_string()),
            BuildStep::Adapter("planner".to_string()),
        ],
        "I-BO1 sequence"
    );
    assert_eq!(report.built, ["controller", "planner"]);
    assert_eq!(recorder.calls.len(), 2, "one adapter call per node");
    assert!(
        root.join(".drm/deploy/deployment.json").is_file(),
        "deployment artifact written"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-012 — a verify failure aborts before any adapter runs.
#[test]
fn verify_failure_aborts_before_adapters() {
    let root = temp_root("verify");
    write(&root.join("packages/control/topics/target.drm"), "v: u32\n");
    write(
        &root.join("packages/control/nodes/a/drm.toml"),
        "name = \"a\"\npublishes = [\"control/target\"]\n[build]\ncommand = [\"true\"]\n",
    );
    write(
        &root.join("packages/control/nodes/b/drm.toml"),
        "name = \"b\"\npublishes = [\"control/target\"]\n[build]\ncommand = [\"true\"]\n",
    );
    let req = BuildRequest {
        workspace: root.clone(),
        targets: None,
    };
    let mut recorder = Recorder::default();
    let mut steps: Vec<BuildStep> = Vec::new();
    let result = build(&req, &HandleTable::default(), &mut recorder, &mut |step| {
        steps.push(step);
    });
    assert!(matches!(result, Err(BuildError::Verify(_))), "{result:?}");
    assert!(recorder.calls.is_empty(), "adapters must not run");
    assert!(!steps.iter().any(|s| matches!(s, BuildStep::Adapter(_))));
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-012, REQ-TOOL-011 — the adapter matrix resolves each declared
/// system to its canonical program/argv and carries the environment hooks.
#[test]
fn adapter_matrix() {
    let ws = PathBuf::from("/ws");
    let cases = [
        (
            BuildAdapter::Cmake {
                dir: PathBuf::from("build"),
                target: "controller".to_string(),
            },
            "cmake",
            vec!["--build", "/ws/build", "--target", "controller"],
        ),
        (
            BuildAdapter::Cargo {
                package: "driver".to_string(),
            },
            "cargo",
            vec!["build", "--package", "driver"],
        ),
        (
            BuildAdapter::Command(vec!["my-builder".to_string(), "--target".to_string()]),
            "my-builder",
            vec!["--target"],
        ),
    ];
    for (adapter, program, args) in cases {
        let entry = drm_workspace::model::NodeEntry {
            build: Some(adapter),
            language: Some(Language::C),
            ..drm_workspace::model::NodeEntry::default()
        };
        let inv = invocation_for("n", &entry, &ws).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(inv.program, program);
        assert_eq!(inv.args, args);
        assert!(
            inv.env
                .iter()
                .any(|(k, _)| k == drm_workspace::orchestrator::GEN_INCLUDE_ENV),
            "include hook present"
        );
        assert!(
            inv.env
                .iter()
                .any(|(k, _)| k == drm_workspace::orchestrator::DEPLOY_METADATA_ENV),
            "deployment hook present"
        );
    }
}

/// REQ: REQ-TOOL-045 — `drm info` is deterministic and read-only: the same
/// model renders byte-identical text and no files are mutated.
#[test]
fn info_projection_is_deterministic() {
    let root = fixture("info");
    let req = BuildRequest {
        workspace: root.clone(),
        targets: None,
    };
    let mut recorder = Recorder::default();
    build(&req, &HandleTable::default(), &mut recorder, &mut |_| {})
        .unwrap_or_else(|e| unreachable!("build: {e}"));

    let tree = drm_workspace::orchestrator::discover_tree(&root)
        .unwrap_or_else(|e| unreachable!("discover: {e}"));
    let model = drm_workspace::model::build(&tree).unwrap_or_else(|e| unreachable!("{e:?}"));
    let findings = verify_lite(&model);
    let first = info(&model, &findings);
    let second = info(&model, &findings);
    assert_eq!(first, second, "deterministic projection");
    assert!(first.contains("verify green"), "green summary: {first}");
    assert!(first.contains("node controller"), "model render: {first}");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-012 — an explicit target subset invokes only those adapters.
#[test]
fn build_target_subset() {
    let root = fixture("subset");
    let req = BuildRequest {
        workspace: root.clone(),
        targets: Some(BTreeSet::from(["planner".to_string()])),
    };
    let mut recorder = Recorder::default();
    build(&req, &HandleTable::default(), &mut recorder, &mut |_| {})
        .unwrap_or_else(|e| unreachable!("build: {e}"));
    assert_eq!(recorder.calls.len(), 1);
    assert_eq!(recorder.calls[0].node, "planner");
    let _ = std::fs::remove_dir_all(&root);
}
