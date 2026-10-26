//! Consumer registration: claim, identity publication, clean close, and the
//! acquisition recheck (`docs/algorithms/runtime/consumer-registration.md`).
//!
//! A consumer record is a fixed slot reserved at activation. Exactly one live
//! endpoint may own it, and ownership is proved by an OS registration lock held
//! for the registration's lifetime — never by process inspection
//! (`exhaustion-reclamation.md`: process identity is diagnostic only).
//!
//! State machine (`RegistrationState`):
//!
//! ```text
//! Vacant --claim--> Claiming --publish--> Active --close--> Closing --release--> Vacant
//! ```
//!
//! The order inside `claim` is the safety argument: park a generation-0 hazard
//! word (which matches no live generation, so it protects and pins nothing)
//! *before* publishing `Claiming`. A crash anywhere in `Claiming` therefore
//! leaves a word that the next claimant can overwrite under the same lock.
//!
//! REQ: REQ-CORE-006, REQ-CORE-007, REQ-CORE-008, REQ-CORE-009, REQ-CORE-005.
#![allow(
    clippy::cast_possible_truncation,
    reason = "the record fields are u32 by the wire layout; every value stored is range-checked first"
)]

use std::path::Path;
use std::sync::atomic::Ordering;

use crate::hazard::{ConsumerRec, HAZARD_NONE, pack};
use crate::shm::{LockBackend, LockHandle, ProcessIdentity};
use drm_types::Status;

/// Record state (`RegistrationState` in the algorithm doc). Values are the
/// on-segment encoding, so they are explicit and never renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RegistrationState {
    /// No owner; claimable.
    Vacant = 0,
    /// A claimant holds the lock and is publishing its identity.
    Claiming = 1,
    /// Fully published; borrows permitted.
    Active = 2,
    /// Demoted by close; existing borrows keep protecting their slots.
    Closing = 3,
}

impl RegistrationState {
    /// Decodes a stored state word; an unknown value is `InvalidData` (a segment
    /// written by a newer runtime is refused rather than misread).
    /// REQ: REQ-CORE-007.
    ///
    /// # Errors
    /// [`Status::InvalidData`] for a value this runtime does not know.
    pub fn from_raw(raw: u32) -> Result<Self, Status> {
        match raw {
            0 => Ok(Self::Vacant),
            1 => Ok(Self::Claiming),
            2 => Ok(Self::Active),
            3 => Ok(Self::Closing),
            _ => Err(Status::InvalidData),
        }
    }
}

/// A held registration: the record it owns, its generation, and the OS lock that
/// proves no other process holds it.
///
/// Dropping the value releases the lock, so a panic cannot leave a record
/// permanently claimed.
/// REQ: REQ-CORE-006, REQ-CORE-008.
pub struct Registration<'a> {
    rec: &'a ConsumerRec,
    generation: u32,
    next_needed: u64,
    lock: Option<LockHandle>,
}

impl std::fmt::Debug for Registration<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registration")
            .field("generation", &self.generation)
            .field("next_needed", &self.next_needed)
            .finish_non_exhaustive()
    }
}

impl<'a> Registration<'a> {
    /// This registration's generation: a stale word or lock from a previous
    /// owner can never clear or claim it (I-CR3).
    /// REQ: REQ-CORE-007.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The first sequence this consumer still needs: the persisted
    /// acknowledgement floor, never forgotten across reattachment.
    /// REQ: REQ-CORE-005.
    #[must_use]
    pub fn next_needed(&self) -> u64 {
        self.next_needed
    }

