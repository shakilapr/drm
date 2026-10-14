# ADR-0007: Cross-process channel data operations over the segment layout

## Status

`accepted`
Date: 2026-10-14 · Deciders: E1 (runtime core), E2 (toolchain)

## Context and Problem Statement

`docs/architecture.md` §Local Data Delivery promises a process-spanning local
data fabric: "Programs attach directly to the pre-allocated shared memory
segments using static IDs… A microcontroller driver in C, a computer vision
pipeline in Python, a state estimator in Rust, and a user interface in
TypeScript all read the exact same memory buffers."

An interop exercise against real ROS 2 packages (rviz2 14.1.24, nav2,
`tf2_ros`, `image_common`, on ROS 2 Jazzy / Ubuntu 24.04) required exactly that
capability — a second process publishing into, and reading from, a channel a
different process owns. It falsified the assumption that the capability exists.

Measured state of v1.0:

- The segment layout reserves the full state machine — prefix, descriptor,
  name, CCB, ring cells, consumer records, slot generations, payload pool
  (`channel::lifecycle::segment_layout`); `validate_on_attach` returns every
  region offset as `AttachedView`.
- The publisher writes a subset of CCB fields atomically through mapped offsets
  (`lifecycle.rs` `ccb_ptr!`: `publish_busy`, `reclaim_requested`,
  `first_valid_seq`, `publisher_epoch`) and `init_fresh` writes prefix +
  descriptor + topic name.
- **No function publishes a sample into, or reads a sample from, that layout.**
  The only publish/read state machine is `Channel::publish` / `Channel::latest`
  (`channel/mod.rs`), and `Channel` is process-local by construction:
  `Channel::activate` allocates its own heap arena (`vec![0_u8; arena_bytes]`)
  plus in-struct atomics.
- Consequently `publisher_open` / `consumer_open` provide authority, epoch,
  stale-object classification, and validated attachment — but hand back only
  checked offsets plus a raw `Mapping` (`ptr()`/`len()`).
- Two-process probe (public API only, `drm-core` as a path dependency): a
  second process attached as consumer to a publisher's segment and read the
  publisher's bytes at `payload_offset` — shared memory and the offsets are
  real, so only the protocol operations are absent.
- The C ABI cannot substitute: `drm-ffi` is an in-process endpoint
  (`drm_context_open` calls `Channel::activate`), fixed to one topic id and
  64-byte samples. `xtask/src/cross_os.rs` marks the C/C++/Python interop cells
  "scheduled until their router-transport bindings land (Phase 32/33)".

Effect: an application process cannot exchange topic data with another process
over DRM shared memory. In-process Rust is the only working local data path,
so the multi-language shared-memory promise is unmet, and no external runtime
(ROS 2 included) can be integrated.

## Decision Drivers

- The architecture statement above is normative; the implementation must
  satisfy it or the statement must change.
- AGENTS.md #4 brokerless single-writer; #5 transient borrow lifecycle (the
  hazard protocol must keep working unchanged, and no borrowed sample may be
  retained by external I/O).
- Frozen v1.0 surfaces (API inventory, C ABI, golden vectors): the fix must be
  additive, never a redefinition.
- Testability: the claim must be provable by a two-process round trip, not by
  review.

## Considered Options

- **Segment-resident data operations in `drm-core`** — chosen. Teach the
  attached segment to run the existing protocol: publish (slot selection,
  generation bump, ring cell, CCB counters under Release) and read (cursor,
  hazard slot, borrow/release) over the mapped regions already reserved by
  `segment_layout`, using the `ccb_ptr!` atomic-over-offset pattern. Exposed as
  a safe API; `unsafe` stays inside `drm-core` (codebase §3.1). The in-process
  `Channel` keeps its fast path.
- **In-process-only applications** — rejected: contradicts the architecture
  statement and makes every cross-language integration impossible.
- **Router-link bindings for C/C++/Python first** (the Phase 32/33 item) —
  rejected as the *primary* fix: it delivers bytes over sockets, not the
  zero-copy shared-memory fabric the architecture promises, and it still leaves
  a local co-located reader without a DRM path.
- **Expose the raw mapping + offsets to applications** — rejected: it pushes
  hazard-protocol and atomics code into every application, contradicts the
  pinned `unsafe` confinement, and cannot be validated as "the exact same
  memory buffers" semantics.

## Decision Outcome

Chosen: **segment-resident data operations in `drm-core`, exposed additively.**

- `AttachedPublisher` / `AttachedConsumer` operate over a `Mapping` +
  `AttachedView`: `publish`, `read_latest`, `read_next`, borrow/release under
  the unchanged hazard protocol and overflow policy.
- Per AGENTS.md #4 a topic keeps exactly one publisher: the segment's publisher
  authority (already lock-guarded) remains the single writer; consumers attach
  read-only.
- `drm-ffi` gains append-only entry points for attach/publish/read so
  non-Rust bindings (and language bridges such as a ROS 2 bridge) can use the
  data path without new `unsafe`.
- No change to the segment layout, the fingerprint/codegen contracts, or any
  frozen artifact.

### Positive Consequences

- The architecture statement becomes true: separate processes, in any bound
  language, exchange topic data over shared memory.
- Existing lifecycle guarantees (authority, epoch restart, stale
  classification, crash cleanup) extend to the data path unchanged.
- Language bridges — ROS 2 first — become implementable against a supported
  API instead of a private layout.

### Negative Consequences

- Two implementations of one protocol must stay in agreement: the in-process
  `Channel` fast path and the segment-resident path. Mitigation: a shared
  conformance test asserts both produce identical observable behaviour on the
  same inputs.
- The public surface grows (new types + C entry points); the freeze set must be
  re-resolved append-only.

## Confirmation

- Two-process round trip: a publisher process and a consumer process exchange
  N samples over one segment; payload bytes, sequence numbers, and gap
  reporting are asserted; a third process attempting publisher authority is
  refused (`REQ-CORE-102`…`REQ-CORE-107`).
- In-process/attached equivalence: the same sample sequence through `Channel`
  and through the attached path yields identical observable state.
- `cargo xtask check-layering`, `cargo xtask audit-traceability`, the full
  `cargo xtask test` suite, and `clippy --workspace --all-targets -D warnings`
  stay green.

## More Information

- [architecture.md](../architecture.md) §Local Data Delivery.
- [attached-data-path.md](../algorithms/runtime/attached-data-path.md)
  (normative algorithm + test matrix).
- ADRs are immutable; supersession creates a new record (codebase.md 6.3).
