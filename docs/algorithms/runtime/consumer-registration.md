# Consumer Registration Algorithms

Scope: fixed consumer-record claim, identity publication, clean close, dead-owner takeover,
generation-safe hazard reuse, and stale-record normalization. Implements architecture.md §Consumer
Registration and Reuse, §Crash Recovery via OS Registration-Lock Authority. Owner:
E1 (`drm-core::channel::consumer`). Phases 14–16.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-consumer-static-record` | records fixed at activation | `REQ-CORE-009` |
| `REQ-CORE-consumer-exclusive-claim` | one live owner per declared endpoint | `REQ-CORE-006` |
| `REQ-CORE-consumer-incarnation` | owner identity is incarnation-safe | `REQ-CORE-008` |
| `REQ-CORE-consumer-generation` | recycled record cannot be cleared by stale owner/reclaimer | `REQ-CORE-007` |
| `REQ-CORE-consumer-clean-close` | close demotes `Active→Closing`; completes only with no outstanding borrow | `REQ-CORE-005` |
| `REQ-CORE-consumer-diagnostic-bound` | ad-hoc tools use a fixed diagnostic record range | `REQ-CORE-079` |

## Data Structures

```rust
pub enum RegistrationState { Vacant = 0, Claiming = 1, Active = 2, Closing = 3 }

pub struct ConsumerRecord {
 state: AtomicU32,
 registration_generation: AtomicU32,
 owner_pid: AtomicU32, // diagnostic only — never a liveness authority
 owner_start: AtomicU64, // diagnostic only — never a liveness authority
 hazard: AtomicU64, // high32 = registration_generation; low32 = slot or NONE
 ack_seq: AtomicU64, // only used by declared required-sequential endpoints
}
```

All fields are naturally aligned. Generation zero is invalid. Generation exhaustion refuses reuse.
Application endpoints use a generated record index and
`t_<topic_id>.<consumer_id>.consumer.lock` (the same generated topic identifier family as the
segment name; deployment-identity.md — never canonical topic strings). Diagnostic tools scan only
the fixed diagnostic subrange at attachment. The OS lock is held for the registration lifetime and
released by the kernel on death.

Record count per topic is statically fixed at activation: `Σ declared borrowers` over consuming
nodes (architecture §Consumer manifest `borrowers = N`, default 1) plus the reserved
diagnostic-consumer range. Each borrower claims one record at consumer-endpoint open; no vacant
record ⇒ `Busy`. There is no runtime record allocation — this static count is what makes the
payload-pool `H` (and the `pool_size >= R + H + 1` proof) verifiable.

## Invariants

- **I-CR1 ExclusiveOwner**: two live processes cannot own one record. Test: `consumer_claim_race`.
- **I-CR2 PublishOrder**: `Active` implies complete identity and `hazard={generation,NONE}` are visible.
  Test: `consumer_claim_order`.
- **I-CR3 StaleClearSafe**: an old guard or reclaimer cannot clear a new owner's hazard. Test:
  `consumer_generation_reuse`.
- **I-CR4 CloseSafe**: clean close succeeds only with the exact generation-bearing `{g, NONE}` word;
  an outstanding borrow preserves the record (state stays `Closing`). Test: `consumer_close_borrowed`.
- **I-CR5 TakeoverProven**: takeover occurs only after the registration lock proves no live owner
  retains authority. Test: `consumer_takeover_identity`.
- **I-CR6 CloseAcquireRace**: a borrow created concurrently with a close either observes `Active`
  before the close, or loses the hazard CAS / fails the acquisition recheck and aborts before
  touching payload bytes; a close never vacates a record whose hazard was installed after its check.
  Test: `consumer_close_vs_acquire` (Loom: close at every instruction boundary of acquisition).
- **I-CR7 WordIsProtection**: a hazard word protects its slot iff it names the record's **current**
  registration generation and a slot — regardless of record state (`Active` or `Closing` with a
  live borrow both protect). A record interrupted in `Claiming` carries the parked generation-0 word
  (`claim` parses it under the registration lock before publishing `Claiming`): generation zero is
  invalid, so it protects nothing, pins nothing, and is overwritten under the registration lock.
  Test: `consumer_claiming_orphan`.

## Functions

```rust
fn claim(rec: &ConsumerRecord, me: ProcessIdentity, floor: u64) -> Result<Registration, Status> {
 let lock = lock_backend.try_exclusive(rec.generated_lock_path())?; // Busy if live owner
 // Lock held: prior owner is dead or closed. Normalize FIRST — park a generation-0 word
 // (generation zero is invalid, so it matches no current generation) before publishing Claiming.
 // A crash anywhere in Claiming therefore leaves a word that protects/pins nothing (I-CR7);
 // without this, the stale prior-owner word can still carry the *current* generation.
 rec.hazard.store(pack(0, NONE), SeqCst);
 rec.state.store(Claiming, Release);
 let Some(generation) = rec.registration_generation.load(Relaxed).checked_add(1)
  .filter(|g| *g != 0) else {
  rec.state.store(Vacant, Release);
  return Err(GenerationExhausted);
 };
 rec.owner_pid.store(me.pid, Relaxed);
 rec.owner_start.store(me.start, Relaxed);
 rec.registration_generation.store(generation, Relaxed);
 rec.hazard.store(pack(generation, NONE), SeqCst);
 rec.ack_seq.fetch_max(floor, Relaxed); // never forget completed required sequences
 let next_needed = rec.ack_seq.load(Acquire);
 rec.state.store(Active, Release);
 Ok(Registration { rec, generation, next_needed, lock })
}

