//! Dead-reader reclamation slow path, verbatim from
//! `docs/algorithms/runtime/exhaustion-reclamation.md` §Functions (`try_reclaim`)
//! and §Protocols (steps 1–5): trigger-only-on-exhaustion → probe → prove +
//! inspect under the lock → reclaim → resume. A hazard word is cleared only once
//! its owner is proven dead and the record is re-read under the record's
//! non-blocking registration lock (I-E5); the pass runs only after a pool
//! exhaustion drop set `reclaim_requested` — never speculatively, no background
//! polling. REQ: REQ-CORE-058 (triggered only), REQ-CORE-056 (lock authority +
//! under-lock reread), REQ-CORE-057 (identity is diagnostic; a live reader is
//! never reclaimed), REQ-CORE-095 (RT publisher defers to this off-path scan),
//! REQ-CORE-023 (publishing resumes when capacity returns).

use std::sync::atomic::Ordering;

use crate::channel::Channel;
use crate::hazard::{ConsumerRec, HAZARD_NONE, pack};
use crate::shm::identity_alive;

impl Channel {
    /// Bounded reclamation pass over the registration records, run only after a
    /// pool-exhaustion drop set `reclaim_requested` (I-E7/REQ-CORE-095: the RT
    /// publish path sets the flag only; this maintenance pass does the work).
    /// Gated on the flag — with no trigger it reclaims nothing and leaves every
    /// word untouched (no speculative polling). For each record whose hazard word
    /// names a slot at the record's current registration generation, the owner is
    /// proven dead (identity + start-ticks incarnation guard, then a non-blocking
    /// registration lock with a word/generation re-read under it); only then is
    /// the word cleared back to `NONE` (I-E5, I-E6). The trigger flag is consumed
    /// exactly once per pass (coalescing wakeup). Returns the number of slots
    /// reclaimed.
    /// REQ: REQ-CORE-058, REQ-CORE-056, REQ-CORE-057, REQ-CORE-095.
    #[must_use]
    pub fn try_reclaim(&self) -> usize {
        if !self.ccb.reclaim_requested.load(Ordering::Acquire) {
            return 0; // gated: no reclamation runs speculatively
        }
        let mut reclaimed = 0;
        for rec in &self.recs {
            if try_reclaim_record(rec) {
                reclaimed += 1;
            }
        }
        // Consume the trigger exactly once (maintenance-wakeup contract).
        self.ccb.reclaim_requested.store(false, Ordering::Release);
        reclaimed
    }
}

/// Per-record reclaim: prove the owner dead, re-read the hazard word and
/// registration generation under the non-blocking registration lock, then clear
/// the exact generation-bearing word (I-E5, I-E6). Returns `true` iff a slot was
/// freed. Never touches a word whose owner may still be live.
/// REQ: REQ-CORE-056, REQ-CORE-057.
fn try_reclaim_record(rec: &ConsumerRec) -> bool {
    let (wgen, slot) = rec.hazard.load(Ordering::SeqCst);
    if slot == HAZARD_NONE {
        return false; // nothing protected
    }
    if wgen != rec.generation.load(Ordering::Acquire) {
        return false; // stale-generation word protects nothing (hazard-validity rule)
    }
    // Sole liveness proof for this in-process mirror: the recorded owner must be a
    // DEAD incarnation (identity + start-ticks guard). A live owner — regardless of
    // PID semantics or namespace visibility — is never reclaimed (I-E5).
    let owner = rec.identity();
    if identity_alive(owner.pid, owner.start_ticks) {
        return false;
    }
    // Probe: non-blocking registration lock. A live owner holds it for its whole
    // lifetime, so a failed acquisition is live authority ⇒ skip (protocol step 2).
    let Some(_lock) = rec.try_reg_lock() else {
        return false;
    };
    // Lock acquired ⇒ no live owner retains authority. Re-read everything under the
    // lock and require the exact generation-matching word before mutating
    // (protocol steps 3–4); the record state is deliberately not consulted — a dead
    // `Closing` record carrying a live borrow's word is exactly the case repaired.
    let gen_now = rec.generation.load(Ordering::Acquire);
    let (wgen2, slot2) = rec.hazard.load(Ordering::SeqCst);
    if wgen2 != wgen || slot2 != slot || gen_now != wgen {
        return false; // raced (owner re-claimed / released) — leave untouched
    }
    // Clear protection via the SeqCst CAS on the exact observed packed word (I-E6).
    let cleared = rec
        .hazard
        .compare_exchange(pack(wgen, slot), pack(wgen, HAZARD_NONE));
    if cleared {
        // Normalize a lock-proven-dead record to its vacant terminal state.
        rec.active.store(false, Ordering::Release);
    }
    cleared
}

