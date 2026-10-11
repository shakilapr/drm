//! Record-core suite (`docs/algorithms/ops/record-replay.md` §Invariants
//! I-RC1/I-RC2/I-RC3/I-RC5, §Test Matrix `record_borrow_scope`,
//! `record_bounded`, `record_policies`, `record_gap_ledger`).
//! REQ: REQ-OPS-005, REQ-OPS-007, REQ-OPS-008, REQ-OPS-018, REQ-OPS-019.

use super::{
    DescEntry, GapLedger, GapReason, IngressStep, OnFull, PendingGap, Recorder, RecorderConfig,
    StopReason, TopicSpec,
};
use crate::disk::{Config as DiskConfig, Stage};
use crate::mcap::Compression;
use crate::replay::{ChannelSpec, Rate, ReplayNamespace, ReplayPublisher, SchedulerMode, Session};
use drm_core::channel::{Channel, OverflowPolicy};
use drm_core::deploy::DeploymentId;
use drm_types::Status;

fn chan(depth: u64, pool: u64) -> Channel {
    Channel::activate(depth, pool, 16, 8, OverflowPolicy::Overwrite, 0, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

fn spec(topic_id: u32, slot_bytes: usize, slot_count: usize, ledger: usize) -> TopicSpec {
    TopicSpec {
        topic_id,
        slot_bytes,
        slot_count,
        ledger_capacity: ledger,
    }
}

fn build(specs: Vec<TopicSpec>, on_full: OnFull, desc_capacity: usize) -> Recorder {
    let buffer_bytes = Recorder::required_bytes(&specs, desc_capacity)
        .unwrap_or_else(|e| unreachable!("required_bytes: {e:?}"));
    let cfg = RecorderConfig {
        buffer_bytes,
        on_full,
        chunk_target: 4096,
        desc_capacity,
    };
    Recorder::new(cfg, specs).unwrap_or_else(|e| unreachable!("new: {e:?}"))
}

/// REQ: REQ-OPS-005, REQ-OPS-007 — I-RC1: a DRM borrow is held only across the
/// copy into a slab slot; after every step zero borrows are held, disk-stage
/// drains hold none, and no two borrows ever overlap.
#[test]
fn record_borrow_scope() {
    let ch = chan(8, 9);
    let mut rec = build(vec![spec(7, 16, 4, 4)], OnFull::Drop, 4);
    for i in 0..200_u64 {
        let byte = u8::try_from(i % 251).unwrap_or(0);
        assert!(ch.publish(&[byte; 16]).is_ok());
        let step = rec
            .ingress_step(0, &ch, 0, i * 1_000, i * 2_000)
            .unwrap_or_else(|e| unreachable!("ingress: {e:?}"));
        assert_eq!(step, IngressStep::Recorded);
        assert_eq!(rec.active_borrows(), 0, "borrow released after the copy");
        while let Some(desc) = rec.drain_one() {
            assert_eq!(
                rec.active_borrows(),
                0,
                "disk-stage work holds zero DRM borrows"
            );
            assert_eq!(desc.len, 16);
        }
    }
    assert_eq!(rec.max_active_borrows(), 1, "at most one concurrent borrow");
    assert_eq!(rec.disk_ops(), 200);
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("slab"))
            .free_count(),
        4,
        "every slot returned after draining"
    );
}

/// REQ: REQ-OPS-019 — I-RC2: recorder memory never exceeds `buffer_bytes`; a
/// small topic pays only its own bound; sustained overload leaves the fixed
/// accounting unchanged.
#[test]
fn record_bounded() {
    const CAMERA: usize = 8 * 1024 * 1024;
    const IMU: usize = 128;
    let specs = vec![spec(1, CAMERA, 1, 4), spec(2, IMU, 4, 4)];
    let desc_capacity = 8;
    let need = Recorder::required_bytes(&specs, desc_capacity)
        .unwrap_or_else(|e| unreachable!("required: {e:?}"));

    let mut rec = build(specs.clone(), OnFull::Drop, desc_capacity);
    assert_eq!(rec.memory_bytes(), need, "exact honest budget");
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("cam"))
            .slot_bytes(),
        CAMERA
    );
    assert_eq!(
        rec.slab(1)
            .unwrap_or_else(|| unreachable!("imu"))
            .slot_bytes(),
        IMU
    );
    assert!(
        need >= u64::try_from(CAMERA + IMU * 4).unwrap_or(0),
        "IMU slots cost IMU-bound bytes only"
    );

    let under = RecorderConfig {
        buffer_bytes: need - 1,
        on_full: OnFull::Drop,
        chunk_target: 4096,
        desc_capacity,
    };
    assert_eq!(
        Recorder::new(under, specs.clone()).err(),
        Some(Status::InvalidConfig),
        "one byte under budget is refused"
    );

    let ch = chan(4, 5);
    for i in 0..64_u64 {
        assert!(ch.publish(&[0x5A; 16]).is_ok());
        let _ = rec.ingress_step(0, &ch, 0, i, 0);
    }
    assert_eq!(rec.memory_bytes(), need, "overload never grows memory");
    assert!(rec.queued() <= desc_capacity);
}