fn close(reg: &mut Registration) -> Result<(), Status> {
 // 1. Demotion, idempotent: Active → Closing; an existing Closing (re-close after a Busy whose
 // borrow was released in between) re-enters the hazard check. No new borrow may start behind
 // a Closing record.
 match reg.rec.state.load(Acquire) {
  Active => {
   if reg.rec.state.compare_exchange(Active, Closing, AcqRel, Acquire).is_err() {
    return Err(InvalidData);
   }
  }
  Closing => {} // re-close after Busy + release
  _ => return Err(InvalidData), // Vacant/Claiming: not ours to close
 }
 // 2. Exact generation-bearing hazard check under Closing.
 if reg.rec.hazard.load(SeqCst) != pack(reg.generation, NONE) {
  return Err(Busy); // outstanding borrow: record stays Closing; release, then re-close
 }
 reg.rec.state.store(Vacant, Release);
 reg.release_lock();
 Ok(())
}
```

If initialization fails after entering `Claiming`, the claimant stores `Vacant`. Death at any claim
step releases the OS lock; the next claimant (or the reclaimer — same lock) overwrites stale state
with a new generation. Fresh segments initialize every record to `Vacant`, generation zero, hazard
`{0,NONE}`, and ack zero.

## Functions — Acquisition Recheck

```rust
// Final step of every borrow/copy acquisition (hazard-protocol.md acquire): after installing
// the hazard and before touching payload bytes.
fn acquisition_valid(rec: &ConsumerRecord, generation: u32) -> bool {
 rec.state.load(Acquire) == Active
  && rec.registration_generation.load(Acquire) == generation
}
// If either check fails: CAS the exact installed word pack(g, slot) → pack(g, NONE) (SeqCst),
// leave the payload untouched, return Busy (record lost authority — reattach or re-claim).
```

## Protocols

*Borrow:* verify `Active` and generation; CAS `pack(g,NONE) → pack(g,slot)`; reread the ring; on
mismatch CAS the exact word back to `pack(g,NONE)`. **After a successful CAS, recheck
`acquisition_valid` (I-CR6); on failure clear the exact installed word and abort without reading
payload bytes.** Guard drop performs the same exact CAS.

*Close vs borrow (I-CR6):* `close()` demotes `Active → Closing` first, so a racing acquisition's
hazard CAS from `pack(g,NONE)` either precedes the demotion (close then sees the slot and returns
`Busy`) or the acquisition fails its post-install recheck (`state == Closing` or generation changed)
and aborts before reading payload bytes. A close that observes an outstanding borrow leaves the
record in `Closing`; the guard's release clears the exact hazard word only (no state transition),
and the application re-closes — `close()` re-enters from `Closing` and completes
(`Closing → Vacant`, lock release). An abandoned `Closing` record is eventually taken over via
kernel lock release (owner death) or normalized by the reclaimer holding the lock.

*Dead takeover:* acquiring the endpoint OS lock proves no live DRM owner retains authority. The new
claimant increments generation and overwrites stale registration state. Hazard reclamation uses the
same authority: see exhaustion-reclamation.md.

*Stale-`Claiming` normalization (I-CR7):* `claim` parks the generation-0 word under the registration
lock before `state = Claiming`, so a crash inside `Claiming` leaves `{Claiming, pack(0, NONE)}` — a
generation that never matches any record. Per the hazard-validity rule it protects nothing, so the
publisher never honors it and it is overwritten under the registration lock by the next claimant or
reclaimer.

*Required consumer:* initial `ack_seq` is the current `first_valid_seq`, meaning prior-run sequences
need no acknowledgement. Reattachment initializes its local cursor to `ack_seq - 1` (`None` when
`ack_seq == 0`), so the first `next()` requests exactly `ack_seq`. Successful completion advances it
monotonically with a CAS-max.

## Edge Cases & Errors

| Case | Result |
|---|---|
| live declared endpoint already attached | `Busy` |
| dead owner | lock-proven takeover with generation increment |
| close with borrow | `Busy`; record stays `Closing` until release + re-close |
| close racing a new borrow | borrow loses (aborts before payload) or close sees `Busy` (I-CR6) |
| borrow released in `Closing` | guard clears exact word; re-close completes (`Closing → Vacant`) |
| `Registration` dropped without `close()` | kernel releases lock; next claim takes over; bindings scope borrows to the registration so no orphan guard can exist |
| crash in `Claiming` | stale word protects nothing (I-CR7); normalized under registration lock |
| generation exhausted | `GenerationExhausted`; no reuse |
| undeclared record index | `Permission` |
| every reserved diagnostic record owned | `Busy` |

## Test Matrix

| Case | Expected |
|---|---|
| `consumer_claim_race` | exactly one owner |
| `consumer_claim_order` | I-CR2 |
| `consumer_generation_reuse` | stale clear fails |
| `consumer_close_borrowed` | I-CR4 |
| `consumer_close_vs_acquire` (Loom: close interleaved at every acquisition boundary) | I-CR6: hazard installed after close's check never survives to a Vacant record; payload never read after demotion |
| `consumer_close_reentrant` (close → Busy → guard release → re-close) | I-CR4: second close completes `Closing → Vacant` |
| `consumer_takeover_identity` (lock-based) | I-CR5 |
| `consumer_claiming_orphan` (kill between `Claiming` and hazard store) | I-CR7: slot selectable, normalizer repairs under lock |
| kill in each claim step | recoverable without live-owner takeover |
| `required_reattach_resume` | first request equals persisted `ack_seq`; no acknowledged replay |
| `diagnostic_claim_bound` | scans at most `diagnostic_count`; no channel growth |
