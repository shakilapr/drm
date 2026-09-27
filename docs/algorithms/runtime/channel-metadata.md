# Channel Metadata & Attachment Algorithms

Scope: channel descriptor block (channel-level metadata) as a naturally aligned control block, one-time attachment validation, epoch observe/reset protocol, and the per-run addressability watermark. Implements architecture.md §No Universal Envelope (channel/sample/transport layers), §Publisher Epoch, §Startup Compatibility Guard, §Individual Segment Validation, §Channel Metadata (aligned control block). Owner: E1 (crate `drm-core::channel`), E2 (descriptor spec + layout). Phases 5, 17, 18.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-desc-byte-exact` | §Channel Metadata — stored once per segment | `REQ-CORE-015` |
| `REQ-CORE-desc-aligned` | control state naturally aligned; no packed atomics; offset asserts | `REQ-CORE-014` |
| `REQ-CORE-desc-topic-identity` | descriptor carries channel identifier + topic name | `REQ-CORE-017` |
| `REQ-CORE-attach-validate-once` | compared once at attach; steady-state zero checks | `REQ-CORE-002` |
| `REQ-CORE-epoch-increment` | §Publisher Epoch — restart ⇒ epoch+1, consumers reset | `REQ-CORE-019` |
| `REQ-CORE-first-valid` | restart advances `first_valid_seq`; prior run never addressable | `REQ-CORE-026` |
| `REQ-CORE-fingerprint-guard` | node fingerprint checked against deployment before mapping | `REQ-CORE-025` |
| `REQ-CORE-segment-validation` | §Individual Segment Validation — secondary schema/layout check | `REQ-CORE-068` |
| `REQ-CORE-desc-exact-layout` | exact offsets, counts, and segment size checked with overflow-safe math | `REQ-CORE-016` |

## Data Structures

```rust
#[repr(C, align(8))]
pub struct SegmentPrefix { // fixed 16-byte bootstrap at offset zero
 magic: [u8; 4],
 layout_version: u32,
 segment_state: AtomicU32, // Zero/Init/Ready
 descriptor_size: u32,
}

// Immutable descriptor follows SegmentPrefix. Dynamic arrays use checked offsets, never pointers.
#[repr(C, align(8))]
pub struct ChannelDescriptor {
 flags: u32,
 _pad_header: u32,
 schema_hash: [u8; 16], // 128-bit canonical schema content hash (workspace-model contract inputs)
 layout_hash: [u8; 16], // exact descriptor + CCB + canonical payload layout
 topic_handle: u32, // compact runtime handle (identity; handle-system.md)
 topic_name_len: u16, // length of topic name stored contiguously after the header
 overflow_policy: u8, // 0 = overwrite, 1 = reject
 _pad0: u8,
 payload_capacity: u64, // fixed size or max bound (bytes)
 history_depth: u64, // R (1 for `@history: latest`)
 pool_size: u64, // slots (payload-pool.md; invariant pool_size ≥ R+1)
 consumer_count: u32,
 required_count: u32,
 diagnostic_count: u32,
 _pad_counts: u32,
 ccb_offset: u64,
 ring_offset: u64,
 consumer_offset: u64,
 slot_generation_offset: u64,
 payload_offset: u64,
 segment_size: u64,
 reserved: [u8; 16], // zeroed; forward-compat
}

// Fixed bootstrap/control prefix. Dynamic ring and consumer records follow at descriptor offsets.
#[repr(C, align(8))]
pub struct ChannelControlBlock {
 publisher_epoch: AtomicU32,
 _pad_epoch: u32,
 first_valid_seq: AtomicU64, // sole physical owner
 write_seq: AtomicU64,       // next sequence
 pool_exhausted: AtomicU64,
 publish_busy: AtomicBool,
 _pad: [u8; 7],
}
// Compile-time asserts:
// const _: () = assert!(align_of::<ChannelDescriptor>() == 8);
// const _: () = assert!(offset_of!(ChannelControlBlock, first_valid_seq) % 8 == 0);
// // exact golden offsets pinned in tests/fixtures/golden/descriptor/ and checked by I-M1.
```

## Invariants

- **I-M1 ByteExact**: descriptor bytes match the golden table field-for-field (including padding). Test: `descriptor_golden` (byte-inspection harness), `descriptor_offset_asserts` (compile-time + runtime).
- **I-M2 AttachOnce**: schema/fingerprint validation happens exactly once per attachment; 1e6 steady-state reads ⇒ 0 re-checks. Test: `attach_once`.
- **I-M3 EpochReset**: consumer observing `epoch' > epoch` resets cursor state; pre-restart samples are never interpreted as post-restart data. Test: `epoch_restart` (publisher-lifecycle.md).
- **I-M4 GuardRefusal**: binary whose compiled node fingerprint differs from its deployment record refuses before opening any segment. Test: `fingerprint_guard`.
- **I-M5 AlignedAtomics**: every atomic field is naturally aligned; no packed atomic is addressable. Test: `descriptor_offset_asserts`.
- **I-M6 SegmentReady**: attachment returns `NotReady` while the fixed prefix state is not `Ready`; no other bytes are interpreted. Test: `attach_init_refused`.
- **I-M7 ExactLayout**: attachment accepts only exact immutable fields, zero reserved bytes, valid checked ranges, and non-overlapping aligned regions within `segment_size`. Test: `attach_layout_matrix`.

## Functions

