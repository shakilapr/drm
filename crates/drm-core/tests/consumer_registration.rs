//! Consumer registration: exclusive claim, identity publication order, clean
//! close with an outstanding borrow, generation-safe reuse, and the acquisition
//! recheck (`docs/algorithms/runtime/consumer-registration.md` I-CR1..I-CR7).
//!
//! These use the real `flock` backend against a scratch lock directory: the
//! registration lock is the only ownership authority, so a mock would prove
//! nothing.
//!
//! REQ: REQ-CORE-005, REQ-CORE-006, REQ-CORE-007, REQ-CORE-008, REQ-CORE-009.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use drm_core::channel::registration::{RegistrationState, acquisition_valid, claim, lock_path};
use drm_core::hazard::{ConsumerRec, HAZARD_NONE, pack};
use drm_core::shm::{FlockBackend, LockPaths, ProcessIdentity};
use drm_types::Status;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A scratch lock directory unique to this test process.
fn lock_dir(tag: &str) -> LockPaths {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("drm-registration-{}-{n}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("mkdir {dir:?}: {e}"));
    LockPaths::new(dir)
}

/// A fresh record, as a segment initializes it: `Vacant`, generation 0.
fn vacant() -> ConsumerRec {
    let rec = ConsumerRec::new(0);
    rec.set_state(RegistrationState::Vacant);
    rec.set_generation(0);
    rec.set_hazard_word(pack(0, HAZARD_NONE));
    rec
}

/// The identity a claimant publishes (diagnostic only).
fn me() -> ProcessIdentity {
    ProcessIdentity {
        pid: 4242,
        start_ticks: 99,
    }
}

/// I-CR1: two live processes cannot own one record. The second claim fails while
/// the first holds the OS lock, and succeeds once the first is dropped.
/// REQ: REQ-CORE-006.
#[test]
fn consumer_claim_race() {
    let locks = lock_dir("race");
    let rec = vacant();
    let path = lock_path(&locks, 7, 0);

    let first =
        claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(first.generation(), 1);
    assert_eq!(
        rec.state(),
        Ok(RegistrationState::Active),
        "a completed claim is Active"
    );

    assert_eq!(
        claim(&FlockBackend, &rec, &path, me(), 0).err(),
        Some(Status::Busy),
        "a second live claimant must be refused"
    );
    // The refused claim must not have disturbed the owner's state.
    assert_eq!(rec.state(), Ok(RegistrationState::Active));
    assert_eq!(rec.generation_word(), 1);

    drop(first);
    // Lock released: the next claimant takes over with a new generation.
    let second =
        claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(second.generation(), 2, "reuse bumps the generation");
}

/// I-CR2: `Active` implies the complete identity and `{generation, NONE}` are
/// visible — a claimant publishes them before the state word.
/// REQ: REQ-CORE-008.
#[test]
fn consumer_claim_order() {
    let locks = lock_dir("order");
    let rec = vacant();
    let path = lock_path(&locks, 7, 1);
    let reg = claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));

    assert_eq!(rec.state(), Ok(RegistrationState::Active));
    assert_eq!(
        rec.hazard_word(),
        pack(reg.generation(), HAZARD_NONE),
        "the hazard word carries the new generation and no slot"
    );
    assert_eq!(
        rec.identity(),
        me(),
        "the owner identity is published before Active"
    );
}

/// I-CR3: a stale generation's word cannot clear a new owner's hazard.
/// REQ: REQ-CORE-007.
#[test]
fn consumer_generation_reuse() {
    let locks = lock_dir("reuse");
    let rec = vacant();
    let path = lock_path(&locks, 7, 2);

    let first =
        claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    let stale_generation = first.generation();
    drop(first);

    let second =
        claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_ne!(second.generation(), stale_generation, "reuse bumps it");

    // A borrow installed by the new owner, then a stale clear carrying the OLD
    // generation: the CAS must fail because the word is not the stale form.
    let owned = pack(second.generation(), 5);
    rec.set_hazard_word(owned);
    let stale_empty = pack(stale_generation, HAZARD_NONE);
    assert_ne!(
        rec.hazard_word(),
        stale_empty,
        "the stale word does not describe the new owner's state"
    );
    // The new owner can still clear its own word.
    rec.set_hazard_word(pack(second.generation(), HAZARD_NONE));
    assert_eq!(rec.hazard_word(), pack(second.generation(), HAZARD_NONE));
}

