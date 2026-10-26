//! Phase 1 acceptance: two processes exchange data using only the generated
//! deployment configuration (`docs/spec/ros2-integration-plan.md` Phase 1).
//!
//! Criteria 1-5 and 7 are exercised here against the real toolchain and the real
//! runtime: the workspace is built, `deployment.json` is loaded, and both sides
//! derive their segment name, contract hashes, slot, capacity, depth, pool,
//! overflow policy and slot alignment from it. Nothing is hand-identified.
//!
//! Criterion 6 (publisher restart ⇒ epoch handling) is covered by
//! `drm_core::channel::lifecycle::restart_publication` through
//! `crates/drm-core/tests/counters.rs` (`restart_run` bumps `publisher_epoch`),
//! because an epoch is a property of a live publisher, not of the artifact.
//!
//! REQ: REQ-TOOL-083, REQ-CORE-108, REQ-CORE-088, REQ-CORE-092, REQ-CORE-107.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use drm_core::channel::attached::AttachedChannel;
use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{LayoutSpec, consumer_open, publisher_open};
use drm_core::deploy::{ContractFingerprint, DeploymentConfig, guard_fingerprint};
use drm_core::shm::{FlockBackend, LockPaths, PosixShm, SegmentName, ShmBackend};
use drm_types::Status;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Samples exchanged when both processes keep pace (ring depth is small, so a
/// reader that stalls would see a gap — that is the point of criterion 4).
const SAMPLES: u64 = 1_000;

/// A scratch workspace whose exchanged topic retains a real history.
///
/// `examples/mixed_robot` declares `control/target` with the default `latest`
/// policy (depth 1), under which a thousand-sample exchange cannot be verified
/// against the ring — criterion 4 explicitly asks for "a history policy that
/// retains them", so the fixture declares one.
fn scratch_workspace() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dst = std::env::temp_dir().join(format!("drm-phase1-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    let topics = dst.join("packages/counter/topics");
    let producer = dst.join("packages/counter/nodes/producer");
    let consumer = dst.join("packages/counter/nodes/consumer");
    for dir in [&topics, &producer, &consumer] {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| unreachable!("mkdir {dir:?}: {e}"));
    }
    // Depth 1024 > the 1000 exchanged samples: the ring itself can retain the
    // whole run, so an in-order read is a property of the path, not of timing.
    std::fs::write(
        topics.join("value.drm"),
        "@history: 1024\n@pool_size: 2048\n@overflow: overwrite\nvalue: u64\n",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    std::fs::write(
        producer.join("drm.toml"),
        "name = \"producer\"\npublishes = [\"counter/value\"]\nconsumes = []\n\n[build]\ncommand = ['sh', '-c', 'test -n \"$DRM_GEN_INCLUDE\" && test -f \"$DRM_DEPLOY_METADATA\"']\n",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    std::fs::write(
        consumer.join("drm.toml"),
        "name = \"consumer\"\npublishes = []\nconsumes = [\"counter/value\"]\n\n[build]\ncommand = ['sh', '-c', 'test -n \"$DRM_GEN_INCLUDE\" && test -f \"$DRM_DEPLOY_METADATA\"']\n",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    dst
}

/// Runs the `drm` binary against `root`.
fn drm(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drm"))
        .arg("--workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap_or_else(|e| unreachable!("run drm: {e}"))
}

/// Builds the example and loads the emitted deployment configuration.
fn built(tag: &str) -> (PathBuf, DeploymentConfig) {
    let root = scratch_workspace();
    let out = drm(&root, &["build"]);
    assert!(
        out.status.success(),
        "{tag}: drm build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config = DeploymentConfig::load(&root.join(".drm/deploy/deployment.json"))
        .unwrap_or_else(|e| unreachable!("{tag}: load deployment config: {e:?}"));
    (root, config)
}

/// The per-topic expectation a node derives from the artifact alone.
fn expectation_from(config: &DeploymentConfig, topic: &str) -> (Expectation, LayoutSpec) {
    let entry = config
        .topic(topic)
        .unwrap_or_else(|| unreachable!("deployment declares no topic `{topic}`"));
    let expected = Expectation {
        schema_hash: hex16(&entry.schema_hash),
        layout_hash: hex16(&entry.layout_hash),
        topic_handle: u32::try_from(entry.slot).unwrap_or_else(|e| unreachable!("slot: {e}")),
        topic_name: topic.to_string(),
        payload_capacity: entry.payload_capacity,
        history_depth: entry.history_depth,
        pool_size: entry.pool_size,
        overflow_policy: u8::from(entry.overflow == "reject"),
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    };
    let spec = LayoutSpec {
        flags: 0,
        slot_align: entry.slot_align,
    };
    (expected, spec)
}

/// Parses a 32-lowercase-hex digest.
fn hex16(text: &str) -> [u8; 16] {
    assert_eq!(text.len(), 32, "digest width");
    let mut out = [0_u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let pair = text.get(i * 2..i * 2 + 2).unwrap_or_else(|| unreachable!());
        *slot = u8::from_str_radix(pair, 16).unwrap_or_else(|e| unreachable!("hex: {e}"));
    }
    out
}

/// Phase 1 criteria 1-5: two processes, one generated configuration, a thousand
/// numbered samples in order, and the refusals that make the identity meaningful.
/// Phase 1 criterion 5: the artifact's fingerprints refuse a mismatched or
/// undeployed contract, so the exchange above is not merely two processes that
/// happened to agree.
#[test]
fn deployment_refuses_mismatched_contracts() {
    let (root, config) = built("phase1-guard");
    let controller = config
        .expectation("producer")
        .unwrap_or_else(|| unreachable!("deployment records no producer fingerprint"));
    assert_eq!(
        guard_fingerprint(controller, config.expectation("producer")),
        Ok(()),
        "a matching contract starts"
    );
    assert_ne!(
        controller,
        ContractFingerprint([0; 16]),
        "the artifact carries a real fingerprint, not a placeholder"
    );
    assert_eq!(
        guard_fingerprint(controller, config.expectation("consumer")),
        Err(Status::WrongAbi),
        "another node's contract is refused"
    );
    assert_eq!(
        guard_fingerprint(controller, None),
        Err(Status::NotFound),
        "an undeployed node is refused"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Phase 1 criteria 1-4: two processes exchange a thousand numbered samples
/// using only the generated configuration.
#[test]
fn deployment_provisions_two_processes() {
    let (root, config) = built("phase1");
    let id = config
        .id()
        .unwrap_or_else(|e| unreachable!("deployment id: {e:?}"));
    let topic = "counter/value";
    let (expected, spec) = expectation_from(&config, topic);
    let segment = id
        .segment_name(expected.topic_handle.into())
        .unwrap_or_else(|e| unreachable!("segment name: {e:?}"));
    let _ = PosixShm.unlink(&segment);
    let lock_dir = root.join("target/phase1-locks");
    std::fs::create_dir_all(&lock_dir).unwrap_or_else(|e| unreachable!("lock dir: {e}"));
    let locks = LockPaths::new(lock_dir.display().to_string());

    // Criteria 1-3: one publisher process (this one) and one consumer process
    // (a child of this binary), both from the same artifact.
    let publisher = publisher_open(
        &PosixShm,
        &FlockBackend,
        &locks.publisher(expected.topic_handle),
        &segment,
        topic.as_bytes(),
        &expected,
        &spec,
    )
    .unwrap_or_else(|e| unreachable!("publisher_open: {e:?}"));
    let writer = AttachedChannel::new(publisher.mapping(), &publisher.view, spec.slot_align)
        .unwrap_or_else(|e| unreachable!("attached view: {e:?}"));

    // Criterion 5b: a second live publisher is refused (single-writer authority).
    let second = publisher_open(
        &PosixShm,
        &FlockBackend,
        &locks.publisher(expected.topic_handle),
        &segment,
        topic.as_bytes(),
        &expected,
        &spec,
    );
    assert!(
        second.is_err(),
        "a second live publisher must be refused, got {second:?}"
    );

    // Criterion 4: 1,000 numbered samples, reader keeping pace. The child runs
    // concurrently and reports the sequences it saw.
    // The ring retains `history_depth` samples, so a reader that attaches after a
    // burst sees only its tail. Criterion 4 says the reader keeps pace — so the
    // child announces that it is attached, and only then does publishing start.
    let ready = root.join("target/phase1-ready");
    let _ = std::fs::remove_file(&ready);
    let child = Command::new(std::env::current_exe().unwrap_or_else(|e| unreachable!("{e}")))
        .args(["--exact", "deployment_child_reader", "--nocapture"])
        .env("DRM_PHASE1_CHILD", segment.as_str())
        .env("DRM_PHASE1_TOPIC", topic)
        .env("DRM_PHASE1_ROOT", &root)
        // Let the child's own panic message through: it is the only place a
        // failed attach or a wrong sequence is reported.
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| unreachable!("spawn child: {e}"));

    let wait_deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !ready.exists() && std::time::Instant::now() < wait_deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(ready.exists(), "the consumer process never attached");

    let mut payload = vec![0_u8; usize::try_from(expected.payload_capacity).unwrap_or(0)];
    for n in 0..SAMPLES {
        if let Some(head) = payload.get_mut(..8) {
            head.copy_from_slice(&n.to_le_bytes());
        }
        writer
            .publish(&payload)
            .unwrap_or_else(|e| unreachable!("publish {n}: {e:?}"));
        // Paced so the reader can follow: the ring is `history_depth` deep, and
        // the point of the criterion is an in-order exchange, not a burst.
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    let output = child
        .wait_with_output()
        .unwrap_or_else(|e| unreachable!("{e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The child's own report is the evidence: it must have read every sample, in
    // order, from a segment it resolved out of the same artifact.
    println!("consumer process: {}", stdout.trim());
    assert!(
        stdout.contains(&format!("CHILD_OK read={SAMPLES} in_order=yes")),
        "the consumer did not read the whole exchange"
    );
    assert!(
        stdout.contains("CHILD_OK"),
        "the consumer process did not succeed (status {:?}):\nstdout:\n{stdout}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = PosixShm.unlink(&segment);
    let _ = std::fs::remove_dir_all(&root);
}

/// The consumer process: attaches using only the generated configuration and
/// verifies the samples arrive in order and intact.
///
/// A no-op in the parent run (no selector set).
#[test]
fn deployment_child_reader() {
    let Ok(name) = std::env::var("DRM_PHASE1_CHILD") else {
        return;
    };
    let topic = std::env::var("DRM_PHASE1_TOPIC").unwrap_or_else(|e| unreachable!("{e}"));
    let root = std::env::var("DRM_PHASE1_ROOT").unwrap_or_else(|e| unreachable!("{e}"));
    let config = DeploymentConfig::load(&PathBuf::from(&root).join(".drm/deploy/deployment.json"))
        .unwrap_or_else(|e| unreachable!("child: load config: {e:?}"));
    let (expected, spec) = expectation_from(&config, &topic);
    let segment = SegmentName::new(&name).unwrap_or_else(|e| unreachable!("{e:?}"));
    let lock_dir = PathBuf::from(&root).join("target/phase1-locks");

    let consumer = consumer_open(
        &PosixShm,
        &segment,
        &expected,
        &spec,
        Some(std::time::Duration::from_secs(20)),
    )
    .unwrap_or_else(|e| unreachable!("child: consumer_open: {e:?}"));
    let reader = AttachedChannel::new(consumer.mapping(), &consumer.view, spec.slot_align)
        .unwrap_or_else(|e| unreachable!("child: attached view: {e:?}"));
    // Announce the attachment so the publisher does not start before the reader
    // can keep pace (the ring retains only `history_depth` samples).
    std::fs::write(
        lock_dir
            .parent()
            .unwrap_or_else(|| unreachable!())
            .join("phase1-ready"),
        b"1",
    )
    .unwrap_or_else(|e| unreachable!("child: ready marker: {e}"));

    let mut state = reader.reader();
    let mut buf = vec![0_u8; usize::try_from(expected.payload_capacity).unwrap_or(0)];
    let mut seen: Vec<u64> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while seen.len() < usize::try_from(SAMPLES).unwrap_or(0) && std::time::Instant::now() < deadline
    {
        match reader.read_next(&mut state, &mut buf) {
            Ok(drm_core::channel::ReadOutcome::Sample) => {
                let mut head = [0_u8; 8];
                head.copy_from_slice(buf.get(..8).unwrap_or_else(|| unreachable!()));
                seen.push(u64::from_le_bytes(head));
            }
            // A reader that keeps pace sees no gap; one that falls behind would.
            Ok(drm_core::channel::ReadOutcome::Gap { missed }) => {
                unreachable!("child: unexpected gap of {missed}")
            }
            Ok(drm_core::channel::ReadOutcome::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(drm_core::channel::ReadOutcome::WouldRetry) => {}
            Err(e) => unreachable!("child: read: {e:?}"),
        }
    }
    assert_eq!(
        seen.len(),
        usize::try_from(SAMPLES).unwrap_or(0),
        "child read {} of {SAMPLES} samples",
        seen.len()
    );
    let in_order = seen
        .iter()
        .enumerate()
        .all(|(i, seq)| u64::try_from(i).ok() == Some(*seq));
    assert!(in_order, "child saw samples out of sequence");
    println!("CHILD_OK read={} in_order=yes", seen.len());
}
