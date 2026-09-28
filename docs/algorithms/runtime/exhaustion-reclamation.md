# Pool Exhaustion & Dead-Reader Reclamation Algorithms

Scope: publisher behavior when the payload pool is exhausted (drop + counter, never block), and the
slow-path reclamation of hazard slots abandoned by dead or closed endpoints, proven via the
endpoint's OS registration lock. Implements architecture.md §Safe Handling of Pool Exhaustion,
§Crash Recovery via OS Registration-Lock Authority, §Process Lifecycle (fork). Owner: E1 (crate
`drm-core::hazard::reclaim`). Phases 15–16. Depends on [hazard-protocol.md](hazard-protocol.md),
[shm-backend.md](shm-backend.md), and [consumer-registration.md](consumer-registration.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-exhaustion-never-block` | publisher drops + `pool_exhausted++`; never waits for a reader | `REQ-CORE-022` |
| `REQ-CORE-exhaustion-latest-preserved` | current latest sample remains valid | `REQ-CORE-021` |
| `REQ-CORE-exhaustion-resume` | publishing resumes instantly when capacity returns | `REQ-CORE-023` |
| `REQ-CORE-reclaim-slow-path-only` | reclamation runs only when triggered; never speculative polling | `REQ-CORE-058` |
| `REQ-CORE-reclaim-lock-authority` | registration-lock acquisition is the sole liveness proof | `REQ-CORE-056` |
| `REQ-CORE-reclaim-identity-diagnostic` | pid/start identity is diagnostic metadata, never authority | `REQ-CORE-057` |
| `REQ-CORE-reclaim-rt-off-path` | hard-RT publisher threads never run process inspection or lock probes | `REQ-CORE-095` |

## Data Structures

```rust
// ConsumerRecord and its generation-bearing hazard word are defined by consumer-registration.md.

// In CCB: pool_exhausted: AtomicU64 — monotonic counter, surfaced to drm-diag.
// In CCB: reclaim_requested: AtomicBool — set by an RT publisher on exhaustion; consumed by the
// maintenance context (non-RT) which then performs scan_requested(ch).
```

## Invariants

- **I-E1 NeverWaits**: with every slot protected, publish performs one pool scan; no blocking,
  spinning, or retry loop. Test: `exhaustion_never_waits`.
- **I-E2 LatestValid**: the current latest sample remains readable and uncorrupted through
  exhaustion. Test: `exhaustion_latest_valid`.
- **I-E3 CounterExact**: `pool_exhausted` increments exactly once per dropped publish; no over/under
  count under races. Test: `exhaustion_counter_exact`.
- **I-E4 ResumeOnSelectable**: after any release/reclaim that leaves at least one selectable slot,
  the next publish succeeds (no grace period). Releasing protection on a slot still referenced by
  retained history creates no selectable slot and does not by itself unblock publishing. Test:
  `exhaustion_resume`.
- **I-E5 ReclaimOnlyAbandoned**: a hazard word is cleared only by its own owner's guard drop, or by
  a reclaimer holding that record's registration lock with the exact generation-bearing word
  observed under the lock. A live endpoint always holds its lock, so a live borrow is never
  reclaimed — independent of PID semantics, PID reuse, or PID-namespace visibility. Tests:
  `reclaim_kill_matrix` (kill at every borrow stage), `reclaim_live_reader_never_reclaimed`
  (cross-PID-namespace topology).
- **I-E6 ReclaimCasSafe**: reclamation clears protection only via SeqCst CAS on the packed exact
  `{registration_generation, slot}` word it observed under the lock. Test:
  `reclaim_recycled_slot_cas`.
- **I-E7 RtNoInspection**: on a hard-RT profile, an exhausted publish executes zero syscalls beyond
  the drop counters and flag store; reclamation is deferred to a maintenance context. Test:
  `reclaim_rt_off_path` (syscall trace during exhaustion).

## Functions

```rust
// Called by channel-publish after one failed selection.
fn publish_exhausted(ch: &Channel) {
 ch.pool_exhausted.fetch_add(1, Relaxed);
 ch.drop_count.fetch_add(1, Relaxed);
 if !ch.profile.is_hard_rt() {
  scan_requested(ch); // general-purpose profiles: one bounded opportunistic pass
 } else {
  ch.ccb.reclaim_requested.store(true, Release); // maintenance context performs scan_requested
 }
 // incoming sample is always dropped by the caller; never retried here
}

// Maintenance context (non-RT) or opportunistic general-purpose pass.
fn scan_requested(ch: &Channel) {
 ch.ccb.reclaim_requested.store(false, Relaxed);
 for i in 0..ch.consumer_count { try_reclaim(ch, i); } // statically bounded by consumer_count
}

fn try_reclaim(ch: &Channel, i: usize) -> bool {
 let rec = &ch.ccb.consumers[i];
 // NOTE: record state is NOT a filter here — a dead endpoint may sit in Closing with a live
 // borrow's word set. Only the lock proves liveness; only the word proves protection.
 let word = rec.hazard.load(SeqCst);
 let (generation, slot) = unpack(word);
 if slot == NONE || generation != rec.registration_generation.load(Acquire) { return false; }
 // Sole liveness proof: a live owner holds its registration lock for the whole lifetime.
 let Ok(lock) = lock_backend.try_exclusive(rec.generated_lock_path()) else {
  return false; // lock held ⇒ live authority ⇒ never reclaim
 };
 // Lock acquired: no live DRM owner retains authority. Reread everything under the lock.
 let cur = rec.hazard.load(SeqCst);
 if cur == word
  && generation == rec.registration_generation.load(Acquire) {
  // Dead endpoint, generation-bearing word confirmed under the lock: clear protection (I-E6).
  let _ = rec.hazard.compare_exchange(word, pack(generation, NONE), SeqCst, SeqCst);
 }
 // Normalize: with the lock held the endpoint is dead, so Vacant is the correct terminal state
 // for any stale Claiming/Active/Closing record; the next claimant re-claims under the lock.
 rec.state.store(Vacant, Release);
 drop(lock); // released; kernel releases on death anyway
 true
}
```

`ProcessIdentity` (`pid`, start ticks / creation time) is written at registration and surfaced only
through `drm-diag`. It never gates reclamation: PID reuse, `/proc` inspection errors, and
cross-PID-namespace `ESRCH` on a live process cannot cause a mis-reclaim because they are not part
of the proof (I-E5).

## Protocols

*Reclamation slow path (architecture §Crash Recovery via OS Registration-Lock Authority):*

1. **Trigger**: pool-exhaustion event (publisher drop) or maintenance request; reclamation never
   runs speculatively; no background polling.
2. **Probe**: attempt the endpoint's non-blocking registration `try_exclusive`. Failure
   (`WouldBlock`) ⇒ live authority ⇒ skip — this is conservative and cost-free.
3. **Prove + Inspect**: lock acquired ⇒ no live owner. Under the lock, reread generation/hazard;
   require the exact generation-matching word (state is not consulted — a dead `Closing` record
   with a live borrow's word is exactly the case being repaired).
4. **Reclaim**: SeqCst CAS the hazard word to `NONE` (I-E6); normalize the record to `Vacant`
   (terminal state for a lock-proven-dead endpoint, whatever its prior `Claiming`/`Active`/`Closing`
   state); release the lock. The slot returns to the pool; the next publish resumes once a slot is
   selectable (I-E4).
5. **Placement**: hard-RT publishers only set `reclaim_requested` (I-E7); general-purpose profiles
   may run one bounded pass inline as an optimization.

## Edge Cases & Errors

| Case | Result |
|---|---|
| All slots protected, publish arrives | drop + counter (I-E1); latest intact (I-E2); RT sets flag only |
| Reader releases during reclamation scan | benign — release also frees; reclaim sees NONE |
| Reclaimer in a different PID namespace from a live reader | lock probe fails ⇒ no reclaim; namespace topology cannot cause mis-reclaim (I-E5) |
| PID reused by an unrelated process | irrelevant — identity is diagnostic; lock authority decides |
| Reclaim concurrent with genuine release | idempotent: CAS or drop clears NONE; both harmless |
| Consumer registered, never borrowed | record present, protected NONE — no reclaim needed |
| Stale `Claiming`/`Closing`/`Active` record found under lock | hazard cleared when generation-matching; state normalized to `Vacant` (I-CR7) |

## Test Matrix

| Case | Expected |
|---|---|
| `exhaustion_never_waits` (hold all slots) | bounded scan counts; zero waits |
| `exhaustion_latest_valid` | I-E2 |
| `exhaustion_counter_exact` (concurrent drops) | I-E3 |
| `exhaustion_resume` | I-E4 |
| `reclaim_kill_matrix` (kill at every borrow stage) | I-E5: reclaim only under lock authority |
| `reclaim_dead_closing_borrow` (endpoint killed in `Closing` with a live-borrow word) | I-E5 + I-CR7: word cleared under lock; slot recovered |
| `reclaim_live_reader_never_reclaimed` (reader in disjoint PID namespace / simulated ESRCH) | I-E5: live reader never reclaimed |
| `reclaim_recycled_slot_cas` | I-E6: new owner's protection never cleared |
| `reclaim_no_polling` | no reclamation without exhaustion/maintenance trigger |
| `reclaim_rt_off_path` (syscall trace, H = 1/8/32/128) | I-E7: zero inspection syscalls on RT publish path |
| `reclaim_maintenance_wakeup` | `reclaim_requested` flag consumed exactly once |
