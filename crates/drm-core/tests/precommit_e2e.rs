//! Precommit activation end-to-end (work-plan M07W1-E1-T1; memory-budget.md
//! §Protocols *Precommit activation* steps 1–5; architecture.md §Hard Real-Time
//! Precommitment; I-MB5). A verify-passed deployment activates — pre-fault every
//! declared page, then lock ⇒ active — while an injected lock failure refuses
//! active state; the refusal is re-checked after a deployment-identity change.
//! Deterministic: a content-preserving pre-fault over a real mapping plus an
//! injected lock step — no sleeps, no dependence on the environment's
//! `RLIMIT_MEMLOCK` (the real `mlock` proof is covered by the crate unit test
//! `precommit_activation_success`, which asserts the refusal path if the runner
//! cannot lock).
//! REQ: REQ-CORE-090.
#![cfg(unix)]

use std::cell::Cell;

use drm_core::channel::precommit::{PrecommitProver, PrecommittedChannel, activate_precommitted};
use drm_core::channel::{Channel, OverflowPolicy};
use drm_core::deploy::DeploymentId;
use drm_core::shm::{Mapping, PosixShm, SegmentName, ShmBackend, ShmError};
use drm_types::Status;

/// Deterministic precommit prover: write-touches every declared byte of the
/// mapping (a real, content-preserving pre-fault — touching every byte faults
/// every page regardless of OS page size) and reports lock success or an
/// injected refusal. Models the two activation steps without the runner's
/// `mlock` limit entering the assertion.
struct FixtureProver {
    /// `true` ⇒ the lock step refuses with the low-lockable-budget family error.
    refuse_lock: bool,
    prefaulted: Cell<bool>,
    locked: Cell<bool>,
}

impl FixtureProver {
    fn new(refuse_lock: bool) -> Self {
        Self {
            refuse_lock,
            prefaulted: Cell::new(false),
            locked: Cell::new(false),
        }
    }
}

impl PrecommitProver for FixtureProver {
    fn prefault(&self, m: &Mapping) -> Result<(), ShmError> {
        let base = m.ptr().as_ptr();
        for off in 0..m.len() {
            // SAFETY: `off < m.len()`; the mapping covers `m.len()` writable
            // bytes at `base`. Reading then writing the same byte preserves the
            // value while faulting the page in for write.
            unsafe {
                let p = base.add(off);
                p.write_volatile(p.read_volatile());
            }
        }
        self.prefaulted.set(true);
        Ok(())
    }

    fn lock_range(&self, _m: &Mapping) -> Result<(), ShmError> {
        self.locked.set(true);
        if self.refuse_lock {
            return Err(ShmError::QuotaExceeded);
        }
        Ok(())
    }
}

/// A real, uniquely named mapping under `dep`'s namespace, standing in for the
/// channel's declared region; the name is unlinked immediately (the object
/// survives through the mapping) so reruns start clean.
fn identity_mapping(dep: DeploymentId, slot: u64, len: usize) -> (Mapping, SegmentName) {
    let shm = PosixShm;
    let name = dep
        .segment_name(slot)
        .unwrap_or_else(|e| unreachable!("segment name: {e:?}"));
    let _ = shm.unlink(&name);
    let m = shm
        .create_exclusive(&name, len)
        .unwrap_or_else(|e| unreachable!("create mapping: {e:?}"));
    if let Err(e) = shm.unlink(&name) {
        unreachable!("unlink mapping name: {e:?}");
    }
    (m, name)
}

fn build_channel() -> Result<Channel, Status> {
    Channel::activate(4, 5, 16, 8, OverflowPolicy::Overwrite, 0, 2, None)
}

/// Write `value` at `off` in the mapping (seeds a known pattern for the
/// pre-fault content-preservation check).
fn poke(m: &Mapping, off: usize, value: u8) {
    // SAFETY: `off < m.len()`; the mapping covers `m.len()` writable bytes.
    unsafe {
        m.ptr().as_ptr().add(off).write_volatile(value);
    }
}

