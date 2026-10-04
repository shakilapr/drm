# Metrics — Month 3 (M03W1–W4)

Gate packet mixed into W1/W3/W4: variables actual-vs-target across the week
plans (work-plan M03W1–W4). Values marked `CI` are verified on runners only
(local Windows cannot execute them). REQ: REQ-QAL-007, REQ-TOOL-001.

## M03W1 — Phase 5 Gate (Metadata Layering)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 5)` | PASS | PASS — `docs/gates/phase-05.md`: work-plan #5 criteria ⇄ evidence mapped; fixed = payload-only, variable = LE `u32` length, attach-once validation, boundary-only transport framing, byte-inspection |
| `DESCRIPTOR_BYTE_EXACT` | I-M1 golden match | `descriptor_golden` 2/2 — 152 B field table + zero-padding sweep + write/read symmetry; offset asserts (I-M5) |
| `ATTACH_ONCE` | I-M2: 1e6 reads ⇒ 0 re-checks | `attach_once` 1e6 steady-state view reads, 0 re-validations |
| `PUBLISH_READ_BASIC` | fixed-size round-trip green | `drm-core` channel lib 23 tests incl. round-trip/lap/ABA/busy; `ordering_smoke` release/acquire visibility + epoch pairing |
| `UDP_OVERSIZE_REJECT` | rejected pre-send | `transport::udp::udp_oversize_reject`; `mtu::udp_cap_boundary`; 10k-clean `examples/udp_pair.rs` |
| `VERIFY_LITE_NEGATIVES` | multi-publisher + unknown schema rejected | `verify_lite` 4/4 (V-SW-1 no-publisher, V-SW-2 multi-publisher, clean, determinism) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/spec/metadata-layout.md` (derived spec, Phase 5),
`docs/gates/phase-05.md`, transport/language conformance cell scaffold
(`tests/conformance/README.md`).
