//! Two-process attached-channel round trip
//! ([attached-data-path.md](../../../docs/algorithms/runtime/attached-data-path.md)
//! I-AT1/I-AT6, ADR 0007).
//!
//! The parent owns the segment (publisher authority) and *publishes through the
//! attached data path*; a second process spawned from this same test binary
//! attaches read-only and reads the parent's samples back through the same path.
//! This is the falsification test for "a process cannot exchange samples with
//! another process over DRM shared memory" (ADR 0007 context).
//!
//! REQ: REQ-CORE-102, REQ-CORE-103, REQ-CORE-104.

use std::process::Command;
use std::time::Duration;

use drm_core::channel::attached::AttachedChannel;
use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{LayoutSpec, consumer_open, publisher_open};
use drm_core::channel::{ReadOutcome, ReaderState};
use drm_core::shm::{FlockBackend, LockPaths, PosixShm, SegmentName, ShmBackend};

/// Canonical topic name of the round-trip channel.
const TOPIC: &str = "drm/attached/roundtrip";
/// Synthetic topic handle (one topic per segment).
const HANDLE: u32 = 77;
/// Payload capacity of the round-trip channel.
const SAMPLE_MAX: u64 = 32;
/// History depth R.
const DEPTH: u64 = 4;
/// Pool size (must be `>= R + 1`).
const POOL: u64 = 8;
/// Slot alignment the segment is laid out with.
const SLOT_ALIGN: u64 = 8;
/// Samples the parent publishes before the child attaches.
const SAMPLES: u64 = 50;
/// Lock directory (under `target/`).
const LOCK_DIR: &str = "target/attached-locks";
/// Child selector: the child test reads the segment named by this variable.
const CHILD_ENV: &str = "DRM_ATTACHED_CHILD_SEGMENT";

/// Shared expectation for both sides of the round trip.
/// REQ: REQ-CORE-102.
fn expect() -> Expectation {
    Expectation {
        schema_hash: [3; 16],
        layout_hash: [5; 16],
        topic_handle: HANDLE,
        topic_name: TOPIC.to_string(),
        payload_capacity: SAMPLE_MAX,
        history_depth: DEPTH,
        pool_size: POOL,
        overflow_policy: 0,
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    }
}

/// Descriptor flags + slot alignment for the segment.
/// REQ: REQ-CORE-102.
fn spec() -> LayoutSpec {
    LayoutSpec {
        flags: 0,
        slot_align: SLOT_ALIGN,
    }
}

/// Deterministic payload for sequence `n` (first eight bytes carry `n`).
/// REQ: REQ-CORE-102.
fn payload_for(n: u64) -> Vec<u8> {
    let mut buf = vec![0xA5_u8; usize::try_from(SAMPLE_MAX).unwrap_or(0)];
    if let Some(head) = buf.get_mut(..8) {
        head.copy_from_slice(&n.to_le_bytes());
    }
    buf
}

/// Sequence carry of a received payload.
/// REQ: REQ-CORE-103.
fn sequence_of(buf: &[u8]) -> Result<u64, Box<dyn std::error::Error>> {
    let mut head = [0_u8; 8];
    head.copy_from_slice(buf.get(..8).ok_or("short payload")?);
    Ok(u64::from_le_bytes(head))
}

