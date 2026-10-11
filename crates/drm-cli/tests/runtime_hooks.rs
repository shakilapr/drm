//! Model → codegen → runtime hooks end-to-end (work-plan M08W4-E1-T1): the
//! runtime consumes the descriptor expectations and payload-pool sizing the
//! `WorkspaceModel` derives, and the generated bindings land at the stable
//! `.drm/gen` paths the runtime build uses. This is the composition-root test:
//! `drm-cli` is the only layer that may depend on both the toolchain and the
//! runtime (codebase.md 2.2).
//! REQ: REQ-TOOL-045, REQ-TOOL-029, REQ-CORE-045, REQ-CORE-015.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_workspace::codegen::{FingerprintDb, GenKind, Lang, execute, out_path, plan};
use drm_workspace::model::{
    PackageInput, TreeInput, WorkspaceModel, build, layout_hash, runtime_plan,
};

/// Unique scratch directory under the system temp dir.
fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("drm-rt-hooks-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("scratch: {e}"));
    dir
}

/// Example workspace: one package, one sized topic.
fn example_model() -> WorkspaceModel {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "control".to_string(),
            topics: vec![(
                "target".to_string(),
                b"@history: 4\n@pool_size: 6\na: u8\nb: u64\n".to_vec(),
            )],
        }],
        nodes: Vec::new(),
        topology: None,
    };
    build(&tree).unwrap_or_else(|e| unreachable!("example model: {e:?}"))
}

/// REQ: REQ-TOOL-045, REQ-CORE-045 — the model-derived activation parameters
/// drive a live `Channel`: depth and pool size match the declared topic, and a
/// published sample round-trips.
#[test]
fn runtime_activates_from_model_plan() {
    let model = example_model();
    let plan = runtime_plan(&model);
    let rt = plan
        .get("control/target")
        .cloned()
        .unwrap_or_else(|| unreachable!("topic present"));

    assert_eq!(rt.history_depth, 4, "declared @history depth");
    assert_eq!(rt.pool_size, 6, "declared @pool_size");
    assert!(
        rt.pool_size > rt.history_depth,
        "pool floor satisfied (REQ-CORE-045)"
    );

    let channel = Channel::activate(
        rt.history_depth,
        rt.pool_size,
        rt.payload_capacity,
        rt.slot_align,
        OverflowPolicy::Overwrite,
        0,
        1,
        None,
    )
    .unwrap_or_else(|e| unreachable!("activate: {e:?}"));

    assert_eq!(channel.depth(), 4);
    assert_eq!(channel.pool_size(), 6);

    let payload = vec![0xA5_u8; usize::try_from(rt.payload_capacity).unwrap_or(0)];
    channel
        .publish(&payload)
        .unwrap_or_else(|e| unreachable!("publish: {e:?}"));

    let mut reader = channel.reader();
    let mut buf = vec![0_u8; payload.len()];
    let outcome = channel
        .try_next(&mut reader, 0, &mut buf, 4)
        .unwrap_or_else(|e| unreachable!("read: {e:?}"));
    assert_eq!(outcome, ReadOutcome::Sample, "sample delivered");
    assert_eq!(buf, payload, "published bytes round-trip");
}

/// REQ: REQ-TOOL-045, REQ-TOOL-029, REQ-CORE-015 — descriptor expectations
/// mirror the model and the generated binding lands at the stable path.
#[test]
fn runtime_expectations_and_codegen_align() {
    let model = example_model();
    let rt = runtime_plan(&model)
        .get("control/target")
        .cloned()
        .unwrap_or_else(|| unreachable!("topic present"));
    let entry = &model.topics["control/target"];
    assert_eq!(rt.schema_hash, entry.schema_hash, "descriptor schema hash");
    assert_eq!(
        rt.layout_hash,
        layout_hash(&entry.layout),
        "descriptor layout hash"
    );

    let root = scratch("codegen");
    let mut db = FingerprintDb::default();
    execute(&plan(&model, &db, &root), &mut db, &root)
        .unwrap_or_else(|e| unreachable!("generate: {e:?}"));
    let header = out_path(&root, Lang::C, GenKind::Topic, "control/target");
    assert!(header.is_file(), "generated binding at the stable path");
    let _ = fs::remove_dir_all(&root);
}
