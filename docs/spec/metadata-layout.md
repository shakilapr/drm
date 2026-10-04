# Metadata Layout (Derived Spec)

Non-normative extraction of DRM's three metadata layers. On conflict the
architecture wins; on pseudocode conflict the algorithm docs win
([codebase.md](../../codebase.md) §2.4). Phase 5.

`source_ref`: [docs/architecture.md](../architecture.md) §No Universal Envelope;
[docs/algorithms/canon-codec.md](../algorithms/canon-codec.md);
[docs/algorithms/runtime/channel-metadata.md](../algorithms/runtime/channel-metadata.md).
REQ: REQ-CORE-014, REQ-CORE-015, REQ-CORE-016, REQ-ROUTE-032.

## Layer rule

Metadata is stored at the narrowest scope that needs it; no universal envelope
wraps every sample.

| Layer | Lifetime | Location | Contents |
|---|---|---|---|
| Channel | immutable while running | descriptor, once per segment | identity, schema hash, capacity/depth, `layout_hash` |
| Sample | per sample | inside the sample slot | only per-sample or synchronization data |
| Transport | per physical boundary | wire frame only | multiplex id, epoch/sequence, framing, CRCs |

## Sample metadata

| Topic shape | Sample bytes |
|---|---|
| Fixed-size | raw payload only — no length, no header, no per-sample CRC |
| Variable-size | little-endian `u32` byte length, then payload |
| Sensor | declared source timestamp field only (schema-owned); DRM injects no transport timestamp |

`sample_len` is derived from the canonical descriptor (`sample_len = f(counts)`),
never stored in the ring cell. Fixed-size topics never pay for a redundant
length field.

Local integrity uses generation counters plus acquire/release ordering, not
software CRCs. Hop and end-to-end CRCs are transport-plane only.

## Transport metadata

A message is framed only at a physical boundary; wire headers are stripped
before import into shared memory.

- Wire header: `{version, flags, wire_channel_id, stream_epoch, source_seq, payload_len}`,
  fixed little-endian; `stream_epoch: u64`. `wire_channel_id` is a route
  binding identifier, never the runtime topic handle.
- One DRM sample crosses a link as exactly one frame; no DRM-level
  fragmentation or reassembly.
- A receiver enforces the generated per-route maximum record length *before*
  buffering; an over-bound declared length is a protocol error, never an
  allocation (REQ-ROUTE-032).
- Optional end-to-end profile: CRC32C over `{stream_epoch, source_seq, payload_len, payload}`.

## Verification

- Byte-layout inspection: `descriptor_golden` (I-M1; field table + zero-padding
  sweep), `descriptor_offset_asserts` (I-M5).
- Channel metadata validated once at attach: `attach_semantics` (I-M2/I-M6/I-M7).
- Transport boundary-only metadata: `drm-router` wire/framing suites;
  UDP oversize-reject pre-send (`transport::udp::udp_oversize_reject`).
