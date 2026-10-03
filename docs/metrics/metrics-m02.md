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

## M02W2 — Handles, Layout Crosscheck, Framing

| Variable | Target | Measured |
|---|---|---|
| `LAYOUT_CROSSCHECK` | 0 mismatches (I6, permanent) | 10/10 fixtures — `emit_layout_json` regen gate (`layout_emit_stable`) + runtime re-derivation/write/validate/short-buffer probes (`fc7f279`); wired into `xtask test` |
| `HANDLE_STABILITY` | I-H1/I-H2/I-H3 | 7 tests — determinism ×100, fresh-deterministic sorted allocation, add-no-renumber, delete/readd tombstones (no silent reuse), exhaustion typed error incl. last-slot saturation, compact_repack (`db6fcd4`) |
| `FRAMING_PROPERTY_CASES` | ≥ 20 000 | 20 000 × 3 properties (record roundtrip+truncation matrix, multiplex identity+bound, total-decode no-panic) |
| `FRAMING_FIXTURES` | record + mux | 4 tests — roundtrip incl. 4 KiB, every-truncation rejection, stream cursor, exact-fit/one-over bound (`a6599bb`) |
| `FUZZ_FRAMING` | PR smoke | `tests/fuzz/fuzz_targets/framing.rs` (record ×4 positions + mux; decoded-subslice invariant), CI nightly |
| `MODEL_ATTRS` | corpus-level | 4 tests — all keys normalize, latest/overwrite, absent defaults, attrs through build (`0778d0d`) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Carry-forward (M02W3): reconcile (`drm ids reconcile`), CCB v0 struct, zero-alloc struct-path assertion, status-map both-directions gate, nodes+placement parse, force/prefer selection.

## M02W3 — CCB Attach, Fingerprints, Contracts, Router Config

| Variable | Target | Measured |
|---|---|---|
| `BLAKE3_KAT` | official vectors | 35/35 official lengths (0..102 400) pass — clean-room `drm_types::blake3`/`blake3_128`, zero-dep (`2b2634e`); `schema_hash` pinned to BLAKE3-128 (REQ-TOOL-023) |
| `DESCRIPTOR_BYTE_EXACT` | golden byte-table match (I-M1) | `tests/fixtures/golden/descriptor/descriptor_golden.json` field table + zero-padding sweep + write/read symmetry; runtime offset asserts incl. CCB atomic alignment I-M5 (`3f4b1b3`) |
| `ATTACH_SEMANTICS` | I-M6/I-M7 explicit errors | `attach_init_refused` (Zero/Init → NotReady, nothing else interpreted); `attach_layout_matrix` 16 tamper cases (hash/handle/name → InvalidData; layout/capacity/reserved → WrongAbi; depth/pool/count/variable-bound/range-order/alignment/pool-overflow → InvalidConfig); compare-once `attach_once` 1e6 view reads |
| `PUBLISH_ORDERING` | release/acquire smoke (REQ-CORE-051) | 2-thread smoke: 10k samples, observed seq ⇒ payload visibility (no-reuse slots); epoch-word pairing gate |
| `MODEL_NODES` | publish/consume sets + negatives | 6 tests — parse, unknown-topic (node named), duplicate node (both stems), closed manifest surface, multi-publisher query (`conflicting_publishers`, model builds — I-D12) (`973714a`) |
| `FINGERPRINT_ISOLATION` | I-D4 | serialization pinned by test (domain tag/format/ABI/count/direction/len/path/schema+layout hash), isolation (unrelated edit ⇒ unchanged), direction + layout participation, determinism, empty-node header (`973714a`) |
| `MODEL_ROUNDTRIP` | ≥ 5 000 (I-D5) | 5 000 generated trees: model → reconstructed tree → identical model + render; per-case shuffled-order determinism (`973714a`) |
| `ROUTE_CONFIG_PARSE` | fixtures green | 9 tests — links/bandwidth/overrides, malformed/missing/bad-type/unknown-key/empty-name rejection, rate-unit matrix (`868d610`) |
| `ROUTE_TABLE` | handle-keyed (REQ-ROUTE-043) | insert/lookup by u32 handle; strings excluded from the data path |
| `CRC32C_KAT` | known-answer vectors | 6 official vectors + streaming≡one-shot across split patterns (constants cross-verified against a second implementation; 3 supplied values corrected) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Doc gaps found (report-only): router-core.md has no TOML surface (field names derived from architecture.md); export_limit placement derived; serial-mcu draft added as clearly-marked section.

## M02W4 — Phase 4 Gate, Topology, Overrides, Open API, Soak Skeleton

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 4)` | PASS — C/Rust/Python; C++ closure documented | PASS — `docs/gates/phase-04.md`; golden triple green; C++ runner closure recorded in registry `REQ-BIND-013` test field → Phase 20 |
| `GOLDEN_TOTAL` | ≥ 30 ×3 langs | 35 vectors (24 primitives + 10 composites + 1 descriptor): Rust suites, C11 `golden_composites` (gcc), Python `test_golden_bytes.py`; UTF-8-malformed negative parity |
| `MODEL_COVERAGE` | topics+nodes+contracts+placement+links | model topology (computes/links/placement) with referential validation (dangling/unknown/duplicate/self-loop), render + roundtrip updated (`cb2d7ed`) |
| `OPEN_API_SEMANTICS` | I-O1/I-O2 | `publisher_open` held-or-refuse (`Permission` loser), create-or-classify (fresh/adopt/`Init`+undersized recover), restart floor+epoch published last, `EpochExhausted`; `consumer_open` never creates (I-O2), bounded wait retries `NotFound`/`StaleIncomplete`/`NotReady`; 10 in-memory lifecycle tests + 2 real-POSIX fork tests (SIGKILL restart epoch 2 + cross-process single-winner; typechecked, CI-run) (`a5b4d66`) |
| `ROUTE_OVERRIDES` | 8/8 fixtures (I-R1..R3) | 19 tests in `topology_graph`: force pins viable/not-viable/conflict/multi-consumer-ambiguity, prefer resolves ambiguity/off-path, deny→force→prefer pipeline (`3f8b98d`) |
| `SOAK_SKELETON` | `xtask soak --duration 1` clean | 118 clean iterations within the 1 h budget (every failure aborts; parser unit test) |
| `GAP_SCAFFOLD` | outcome + 1:1 status mapping | `drm_types::Gap { missed }`, `From<Gap> for Status` (`-6`), mapping test |
| `DESCRIPTOR_SPEC` | byte-exact maps (Phase 5 feed) | SegmentPrefix 16 B / ChannelDescriptor 152 B / CCB 56 B (+TIMING 64) offset tables + sample metadata rules in `channel-metadata.md`; hand check: 178→184/248 region math |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Architecture catch (report-only): `validate_prefix` checked magic before state; channel-metadata.md I-M6 is normative state FIRST — zero-filled in-progress segments returned terminal `InvalidData` instead of retryable `NotReady`. Fixed in `a5b4d66`; consequence: first `From<_> for Status` impl exposed a brittle never-type `?` pattern in drm-router (5 sites, `map_err(|_| unreachable!())?` → `unwrap_or_else`).
