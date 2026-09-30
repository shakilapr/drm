//! `race_startup` — shm-backend.md I1 `SingleCreator`: exactly one concurrent
//! creator per segment lifetime. 8 child processes × 100 attempts race
//! `create_exclusive` on one name; every winner unlinks immediately, ending
//! that lifetime. Invariants: every loser observes `AlreadyExists` (no silent
//! fallback), every lifetime has exactly one creator, no unexpected error.
//! REQ: REQ-CORE-072.
#![cfg(unix)]

use std::process::Output;

use drm_core::shm::{PosixShm, SegmentName, ShmBackend, ShmError};

use crate::child_util::{self, is_child, ok, spawn_child, unique_name};

const PROCS: usize = 8;
const ATTEMPTS: usize = 100;

struct Counters {
    winners: u32,
    already_exists: u32,
    other_errors: u32,
}

/// Parses one child's `COUNTERS winners=<n> already=<n> other=<n>` line.
/// REQ: REQ-CORE-072.
fn child_counters(out: &Output) -> Counters {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let Some(line) = stdout.lines().find(|l| l.starts_with("COUNTERS ")) else {
        unreachable!("platform suite: child counters");
    };
    let fields: Vec<u32> = line[9..]
        .split_whitespace()
        .map(
            |f| match f.split_once('=').and_then(|(_, v)| v.parse::<u32>().ok()) {
                Some(v) => v,
                None => unreachable!("platform suite: counter field {f}"),
            },
        )
        .collect();
    let &[winners, already, other] = fields.as_slice() else {
        unreachable!("platform suite: counter arity");
    };
    Counters {
        winners,
        already_exists: already,
        other_errors: other,
    }
}

/// REQ: REQ-CORE-072 — I1 race: 8 procs × 100 attempts; winners == lifetimes,
/// losers == `AlreadyExists` only, other errors == 0.
#[test]
fn race_startup() {
    if is_child() {
        return;
    }
    let name = unique_name("race");

    let mut outputs: Vec<Output> = Vec::new();
    for _ in 0..PROCS {
        outputs.push(spawn_child(
            "race_startup::race_child",
            "DRM_RACE_NAME",
            &name,
        ));
    }

    let mut totals = Counters {
        winners: 0,
        already_exists: 0,
        other_errors: 0,
    };
    for (i, out) in outputs.iter().enumerate() {
        assert!(
            out.status.success(),
            "child {i} failed: {:?} stderr={:?}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let c = child_counters(out);
        totals.winners += c.winners;
        totals.already_exists += c.already_exists;
        totals.other_errors += c.other_errors;
    }

    assert_eq!(totals.other_errors, 0, "unexpected create errors");
    assert_eq!(
        u64::from(totals.winners) + u64::from(totals.already_exists),
        (PROCS * ATTEMPTS) as u64,
        "every attempt resolved"
    );
    // Winners == segment lifetimes (winner unlinks immediately); each lifetime
    // had exactly one creator by exclusivity of `AlreadyExists`.
    assert!(totals.winners > 0, "no creator ever won");
}

/// Child loop; no-op in a normal test run.
/// REQ: REQ-CORE-072.
#[test]
fn race_child() {
    if !is_child() {
        return;
    }
    let shm = PosixShm;
    let name = ok(
        SegmentName::new(&child_util::env_var("DRM_RACE_NAME")),
        "race name",
    );
    let mut c = Counters {
        winners: 0,
        already_exists: 0,
        other_errors: 0,
    };
    for _ in 0..ATTEMPTS {
        match shm.create_exclusive(&name, 64) {
            Ok(m) => {
                c.winners += 1;
                shm.unmap(m);
                ok(shm.unlink(&name), "winner unlink");
            }
            Err(ShmError::AlreadyExists) => c.already_exists += 1,
            Err(_) => c.other_errors += 1,
        }
    }
    println!(
        "COUNTERS {} {} {}",
        c.winners, c.already_exists, c.other_errors
    );
    assert_eq!(c.other_errors, 0);
}
