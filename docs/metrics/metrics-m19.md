# Metrics — Month 19 (M19)

Normative source: [work-plan.md](../../work-plan.md) W7 DRM Spatial (Phase 33).
Month 19 starts the spatial wave: typed quantities, the spatial math library,
grammar extensions, build-time static collapse, the runtime edge-cache
groundwork, clock domains, forests, canonical conventions, and geodetic
boundaries. One section per weekly tranche.

## M19W1 — Spatial Types & Static Collapse Implementation

| Variable | Target | Measured |
|---|---|---|
| `SPATIAL_TYPES_GREEN` | typed semantics vs reference math (I-S1) | 1 — `crates/drm-spatial` (`types`/`transform`/`geodetic`/`clock`/`cache`); `cargo test -p drm-spatial math` compares 25 oracle vectors + 5 geodetic cases against `tests/oracle/spatial_oracle.py` (NumPy); type-semantics (`I-S1`), payload-layout (`I-S2`), clock (`I-S7`), and cache (`REQ-SPATIAL-050`) suites green |
| `SPATIAL_PARSER_GREEN` | `point3/pose3/…` + `space:` parsing green | 1 — `crates/drm-workspace/src/parser/spatial.rs` recognizes all six keywords, the `space:` compile-time binding, and the dynamic `SpatialPoint3` form; `cargo test -p drm-workspace spatial_grammar` green; non-spatial `.drm` corpus byte-for-byte unaffected |
| `SPATIAL_COLLAPSE_V0` | static chains collapsed to constants at build | 1 — `crates/drm-workspace/src/spatial/` parses `spaces/**.drm`, classifies edges (`I-D6`/`I-S10`), folds contiguous static chains to one `SE(3)` constant (`REQ-SPATIAL-049`), and emits a `@generated` C artifact (`REQ-SPATIAL-017`); `cargo test -p drm-workspace spatial_collapse` green (deterministic under shuffled input) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Spatial math library** (`crates/drm-spatial/src/{types,transform,geodetic,clock}.rs`):
  the six distinct typed quantities (`REQ-SPATIAL-036`), rigid `SE(3)` apply /
  compose (`REQ-SPATIAL-038`/`REQ-SPATIAL-049`), the WGS84→ECEF→ENU geodetic
  chain (`REQ-SPATIAL-012`), and affine clock-domain mapping with identity epoch
  and unresolved rejection (`REQ-SPATIAL-004`/`REQ-SPATIAL-041`/`REQ-SPATIAL-051`).
- **Runtime cache groundwork** (`crates/drm-spatial/src/cache.rs`): versioned
  dynamic-edge slots with a lockless seqlock (f64 payloads stored as `AtomicU64`
  bit patterns, no `unsafe`), generation counters, and single-writer authority
  (`REQ-SPATIAL-050`/`REQ-SPATIAL-030`).
- **Grammar extensions** (`crates/drm-workspace/src/parser/spatial.rs`): keyword
  recognition, `space:`/`spatial_id:` bindings, generated type spellings, and
  raw-numerics wire sizes (`REQ-SPATIAL-005`/`REQ-SPATIAL-009`).
- **Collapse engine v0** (`crates/drm-workspace/src/spatial/`): path-derived
  frame identity (`REQ-SPATIAL-047`), static/dynamic classification
  (`REQ-SPATIAL-031`), static-chain fold, and MCU constant emission
  (`REQ-SPATIAL-017`).
- **E4 reference oracle** (`tests/oracle/spatial_oracle.py`,
  `tests/fixtures/spatial/oracle_vectors.json`): an independent NumPy
  implementation of `SE(3)` compose, quaternion rotation, SLERP (including a
  near-`180°` case), and the geodetic chain; `drm-spatial` tests compare against
  it.
- **Docs / evidence**: [m19w1.md](../gates/m19w1.md),
  [phase-33.md](../gates/phase-33.md); registry rows promoted to `tested`.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `SPATIAL_TYPES_GREEN = 1`; `SPATIAL_PARSER_GREEN = 1`; `SPATIAL_COLLAPSE_V0 = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M19W1) = PASS`**

### How to reproduce

