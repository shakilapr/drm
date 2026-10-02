# Metrics — Month 2 (M02W1–)

Gate packet: variables actual-vs-target across the week plans (work-plan
M02W1–W4). Values marked `CI` are verified on runners only (local Windows
cannot execute them). REQ: REQ-QAL-007, REQ-TOOL-001.

## M02W1 — Phase 8 Gate, Layout v2, Topology Start

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 8)` | PASS | PASS — `docs/gates/phase-08.md`: criteria ⇄ evidence mapped; kill/fd/quota stress suites (`5b4da46`); multiprocess depth (race/fd) green; tsan/lsan CI-pending; fork/precommit matrix rows → M02W2 |
| `CITATIONS_BLOCKING` | 0 uncited | 57/57 `.rs` files carry REQ citations; `audit-traceability` bails CI (self-test red→green) |
| `STRESS_LANE` | `xtask test --stress` | 1 — ignored `stress_*` suites via `--ignored stress`; `tests/stress/README.md` |
| `CANON_COMPOSITES` | ≥ 10 vectors, 3-lang parity | 10/10 — enum/fixed-array/empty-bounded/u64-gap two-region cases; Rust + C11 (gcc+clang × c99/c11) + Python strict (ruff + mypy) |
| `PROPERTY_UNIVERSE` | ≥ 10 000 | 10 000 proptest cases — roundtrip/identity, gap zeroing, tail unobserved |
| `MODEL_DETERMINISM` | byte-identical ×100 | 100/100 shuffled-fs-order seeds — render + model equality (`6029be6`+`4e04ec9`) |
| `MODEL_TOPICS` | parse→model | 6-topic/3-package reference tree; conflict collection (I-D2 partial); schema content hash |
| `TOPOLOGY_FIXTURES` | 3 fixtures | 12/12 — local-only/unique/ambiguous + deny/no-route/conflict/fan-out/external/export-limit/error-collection/determinism/cyclic-scale (`388511b`) |
| `ROUTE_CLASSIFICATION` | polynomial | BFS path + bounded per-edge alternate-path check (exact uniqueness, no simple-path enumeration); bridge fast path → later |
| `HANDLE_POLICY_SEED` | stable numbering/append-only/compact | already covered by handle-system.md I-H1/I-H2/I-H4 (no drift; review verdict: PASS) |
| `UDP_NOTES_SEED` | recvmmsg/RCVBUF/two-host | 1 — router-core.md §Protocols UDP transport notes |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Carry-forward (M02W2): fork/flock lock matrix, precommit_mlock matrix, name_conformance generator, active_hold_lifecycle; force/prefer selection semantics; model nodes+placement parse; ws determinism×100 → CI.
