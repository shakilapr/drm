# Consumer Registration Algorithms

Scope: fixed consumer-record claim, identity publication, clean close, dead-owner takeover, and
generation-safe hazard reuse. Implements architecture.md §Consumer Registration and Reuse. Owner:
E1 (`drm-core::channel::consumer`).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-consumer-static-record` | records fixed at activation | `REQ-CORE-009` |
| `REQ-CORE-consumer-exclusive-claim` | one live owner per declared endpoint | `REQ-CORE-006` |
| `REQ-CORE-consumer-incarnation` | owner identity is incarnation-safe | `REQ-CORE-008` |
| `REQ-CORE-consumer-generation` | recycled record cannot be cleared by stale owner/reclaimer | `REQ-CORE-007` |
| `REQ-CORE-consumer-clean-close` | close requires no outstanding borrow | `REQ-CORE-005` |
| `REQ-CORE-consumer-diagnostic-bound` | ad-hoc tools use a fixed diagnostic record range | `REQ-CORE-079` |

## Data Structures

```rust
pub enum RegistrationState { Vacant = 0, Claiming = 1, Active = 2 }

pub struct ConsumerRecord {
 state: AtomicU32,
 registration_generation: AtomicU32,
 owner_pid: AtomicU32,
 owner_start: AtomicU64,
 hazard: AtomicU64, // high32 = registration_generation; low32 = slot or NONE
 ack_seq: AtomicU64, // only used by declared required-sequential endpoints
}
```

All fields are naturally aligned. Generation zero is invalid. Generation exhaustion refuses reuse.
Application endpoints use a generated record index and
`<topic>.<consumer_id>.consumer.lock`. Diagnostic tools scan only the fixed diagnostic subrange at
attachment. The OS lock is held for the registration lifetime and released by the kernel on death.

## Invariants

- **I-CR1 ExclusiveOwner**: two live processes cannot own one record. Test: `consumer_claim_race`.
- **I-CR2 PublishOrder**: `Active` implies complete identity and `hazard={generation,NONE}` are visible.
  Test: `consumer_claim_order`.
- **I-CR3 StaleClearSafe**: an old guard or reclaimer cannot clear a new owner's hazard. Test:
  `consumer_generation_reuse`.
- **I-CR4 CloseSafe**: clean close succeeds only with `NONE`; `Busy` preserves the record. Test:
  `consumer_close_borrowed`.
- **I-CR5 TakeoverProven**: takeover occurs only after exact owner incarnation is proven dead. Test:
  `consumer_takeover_identity`.

## Functions

```rust
fn claim(rec: &ConsumerRecord, me: ProcessIdentity, floor: u64) -> Result<Registration, Status> {
 let lock = lock_backend.try_exclusive(rec.generated_lock_path())?; // Busy if live owner
 rec.state.store(Claiming, Release); // stale Claiming/Active is safe: prior OS lock is gone
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
 if reg.rec.hazard.load(SeqCst) != pack(reg.generation, NONE) { return Err(Busy); }
 reg.rec.state.store(Vacant, Release);
 reg.release_lock();
 Ok(())
}
```

If initialization fails after entering `Claiming`, the claimant restores `Vacant`. Death at any claim
step releases the OS lock; the next claimant overwrites stale state with a new generation. Fresh
segments initialize every record to `Vacant`, generation zero, hazard `{0,NONE}`, and ack zero.

## Protocols

*Borrow:* verify `Active` and generation; CAS `pack(g,NONE) → pack(g,slot)`; reread the ring; on
mismatch CAS the exact word back to `pack(g,NONE)`. Guard drop performs the same exact CAS.

*Dead takeover:* acquiring the endpoint OS lock proves no live DRM owner retains authority. The new
claimant increments generation and overwrites stale registration state. Hazard reclamation remains
separate and still requires exact process-incarnation proof before the old payload slot is reused.

*Required consumer:* initial `ack_seq` is the current `first_valid_seq`, meaning prior-run sequences
need no acknowledgement. Reattachment initializes its local cursor to `ack_seq - 1` (`None` when
`ack_seq == 0`), so the first `next()` requests exactly `ack_seq`. Successful completion advances it
monotonically with a CAS-max.

## Edge Cases & Errors

| Case | Result |
|---|---|
| live declared endpoint already attached | `Busy` |
| dead owner | proven takeover with generation increment |
| close with borrow | `Busy`; record remains active |
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
| `consumer_takeover_identity` | I-CR5 |
| kill in each claim step | recoverable without live-owner takeover |
| `required_reattach_resume` | first request equals persisted `ack_seq`; no acknowledged replay |
| `diagnostic_claim_bound` | scans at most `diagnostic_count`; no channel growth |
