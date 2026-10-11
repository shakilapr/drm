# Metrics — Month 19 (M19)

Normative source: [work-plan.md](../../work-plan.md) W7 DRM Spatial (Phase 33).
Month 19 starts the spatial wave: typed quantities, the spatial math library,
grammar extensions, build-time static collapse, and the runtime edge-cache
groundwork. One section per weekly tranche.

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

## Carry-over

- Clock-domain resolution to full production (degradation paths) ⇒ M19W2 (E2).
- Runtime edge-cache consumption by queries ⇒ M19W2 (E1).
- `spaces/`/`index.drm` directory-layer wiring into `WorkspaceModel` ⇒ M19W2+.

REQ: REQ-SPATIAL-004, REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-012,
REQ-SPATIAL-017, REQ-SPATIAL-030, REQ-SPATIAL-031, REQ-SPATIAL-036,
REQ-SPATIAL-038, REQ-SPATIAL-041, REQ-SPATIAL-047, REQ-SPATIAL-048,
REQ-SPATIAL-049, REQ-SPATIAL-050, REQ-SPATIAL-051.
