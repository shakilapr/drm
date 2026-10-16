//! Interop of the generated deployment configuration across the layer boundary
//! (Phase 1, REQ-CORE-108): what `drm build` emits in the toolchain is what the
//! runtime can load, and the identity it carries is the one the runtime derives
//! segment names from. Until now these were two independently written sides of a
//! gap (G4) with callers hand-building expectations in between.
//! REQ: REQ-TOOL-083, REQ-TOOL-084, REQ-CORE-108, REQ-CORE-088.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use drm_core::deploy::{CONFIG_FORMAT_VERSION, DeploymentConfig};

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
fn scratch_example() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mixed_robot");
    let dst = std::env::temp_dir().join(format!("drm-deploycfg-{}-{n}", std::process::id()));
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

/// Builds the example workspace and loads its deployment configuration the way a
/// node process would.
fn built_config(tag: &str) -> (PathBuf, DeploymentConfig) {
    let root = scratch_example();
    let out = drm(&root, &["build"]);
    assert!(
        out.status.success(),
        "{tag}: drm build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = root.join(".drm/deploy/deployment.json");
    let config = DeploymentConfig::load(&path)
        .unwrap_or_else(|e| unreachable!("{tag}: runtime load of the emitted artifact: {e:?}"));
    (root, config)
}

/// REQ: REQ-TOOL-083, REQ-CORE-108 — the emitted artifact is loadable by the
/// runtime, and the runtime's format version is the toolchain's: a drift between
/// the two constants would silently break every deployment.
#[test]
fn emitted_config_loads_in_the_runtime() {
    assert_eq!(
        drm_workspace::deploy::FORMAT_VERSION,
        CONFIG_FORMAT_VERSION,
        "the toolchain emitter and the runtime loader must agree on the artifact shape"
    );

    let (root, config) = built_config("load");
    assert!(
        !config.topics.is_empty(),
        "the example deploys at least one topic"
    );
    assert!(
        !config.fingerprint_expectations.is_empty(),
        "the example deploys at least one node with a recorded expectation"
    );

    // Every deployed topic carries a validated contract and a real slot.
    for (topic, entry) in &config.topics {
        assert!(entry.schema_hash.len() == 32, "{topic}: schema digest");
        assert!(entry.layout_hash.len() == 32, "{topic}: layout digest");
        assert!(entry.payload_capacity > 0, "{topic}: payload capacity");
        assert!(
            u32::try_from(entry.slot).is_ok(),
            "{topic}: slot fits the runtime"
        );
    }

    // Every node expectation is a 128-bit digest and resolves to a fingerprint.
    for node in config.fingerprint_expectations.keys() {
        assert!(
            config.expectation(node).is_some(),
            "node `{node}` must resolve to a compiled expectation"
        );
        assert!(
            config.expectation(&format!("{node}-absent")).is_none(),
            "an undeployed node has no expectation"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-CORE-088, REQ-TOOL-083 — the loaded identity and slot drive the same
/// generated name family the runtime attaches with: one segment per topic, all
/// within the deployment's own namespace.
#[test]
fn loaded_identity_drives_segment_names() {
    let (root, config) = built_config("names");
    let id = config
        .id()
        .unwrap_or_else(|e| unreachable!("deployment id: {e:?}"));
    let encoded = id.encoded();
    assert_eq!(encoded.len(), 32, "backend-safe token width");

    let slots = config.slots();
    assert_eq!(
        slots.len(),
        config.topics.len(),
        "every deployed topic has a slot"
    );

    let mut names = std::collections::BTreeSet::new();
    for (topic, slot) in &slots {
        let name = id
            .segment_name(u64::from(*slot))
            .unwrap_or_else(|e| unreachable!("{topic}: name: {e:?}"));
        let text = name.as_str().to_string();
        assert!(
            text.starts_with(&format!("/drm.{encoded}.")) && text.matches('/').count() == 1,
            "{topic}: deployment-scoped POSIX name, got {text}"
        );
        assert!(
            text.ends_with(&format!("{:016x}", u64::from(*slot))),
            "{topic}: the slot is the name field, got {text}"
        );
        assert!(
            names.insert(text.clone()),
            "two topics share segment {text}: the slot table is not injective"
        );
    }
    assert_eq!(names.len(), slots.len(), "one segment per deployed topic");
    let _ = std::fs::remove_dir_all(&root);
}