```rust
// Attach validation — once, at open (publisher or consumer)
fn validate_on_attach(map: &Mapping, expect: &Expectation) -> Result<ChannelView, Status> {
 let prefix = map.prefix()?; // fixed offset zero; mapping has at least 16 bytes
 if prefix.segment_state.load(Acquire) != Ready { return Err(NotReady); }
 let desc = map.checked_descriptor()?;
 if prefix.magic != *b"DRMC" { return Err(InvalidData); }
 if !SUPPORTED_LAYOUT_VERSIONS.contains(&prefix.layout_version) { return Err(WrongAbi); }
 if prefix.descriptor_size as usize != size_of::<ChannelDescriptor>() { return Err(WrongAbi); }
 let ccb = map.checked_ccb(desc.ccb_offset)?;
 if desc.schema_hash != expect.schema_hash { return Err(InvalidData); } // secondary per-segment check
 if desc.layout_hash != expect.layout_hash { return Err(WrongAbi); }
 if desc.topic_handle != expect.topic_handle { return Err(InvalidData); } // identity matches
 if map.topic_name(desc)? != expect.topic_name { return Err(InvalidData); }
 if desc.payload_capacity != expect.payload_capacity
  || desc.history_depth != expect.history_depth
  || desc.pool_size != expect.pool_size
  || desc.overflow_policy != expect.overflow_policy
  || desc.consumer_count != expect.consumer_count
  || desc.required_count != expect.required_count
  || desc.diagnostic_count != expect.diagnostic_count
 || desc.segment_size != map.len() as u64 { return Err(WrongAbi); }
 let reserved = desc.required_count.checked_add(desc.diagnostic_count).ok_or(InvalidConfig)?;
 if desc.history_depth == 0 || reserved > desc.consumer_count {
  return Err(InvalidConfig);
 }
 let min_pool = desc.history_depth.checked_add(1).ok_or(InvalidConfig)?;
 if desc.pool_size < min_pool { return Err(InvalidConfig); }
 if desc.pool_size >= u32::MAX as u64 { return Err(InvalidConfig); } // reserves slot NONE
 if desc.flags.contains(VARIABLE) && desc.payload_capacity > u32::MAX as u64 {
  return Err(InvalidConfig); // canonical variable-length header is u32
 }
 validate_aligned_nonoverlapping_ranges(desc, map.len())?; // checked add/mul only
 if desc.reserved.iter().any(|b| *b != 0) { return Err(WrongAbi); }
 require_lock_free_shared_atomics()?;
 Ok(ChannelView::from_checked_offsets(map, desc, ccb))
}
// After Ok: steady-state publish/read never re-validates (I-M2). Diag tools may inspect freely.

// Epoch observe (cursor-delivery.md calls around each acquisition)
fn epoch_changed(ccb: &ChannelControlBlock, local_epoch: u32) -> bool {
 ccb.publisher_epoch.load(Acquire) != local_epoch
}
// first_valid_seq is read on the read path to bound the addressable window (ring-history I5).
fn first_valid(ccb: &ChannelControlBlock) -> u64 { ccb.first_valid_seq.load(Acquire) }
```

## Protocols

*Attachment (both roles):*

1. Validate the node contract fingerprint against `deployment.json` before mapping.
2. Map the exact expected segment size (shm-backend.md).
3. If state is `Init`, wait with a bounded/cancellable startup policy; consumers never repair it.
4. `validate_on_attach(mapping, expectation)` checks every immutable field and range exactly.
5. Claim the generated consumer record when applicable (consumer-registration.md).
6. Snapshot `local_epoch = ccb.publisher_epoch.load(Acquire)`.
7. Steady state: no further immutable descriptor validation; epoch observation resets local state.

*Epoch observe/reset (consumer, per read loop):*

1. `if epoch_changed(desc, local_epoch)`: reset local cursor state (cursor-delivery.md), adopt `local_epoch = e`.
2. The restarted publisher has already advanced `first_valid_seq` (publisher-lifecycle.md), so no prior-run sample is addressable (ring-history I5).
3. Freshness transitions handled by publisher-lifecycle.md.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Bootstrap state `Init` | `NotReady`; caller's bounded startup wait may retry |
| Wrong magic / unknown layout version | refuse attachment (`InvalidData` / `WrongAbi`) |
| Schema hash or topic handle mismatch | `InvalidData` — stale binary vs deployment |
| `pool_size < R+1` | `InvalidConfig` at attach (verifier also rejects at build) |
| Epoch race during attach | snapshot after validation; consumer resets on first observation |
| Atomic field placed in packed layout | compile error from alignment asserts (I-M5) |
| Stale segment from prior deployment | namespace scoping (deployment-identity.md) prevents attachment; belt-and-braces fingerprint guard |

## Test Matrix

| Case | Expected |
|---|---|
| `descriptor_golden` | I-M1 byte-exact |
| `descriptor_offset_asserts` | alignment/offset asserts hold (I-M5) |
| `attach_once` (1e6 reads) | 0 re-validations |
| `attach_init_refused` (`segment_state != Ready`) | I-M6 |
| `attach_mismatch` (hash/handle/capacity/pool invariant) | explicit errors |
| `epoch_restart` | I-M3 (with publisher-lifecycle.md) |
| `fingerprint_guard` | I-M4 (with deployment-identity.md) |
| `attach_layout_matrix` (each field/range/alignment) | I-M7 |
