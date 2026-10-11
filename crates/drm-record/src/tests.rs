//! Record-core suite (`docs/algorithms/ops/record-replay.md` §Invariants
//! I-RC1/I-RC2/I-RC3/I-RC5, §Test Matrix `record_borrow_scope`,
//! `record_bounded`, `record_policies`, `record_gap_ledger`).
//! REQ: REQ-OPS-005, REQ-OPS-007, REQ-OPS-008, REQ-OPS-018, REQ-OPS-019.

use super::{
    DescEntry, GapLedger, GapReason, IngressStep, OnFull, PendingGap, Recorder, RecorderConfig,
    StopReason, TopicSpec,
};
use drm_core::channel::{Channel, OverflowPolicy};
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
    assert_eq!(rec.chunk_target(), 4096);
    assert_eq!(rec.on_full(), OnFull::Drop);
    assert_eq!(rec.topic_count(), 1);
}
