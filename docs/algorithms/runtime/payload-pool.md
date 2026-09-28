# Payload Pool Algorithms

Scope: per-topic fixed-capacity payload slot pools — dynamic-count layout at static offsets, structural isolation, size rejection before write, the payload pool invariant (`pool_size ≥ R+1`; drop-free at `pool_size ≥ R+H+1`), and the single in-progress-write guard. Implements architecture.md §Memory Architecture: Fixed Channels, §Fixed-Capacity Slot Pools, §Payload Pool Invariant, §Single Publisher (one in-progress write). Owner: E1 (crate `drm-core::channel::pool`), E2 (verifier hooks). Phase 13; invariant enforcement shared with Phase 19 ([memory-budget.md](../memory-budget.md)).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-pool-fixed-offsets` | §Fixed-Capacity Slot Pools — offsets static constants | `REQ-CORE-044` |
| `REQ-CORE-pool-dynamic-count` | pool size is deployment config; layout is `{base, stride, count}` | `REQ-CORE-043` |
| `REQ-CORE-pool-isolation` | §Structural Isolation — topics never compete for capacity | `REQ-CORE-046` |
| `REQ-CORE-pool-invariant-min` | `pool_size >= R + 1` mandatory; `drm verify` fails otherwise | `REQ-CORE-045` |
| `REQ-CORE-pool-dropfree` | `pool_size >= R + H + 1` ⇒ publisher progress with H borrowers | `REQ-CORE-042` |
| `REQ-CORE-pool-size-reject` | publish beyond structural max rejected **before any write** | `REQ-CORE-048` |
| `REQ-CORE-pool-one-writer` | one in-progress write per topic (non-blocking `Busy`) | `REQ-CORE-047` |
| `REQ-CORE-pool-default-latest` | `@history: latest` defaults to a pool of 3 slots | `REQ-CORE-041` |

## Data Structures

```rust
pub struct PoolLayout {
 base: u64, // offset of slot 0 from segment start (static constant)
 stride: u64, // offset(i) = base + i*stride; stride = align_up(slot_size, slot_align)
 count: u64, // pool_size — deployment configuration, NOT compile-time
 header_size: u64, // codec header_end: fixed fields + descriptor block (canon-codec two-region)
 sample_max: u64, // codec sample_max = header_end + max tail (one canonical sample bound)
 slot_size: u64, // = align_up(sample_max, slot_align)
 slot_align: u64, // canon-codec align rules (incl. tail element alignment)
 variable: bool, // struct has ≥1 variable member (descriptor block + tail)
}
// The pool carries CODE-OWNED sizing only: header_size/sample_max come from drm-workspace::canon
// (layout.json). It never assumes a fixed-offset u32 length header — a composite variable topic
// (e.g. {stamp_ns: u64, data: bytes[<=N]}) writes the descriptor inside the header and the body
// at an element-aligned tail offset (canon-codec.md §Fixed Header + Bounded Tail).
// Emitted by drm-workspace::canon (layout.json), consumed by drm-core (I6 canon-codec).
// Offsets never relocate for the mapping lifetime — no realloc, no remap, no coalescing.
// count is runtime data, so the layout is {base, stride, count}, never [u64; POOL].

