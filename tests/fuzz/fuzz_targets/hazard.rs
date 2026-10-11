//! Structured hazard-protocol storm fuzz target (work-plan M18W1-E1-T1):
//! semantics-aware random inputs (reader count, depth, pool, per-op kind) drive
//! the public publish/read/reclaim surface — `publish`, `try_latest`,
//! `try_next`, `acquire`, dead-identity injection, `try_reclaim`, and
//! `restart_run` — so acquisition and reclamation paths are exercised with no
//! panic or hang. Every decode is bounded and every call is fallible; a status
//! error is a valid outcome. `-timeout=5` budgets the hang side.
//! REQ: REQ-QAL-004, REQ-QAL-007, REQ-QAL-009, REQ-CORE-030, REQ-CORE-031,
//! REQ-CORE-035, REQ-CORE-056, REQ-CORE-058.

#![no_main]

use drm_core::channel::{Channel, OverflowPolicy};
use drm_core::hazard::{HAZARD_NONE, acquire};
use drm_core::shm::ProcessIdentity;
use libfuzzer_sys::fuzz_target;

/// The dead-owner incarnation used to simulate a killed reader (identity
/// `{0, 0}` never matches a live process). REQ: REQ-CORE-057.
const DEAD: ProcessIdentity = ProcessIdentity {
    pid: 0,
    start_ticks: 0,
};

/// Structured storm driven by `data`: the first bytes select the topology
/// (depth, pool, sample size, reader count), the remainder selects one
/// operation per byte. REQ: REQ-QAL-004, REQ-QAL-009.
fn storm(data: &[u8]) {
    if data.len() < 5 {
        return;
    }
    let depth = u64::from(data[0] % 4) + 1;
    let extra = u64::from(data[1] % 3);
    let pool = depth + extra + 1;
    let sample = u64::from(data[2] % 24) + 8;
    let readers = usize::from(data[3] % 4) + 1;
    let budget = u32::from(data[4] % 8) + 1;

    let Ok(ch) = Channel::activate(
        depth,
        pool,
        sample,
        8,
        OverflowPolicy::Overwrite,
        0,
        readers,
        None,
    ) else {
        return;
    };
    let len = usize::try_from(sample).unwrap_or(0);
    let mut st = ch.reader();
    let mut buf = vec![0_u8; len];

    for (i, &op) in data[5..].iter().enumerate() {
        let idx = i % readers;
        match op % 7 {
            0 => {
                let fill = op;
                let body = vec![fill; len];
                let _ = ch.publish(&body);
            }
            1 => {
                let _ = ch.try_latest(&mut st, idx, &mut buf, budget);
            }
            2 => {
                let _ = ch.try_next(&mut st, idx, &mut buf, budget);
            }
            3 => {
                // Simulated kill: the record's owner is the dead incarnation,
                // then a hazard-protected borrow is installed and abandoned.
                if let Some(rec) = ch.rec(idx) {
                    rec.record_identity(DEAD);
                    let g = rec.generation.load(std::sync::atomic::Ordering::Acquire);
                    let _ = rec.hazard.compare_exchange(
                        drm_core::hazard::pack(g, HAZARD_NONE),
                        drm_core::hazard::pack(g, 0),
                    );
                }
            }
            4 => {
                let _ = ch.try_reclaim();
            }
            5 => {
                if let Some(rec) = ch.rec(idx) {
                    let _ = acquire(ch.ring(), ch.ccb(), ch.write_seq(), rec);
                }
            }
            6 => {
                let _ = ch.restart_run();
            }
            _ => unreachable!(),
        }
    }

    // A final read must be payload-or-empty, never a panic (I-HZ8).
    let _ = ch.latest(&mut st, 0, &mut buf);
    let _ = ch.next(&mut st, 0, &mut buf);
}

fuzz_target!(|data: &[u8]| {
    storm(data);
});
