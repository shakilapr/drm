# Phase 5 Gate — Metadata Layering

Evidence mapping, [work-plan.md](../../work-plan.md) #5 criteria ⇄ tests.
Implementation: `drm-core::channel` descriptor/attach
([channel-metadata.md](../algorithms/runtime/channel-metadata.md)),
`drm-router` boundary framing, derived spec [metadata-layout.md](../spec/metadata-layout.md).
Verdict: **PASS** (local suites green; stress/sanitizer lanes CI-replayed).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Fixed samples contain no unnecessary length/schema/topic envelope | `channel-metadata.md` §Sample metadata rules (fixed = raw payload only); publish/read copy path writes exact `src` with no header; `channel::publish_read_roundtrip` | PASS |
| Variable samples contain required length only | canonical single-variable degeneration to a LE `u32` length header (`canon-codec.md`); `sample_len` derived from counts, not stored in the ring cell | PASS |
| Channel metadata validated on attachment | `attach_semantics.rs` — `attach_accepts_matching_segment`, `attach_init_refused` (I-M6), `attach_layout_matrix` 16 tamper cases (I-M7) | PASS |
| Transport metadata appears only at physical boundaries | `drm-router` `wire`/`framing` encode `{version,flags,wire_channel_id,stream_epoch,source_seq,payload_len}`; shared-memory samples carry none; `transport::udp` frames only across the socket | PASS |
| Byte-layout inspection tests prove each rule | `descriptor_golden.rs` field-for-field byte table + zero-padding sweep + write/read symmetry; `tests/fixtures/golden/descriptor/descriptor_golden.json` | PASS |

## Week variables (M03W1)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 5)` | PASS | PASS |
| `DESCRIPTOR_BYTE_EXACT` | I-M1 golden match | `descriptor_golden` 2/2; 152 B field table, padding zeroed |
| `ATTACH_ONCE` | I-M2: 1e6 reads ⇒ 0 re-checks | `attach_once` 1e6 steady-state reads, 0 re-validations |
| `PUBLISH_READ_BASIC` | fixed-size round-trip green | `channel` lib 23 tests incl. `publish_read_roundtrip`; `ordering_smoke` release/acquire visibility |
| `UDP_OVERSIZE_REJECT` | rejected pre-send | `transport::udp::udp_oversize_reject`; `mtu::udp_cap_boundary` |
| `VERIFY_LITE_NEGATIVES` | multi-publisher + no-publisher rejected | `verify_lite` 4/4 (V-SW-1, V-SW-2, clean, determinism) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Test-matrix coverage ([channel-metadata.md](../algorithms/runtime/channel-metadata.md) §Test Matrix)

| Matrix row | Test | Status |
|---|---|---|
| `descriptor_golden` | `descriptor_golden` | PASS (I-M1) |
| `descriptor_offset_asserts` | `descriptor_offset_asserts` | PASS (I-M5) |
| `attach_once` (1e6 reads) | `attach_once` | PASS (I-M2) |
| `attach_init_refused` | `attach_init_refused` | PASS (I-M6) |
| `attach_mismatch` (hash/handle/name) | `attach_layout_matrix` | PASS |
| `attach_layout_matrix` (field/range/alignment) | `attach_layout_matrix` | PASS (I-M7) |
| `epoch_restart` | `channel::epoch_restart` | PASS (I-M3) |
| `fingerprint_guard` | `drm-workspace` model fingerprint suites | PASS (I-M4) |
| `ccb_counter_layout` | `descriptor_offset_asserts` (CCB atomic alignment) | PASS (I-M8) |
| `ccb_timing_variant` | `attach_layout_matrix` TIMING flag; CCB variant sizing in `validate_ranges` | PASS (I-M9) |

## Requirements covered

Channel: REQ-CORE-014 (aligned control state, offset asserts), REQ-CORE-015
(stored once per segment), REQ-CORE-016 (exact offsets/counts/size,
overflow-safe math), REQ-CORE-017 (identity + topic name), REQ-CORE-002
(compare-once), REQ-CORE-068 (secondary per-segment check). Transport:
REQ-ROUTE-032 (one sample per datagram ≤ `max_datagram_payload`, oversize
rejected pre-send). Toolchain: REQ-TOOL-064/070 (verify-lite single-writer).

## Notes

- Descriptor byte contract (I-M1) is frozen by the golden fixture; any change
  regenerates and re-verifies the table in the same change.
- The layer split is normative in architecture §No Universal Envelope; this
  gate proves only the byte-level consequences listed above.
- Stress/sanitizer lanes replayed in CI (see [phase-08.md](phase-08.md) notes);
  Windows/Linux filesystem specifics unaffected by metadata layering.