/// Parent: publishes `SAMPLES` through the attached path, then a second process
/// attaches as a consumer and reads the parent's bytes back.
///
/// # Errors
/// Propagates setup/open/IO failures (test scaffolding, not library paths).
/// REQ: REQ-CORE-102, REQ-CORE-103, REQ-CORE-104.
#[test]
fn attached_publish_read_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var(CHILD_ENV).is_ok() {
        return Ok(());
    }
    std::fs::create_dir_all(LOCK_DIR)?;
    let name = format!("/drm.attached.rt.{}", std::process::id());
    let segment = SegmentName::new(&name)?;
    let _ = PosixShm.unlink(&segment);
    let locks = LockPaths::new(LOCK_DIR);
    let expected = expect();
    let publisher = publisher_open(
        &PosixShm,
        &FlockBackend,
        &locks.publisher(HANDLE),
        &segment,
        TOPIC.as_bytes(),
        &expected,
        &spec(),
    )
    .map_err(|e| format!("publisher_open: {e:?}"))?;

    let attached = AttachedChannel::new(publisher.mapping(), &publisher.view, SLOT_ALIGN)
        .map_err(|e| format!("attached view: {e:?}"))?;

    // Publish through the attached path; the ring retains the last `DEPTH`.
    for n in 0..SAMPLES {
        attached
            .publish(&payload_for(n))
            .map_err(|e| format!("publish {n}: {e:?}"))?;
    }
    assert_eq!(attached.write_seq(), SAMPLES);

    // Second process: same binary, child selector set.
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .args(["--exact", "attached_child_reader", "--nocapture"])
        .env(CHILD_ENV, &name)
        .output()?;
    let stdout = String::from_utf8_lossy(&child.stdout);
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(
        stdout.contains("CHILD_OK"),
        "child did not report success.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("newest={}", SAMPLES - 1)),
        "child did not read the newest sample.\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("window={}", DEPTH.min(SAMPLES))),
        "child window size mismatch.\nstdout:\n{stdout}"
    );

    drop(publisher);
    let _ = PosixShm.unlink(&segment);
    Ok(())
}

/// Child: attaches read-only to the parent's segment and reads through the
/// attached path — once for the newest sample, then the retained window in
/// order — reporting what it saw.
///
/// # Errors
/// Propagates open/read failures; a missing selector is a no-op parent run.
/// REQ: REQ-CORE-103.
#[test]
fn attached_child_reader() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(name) = std::env::var(CHILD_ENV) else {
        return Ok(());
    };
    let segment = SegmentName::new(&name)?;
    let expected = expect();
    let consumer = consumer_open(
        &PosixShm,
        &segment,
        &expected,
        &spec(),
        Some(Duration::from_secs(10)),
    )
    .map_err(|e| format!("consumer_open: {e:?}"))?;
    let attached = AttachedChannel::new(consumer.mapping(), &consumer.view, SLOT_ALIGN)
        .map_err(|e| format!("attached view: {e:?}"))?;

    // 1. Newest sample.
    let mut state: ReaderState = attached.reader();
    let mut buf = vec![0_u8; usize::try_from(SAMPLE_MAX).unwrap_or(0)];
    let newest = match attached
        .read_latest(&mut state, &mut buf)
        .map_err(|e| format!("read_latest: {e:?}"))?
    {
        ReadOutcome::Sample => sequence_of(&buf)?,
        other => return Err(format!("read_latest: expected Sample, got {other:?}").into()),
    };

    // 2. Retained window in order, from a fresh reader.
    let mut state: ReaderState = attached.reader();
    let mut window: Vec<u64> = Vec::new();
    loop {
        match attached
            .read_next(&mut state, &mut buf)
            .map_err(|e| format!("read_next: {e:?}"))?
        {
            ReadOutcome::Sample => window.push(sequence_of(&buf)?),
            ReadOutcome::Empty => break,
            ReadOutcome::Gap { missed } => {
                return Err(format!("unexpected gap of {missed}").into());
            }
            ReadOutcome::WouldRetry => {
                return Err("read_next exhausted its retry budget".into());
            }
        }
    }
    // Contiguous, ascending, ending at the newest published sequence.
    let contiguous = window
        .windows(2)
        .all(|w| matches!(w, [a, b] if *b == a + 1));
    if !contiguous {
        return Err(format!("window not contiguous: {window:?}").into());
    }
    if window.last().copied() != Some(newest) {
        return Err(format!("window {window:?} does not end at newest {newest}").into());
    }
    println!("CHILD_OK newest={newest} window={}", window.len());
    Ok(())
}
