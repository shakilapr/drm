# Phase 23 Gate — Workspace & Manifest Model

Evidence is executable and stays green under the permanent gates.
Verdict: **PASS** (one deterministic model per valid tree; conflicts rejected
with locations; filesystem order cannot leak; parse→model reproducible).

Implementation: `crates/drm-workspace/src/model`
([workspace-model.md](../algorithms/toolchain/workspace-model.md)
I-D1..I-D5). Owner: E2. Gate chair: E3.

## Criteria ⇄ evidence

| Criterion | Artifact | Test | Result |
|---|---|---|---|
| Every valid workspace structure builds exactly one model | `WorkspaceModel` + total-reload `build` (`crates/drm-workspace/src/model/mod.rs`) | `model_baseline`, `model_determinism` | PASS |
| Duplicate/conflicting identities rejected with both sites named | `ModelError::{DuplicateTopic,DuplicateNode,UnknownTopic,DanglingPlacement,PlacementUnknownCompute,UnknownLinkEndpoint,DuplicateLink}` | `model_conflicts`, `model_all_duplicates_reported`, `nodes_unknown_topic_rejected`, `nodes_duplicate_identity_rejected`, `dangling_placement_rejected`, `unknown_link_endpoint_rejected`, `duplicate_link_rejected` | PASS |
| Filesystem ordering cannot change model output | `BTree*` maps/sets everywhere; sorted discovery | `model_determinism` (shuffled fs order ×100, byte-identical render) | PASS |
| Parse→model results are reproducible | `render_model` round-trip | `model_roundtrip_5000` (×5 000), `render_topology_roundtrip` | PASS |
| Communication loops are not build cycles | model retains `publishes`/`consumes` only, no build edges | `communication_loop` | PASS |
| Unrelated schema edits leave unaffected fingerprints unchanged | `contract_fingerprint` over a pinned directed-capability serialization | `fingerprint_isolation`, `fingerprint_serialization_is_pinned` | PASS |
| Placement and link referential integrity | `topology::parse_topology_manifest` + `build_topology` | `topology_parses_and_populates`, `unknown_link_endpoint_rejected`, `dangling_placement_rejected`, `duplicate_link_rejected` | PASS |

## Invariant coverage (workspace-model.md I-D1..I-D5)

| Invariant | Test |
|---|---|
| I-D1 Deterministic | `model_determinism` (×100) |
| I-D2 ConflictRejection | `model_conflicts`, `model_all_duplicates_reported` |
| I-D3 LoopsHarmless | `communication_loop` |
| I-D4 FingerprintIsolation | `fingerprint_isolation` |
| I-D5 ReproducibleModel | `model_roundtrip_5000` (×5 000) |

## Runtime hooks (M08W4-E1-T1)

The model is the single source for runtime activation expectations
(`runtime_plan`, `crates/drm-workspace/src/model/runtime.rs`). The
composition-root e2e (`crates/drm-cli/tests/runtime_hooks.rs`) derives depth and
pool size from the model, activates a live `Channel` with them, and round-trips a
published sample; descriptor expectations mirror the model's schema/layout
hashes and the generated binding lands at the stable `.drm/gen` path.

| Case | Test | Result |
|---|---|---|
| Model-derived depth/pool activate a `Channel` | `runtime_activates_from_model_plan` | PASS |
| Descriptor hashes + codegen path align | `runtime_expectations_and_codegen_align` | PASS |
| Pool floor `pool_size >= R + 1` | `runtime_plan_pool_defaults` | PASS |

## Requirements covered

REQ-TOOL-045 (deterministic in-memory model), REQ-TOOL-047
(publishes/consumes/placed_on/links), REQ-TOOL-050 (communication loops never
create build cycles), REQ-TOOL-023 (fingerprint isolation), REQ-TOOL-013
(manifest single source of truth), REQ-CORE-045 (pool invariant), REQ-CORE-015
(descriptor stored once).

## Notes

- Phase 23 gate scope is the structural model slice I-D1..I-D5 plus
  placement/link integrity. The remaining workspace-model.md invariants are
  phase-gated: spatial inference/single-writer (I-D6/I-D7, Phase 33), external
  endpoints (I-D8, Phase 28), timing/route overrides (I-D9/I-D13, Phase 27),
  typed rates (I-D10, Phase 27), and contract metadata (I-D11). The model
  deliberately performs no policy checks (I-D12); `drm verify` owns single-writer,
  routing, reliability, and budget checks.
- The model is total-reload: every parse rebuilds the full model (no incremental
  caches), which is what makes "exactly one deterministic model" a testable
  property rather than a cache-coherence claim.
