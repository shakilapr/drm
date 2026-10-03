//! Publish-visibility ordering smoke (work-plan M02W3-E1-T2): the release
//! store that commits a sample happens-after the payload write; the acquire
//! load that observes the sequence sees the payload (channel-publish.md
//! REQ-CORE-051; epoch observation REQ-CORE-002). REQ: REQ-CORE-051,
//! REQ-CORE-002.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

const SAMPLES: u64 = 10_000;
/// One payload slot per sample (no reuse inside the smoke window).
const SLOTS: usize = 10_000;

/// REQ: REQ-CORE-051 — release-store of `write_seq` after the payload write
/// makes that sample visible to an acquire reader; every observed sequence
/// implies every earlier payload byte is visible.
#[test]
fn publish_release_acquire_visibility() {
    let payload: Vec<AtomicU8> = (0..SLOTS).map(|_| AtomicU8::new(0)).collect();
    let write_seq = Arc::new(AtomicU64::new(0));
    let payload = Arc::new(payload);

    let writer = {
        let payload = Arc::clone(&payload);
        let seq = Arc::clone(&write_seq);
        std::thread::spawn(move || {
            for i in 0..SAMPLES {
                let v = u8::try_from((i ^ 0xA5) & 0xFF).unwrap_or(0);
                let slot = usize::try_from(i).unwrap_or(0);
                payload[slot].store(v, Ordering::Relaxed);
                seq.store(i + 1, Ordering::Release);
            }
        })
    };

    let mut seen: u64 = 0;
    while seen < SAMPLES {
        let s = write_seq.load(Ordering::Acquire);
        for i in seen..s {
            let expected = u8::try_from((i ^ 0xA5) & 0xFF).unwrap_or(0);
            let got = payload[usize::try_from(i).unwrap_or(0)].load(Ordering::Acquire);
            assert_eq!(
                got, expected,
                "sample {i} invisible after observing seq {s}"
            );
        }
        seen = s;
    }
    writer.join().unwrap_or(());
}

/// REQ: REQ-CORE-002 — the epoch word uses the same release/acquire pair:
/// the reader's `epoch_changed` gate observes a restart-incremented epoch
/// only after every write the new publisher performed before the increment.
#[test]
fn epoch_word_pairing() {
    let ccb = Arc::new(drm_core::channel::ccb::ChannelControlBlock::zeroed());
    let writer_ccb = Arc::clone(&ccb);
    let writer = std::thread::spawn(move || {
        for _ in 0..1_000 {
            writer_ccb.first_valid_seq.store(1, Ordering::Relaxed);
            writer_ccb.publisher_epoch.store(1, Ordering::Release);
        }
    });
    let mut ever_epoch: Option<u32> = None;
    while ever_epoch.is_none() {
        if ccb.epoch_changed(0) {
            ever_epoch = Some(ccb.publisher_epoch.load(Ordering::Acquire));
        } else {
            std::hint::spin_loop();
        }
    }
    assert_eq!(ever_epoch, Some(1));
    assert_eq!(ccb.first_valid(), 1);
    writer.join().unwrap_or(());
}