#[cfg(test)]
mod tests {
    use super::Channel;
    use crate::channel::OverflowPolicy as Pol;
    use crate::hazard::{HAZARD_NONE, pack};
    use crate::shm::{ProcessIdentity, current_identity};
    use drm_types::Status;
    use std::sync::atomic::Ordering;

    /// Small overwrite channel with two borrower records (`rec(0)`, `rec(1)`).
    fn ch(depth: u64, pool: u64) -> Channel {
        Channel::activate(depth, pool, 16, 8, Pol::Overwrite, 0, 2, None)
            .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
    }

    fn pat(v: u8) -> [u8; 16] {
        [v; 16]
    }

    /// Claim `slot` on `rec(i)` from its empty word; returns that record's
    /// registration generation.
    fn hold(c: &Channel, i: usize, slot: u32) -> u32 {
        let rec = c.rec(i).unwrap_or_else(|| unreachable!("rec {i}"));
        let g = rec.generation.load(Ordering::Acquire);
        assert!(
            rec.hazard
                .compare_exchange(pack(g, HAZARD_NONE), pack(g, slot)),
            "hold rec {i} failed"
        );
        g
    }

    fn word(c: &Channel, i: usize) -> (u32, u32) {
        c.rec(i)
            .unwrap_or_else(|| unreachable!("rec {i}"))
            .hazard
            .load(Ordering::SeqCst)
    }

    fn freed(c: &Channel, i: usize, g: u32) -> bool {
        word(c, i) == (g, HAZARD_NONE)
    }

    /// REQ: REQ-CORE-058, REQ-CORE-022, REQ-CORE-023 — with an abandoned hazard
    /// word but no exhaustion trigger, `try_reclaim` reclaims nothing (no
    /// speculative polling); once the publish-drop path sets `reclaim_requested`
    /// it frees the slot, consumes the flag, and the next publish resumes (I-E4).
    #[test]
    fn reclaim_requires_trigger() {
        let c = ch(2, 3);
        assert_eq!(c.publish(&pat(1)), Ok(0));
        assert_eq!(c.publish(&pat(2)), Ok(1));
        let g = hold(&c, 0, 2); // rec 0 abandons slot 2; identity {0,0} == dead

        // Untriggered: the stale word is left untouched.
        assert!(!c.reclaim_requested(), "no exhaustion yet");
        assert_eq!(c.try_reclaim(), 0, "no reclamation without a trigger");
        assert_eq!(word(&c, 0), (g, 2), "abandoned word preserved");

        // The drop path sets the trigger (every slot is now taken).
        assert_eq!(c.publish(&pat(3)), Err(Status::PoolExhausted));
        assert!(c.reclaim_requested(), "drop path sets the trigger");

        // The maintenance pass reclaims the abandoned slot and consumes the flag.
        assert_eq!(c.try_reclaim(), 1);
        assert!(freed(&c, 0, g));
        assert!(!c.reclaim_requested(), "trigger consumed exactly once");

        // Publishing resumes instantly (REQ-CORE-023).
        assert_eq!(c.publish(&pat(4)), Ok(2));
    }

    /// REQ: REQ-CORE-057, REQ-CORE-056 — a LIVE owner's held slot is never
    /// reclaimed; a dead owner's is (I-E5). The live record's registered identity
    /// is the running test process.
    #[test]
    fn reclaim_proof_required() {
        let c = ch(1, 3);
        assert_eq!(c.publish(&pat(1)), Ok(0)); // seq 0 -> slot 0 (history)
        c.rec(0)
            .unwrap_or_else(|| unreachable!("rec0"))
            .record_identity(current_identity()); // rec 0 is a LIVE reader
        // rec 1 keeps the default (dead) identity {0,0}.
        let g0 = hold(&c, 0, 1);
        let g1 = hold(&c, 1, 2);
        c.ccb().reclaim_requested.store(true, Ordering::Release);

        assert_eq!(c.try_reclaim(), 1, "only the dead owner is reclaimed");
        assert_eq!(word(&c, 0), (g0, 1), "a live reader is never reclaimed");
        assert!(freed(&c, 1, g1), "the dead owner is reclaimed");
        assert!(!c.reclaim_requested());
    }

