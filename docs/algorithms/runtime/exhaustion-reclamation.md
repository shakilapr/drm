# Pool Exhaustion & Dead-Reader Reclamation Algorithms

Scope: publisher behavior when the payload pool is exhausted (drop + counter, never block), and the slow-path reclamation of hazard slots owned by dead process incarnations. Implements architecture.md §Safe Handling of Pool Exhaustion, §Crash Recovery via OS Process Inspection. Owner: E1 (crate `drm-core::hazard::reclaim`). Phases 15–16. Depends on [hazard-protocol.md](hazard-protocol.md), [shm-backend.md](shm-backend.md), and [consumer-registration.md](consumer-registration.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-exhaustion-never-block` | publisher drops + `pool_exhausted++`; never waits for a reader | `REQ-CORE-022` |
| `REQ-CORE-exhaustion-latest-preserved` | current latest sample remains valid | `REQ-CORE-021` |
| `REQ-CORE-exhaustion-resume` | publishing resumes instantly when capacity returns | `REQ-CORE-023` |
| `REQ-CORE-reclaim-slow-path-only` | reclamation runs only when needed: exhausted → inspect → prove → reclaim | `REQ-CORE-058` |
| `REQ-CORE-reclaim-incarnation-proof` | reclaim only after exact process incarnation proven dead | `REQ-CORE-056` |
| `REQ-CORE-reclaim-pid-reuse-safe` | PID reuse at worst delays reclamation; never mis-reclaims a live reader | `REQ-CORE-057` |

## Data Structures

```rust
// ConsumerRecord and its generation-bearing hazard word are defined by consumer-registration.md.

// In CCB: pool_exhausted: AtomicU64 — monotonic counter, surfaced to drm-diag.
```

## Invariants

- **I-E1 NeverWaits**: with every slot protected, publish performs one pool scan and one consumer scan; no blocking, spinning, or retry loop. Test: `exhaustion_never_waits`.
- **I-E2 LatestValid**: the current latest sample remains readable and uncorrupted through exhaustion. Test: `exhaustion_latest_valid`.
- **I-E3 CounterExact**: `pool_exhausted` increments exactly once per dropped publish; no over/under count under races. Test: `exhaustion_counter_exact`.
- **I-E4 ResumeOnSelectable**: after any release/reclaim that leaves at least one selectable slot, the next publish succeeds (no grace period). Releasing protection on a slot still referenced by retained history creates no selectable slot and does not by itself unblock publishing. Test: `exhaustion_resume`.
- **I-E5 ReclaimOnlyDead**: a hazard slot is reclaimed only after the owning incarnation is proven dead; PID reuse never mis-reclaims a live reader (start-ticks mismatch ⇒ dead-for-us; match ⇒ live ⇒ no reclaim). Tests: `reclaim_kill_matrix` (kill at every borrow stage), `reclaim_pid_reuse` (simulated reuse fixture).
- **I-E6 ReclaimCasSafe**: reclamation clears protection only via SeqCst CAS on the packed exact `{registration_generation, slot}` word it observed. Test: `reclaim_recycled_slot_cas`.

## Functions

```rust
// Called only by channel-publish after one failed selection.
fn try_reclaim(ch: &Channel, i: usize) -> bool {
 let rec = &ch.ccb.consumers[i];
 if rec.state.load(Acquire) != Active { return false; }
 let word = rec.hazard.load(SeqCst);
 let (generation, slot) = unpack(word);
 if slot == NONE || generation != rec.registration_generation.load(Acquire) { return false; }
 let Some(identity) = snapshot_identity(rec) else { return false; };
 if rec.state.load(Acquire) != Active || rec.hazard.load(SeqCst) != word { return false; }
 if identity_dead_proven(&identity) {
  return rec.hazard.compare_exchange(word, pack(generation, NONE), SeqCst, SeqCst).is_ok();
 }
 false
}

fn scan_exhausted(ch: &Channel) {
 for i in 0..ch.consumer_count { try_reclaim(ch, i); } // statically bounded
}

fn identity_dead_proven(id: &ProcessIdentity) -> bool { shm_backend::identity_dead_proven(id) }
```

## Protocols

*Reclamation slow path (architecture §Crash Recovery, verbatim ordering):*

1. **Trigger**: pool exhaustion event (publisher drop) — reclamation never runs speculatively; no background polling.
2. **Inspect**: iterate consumer hazard records; identify owners of protected slots.
3. **Prove**: `identity_dead_proven` returns true only for termination, absence, or start-identity mismatch. Inspection errors mean no proof and no reclaim.
4. **Reclaim**: proven-dead ⇒ SeqCst CAS `protected[i]` to `NONE` (I-E6); slot returns to pool; next publish resumes once a slot is selectable (I-E4).
5. **Delay bound**: PID reuse may conservatively delay reclamation until the replacement PID exits; it can never reclaim a live reader (I-E5).

## Edge Cases & Errors

| Case | Result |
|---|---|
| All slots protected, publish arrives | drop + counter (I-E1); latest intact (I-E2) |
| Reader releases during reclamation scan | benign — release also frees; reclaim sees NONE |
| Owner PID reused before pidfd proof | replacement appears alive ⇒ conservative delay; fallback start mismatch proves old owner dead |
| Owner PID reused with identical start ticks | impossible by construction (ticks monotonic per boot) |
| Reclaim concurrent with genuine release | idempotent: clearing NONE twice is harmless |
| Consumer registered, never borrowed | record present, protected NONE — no reclaim needed |

## Test Matrix

| Case | Expected |
|---|---|
| `exhaustion_never_waits` (hold all slots) | bounded scan counts; zero waits/syscalls loops |
| `exhaustion_latest_valid` | I-E2 |
| `exhaustion_counter_exact` (concurrent drops) | I-E3 |
| `exhaustion_resume` | I-E4 |
| `reclaim_kill_matrix` (kill at every borrow stage) | I-E5: reclaim only after proof |
| `reclaim_pid_reuse` (simulated reuse fixture) | never mis-reclaims live reader |
| `reclaim_recycled_slot_cas` | I-E6: new owner's protection never cleared |
| `reclaim_no_polling` | no reclamation without exhaustion trigger |