/// Read the byte at `off` in the mapping.
fn peek(m: &Mapping, off: usize) -> u8 {
    // SAFETY: `off < m.len()`; the mapping covers `m.len()` readable bytes.
    unsafe { m.ptr().as_ptr().add(off).read_volatile() }
}

/// REQ: REQ-CORE-090 (I-MB5) — verify-passed activation (pre-fault + lock ⇒
/// active) succeeds; an injected lock failure refuses and never builds the
/// channel; the refusal is re-checked after a deployment-identity change and
/// the verify-passed path still activates under the new identity.
#[test]
fn precommit_e2e() {
    let len = 4096 * 3; // multi-page declared region

    // --- Identity A: verify-passed activation succeeds (pre-fault + lock) ---
    let dep_a = DeploymentId::new(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10);
    let (ma, name_a) = identity_mapping(dep_a, 1, len);
    // Seed a known pattern; a content-preserving pre-fault must leave it intact.
    poke(&ma, 0, 0xA5);
    poke(&ma, len - 1, 0x5A);

    let ok_a = FixtureProver::new(false);
    let built_a = Cell::new(false);
    let activated = match PrecommittedChannel::activate(&ok_a, &ma, || {
        built_a.set(true);
        build_channel()
    }) {
        Ok(ch) => ch,
        Err(e) => unreachable!("verify-passed activation must succeed: {e:?}"),
    };
    assert!(built_a.get(), "channel built after a successful proof");
    assert_eq!(activated.proof().len(), len);
    assert!(!activated.proof().is_empty());
    assert_eq!(activated.channel().pool_size(), 5);
    assert!(ok_a.prefaulted.get(), "step 1 pre-fault ran");
    assert!(ok_a.locked.get(), "step 2 lock ran");
    assert_eq!(peek(&ma, 0), 0xA5, "pre-fault preserves payload bytes");
    assert_eq!(
        peek(&ma, len - 1),
        0x5A,
        "pre-fault preserves payload bytes"
    );

    // --- Identity A: injected lock failure refuses; no channel is built ---
    let bad_a = FixtureProver::new(true);
    let built_refuse_a = Cell::new(false);
    let refused_a = PrecommittedChannel::activate(&bad_a, &ma, || {
        built_refuse_a.set(true);
        build_channel()
    });
    assert_eq!(refused_a.err(), Some(Status::InvalidConfig));
    assert!(!built_refuse_a.get(), "no channel without a proof (I-MB5)");
    assert!(
        bad_a.prefaulted.get() && bad_a.locked.get(),
        "both steps attempted before the refusal"
    );

    // --- Deployment-identity change: B is a distinct namespace ---
    let dep_b = DeploymentId::new(0x9999_AAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000);
    let (mb, name_b) = identity_mapping(dep_b, 1, len);
    assert_ne!(
        name_a.as_str(),
        name_b.as_str(),
        "distinct deployments derive distinct namespaces"
    );

    // --- Refusal re-checked under the new identity (not cached per identity) ---
    let bad_b = FixtureProver::new(true);
    let built_refuse_b = Cell::new(false);
    let refused_b = PrecommittedChannel::activate(&bad_b, &mb, || {
        built_refuse_b.set(true);
        build_channel()
    });
    assert_eq!(refused_b.err(), Some(Status::InvalidConfig));
    assert!(!built_refuse_b.get(), "no channel without a proof (I-MB5)");
    // Step-level refusal also re-checks under the new identity.
    assert_eq!(
        activate_precommitted(&FixtureProver::new(true), &mb),
        Err(Status::InvalidConfig)
    );

    // --- Verify-passed path still activates under the new identity ---
    let ok_b = FixtureProver::new(false);
    match PrecommittedChannel::activate(&ok_b, &mb, build_channel) {
        Ok(ch) => {
            assert_eq!(ch.proof().len(), len);
            assert_eq!(ch.channel().pool_size(), 5);
        }
        Err(e) => unreachable!("activation under new identity: {e:?}"),
    }
}