/// REQ: REQ-OPS-008 — I-RC3: `Drop` coalesces into the ledger and frees nothing
/// on the data path; `Stop` aborts `BUFFER_EXHAUSTED`; `Backpressure` pauses
/// with the slot retained and loses nothing.
#[test]
fn record_policies() {
    // Drop: discard + coalesce, never a payload write.
    let ch = chan(8, 9);
    let specs = vec![spec(1, 16, 1, 4)];
    let mut drop = build(specs.clone(), OnFull::Drop, 8);
    assert!(ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        drop.ingress_step(0, &ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(ch.publish(&[2_u8; 16]).is_ok());
    assert_eq!(
        drop.ingress_step(0, &ch, 0, 1, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::PolicyDropped
    );
    assert!(ch.publish(&[3_u8; 16]).is_ok());
    assert_eq!(
        drop.ingress_step(0, &ch, 0, 2, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::PolicyDropped
    );
    let gaps: Vec<PendingGap> = drop
        .ledger(0)
        .unwrap_or_else(|| unreachable!("ledger"))
        .iter()
        .copied()
        .collect();
    assert_eq!(
        gaps,
        vec![PendingGap {
            first_seq: 1,
            last_seq: 2,
            reason: GapReason::PolicyDrop,
        }],
        "drops coalesce into one ledger range"
    );
    assert!(drop.drain_one().is_some());
    assert!(ch.publish(&[4_u8; 16]).is_ok());
    assert_eq!(
        drop.ingress_step(0, &ch, 0, 3, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded,
        "recording resumes after the slot is drained"
    );

    // Stop: abort with the explicit reason; further steps stay aborted.
    let stop_ch = chan(8, 9);
    let mut stop = build(specs.clone(), OnFull::Stop, 8);
    assert!(stop_ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        stop.ingress_step(0, &stop_ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(stop_ch.publish(&[2_u8; 16]).is_ok());
    assert_eq!(
        stop.ingress_step(0, &stop_ch, 0, 1, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Aborted
    );
    assert!(stop.is_aborted());
    assert_eq!(stop.stop_reason(), Some(StopReason::BufferExhausted));
    assert_eq!(
        stop.ingress_step(0, &stop_ch, 0, 2, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Aborted
    );

    // Backpressure: pause, retain the slot, lose nothing.
    let bp_ch = chan(8, 9);
    let mut bp = build(specs.clone(), OnFull::Backpressure, 8);
    assert!(bp_ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        bp.ingress_step(0, &bp_ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(bp_ch.publish(&[2_u8; 16]).is_ok());
    assert_eq!(
        bp.ingress_step(0, &bp_ch, 0, 1, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Paused
    );
    assert_eq!(bp.active_borrows(), 0, "paused ingress borrows nothing");
    let first = bp.drain_one().unwrap_or_else(|| unreachable!("desc"));
    assert_eq!(first.seq, 0);
    assert_eq!(
        bp.ingress_step(0, &bp_ch, 0, 2, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded,
        "the retained sample is recorded once space frees"
    );
    assert_eq!(
        bp.ledger(0)
            .unwrap_or_else(|| unreachable!("ledger"))
            .live_ranges(),
        0,
        "backpressure loses nothing"
    );
}

/// REQ: REQ-OPS-018 — I-RC5: history laps, policy drops, and epoch boundaries
/// land in the ledger as coalesced ranges; overflow merges conservatively and
/// sets `gap_detail_lost`.
#[test]
fn record_gap_ledger() {
    // History lap: a shallow ring overwrites seq 1..=3 before ingress reads.
    let ch = chan(1, 2);
    let mut rec = build(vec![spec(1, 16, 8, 4)], OnFull::Drop, 8);
    assert!(ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    for _ in 0..4 {
        assert!(ch.publish(&[2_u8; 16]).is_ok());
    }
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 1, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Gap
    );
    let gaps: Vec<PendingGap> = rec
        .ledger(0)
        .unwrap_or_else(|| unreachable!("ledger"))
        .iter()
        .copied()
        .collect();
    assert_eq!(
        gaps,
        vec![PendingGap {
            first_seq: 1,
            last_seq: 3,
            reason: GapReason::HistoryLap,
        }]
    );

    // Epoch boundary: a publisher restart coalesces the unrecorded range.
    let epoch_ch = chan(4, 5);
    let mut epoch = build(vec![spec(1, 16, 4, 4)], OnFull::Drop, 8);
    assert!(epoch_ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        epoch
            .ingress_step(0, &epoch_ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    let _ = epoch_ch.restart_run();
    assert!(epoch_ch.publish(&[2_u8; 16]).is_ok());
    let _ = epoch
        .ingress_step(0, &epoch_ch, 0, 1, 0)
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert!(
        epoch
            .ledger(0)
            .unwrap_or_else(|| unreachable!("ledger"))
            .iter()
            .any(|g| g.reason == GapReason::PublisherEpoch),
        "epoch boundary accounted"
    );

    // Ledger overflow: conservative merge + `gap_detail_lost`.
    let mut ledger = GapLedger::new(1);
    ledger.coalesce(1, GapReason::HistoryLap);
    assert_eq!(ledger.live_ranges(), 1);
    assert!(!ledger.gap_detail_lost());
    ledger.coalesce(5, GapReason::PolicyDrop);
    assert!(ledger.gap_detail_lost(), "overflow flagged");
    assert_eq!(
        ledger.live_ranges(),
        1,
        "conservative merge keeps one range"
    );
    assert_eq!(
        ledger.iter().copied().collect::<Vec<_>>(),
        vec![PendingGap {
            first_seq: 1,
            last_seq: 5,
            reason: GapReason::HistoryLap,
        }]
    );
}

/// REQ: REQ-OPS-007, REQ-OPS-006 — the descriptor carries the copied length,
/// sequence, both receive clocks, and the recorder-observed ingress order.
#[test]
fn desc_fields() {
    let ch = chan(4, 5);
    let mut rec = build(vec![spec(3, 16, 2, 2)], OnFull::Drop, 4);
    assert!(ch.publish(&[0xAB_u8; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 11, 22)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    let desc: DescEntry = rec.drain_one().unwrap_or_else(|| unreachable!("desc"));
    assert_eq!(desc.topic_id, 3);
    assert_eq!(desc.len, 16);
    assert_eq!(desc.seq, 0);
    assert_eq!(desc.recv_mono_ns, 11);
    assert_eq!(desc.recv_wall_ns, 22);
    assert_eq!(desc.ingress_order, 0);
    assert_eq!(desc.publisher_epoch, 0);
    assert_eq!(rec.chunk_target(), 4096);
    assert_eq!(rec.on_full(), OnFull::Drop);
    assert_eq!(rec.topic_count(), 1);
}

fn disk_cfg(compression: Compression) -> DiskConfig {
    DiskConfig {
        chunk_target: 4096,
        compression,
    }
}

/// Reads a `u64` record length from an 8-byte slice.
fn read_len(slice: &[u8]) -> usize {
    let arr: [u8; 8] = slice.try_into().unwrap_or([0; 8]);
    usize::try_from(u64::from_le_bytes(arr)).unwrap_or(0)
}

/// Scans an MCAP byte stream for `drm.sample`/`drm.gap` metadata records,
/// decompressing nothing (uncompressed chunks only). Uses the third-party
/// `mcap` reader's record parser (`mcap_drm_meta`; I-RC6).
fn scan_metadata(bytes: &[u8]) -> Vec<mcap::records::Metadata> {
    let mut out = Vec::new();
    let mut i = crate::mcap::MAGIC.len();
    while i + 9 <= bytes.len() {
        let op = bytes[i];
        let len = read_len(&bytes[i + 1..i + 9]);
        let Some(body) = bytes.get(i + 9..i + 9 + len) else {
            break;
        };
        match mcap::parse_record(op, body) {
            Ok(mcap::records::Record::Chunk { data, .. }) => scan_inner(&data, &mut out),
            Ok(mcap::records::Record::Metadata(m)) => out.push(m),
            _ => {}
        }
        i += 9 + len;
    }
    out
}

/// Scans chunk-contained records for metadata (`scan_metadata` helper).
fn scan_inner(data: &[u8], out: &mut Vec<mcap::records::Metadata>) {
    let mut j = 0;
    while j + 9 <= data.len() {
        let op = data[j];
        let len = read_len(&data[j + 1..j + 9]);
        let Some(body) = data.get(j + 9..j + 9 + len) else {
            break;
        };
        if let Ok(mcap::records::Record::Metadata(m)) = mcap::parse_record(op, body) {
            out.push(m);
        }
        j += 9 + len;
    }
}

/// REQ: REQ-OPS-006, REQ-OPS-007 — the disk stage drains the descriptor ring
/// into a well-formed MCAP stream (magic at both ends), frees every slab slot,
/// and holds zero DRM borrows throughout.
#[test]
fn record_disk_mcap() {
    let ch = chan(8, 9);
    let mut rec = build(vec![spec(1, 16, 5, 4)], OnFull::Drop, 5);
    for i in 0..5_u64 {
        assert!(ch.publish(&[u8::try_from(i).unwrap_or(0); 16]).is_ok());
        assert_eq!(
            rec.ingress_step(0, &ch, 0, i, i)
                .unwrap_or_else(|e| unreachable!("{e:?}")),
            IngressStep::Recorded
        );
    }
    let stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(1, "imu")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    let (summary, bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    assert_eq!(summary.messages, 5);
    assert_eq!(summary.gaps, 0);
    assert_eq!(summary.dropped, 0);
    assert!(summary.chunks >= 1, "at least one chunk");
    let magic = crate::mcap::MAGIC;
    assert_eq!(bytes[..magic.len()], magic);
    assert_eq!(&bytes[bytes.len() - magic.len()..], &magic);
    assert_eq!(rec.active_borrows(), 0, "disk stage holds no DRM borrow");
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("slab"))
            .free_count(),
        5,
        "every slot returned after the disk stage"
    );
}

/// REQ: REQ-OPS-006 — I-RC6: every recorded message carries a `drm.sample`
/// metadata record with the full `(publisher_epoch, seq, ingress_order)`; a
/// policy drop lands as a coalesced `drm.gap` marker. Parsed by the third-party
/// `mcap` reader.
#[test]
fn record_mcap_drm_meta() {
    let ch = chan(4, 5);
    let mut rec = build(vec![spec(7, 16, 2, 8)], OnFull::Drop, 2);
    assert!(ch.publish(&[0xAA_u8; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 100, 200)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(ch.publish(&[0xBB_u8; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 110, 210)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(ch.publish(&[0xCC_u8; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 120, 220)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::PolicyDropped
    );

    let stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(7, "imu")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    let (summary, bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    assert_eq!(summary.messages, 2);
    assert_eq!(summary.gaps, 1);
    assert_eq!(summary.dropped, 1);

    let meta = scan_metadata(&bytes);
    let samples: Vec<&mcap::records::Metadata> =
        meta.iter().filter(|m| m.name == "drm.sample").collect();
    assert_eq!(samples.len(), 2, "one drm.sample per recorded message");
    assert_eq!(
        samples[0].metadata.get("seq").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        samples[0].metadata.get("ingress_order").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        samples[1].metadata.get("seq").map(String::as_str),
        Some("1")
    );
    for s in &samples {
        assert_eq!(s.metadata.get("topic").map(String::as_str), Some("7"));
        assert_eq!(
            s.metadata.get("publisher_epoch").map(String::as_str),
            Some("0")
        );
    }
    let gaps: Vec<&mcap::records::Metadata> = meta.iter().filter(|m| m.name == "drm.gap").collect();
    assert_eq!(gaps.len(), 1, "one coalesced gap marker");
    assert_eq!(
        gaps[0].metadata.get("reason").map(String::as_str),
        Some("policy_drop")
    );
    assert_eq!(
        gaps[0]
            .metadata
            .get("first_missing_seq")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        gaps[0].metadata.get("missed").map(String::as_str),
        Some("1")
    );
}

/// REQ: REQ-OPS-006, REQ-OPS-009 — zstd chunk compression keeps the stream
/// parseable by the third-party reader.
#[test]
fn record_mcap_zstd() {
    let ch = Channel::activate(8, 9, 64, 8, OverflowPolicy::Overwrite, 0, 2, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"));
    let mut rec = build(vec![spec(1, 64, 8, 8)], OnFull::Drop, 8);
    let mut stage = Stage::new(Vec::new(), &disk_cfg(Compression::Zstd), &[(1, "imu")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    for i in 0..40_u64 {
        assert!(
            ch.publish(&[u8::try_from(i % 251).unwrap_or(0); 64])
                .is_ok()
        );
        assert_eq!(
            rec.ingress_step(0, &ch, 0, i, i)
                .unwrap_or_else(|e| unreachable!("{e:?}")),
            IngressStep::Recorded
        );
        let _ = stage
            .pump(&mut rec, 1)
            .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
    }
    let (summary, bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    assert!(summary.compressed);
    assert_eq!(summary.messages, 40);
    let messages: Vec<mcap::Message<'static>> = mcap::MessageStream::new(&bytes)
        .unwrap_or_else(|e| unreachable!("stream: {e:?}"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| unreachable!("messages: {e:?}"));
    assert_eq!(messages.len(), 40);
    assert_eq!(messages[0].channel.topic, "imu");
    assert_eq!(messages[0].data.len(), 64);
    assert_eq!(messages[39].sequence, 39);
}

/// REQ: REQ-OPS-007, REQ-OPS-008 — I-RC1/I-RC3: a slow disk (bounded `pump`)
/// lets the ingress policy act; `Drop` coalesces the backlog into the ledger
/// while `Backpressure` loses nothing.
#[test]
fn record_slow_disk_policy() {
    let ch = chan(8, 9);
    let mut rec = build(vec![spec(1, 16, 1, 16)], OnFull::Drop, 4);
    let mut stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(1, "t")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    for i in 0..12_u64 {
        assert!(ch.publish(&[u8::try_from(i).unwrap_or(0); 16]).is_ok());
        let _ = rec.ingress_step(0, &ch, 0, i, i);
        if i % 3 == 2 {
            let _ = stage
                .pump(&mut rec, 1)
                .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
        }
    }
    let (summary, _bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    assert_eq!(
        summary.messages + summary.dropped,
        12,
        "every sample is recorded or policy-dropped"
    );
    assert!(summary.dropped >= 1, "slow disk exercised the Drop policy");
    assert!(summary.gap_sequences >= summary.dropped);
    assert_eq!(rec.active_borrows(), 0);

    let bp_ch = chan(8, 9);
    let mut bp = build(vec![spec(1, 16, 1, 16)], OnFull::Backpressure, 4);
    let mut bp_stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(1, "t")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    for i in 0..12_u64 {
        assert!(bp_ch.publish(&[u8::try_from(i).unwrap_or(0); 16]).is_ok());
        let _ = bp.ingress_step(0, &bp_ch, 0, i, i);
        let _ = bp_stage
            .pump(&mut bp, 1)
            .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
    }
    let (bp_summary, _bytes) = bp_stage
        .finish(&mut bp)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    assert_eq!(bp_summary.dropped, 0, "backpressure loses nothing");
    assert_eq!(bp_summary.gap_sequences, 0);
}

/// REQ: REQ-OPS-008, REQ-OPS-019 — I-RC2/I-RC3: the slab pool admits exactly
/// `slot_count` samples; the next sample hits the boundary and follows the
/// policy, and freeing one slot restores admission (no off-by-one in the
/// accounting). `Backpressure` pauses at the same boundary with the slot
/// retained and loses nothing.
#[test]
fn record_slab_full_boundary_exact() {
    let ch = chan(8, 9);
    let mut rec = build(vec![spec(1, 16, 2, 8)], OnFull::Drop, 8);
    for i in 0..2_u64 {
        assert!(ch.publish(&[u8::try_from(i).unwrap_or(0); 16]).is_ok());
        assert_eq!(
            rec.ingress_step(0, &ch, 0, i, i)
                .unwrap_or_else(|e| unreachable!("{e:?}")),
            IngressStep::Recorded,
            "sample {i} admitted at the boundary"
        );
    }
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("slab"))
            .free_count(),
        0,
        "exactly capacity slots consumed"
    );
    assert!(ch.publish(&[0xEE; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 2, 2)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::PolicyDropped,
        "one past capacity drops"
    );
    assert_eq!(
        rec.ledger(0)
            .unwrap_or_else(|| unreachable!("ledger"))
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![PendingGap {
            first_seq: 2,
            last_seq: 2,
            reason: GapReason::PolicyDrop,
        }],
        "the boundary drop is exact"
    );
    assert!(rec.drain_one().is_some(), "frees exactly one slot");
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("slab"))
            .free_count(),
        1
    );
    assert!(ch.publish(&[0x11; 16]).is_ok());
    assert_eq!(
        rec.ingress_step(0, &ch, 0, 3, 3)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded,
        "admission resumes after a slot frees"
    );
    assert_eq!(rec.active_borrows(), 0);

    let bp_ch = chan(8, 9);
    let mut bp = build(vec![spec(1, 16, 1, 8)], OnFull::Backpressure, 8);
    assert!(bp_ch.publish(&[1_u8; 16]).is_ok());
    assert_eq!(
        bp.ingress_step(0, &bp_ch, 0, 0, 0)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Recorded
    );
    assert!(bp_ch.publish(&[2_u8; 16]).is_ok());
    assert_eq!(
        bp.ingress_step(0, &bp_ch, 0, 1, 1)
            .unwrap_or_else(|e| unreachable!("{e:?}")),
        IngressStep::Paused,
        "backpressure pauses at the boundary"
    );
    assert_eq!(bp.active_borrows(), 0);
    assert_eq!(
        bp.ledger(0)
            .unwrap_or_else(|| unreachable!("ledger"))
            .live_ranges(),
        0,
        "no loss at the boundary"
    );
}

/// REQ: REQ-OPS-007 — I-RC1: the disk stage runs on a dedicated OS thread,
/// holds no DRM handle, and the recorder observes zero outstanding borrows
/// (thread-affinity isolation).
#[test]
fn record_disk_isolation() {
    fn assert_send<T: Send>() {}
    assert_send::<Stage<Vec<u8>>>();

    let ch = chan(8, 9);
    let mut rec = build(vec![spec(1, 16, 4, 4)], OnFull::Drop, 4);
    for i in 0..20_u64 {
        assert!(ch.publish(&[u8::try_from(i).unwrap_or(0); 16]).is_ok());
        let _ = rec.ingress_step(0, &ch, 0, i, i);
    }
    let (summary, _bytes) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            let stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(1, "t")])
                .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
            stage
                .finish(&mut rec)
                .unwrap_or_else(|e| unreachable!("finish: {e:?}"))
        });
        handle
            .join()
            .unwrap_or_else(|_| unreachable!("disk thread panicked"))
    });
    assert_eq!(summary.messages + summary.dropped, 20);
    assert_eq!(
        rec.active_borrows(),
        0,
        "no borrow survives the disk thread"
    );
    assert!(
        rec.max_active_borrows() <= 1,
        "at most one concurrent borrow"
    );
    assert_eq!(
        rec.slab(0)
            .unwrap_or_else(|| unreachable!("slab"))
            .free_count(),
        4
    );
}

/// Records a three-topic session with an injected history lap (topic 2) and a
/// policy-drop run (topic 3), returning the MCAP bytes and the record summary.
/// REQ: REQ-OPS-006, REQ-OPS-009.
fn record_known_session() -> (Vec<u8>, crate::disk::SessionSummary) {
    let ch1 = chan(8, 9);
    let ch2 = chan(1, 2);
    let ch3 = chan(8, 9);
    let specs = vec![spec(1, 16, 8, 8), spec(2, 16, 8, 8), spec(3, 16, 1, 8)];
    let mut rec = build(specs, OnFull::Drop, 32);
    let mut t = 1_000_u64;
    let mut step = |rec: &mut Recorder, topic: usize, ch: &Channel| {
        t += 1_000;
        let _ = rec.ingress_step(topic, ch, 0, t, t * 2);
    };

    // Topic 1: four clean samples.
    for i in 0..4_u8 {
        assert!(ch1.publish(&[0x10 + i; 16]).is_ok());
        step(&mut rec, 0, &ch1);
    }
    // Topic 2: one recorded, a history lap, then the newest sample recorded.
    assert!(ch2.publish(&[0x20_u8; 16]).is_ok());
    step(&mut rec, 1, &ch2);
    for _ in 0..4 {
        assert!(ch2.publish(&[0x21_u8; 16]).is_ok());
    }
    step(&mut rec, 1, &ch2); // Gap missed = 3
    step(&mut rec, 1, &ch2); // catches up to the newest retained sample
    // Topic 3: one recorded, then two policy drops (single slab slot).
    assert!(ch3.publish(&[0x30_u8; 16]).is_ok());
    step(&mut rec, 2, &ch3);
    assert!(ch3.publish(&[0x31_u8; 16]).is_ok());
    step(&mut rec, 2, &ch3);
    assert!(ch3.publish(&[0x32_u8; 16]).is_ok());
    step(&mut rec, 2, &ch3);

    let stage = Stage::new(
        Vec::new(),
        &disk_cfg(Compression::None),
        &[(1, "a"), (2, "b"), (3, "c")],
    )
    .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    let (summary, bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    (bytes, summary)
}

/// REQ: REQ-OPS-009, REQ-OPS-006, REQ-OPS-018 — I-RC4: record → wipe → replay
/// preserves payload bytes, per-topic `(publisher_epoch, seq)` order, the
/// recorder-observed global order `(recv_mono_ns, ingress_order)`, and gap
/// boundaries; source timestamps replay verbatim.
#[test]
fn replay_roundtrip() {
    let (bytes, summary) = record_known_session();
    assert_eq!(summary.messages, 7, "4 + 2 + 1 recorded samples");
    assert_eq!(summary.gaps, 2, "history lap + policy drop");
    assert_eq!(summary.dropped, 2);

    // Wipe: decode from bytes only.
    let session = Session::decode(&bytes).unwrap_or_else(|e| unreachable!("decode: {e:?}"));
    assert_eq!(session.samples().len(), 7);
    assert_eq!(session.channels().len(), 3);
    assert_eq!(session.gaps().len(), 2);
    assert_eq!(session.topic_ids(), vec![1, 2, 3]);

    // (a) payload bytes per topic, byte-identical.
    let topic1 = session.topic_stream(1);
    let payloads1: Vec<u8> = topic1
        .iter()
        .map(|&i| session.samples()[i].payload[0])
        .collect();
    assert_eq!(payloads1, vec![0x10, 0x11, 0x12, 0x13]);
    let topic2 = session.topic_stream(2);
    let seqs2: Vec<u64> = topic2.iter().map(|&i| session.samples()[i].seq).collect();
    assert_eq!(seqs2, vec![0, 4], "per-topic causal order after the lap");
    let topic3 = session.topic_stream(3);
    assert_eq!(topic3.len(), 1);

    // (b) gap boundaries reconstructed with reasons.
    assert!(
        session
            .gaps()
            .iter()
            .any(|g| g.reason == GapReason::HistoryLap
                && g.topic_id == 2
                && g.first_missing_seq == 1
                && g.missed == 3)
    );
    assert!(
        session
            .gaps()
            .iter()
            .any(|g| g.reason == GapReason::PolicyDrop
                && g.topic_id == 3
                && g.first_missing_seq == 1
                && g.missed == 2)
    );

    // (c) recorder-observed global order is reproducible and monotone.
    let global = session.global_order();
    let keys: Vec<(u64, u64)> = global
        .iter()
        .map(|&i| {
            let s = &session.samples()[i];
            (s.recv_mono_ns, s.ingress_order)
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "global order key is monotone");
}

/// Reads every payload byte from a replay channel in causal order.
/// REQ: REQ-OPS-009.
fn read_back(publisher: &ReplayPublisher, topic_id: u32) -> Vec<u8> {
    let channel = publisher
        .channel(topic_id)
        .unwrap_or_else(|| unreachable!("channel {topic_id}"));
    let mut state = channel.reader();
    let mut buf = [0_u8; 16];
    let mut seen = Vec::new();
    loop {
        match channel.next(&mut state, 0, &mut buf) {
            Ok(drm_core::channel::ReadOutcome::Sample) => seen.push(buf[0]),
            Ok(drm_core::channel::ReadOutcome::Empty) => break,
            other => unreachable!("unexpected read outcome {other:?}"),
        }
    }
    seen
}

/// REQ: REQ-OPS-009, REQ-CORE-088 — the replay publisher republishes under an
/// isolated namespace and consumers observe faithful per-topic payload order
/// with the replay metrics summary.
#[test]
fn replay_publisher_faithful() {
    let (bytes, _summary) = record_known_session();
    let session = Session::decode(&bytes).unwrap_or_else(|e| unreachable!("decode: {e:?}"));

    let base = DeploymentId::new(0x1234_5678);
    let namespace = ReplayNamespace::derive(base, 1);
    assert_ne!(namespace.id(), base, "replay never adopts the live id");
    assert_ne!(
        namespace
            .segment_name(1)
            .unwrap_or_else(|e| unreachable!("ns: {e:?}")),
        base.segment_name(1)
            .unwrap_or_else(|e| unreachable!("base: {e:?}")),
        "replay namespace is isolated (I-DP2)"
    );
    let specs = [
        ChannelSpec {
            topic_id: 1,
            depth: 16,
            pool_size: 17,
            sample_max: 16,
        },
        ChannelSpec {
            topic_id: 2,
            depth: 16,
            pool_size: 17,
            sample_max: 16,
        },
        ChannelSpec {
            topic_id: 3,
            depth: 16,
            pool_size: 17,
            sample_max: 16,
        },
    ];
    let publisher = ReplayPublisher::new(namespace, &specs)
        .unwrap_or_else(|e| unreachable!("publisher: {e:?}"));
    let report = crate::replay::replay(&session, &publisher, SchedulerMode::AsFast)
        .unwrap_or_else(|e| unreachable!("replay: {e:?}"));
    assert_eq!(report.samples, 7);
    assert_eq!(report.topics, 3);
    assert_eq!(report.gaps, 2);
    assert_eq!(report.gap_sequences, 5);
    assert_eq!(report.drops, 2);
    assert_eq!(report.namespace, Some(namespace.id()));

    assert_eq!(read_back(&publisher, 1), vec![0x10, 0x11, 0x12, 0x13]);
    assert_eq!(read_back(&publisher, 2), vec![0x20, 0x21]);
    assert_eq!(read_back(&publisher, 3), vec![0x30]);
}

/// REQ: REQ-OPS-009 — I-RC4: the replay scheduler modes produce the documented
/// target offsets; `rate(x)` scales the wall duration and `as_fast` collapses
/// it to zero.
#[test]
fn replay_scheduler_modes() {
    let (bytes, _summary) = record_known_session();
    let session = Session::decode(&bytes).unwrap_or_else(|e| unreachable!("decode: {e:?}"));

    let realtime = session.plan(SchedulerMode::Realtime);
    assert!(
        realtime.duration_ns > 0,
        "realtime preserves recorded deltas"
    );

    let fast = session.plan(SchedulerMode::AsFast);
    assert_eq!(fast.duration_ns, 0, "as_fast waits for nothing");
    assert!(fast.steps.iter().all(|s| s.target_offset_ns == 0));

    let rate = Rate::new(2, 1).unwrap_or_else(|e| unreachable!("rate: {e:?}"));
    let doubled = session.plan(SchedulerMode::Rate(rate));
    assert_eq!(
        doubled.duration_ns,
        realtime.duration_ns / 2,
        "rate x2 halves the wall duration"
    );
    assert_eq!(
        session
            .plan(SchedulerMode::Rate(rate))
            .steps
            .iter()
            .map(|s| s.sample)
            .collect::<Vec<_>>(),
        fast.steps.iter().map(|s| s.sample).collect::<Vec<_>>(),
        "mode changes timing, never order"
    );
    assert_eq!(Rate::new(0, 1).err(), Some(Status::InvalidArg));
}

/// REQ: REQ-CORE-088, REQ-CORE-089 — replay namespaces are isolated from the
/// live deployment and from one another (I-DP2).
#[test]
fn replay_namespace_isolated() {
    let base = DeploymentId::new(0xDEAD_BEEF);
    let run1 = ReplayNamespace::derive(base, 1);
    let run2 = ReplayNamespace::derive(base, 2);
    assert_ne!(run1.id(), base);
    assert_ne!(run2.id(), base);
    assert_ne!(run1.id(), run2.id(), "each replay run is a fresh namespace");
    assert_ne!(
        run1.segment_name(5)
            .unwrap_or_else(|e| unreachable!("ns: {e:?}")),
        base.segment_name(5)
            .unwrap_or_else(|e| unreachable!("base: {e:?}"))
    );
}

/// REQ: REQ-OPS-009, REQ-OPS-018 — epoch handling: a publisher restart lands a
/// `PublisherEpoch` gap boundary and the per-topic stream stays ordered by
/// `(publisher_epoch, seq)`.
#[test]
fn replay_reader_epoch() {
    let ch = chan(8, 9);
    let mut rec = build(vec![spec(1, 16, 8, 8)], OnFull::Drop, 8);
    assert!(ch.publish(&[0x01_u8; 16]).is_ok());
    let _ = rec.ingress_step(0, &ch, 0, 1_000, 1_000);
    assert!(ch.publish(&[0x02_u8; 16]).is_ok());
    let _ = rec.ingress_step(0, &ch, 0, 2_000, 2_000);
    let _ = ch.restart_run();
    assert!(ch.publish(&[0x03_u8; 16]).is_ok());
    let _ = rec.ingress_step(0, &ch, 0, 3_000, 3_000);

    let stage = Stage::new(Vec::new(), &disk_cfg(Compression::None), &[(1, "a")])
        .unwrap_or_else(|e| unreachable!("stage: {e:?}"));
    let (_summary, bytes) = stage
        .finish(&mut rec)
        .unwrap_or_else(|e| unreachable!("finish: {e:?}"));
    let session = Session::decode(&bytes).unwrap_or_else(|e| unreachable!("decode: {e:?}"));

    assert!(
        session
            .gaps()
            .iter()
            .any(|g| g.reason == GapReason::PublisherEpoch),
        "restart accounted as an epoch gap"
    );
    let stream = session.topic_stream(1);
    let epochs: Vec<u32> = stream
        .iter()
        .map(|&i| session.samples()[i].publisher_epoch)
        .collect();
    assert!(
        epochs.windows(2).all(|w| w[0] <= w[1]),
        "per-topic epoch order is non-decreasing"
    );
    assert!(
        epochs.contains(&1),
        "post-restart sample carries the new epoch"
    );
}

/// REQ: REQ-OPS-009 — the replay summary reports counts, gaps, and drops, and
/// source timestamps are carried verbatim into the plan.
#[test]
fn replay_summary_metrics() {
    let (bytes, _summary) = record_known_session();
    let session = Session::decode(&bytes).unwrap_or_else(|e| unreachable!("decode: {e:?}"));
    let report = session.report();
    assert_eq!(report.samples, 7);
    assert_eq!(report.topics, 3);
    assert_eq!(report.gaps, 2);
    assert_eq!(report.gap_sequences, 5);
    assert_eq!(report.drops, 2);
    assert!(report.start_time <= report.end_time);

    let plan = session.plan(SchedulerMode::Realtime);
    for step in &plan.steps {
        assert_eq!(
            step.source_time,
            session.samples()[step.sample].source_time,
            "source timestamps replay exactly as recorded"
        );
    }
}