```text
python3 tests/oracle/spatial_oracle.py
cargo test -p drm-spatial math
cargo test -p drm-spatial
cargo test -p drm-workspace spatial_grammar
cargo test -p drm-workspace spatial_collapse
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## M19W2 — Clock Domains, Versioned Cache Runtime

| Variable | Target | Measured |
|---|---|---|
| `CLOCKS_GREEN` | four-domain mapping + drift/uncertainty + unresolved rejection (I-S6/I-S7) | 1 — `crates/drm-spatial/src/clock.rs`; `cargo test -p drm-spatial clocks` compares 18 oracle cases against `tests/oracle/clock_oracle.py` (independent offset/drift/uncertainty math); four-domain distinctness, epoch-bump invalidation, and monotonic-uncertainty suites green |
| `CACHE_GREEN` | versioned cache read/write + generation guards green | 1 — `crates/drm-spatial/src/cache.rs` (versioned seqlock `EdgeSlot`, sample/receive `TimePoint`s, `FLAG_DISCONTINUOUS`) + `crates/drm-spatial/src/plan.rs` (`TransformPlan`, `execute_plan`, `PlanCache`); `cargo test -p drm-spatial cache` green (`spatial_cache_shortcircuit`, `spatial_cache_aging`, version semantics, discontinuity round trip, single-writer boundary) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Clock mapping** (`crates/drm-spatial/src/clock.rs`): the four exact domains
  (`REQ-SPATIAL-004`), drift application via the affine rate ratio, bounded
  uncertainty propagation into `TransformResult.clock_uncertainty_ns`
  (`REQ-SPATIAL-041`), and unresolved/out-of-validity rejection as
  `SpatialError::ClockUnresolved` (`I-S7`, `REQ-SPATIAL-051`).
- **Cache runtime** (`crates/drm-spatial/src/cache.rs`): dynamic-edge SHM slots
  (id, transform, sample + receive `TimePoint`, version, discontinuous flag)
  with an odd/even seqlock version counter; generation-guarded delete
  (`REQ-SPATIAL-050`/`REQ-SPATIAL-035`/`REQ-SPATIAL-011`); single-writer
  authority (`REQ-SPATIAL-030`).
- **Plan cache** (`crates/drm-spatial/src/plan.rs`): compiled
  `TransformPlan`/`PlanStep` execution (fixed stack, zero heap, `I-Q1`/`I-Q15`),
  the per-process `PlanCache` version-stamp geometry shortcircuit with an exact
  observed-version vector (`I-Q2`, `REQ-SPATIAL-002`), and the `TransformResult`
  status object (`REQ-SPATIAL-032`).
- **E4 clock oracle** (`tests/oracle/clock_oracle.py`,
  `tests/fixtures/spatial/clock_vectors.json`): an independent implementation of
  the affine mapping and bounded uncertainty growth.
- **Docs / evidence**: [m19w2.md](../gates/m19w2.md),
  [phase-33.md](../gates/phase-33.md); registry rows promoted to `tested`.

### Weekly gate checklist (chaired by E1, Friday)

- [x] `CLOCKS_GREEN = 1`; `CACHE_GREEN = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M19W2) = PASS`**

### How to reproduce

```text
python3 tests/oracle/clock_oracle.py
cargo test -p drm-spatial clocks
cargo test -p drm-spatial cache
cargo test -p drm-spatial
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## M19W3 — Geodetic Boundaries, Forest Queries

