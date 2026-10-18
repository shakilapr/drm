//! `drm deploy` / `drm clean` integration (Phase 1 slice 1c; REQ-TOOL-083,
//! REQ-TOOL-084, REQ-CORE-080). Runs the real binary against a scratch copy of
//! `examples/mixed_robot`; cleanup is exercised against a scratch runtime root
//! with real POSIX segments and a real `flock` — not mocks.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use drm_core::deploy::DeploymentConfig;
use drm_core::shm::{
    FlockBackend, LockBackend, LockPaths, PosixShm, SegmentName, ShmBackend, ShmError,
};

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
    let dst = std::env::temp_dir().join(format!("drm-deployclean-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_tree(&src, &dst);
    dst
}

/// Runs the `drm` binary against `root`, optionally with a scratch runtime root.
fn drm(root: &Path, runtime: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_drm"));
    command.arg("--workspace").arg(root).args(args);
    if let Some(runtime) = runtime {
        command.env("DRM_RUNTIME_ROOT", runtime);
    }
    command
        .output()
        .unwrap_or_else(|e| unreachable!("run drm: {e}"))
}

/// The deployment config the build emitted.
fn config(root: &Path) -> DeploymentConfig {
    DeploymentConfig::load(&root.join(".drm/deploy/deployment.json"))
        .unwrap_or_else(|e| unreachable!("load deployment config: {e:?}"))
}

/// REQ: REQ-TOOL-083, REQ-TOOL-084 — `drm deploy` reports the deployment
/// identity, mints it on the first deployment and REUSES it on an unchanged
/// rebuild (a re-mint would orphan the live namespace).
#[test]
fn deploy_reports_and_reuses_identity() {
    let root = scratch_example();

    let first = drm(&root, None, &["deploy"]);
    let stdout = String::from_utf8_lossy(&first.stdout).into_owned();
    assert!(
        first.status.success(),
        "drm deploy failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        stdout.contains("identity "),
        "reports the identity: {stdout}"
    );
    assert!(
        stdout.contains("(minted)"),
        "first deployment mints: {stdout}"
    );
    assert!(
        stdout.contains(" -> slot "),
        "prints the slot table a process attaches with: {stdout}"
    );
    let first_id = config(&root).deployment_id;
    assert_eq!(first_id.len(), 32, "128-bit hex identity");

    let second = drm(&root, None, &["deploy"]);
    let stdout = String::from_utf8_lossy(&second.stdout).into_owned();
    assert!(second.status.success(), "second deploy failed");
    assert!(
        stdout.contains("(reused)"),
        "an unchanged rebuild must reuse the identity: {stdout}"
    );
    assert_eq!(
        config(&root).deployment_id,
        first_id,
        "the same deployment keeps its identity"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-CORE-080 — `drm clean` refuses while a node holds the shared active
/// hold, then unlinks every deployed segment once the deployment is idle.
#[test]
fn clean_respects_the_active_guard() {
    let root = scratch_example();
    let runtime = std::env::temp_dir().join(format!(
        "drm-runtime-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&runtime);
    std::fs::create_dir_all(&runtime).unwrap_or_else(|e| unreachable!("runtime root: {e}"));

    let deployed = drm(&root, Some(&runtime), &["deploy"]);
    assert!(
        deployed.status.success(),
        "drm deploy failed: {}",
        String::from_utf8_lossy(&deployed.stderr)
    );
    let config = config(&root);
    let id = config.id().unwrap_or_else(|e| unreachable!("id: {e:?}"));
    let namespace = runtime.join(id.encoded());
    std::fs::create_dir_all(&namespace).unwrap_or_else(|e| unreachable!("namespace: {e}"));

    let names: Vec<SegmentName> = config
        .slots()
        .values()
        .map(|slot| {
            id.segment_name(u64::from(*slot))
                .unwrap_or_else(|e| unreachable!("name: {e:?}"))
        })
        .collect();
    let shm = PosixShm;
    for name in &names {
        match shm.create_exclusive(name, 64) {
            Ok(mapping) => shm.unmap(mapping),
            Err(e) => unreachable!("create {name:?}: {e:?}"),
        }
    }

    // A live node holds the shared active hold: cleanup refuses, segments stay.
    let locks = FlockBackend;
    let paths = LockPaths::new(&namespace);
    let hold = locks
        .try_shared(&paths.active())
        .unwrap_or_else(|e| unreachable!("shared hold: {e:?}"));
    let active = drm(&root, Some(&runtime), &["clean"]);
    let text = String::from_utf8_lossy(&active.stdout).into_owned();
    assert!(
        active.status.success(),
        "clean must not fail on an active deployment"
    );
    assert!(
        text.contains("active; nothing removed"),
        "active deployment is left untouched: {text}"
    );
    for name in &names {
        assert!(
            shm.open_existing(name, 64).is_ok(),
            "segment intact while live: {name:?}"
        );
    }
    locks.release(hold);

    // No holder: the namespace is unowned and every deployed segment is removed.
    let cleaned = drm(&root, Some(&runtime), &["clean"]);
    let text = String::from_utf8_lossy(&cleaned.stdout).into_owned();
    assert!(
        cleaned.status.success(),
        "clean failed: {}",
        String::from_utf8_lossy(&cleaned.stderr)
    );
    assert!(text.contains("unlinked"), "reports the teardown: {text}");
    for name in &names {
        assert!(
            matches!(shm.open_existing(name, 64), Err(ShmError::NotFound)),
            "segment unlinked: {name:?}"
        );
    }

    // Idempotent: cleaning an already-clean deployment is not an error.
    let again = drm(&root, Some(&runtime), &["clean"]);
    assert!(again.status.success(), "clean must be idempotent");

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&runtime);
}