    /// The record this registration owns (for acquisition).
    /// REQ: REQ-CORE-006.
    #[must_use]
    pub fn record(&self) -> &'a ConsumerRec {
        self.rec
    }

    /// Clean close (`close` in the algorithm doc): demote to `Closing`, then
    /// complete only when the hazard word is exactly this generation's empty
    /// word. An outstanding borrow leaves the record `Closing` and returns
    /// `Busy`; the caller releases the borrow and closes again (I-CR4).
    /// REQ: REQ-CORE-005.
    ///
    /// # Errors
    /// [`Status::Busy`] while a borrow is outstanding; [`Status::InvalidData`]
    /// when the record is not in a state this registration may close.
    pub fn close(mut self) -> Result<(), Status> {
        match self.rec.state()? {
            RegistrationState::Active => {
                // Demotion is the only transition; a concurrent close is not ours.
                self.rec.set_state(RegistrationState::Closing);
            }
            // Re-close after a `Busy` whose borrow was released in between.
            RegistrationState::Closing => {}
            // Vacant/Claiming: not this registration's to close.
            RegistrationState::Vacant | RegistrationState::Claiming => {
                return Err(Status::InvalidData);
            }
        }
        let empty = pack(self.generation, HAZARD_NONE);
        if self.rec.hazard_word() != empty {
            // Outstanding borrow: the record stays Closing, and the lock stays
            // held so no other claimant can take it while a borrow is live.
            return Err(Status::Busy);
        }
        self.rec.set_state(RegistrationState::Vacant);
        self.release_lock();
        Ok(())
    }

    /// Releases the OS registration lock (idempotent).
    fn release_lock(&mut self) {
        if let Some(handle) = self.lock.take() {
            // Dropping the handle releases; keeping it explicit documents intent.
            drop(handle);
        }
    }
}

impl Drop for Registration<'_> {
    /// Dropping without an explicit `close` leaves the record as it was and
    /// releases the lock, so the next claimant (or the reclaimer, which takes the
    /// same lock) can take over a crashed or abandoned endpoint.
    /// REQ: REQ-CORE-006.
    fn drop(&mut self) {
        self.release_lock();
    }
}

/// Claims a record for a live endpoint (`claim` in the algorithm doc).
///
/// The OS registration lock is the only authority: it is acquired
/// non-blocking, and a live holder ⇒ [`Status::Busy`] with no state touched.
/// `floor` is the persisted acknowledgement floor (0 when the endpoint declares
/// no required-sequential obligation); it is folded in with `max`, so a restart
/// never re-delivers an already-acknowledged sequence.
/// REQ: REQ-CORE-006, REQ-CORE-008, REQ-CORE-009.
///
/// # Errors
/// [`Status::Busy`] when another live process owns the record;
/// [`Status::GenerationExhausted`] when the record has been reused 2^32 times.
pub fn claim<'a, L: LockBackend>(
    locks: &L,
    rec: &'a ConsumerRec,
    path: &Path,
    me: ProcessIdentity,
    floor: u64,
) -> Result<Registration<'a>, Status> {
    let lock = locks.try_exclusive(path).map_err(|_| Status::Busy)?;

    // Under the lock the prior owner is dead or closed. Normalize FIRST: park a
    // generation-0 word (invalid, matches no live generation) before publishing
    // `Claiming`, so a crash in `Claiming` protects and pins nothing (I-CR7).
    rec.set_hazard_word(pack(0, HAZARD_NONE));
    rec.set_state(RegistrationState::Claiming);

    let Some(generation) = rec.generation_word().checked_add(1).filter(|g| *g != 0) else {
        rec.set_state(RegistrationState::Vacant);
        return Err(Status::GenerationExhausted);
    };
    rec.set_identity(me);
    rec.set_generation(generation);
    rec.set_hazard_word(pack(generation, HAZARD_NONE));
    // Never forget completed required sequences (the floor only rises).
    rec.raise_ack_floor(floor);
    let next_needed = rec.ack_floor();
    rec.set_state(RegistrationState::Active);

    Ok(Registration {
        rec,
        generation,
        next_needed,
        lock: Some(lock),
    })
}

/// Acquisition recheck (final step of every borrow/copy acquisition): the record
/// must still be `Active` with the generation the acquisition started under.
/// A failure means this consumer lost authority (closed or reused) and must not
/// touch payload bytes; the caller clears the installed word and returns `Busy`.
/// REQ: REQ-CORE-005, REQ-CORE-007.
#[must_use]
pub fn acquisition_valid(rec: &ConsumerRec, generation: u32) -> bool {
    rec.active.load(Ordering::Acquire) && rec.generation.load(Ordering::Acquire) == generation
}

/// The registration lock path for one consumer record
/// (`t_<topic_id>.<consumer_id>.consumer.lock`).
///
/// Kept here rather than at each call site so the generated-name family is
/// applied in exactly one place (deployment-identity.md: never canonical topic
/// strings).
/// REQ: REQ-CORE-006, REQ-CORE-069.
#[must_use]
pub fn lock_path(
    locks: &crate::shm::LockPaths,
    topic_id: u32,
    consumer_id: u32,
) -> std::path::PathBuf {
    locks.consumer(topic_id, consumer_id)
}