| Variable | Target | Measured |
|---|---|---|
| `GEODETIC_GREEN` | WGS84→ECEF→ENU precision within bound (I-S5) | 1 — `crates/drm-spatial/src/geodetic.rs` (`geodetic_to_enu`, documented `GEODETIC_LOCAL_BOUND_M`); `cargo test -p drm-spatial geodetic` (`spatial_geodetic_precision` vs an independent ellipsoid-radius reference; `geodetic_precision_oracle` vs `tests/oracle/spatial_oracle.py`) |
| `FOREST_GREEN` | disconnected trees ⇒ PathNotFound; bridges explicit (I-S3) | 1 — `crates/drm-spatial/src/forest.rs` (`SpaceForest`, `path_between`, `PathEdge`, explicit bridges, common-ancestor resolution); `cargo test -p drm-spatial forest` green |
| `CONVENTIONS_GREEN` | boundary conversions exact (I-S4) | 1 — `crates/drm-spatial/src/conventions.rs` (`BoundaryConvention` NED/OpenCV/Unity/Unreal signed-axis tables); `cargo test -p drm-spatial conventions` compares all four against `tests/oracle/conventions_oracle.py` (independent NumPy math) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Forest path search** (`crates/drm-spatial/src/forest.rs`): a runtime
  `SpaceForest` over the compiled base graph plus overlay frames; a bounded
  ancestor walk (`I-R11` pattern reuse) that returns a directed `PathEdge`
  sequence; explicit bridge registration; multi-robot common-ancestor
  resolution; cross-tree queries ⇒ `SpatialError::PathNotFound`
  (`REQ-SPATIAL-010`/`REQ-SPATIAL-039`).
- **Historical evaluation** (`crates/drm-spatial/src/historical.rs`):
  `HistoricalForest` replays topology events (`AddStatic`/`AddDynamic`/`Delete`/
  `AddBridge`) effective at the query's sample timestamp, so conversions never
  see the current hierarchy (`I-S9`, `REQ-SPATIAL-029`).
- **Geodetic precision** (`crates/drm-spatial/src/geodetic.rs`): the documented
  local-precision bound `GEODETIC_LOCAL_BOUND_M`; the chain keeps downstream
  operations local (small metric offsets, never million-meter `ECEF` deltas,
  `I-S5`).
- **Convention boundaries** (`crates/drm-spatial/src/conventions.rs`): canonical
  internals (RH, SI, `+X`/`+Y`/`+Z`, quats) with explicit NED/OpenCV/Unity/Unreal
  signed-axis conversions applied by conjugation `M R Mᵀ`
  (`REQ-SPATIAL-007`/`REQ-SPATIAL-040`).
- **Rigid inverse** (`crates/drm-spatial/src/transform.rs`): `invert` completes
  the `SE(3)` operations needed to fold a reference→target path
  (`REQ-SPATIAL-038`).
- **E4 oracles** (`tests/oracle/{spatial,conventions}_oracle.py`): the geodetic
  local-precision section and an independent NumPy boundary-convention oracle.
- **Docs / evidence**: [m19w3.md](../gates/m19w3.md),
  [phase-33.md](../gates/phase-33.md); registry rows promoted to `tested`.

### Weekly gate checklist (chaired by E2, Friday)

- [x] `GEODETIC_GREEN = 1`; `FOREST_GREEN = 1`; `CONVENTIONS_GREEN = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M19W3) = PASS`**

### How to reproduce

```text
python3 tests/oracle/spatial_oracle.py
python3 tests/oracle/conventions_oracle.py
cargo test -p drm-spatial geodetic
cargo test -p drm-spatial forest
cargo test -p drm-spatial conventions
cargo test -p drm-spatial historical
cargo test -p drm-spatial
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## Carry-over

- Clock-domain degradation paths to full production ⇒ M20 (E2).
- Coherent `latest`/`at`/`predict`/`delayed_view` query semantics ⇒ M20 (E1).
- `spaces/`/`index.drm` directory-layer wiring into `WorkspaceModel` ⇒ M19W3+.
- Full transactional reparenting (Preserve World/Local/Snap, atomic commit) ⇒
  M20 (E1); historical topology-event evaluation groundwork lands M19W3.

REQ: REQ-SPATIAL-002, REQ-SPATIAL-004, REQ-SPATIAL-005, REQ-SPATIAL-007,
REQ-SPATIAL-009, REQ-SPATIAL-010, REQ-SPATIAL-012, REQ-SPATIAL-017,
REQ-SPATIAL-029, REQ-SPATIAL-030, REQ-SPATIAL-031, REQ-SPATIAL-034,
REQ-SPATIAL-035, REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-039,
REQ-SPATIAL-040, REQ-SPATIAL-041, REQ-SPATIAL-047, REQ-SPATIAL-048,
REQ-SPATIAL-049, REQ-SPATIAL-050, REQ-SPATIAL-051.
