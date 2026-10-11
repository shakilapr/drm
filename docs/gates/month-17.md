# Month-17 Review — Phase 32 Close-Out, Studio Protocol + MVP, M18 Go/No-Go

Chaired by E3 (review chair) at M17W4, Friday. Verdict: **ON_TRACK** — Phase 32
closed with all submodule conformance suites green; the Studio protocol doc and
its host implementation landed; the read-only MVP (inspectors + Problems panel)
and the run-session supervisor are functional. Go for M18 (hardening sprint).

Normative sources: [work-plan.md](../../work-plan.md) W6 Phase 32 →
[studio-protocol.md](../algorithms/studio/studio-protocol.md) (I-ST1..I-ST13),
[ui-architecture.md](../../ui-architecture.md),
[observability-plane.md](../algorithms/ops/observability-plane.md),
[rpc-actions.md](../algorithms/ops/rpc-actions.md). REQ: REQ-QAL-006,
REQ-STUDIO-001..REQ-STUDIO-016, REQ-OPS-001, REQ-OPS-003, REQ-OPS-011,
REQ-OPS-013, REQ-OPS-015, REQ-OPS-017.

## 1. Variables actuals vs targets (M17)

| Week | Variable | Target | Measured |
|---|---|---|---|
| M17W1 | `INTROSPECT_GATE` | read-only, no-pin, rate tolerance | PASS — [m17w1.md](m17w1.md) |
| M17W1 | `TRACE_GATE` | never-blocks, drop transparency | PASS — [m17w1.md](m17w1.md) |
| M17W1 | `RPC_GATE` | correlation, cancel, timeout, capacity | PASS — [m17w1.md](m17w1.md) |
| M17W2 | `GATE_STATUS(Phase 32)` | PASS | PASS — [m17w2.md](m17w2.md) |
| M17W2 | `STUDIO_PROTOCOL_DOC` | merged (protocol-level) | merged — [studio-protocol.md](../algorithms/studio/studio-protocol.md) |
| M17W3 | `STUDIO_PROTOCOL_GREEN` | handshake + typed round-trip | 1 — `studio_host_*` (I-ST1..I-ST4) |
| M17W3 | `MUTATIONS_REVISIONED` | `base_revision`/`CONFLICT` | 1 — `studio_revision_conflict`, `studio_undo_guard`, journal recovery (I-ST7/I-ST8/I-ST9) |
| M17W3 | `STUDIO_SCAFFOLD` | React 19 + Vite + TS strict boots | 1 — `apps/studio` + `packages/studio-protocol` |
| M17W4 | `STUDIO_MVP` | read-only inspectors over RPC | 1 — [m17w4.md](m17w4.md) |
| M17W4 | `RUN_SUPERVISOR` | sessions outlive UI (I-ST10) | 1 — `cargo test -p drm-cli supervisor` |
| M17W4 | `MONTH17_REVIEW` | ON_TRACK | ON_TRACK (this document) |
| M17W4 | `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## 2. Phase 32 + Studio protocol-level status

| Item | Doc | Verdict |
|---|---|---|
| Phase 32 (introspection/trace/RPC/record/serial/handles) | [phase-32.md](phase-32.md) | PASS (M17W2) |
| Studio protocol doc (I-ST1..I-ST13) | [studio-protocol.md](../algorithms/studio/studio-protocol.md) | merged (M17W2) |
| Studio host + revisioning + journal + format-preserving edits | [m17w3.md](m17w3.md) | PASS (M17W3) |
| Read-only inspectors + Problems panel + run supervisor | [m17w4.md](m17w4.md) | PASS (M17W4) |

## 3. Risks

| Risk | Mitigation | Status |
|---|---|---|
| Studio scope creep | read-only inspectors only; deferred list unchanged (creation UI/editors/Inspector forms/remote-web — post-1.0) | contained |
| Run-session flake | injectable `ProcessRunner`; supervisor tests spawn no OS processes | contained |
| Journal/undo correctness under external edits | input-hash guard + `force` gate (I-ST9); recovery idempotent | green |

## 4. M18 commitment

Hardening sprint: fuzz expansion (schema/framing/hazard/datagram/serial) +
chaos suites (kill -9 matrix, PID reuse, pool-exhaustion storms) with nightly
wiring; Studio Graph/Run panels; Electron packaging. Regression permanence:
every green M17 suite remains a blocking gate.

## 5. Carry-over

| Item | Origin | Disposition |
|---|---|---|
| Extension host | M17W2/W3 deferral | M18+ |
| Remaining `ModelCommand` variants (package/topic/compute/link create/delete) | M17W3 carry-over | M18 |
| Studio Graph/Run panels, Electron packaging | M17W4 deferral | M18 |
| MCU 2 h target soak | M15/M16 | deferred to `mcu-cross` hardware lane |

REQ: REQ-STUDIO-001, REQ-STUDIO-002, REQ-STUDIO-003, REQ-STUDIO-004,
REQ-STUDIO-005, REQ-STUDIO-006, REQ-STUDIO-007, REQ-STUDIO-008,
REQ-STUDIO-009, REQ-STUDIO-010, REQ-STUDIO-011, REQ-STUDIO-012,
REQ-STUDIO-013, REQ-STUDIO-014, REQ-STUDIO-015, REQ-STUDIO-016,
REQ-OPS-001, REQ-OPS-003, REQ-OPS-011, REQ-OPS-013, REQ-OPS-015,
REQ-OPS-017, REQ-QAL-006.