pub struct PayloadPool {
 layout: PoolLayout,
 busy: AtomicBool, // VIEW of ccb.publish_busy (channel-metadata.md sole physical owner; I-HZ6)
 scratch: FixedBitSet, // activation-owned publisher scratch; count bits
}
fn slot_offset(layout: &PoolLayout, i: u64) -> Result<usize, Status> {
 if i >= layout.count { return Err(InvalidArg); }
 let offset = layout.stride.checked_mul(i).and_then(|n| layout.base.checked_add(n))
  .ok_or(InvalidConfig)?;
 usize::try_from(offset).map_err(|_| InvalidConfig)
}
```

## Invariants

- **I-P1 OffsetsStatic**: `offset(i)` identical across process restarts and remaps; no pointer relocation ever; `count` may differ between deployments without changing `stride`. Test: `pool_offsets_static`.
- **I-P2 Isolation**: each topic owns its pool; no cross-topic free-list or borrowing is possible (separate mappings/partitions). Test: `pool_isolation` (negative proof).
- **I-P3 RejectBeforeWrite**: publish whose canonical `sample_len` > `sample_max` ⇒ `InvalidArg` before any destination byte changes. Test: `pool_size_reject` (max, max+1).
- **I-P4 InvariantMin**: attach/config with `pool_size < R+1` ⇒ `InvalidConfig` (channel-metadata validate; verifier rejects at build). Test: `pool_invariant_min`.
- **I-P5 DropFree**: with `pool_size ≥ R+H+1` and H borrowers holding aged-out samples and one in-progress write, publisher still progresses (no drop). Test: `pool_dropfree` (property; extends to Phase 14 with hazard-protocol.md).
- **I-P6 DefaultLatest**: an unspecified `@pool_size` on a `@history: latest` topic resolves to 3. Test: `pool_default_latest`.

## Functions

```rust
// Copies the CANONICAL sample (fixed header + bounded tail, produced by the generated codec
// encoder — canon-codec.md) into the slot. The pool never authors header bytes and never assumes
// a fixed-offset length; it only bounds the sample and exposes the slot.
fn write_checked(pool: &PayloadPool, slot: u64, canonical: &[u8]) -> Result<SlotSel, Status> {
 if canonical.len() as u64 > pool.layout.sample_max { // I-P3 — before any write
  return Err(InvalidArg);
 }
 let off = slot_offset(&pool.layout, slot)?;
 copy_to(buf[off .. off + canonical.len()], canonical)?;
 Ok(SlotSel { slot, len: canonical.len() as u64 })
}


```

There is no unprotected round-robin implementation, even in an intermediate phase. The only slot
selector is the history- and hazard-aware bounded scan in hazard-protocol.md. The complete operation
ordering is [channel-publish.md](channel-publish.md).

## Protocols

*Default sizing:*

1. `@history: latest` (R = 1) with no explicit `@pool_size` ⇒ `@pool_size: 3` (I-P6): one latest, one in-progress write, one active borrow.
2. Explicit `@pool_size: N` overrides; verifier enforces `N ≥ R+1` and flags drop-free shortfall below `R+H+1`.

*Drop-free guarantee proof sketch (I-P5; completed with hazard-protocol.md in Phase 14):*

0. `H` is statically known per topic: `H = Σ declared borrowers over consuming nodes + the
   reserved diagnostic-consumer range` (architecture §Consumer; consumer-registration.md reserves
   exactly that many records). Dynamic borrower counts would invalidate this proof — they are
   rejected by the manifest schema.
1. Pool holds `pool_size` slots; history ring references at most `R` distinct slots.
2. Up to `H` consumers hold borrows of aged-out samples ⇒ at most `H` additional slots are protected.
3. Before selection, at most `R + H` slots are unavailable.
4. With `pool_size ≥ R + H + 1`, one slot can be selected as the sole in-progress write.

## Edge Cases & Errors

| Case | Result |
|---|---|
| canonical sample_len > `sample_max` | `InvalidArg`, zero writes (I-P3) |
| sample_len == `sample_max` | accepted; all slot bytes used |
| empty variable body (single-variable topic) | valid: descriptor count 0, no tail bytes |
| `pool_size < R+1` | `InvalidConfig` at attach + build-time verify error |
| concurrent publish | second ⇒ `Status::Busy` (hazard-protocol I-HZ6) |
| all slots busy (pre-hazard simplification) | cannot occur at Phase 13 (no borrows); formalized in Phase 14/15 |

## Test Matrix

| Case | Expected |
|---|---|
| `pool_offsets_static` | I-P1 |
| `pool_isolation` | I-P2 (negative) |
| `pool_size_reject` (max/max+1) | I-P3 |
| `pool_invariant_min` | I-P4 |
| `pool_dropfree` (R+H+1 property) | I-P5 |
| `pool_default_latest` | I-P6 |
| edge sizes (0, 1, max−1, max) | correct behavior |