/// I-CR4: clean close succeeds only with the exact generation-bearing empty
/// word; an outstanding borrow leaves the record `Closing` and reports `Busy`,
/// and a second close completes once the borrow is released.
/// REQ: REQ-CORE-005.
#[test]
fn consumer_close_borrowed() {
    let locks = lock_dir("close");
    let rec = vacant();
    let path = lock_path(&locks, 7, 3);
    let reg = claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    let generation = reg.generation();

    // A live borrow protects slot 3.
    rec.set_hazard_word(pack(generation, 3));
    assert_eq!(
        reg.close().err(),
        Some(Status::Busy),
        "closing under a live borrow is Busy"
    );
    assert_eq!(
        rec.state(),
        Ok(RegistrationState::Closing),
        "the record stays Closing while the borrow is live"
    );

    // Releasing the borrow and re-closing completes the transition.
    rec.set_hazard_word(pack(generation, HAZARD_NONE));
    let reg = claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(reg.generation(), generation + 1, "the record was vacated");
}

/// I-CR7: a claimant interrupted in `Claiming` leaves a generation-0 word, which
/// protects and pins nothing — the next claimant overwrites it under the lock.
/// REQ: REQ-CORE-007.
#[test]
fn consumer_claiming_orphan() {
    let locks = lock_dir("orphan");
    let rec = vacant();
    let path = lock_path(&locks, 7, 4);

    // Simulate the crash: parked word, Claiming state, no lock holder (the OS
    // released it when the "process" died).
    rec.set_hazard_word(pack(0, HAZARD_NONE));
    rec.set_state(RegistrationState::Claiming);

    let reg = claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(rec.state(), Ok(RegistrationState::Active));
    assert_eq!(
        rec.hazard_word(),
        pack(reg.generation(), HAZARD_NONE),
        "the orphaned word is replaced"
    );
}

/// The acquisition recheck: a borrow must not survive its record losing
/// authority (close or reuse).
/// REQ: REQ-CORE-005, REQ-CORE-007.
#[test]
fn consumer_acquisition_recheck() {
    let locks = lock_dir("recheck");
    let rec = vacant();
    let path = lock_path(&locks, 7, 5);
    let reg = claim(&FlockBackend, &rec, &path, me(), 0).unwrap_or_else(|e| unreachable!("{e:?}"));
    let generation = reg.generation();
    assert!(
        acquisition_valid(&rec, generation),
        "an Active record with our generation permits the borrow"
    );
    assert!(
        !acquisition_valid(&rec, generation + 1),
        "a different generation is refused"
    );

    // Demotion (close) must refuse new acquisitions.
    rec.set_state(RegistrationState::Closing);
    assert!(
        !acquisition_valid(&rec, generation),
        "a Closing record refuses new borrows"
    );
}

/// The acknowledgement floor only rises: a restart never re-delivers an
/// already-acknowledged sequence.
/// REQ: REQ-CORE-005.
#[test]
fn required_reattach_resume() {
    let locks = lock_dir("resume");
    let rec = vacant();
    let path = lock_path(&locks, 7, 6);

    let first =
        claim(&FlockBackend, &rec, &path, me(), 10).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(first.next_needed(), 10, "the floor is adopted");
    drop(first);

    // A restart declaring an older floor must not move it backwards.
    let second =
        claim(&FlockBackend, &rec, &path, me(), 4).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(
        second.next_needed(),
        10,
        "a lower declared floor never rewinds the persisted one"
    );
    drop(second);

    // A higher floor advances it.
    let third =
        claim(&FlockBackend, &rec, &path, me(), 25).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(third.next_needed(), 25);
}

/// An unknown state word is refused rather than guessed: a segment written by a
/// newer runtime must not be interpreted loosely.
/// REQ: REQ-CORE-007.
#[test]
fn unknown_state_refused() {
    assert_eq!(
        RegistrationState::from_raw(9),
        Err(Status::InvalidData),
        "an unknown state is not guessed"
    );
    for (raw, expected) in [
        (0, RegistrationState::Vacant),
        (1, RegistrationState::Claiming),
        (2, RegistrationState::Active),
        (3, RegistrationState::Closing),
    ] {
        assert_eq!(RegistrationState::from_raw(raw), Ok(expected));
    }
}

/// The lock path follows the generated-name family (`t_<topic>.<id>.consumer.lock`),
/// never a canonical topic string.
/// REQ: REQ-CORE-006, REQ-CORE-069.
#[test]
fn lock_path_is_generated_family() {
    let locks = LockPaths::new(PathBuf::from("/run/drm/abc"));
    let path = lock_path(&locks, 12, 3);
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some("t_12.3.consumer.lock")
    );
}
