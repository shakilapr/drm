# Attached-channel data path

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: publishing and reading topic samples over an **attached** shared-memory
segment, i.e. by a process other than the one that created it. Segment
creation, attachment validation, publisher authority, epoch restart, and
stale-object classification are owned by
[publisher-lifecycle.md](publisher-lifecycle.md) and
[shm-backend.md](shm-backend.md); the in-process fast path is
[channel-publish.md](channel-publish.md). This document covers only the data
operations over the already-reserved segment regions.
Owner: E1 (crate `drm-core::channel::attached`). Phase 25. Depends on
[channel-metadata.md](channel-metadata.md), [ring-history.md](ring-history.md),
[cursor-delivery.md](cursor-delivery.md),
[hazard-protocol.md](hazard-protocol.md),
[payload-pool.md](payload-pool.md), [overflow-policies.md](overflow-policies.md).

Motivation (ADR [0007](../adr/0007-cross-process-channel-data-operations.md)):
the segment layout reserves the complete state machine, but v1.0 implemented
publish/read only in the heap-resident in-process `Channel`, so no second
process could exchange samples.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-attached-publish` | attached publisher writes a sample through the reserved pool/ring/CCB regions under Release ordering | `REQ-CORE-102` |
| `REQ-CORE-attached-read` | attached consumer reads latest/next with cursor state, gap reporting, and hazard borrowing | `REQ-CORE-103` |
| `REQ-CORE-attached-single-writer` | one publisher authority per segment; consumers attach read-only; a second publisher is refused | `REQ-CORE-104` |
| `REQ-CORE-attached-equivalence` | identical observable behaviour between the in-process fast path and the attached path for the same input sequence | `REQ-CORE-105` |
| `REQ-CORE-attached-epoch` | a restart epoch bump is observed by attached consumers; pre-restart sequences are unaddressable | `REQ-CORE-106` |
| `REQ-CORE-attached-abi` | the C ABI exposes attach/publish/read append-only; no existing symbol changes shape | `REQ-CORE-107` |

## Region Map

The reserved regions (offsets from `AttachedView`, validated once at attach;
never re-checked in steady state — I-M2):

| Region | Offset | Contents |
|---|---|---|
| prefix | 0 | state word (`SEGMENT_INIT`/`SEGMENT_READY`), ABI word |
| descriptor | after prefix | `ChannelDescriptor` (immutable) |
| name | after descriptor | canonical topic name |
| CCB | `ccb_offset` | counters, epoch, `publish_busy`, `reclaim_requested`, `first_valid_seq` |
| ring cells | `ring_offset` | per-sequence slot index + generation reference (depth R) |
| consumer records | `consumer_offset` | registration + hazard slot per consumer |
| slot generations | `slot_generation_offset` | per-slot generation counters (pool size) |
| payload pool | `payload_offset` | `pool_size` slots of `stride = align_up(sample_max, slot_align)` |

## Functions

```rust
// drm-core::channel::attached
fn publish(&self, src: &[u8]) -> Result<u64, Status>;          // sequence number
fn read_latest(&self, cur: &mut Cursor, dst: &mut [u8]) -> Result<Option<u64>, Status>;
fn read_next(&self, cur: &mut Cursor, dst: &mut [u8]) -> Result<ReadOutcome, Status>;
fn borrow_latest(&self, slot: &mut HazardSlot) -> Result<Borrowed, Status>;
fn release(&self, slot: &mut HazardSlot) -> Result<(), Status>;
```

## Invariants

- **I-AT1 RegionDiscipline**: every operation addresses the segment only through
  the `AttachedView` offsets; bounds are established once at attach. The payload
  **stride** is part of that agreement: `slot_align` is not carried in the
  descriptor, so attach proves it rather than trusting the caller —
  `payload_offset + pool_size * align_up(sample_max, slot_align)` must equal the
  descriptor's `segment_size` exactly, or the attachment is refused
  (`InvalidData`). A stride mismatch moves every slot after the first, so
  accepting it would return other samples' bytes as a valid sample.
- **I-AT2 SingleWriter**: `publish` is reachable only through a publisher
  authority acquired by `publisher_open`; consumers hold no write capability
  (AGENTS.md #4).
- **I-AT3 HazardUnchanged**: the borrow/release protocol, slot generations, and
  reclamation path are identical to the in-process path; a stalled or crashed
  consumer is reclaimed without ever invalidating a live borrow.
- **I-AT4 TransientBorrow**: an attached borrow is valid only while held; any
  external I/O must copy first (AGENTS.md #5).
- **I-AT5 EpochOrdering**: the epoch is published last on restart; a consumer
  observing a new epoch discards cursors bound to the previous one (I-L7).
- **I-AT6 Equivalence**: for one input sequence, the attached path and
  `Channel` produce identical observable state (sequences, ring contents,
  counters, per-consumer reads).

## Test Matrix

| Test | Invariant | Requirement | Status |
|---|---|---|---|
| `attached_publish_read_roundtrip` (two processes, N samples) | I-AT1, I-AT6 | `REQ-CORE-102`, `REQ-CORE-103` | implemented |
| `attached_slot_align_mismatch_refused` (stride ≠ descriptor size) | I-AT1 | `REQ-CORE-103` | implemented |
| `attached_second_publisher_refused` (`perf-matrix` cross-process leg) | I-AT2 | `REQ-CORE-104` | covered by the lifecycle harness |
| `attached_consumer_stalled_reclaimed` | I-AT3 | `REQ-CORE-103` | pending |
| `attached_borrow_copy_only` | I-AT4 | `REQ-CORE-103` | pending |
| `attached_epoch_restart_observed` | I-AT5 | `REQ-CORE-106` | pending |
| `attached_equivalence_vs_channel` | I-AT6 | `REQ-CORE-105` | pending |
| `attached_abi_append_only` | — | `REQ-CORE-107` | pending |

## Implementation Status

The copy-form data path is implemented in `crates/drm-core/src/channel/attached.rs`
(`AttachedChannel::publish` / `read_latest` / `read_next`), reusing the ring, pool,
and overflow helpers the in-process `Channel` uses; `HistoryRing::from_mapped`
views a segment's ring reservation, so one algorithm serves both forms.
Outstanding in this document: the zero-copy borrow form, consumer hazard
registration for attached readers, and attached required-acknowledgement floors
(a `reject` channel currently refuses attached publishes).

The C ABI surface for this path (`drm_attached_open` / `_close` / `_publish` /
`_read_latest` / `_read_next`) is implemented and covered by
`crates/drm-ffi/tests/attached_abi.rs` (`REQ-CORE-107`).