    /// REQ: REQ-CORE-058, REQ-CORE-056, REQ-CORE-057, REQ-CORE-023 — kill/abandon
    /// at every borrow stage with synthetic identity records: reclamation happens
    /// only after the death proof, and recovery completes (the next publish
    /// resumes) — I-E5.
    #[test]
    fn reclaim_kill_matrix() {
        let mut cells = 0_u32;

        // Stage 1 — pre-intent: registered, no borrow started (word NONE).
        {
            let c = ch(1, 3);
            assert_eq!(c.publish(&pat(1)), Ok(0));
            c.ccb().reclaim_requested.store(true, Ordering::Release);
            assert_eq!(c.try_reclaim(), 0, "pre-intent: nothing protected");
            cells += 1;
        }

        // Stage 2 — post-intent, before the payload borrow: dead owner, word set.
        {
            let c = ch(1, 3);
            assert_eq!(c.publish(&pat(1)), Ok(0));
            let g = hold(&c, 0, 1);
            c.ccb().reclaim_requested.store(true, Ordering::Release);
            assert_eq!(c.try_reclaim(), 1, "post-intent: dead owner reclaimed");
            assert!(freed(&c, 0, g));
            cells += 1;
        }

        // Stage 3 — mid-borrow: a dead borrow is reclaimed, a live borrow survives.
        {
            let c = ch(1, 3);
            assert_eq!(c.publish(&pat(1)), Ok(0));
            c.rec(0)
                .unwrap_or_else(|| unreachable!("rec0"))
                .record_identity(current_identity()); // live mid-borrow
            let g0 = hold(&c, 0, 1);
            let g1 = hold(&c, 1, 2); // dead mid-borrow
            c.ccb().reclaim_requested.store(true, Ordering::Release);
            assert_eq!(c.try_reclaim(), 1, "dead mid-borrow reclaimed");
            assert_eq!(word(&c, 0), (g0, 1), "live mid-borrow not reclaimed");
            assert!(freed(&c, 1, g1));
            cells += 1;
        }

        // Stage 4 — post-release: the owner released before dying (word NONE).
        {
            let c = ch(1, 3);
            assert_eq!(c.publish(&pat(1)), Ok(0));
            let g = hold(&c, 0, 1);
            let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec0"));
            assert!(
                rec.hazard
                    .compare_exchange(pack(g, 1), pack(g, HAZARD_NONE))
            );
            c.ccb().reclaim_requested.store(true, Ordering::Release);
            assert_eq!(c.try_reclaim(), 0, "post-release: nothing protected");
            cells += 1;
        }

        // Stage 5 — registration lock still held: skip; reclaim + resume once free.
        {
            let c = ch(2, 3);
            assert_eq!(c.publish(&pat(1)), Ok(0));
            assert_eq!(c.publish(&pat(2)), Ok(1));
            let g = hold(&c, 0, 2);
            c.ccb().reclaim_requested.store(true, Ordering::Release);
            let lock = c
                .rec(0)
                .unwrap_or_else(|| unreachable!("rec0"))
                .try_reg_lock()
                .unwrap_or_else(|| unreachable!("lock free"));
            assert_eq!(c.try_reclaim(), 0, "lock held => live authority => skip");
            assert_eq!(word(&c, 0), (g, 2), "word intact while locked");
            drop(lock);
            c.ccb().reclaim_requested.store(true, Ordering::Release); // re-trigger
            assert_eq!(c.try_reclaim(), 1, "reclaimed once the lock is free");
            assert!(freed(&c, 0, g));
            assert_eq!(c.publish(&pat(3)), Ok(2), "recovery completes");
            cells += 1;
        }

        assert_eq!(cells, 5, "kill matrix cell count");
    }

    /// REQ: REQ-CORE-057, REQ-CORE-008 — a synthetic identity whose PID matches a
    /// live process but whose start ticks differ is NOT that process's
    /// incarnation: the recorded owner is dead and is reclaimed, while an
    /// exactly-matching live incarnation is never reclaimed (I-E5).
    #[test]
    fn reclaim_pid_reuse() {
        let me = current_identity();
        assert!(me.pid > 0);

        // (a) Exact same-incarnation identity ⇒ live ⇒ never reclaimed.
        let c = ch(1, 3);
        assert_eq!(c.publish(&pat(1)), Ok(0));
        c.rec(0)
            .unwrap_or_else(|| unreachable!("rec0"))
            .record_identity(me);
        let ga = hold(&c, 0, 1);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 0, "a live incarnation survives");
        assert_eq!(word(&c, 0), (ga, 1));

        // (b) Same PID, different start ticks ⇒ a different (dead) incarnation.
        let c2 = ch(1, 3);
        assert_eq!(c2.publish(&pat(1)), Ok(0));
        let reused = ProcessIdentity {
            pid: me.pid,
            start_ticks: me.start_ticks.wrapping_add(1),
        };
        c2.rec(0)
            .unwrap_or_else(|| unreachable!("rec0"))
            .record_identity(reused);
        let gb = hold(&c2, 0, 1);
        c2.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(
            c2.try_reclaim(),
            1,
            "a reused PID is a different incarnation"
        );
        assert!(freed(&c2, 0, gb));
    }
}
