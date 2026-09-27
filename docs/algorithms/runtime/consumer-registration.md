# Consumer Registration Algorithms

Scope: fixed consumer-record claim, identity publication, clean close, dead-owner takeover, and
generation-safe hazard reuse. Implements architecture.md §Consumer Registration and Reuse. Owner:
E1 (`drm-core::channel::consumer`).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-consumer-static-record` | records fixed at activation | — |
| `REQ-CORE-consumer-exclusive-claim` | one live owner per declared endpoint | — |
| `REQ-CORE-consumer-incarnation` | owner identity is incarnation-safe | — |
| `REQ-CORE-consumer-generation` | recycled record cannot be cleared by stale owner/reclaimer | — |
| `REQ-CORE-consumer-clean-close` | close requires no outstanding borrow | — |

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
The record index is generated from the deployment contract; attachment never searches or allocates.

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
 loop {
  match rec.state.load(Acquire) {
   Vacant => if rec.state.compare_exchange(Vacant, Claiming, AcqRel, Acquire).is_err() { continue; },
   Claiming => return Err(Busy),
   Active => {
    let old = snapshot_owner(rec)?;
    if identity_alive(&old) { return Err(Busy); }
    if rec.state.compare_exchange(Active, Claiming, AcqRel, Acquire).is_err() { continue; }
   }
  }
  let generation = rec.registration_generation.load(Relaxed)
   .checked_add(1).filter(|g| *g != 0).ok_or(GenerationExhausted)?;
  rec.owner_pid.store(me.pid, Relaxed);
  rec.owner_start.store(me.start, Relaxed);
  rec.registration_generation.store(generation, Relaxed);
  rec.hazard.store(pack(generation, NONE), SeqCst);
  rec.ack_seq.store(floor, Relaxed);
  rec.state.store(Active, Release);
  return Ok(Registration { rec, generation });
 }
}

fn close(reg: Registration) -> Result<(), Status> {
 if reg.rec.hazard.load(SeqCst) != pack(reg.generation, NONE) { return Err(Busy); }
 reg.rec.state.compare_exchange(Active, Vacant, AcqRel, Acquire)
  .map(|_| ()).map_err(|_| InvalidData)
}
```

If initialization fails after entering `Claiming`, the claimant restores `Vacant`. Process death in
`Claiming` is recovered by the publisher/next claimant after proving the stored prior identity dead;
fresh segments initialize every record to `Vacant` with generation zero.

## Protocols

*Borrow:* verify `Active` and generation; CAS `pack(g,NONE) → pack(g,slot)`; reread the ring; on
mismatch CAS the exact word back to `pack(g,NONE)`. Guard drop performs the same exact CAS.

*Dead takeover:* snapshot state, generation, identity, and hazard; reread state and generation; prove
the identity dead; CAS `Active → Claiming`; increment generation and publish the new owner. Any
concurrent stale hazard CAS contains the old generation and fails.

*Required consumer:* initial `ack_seq` is the current `first_valid_seq`, meaning prior-run sequences
need no acknowledgement. Successful sequential completion advances it monotonically with a CAS-max.

## Edge Cases & Errors

| Case | Result |
|---|---|
| live declared endpoint already attached | `Busy` |
| dead owner | proven takeover with generation increment |
| close with borrow | `Busy`; record remains active |
| generation exhausted | `GenerationExhausted`; no reuse |
| undeclared record index | `Permission` |

## Test Matrix

| Case | Expected |
|---|---|
| `consumer_claim_race` | exactly one owner |
| `consumer_claim_order` | I-CR2 |
| `consumer_generation_reuse` | stale clear fails |
| `consumer_close_borrowed` | I-CR4 |
| `consumer_takeover_identity` | I-CR5 |
| kill in each claim step | recoverable without live-owner takeover |
