# Spatial math oracle

Independent numeric ground truth for the `drm-spatial` runtime crate
(work-plan M19W1-E4-T1). `spatial_oracle.py` is a clean-room `NumPy`
implementation of the operations under test, derived directly from
[spatial_architecture.md](../../docs/spatial_architecture.md) and
[spatial-core.md](../../docs/algorithms/spatial/spatial-core.md). It shares no
code with the Rust implementation.

## Contents

- `spatial_oracle.py` — `SE(3)` composition, quaternion rotation of
  points/directions/vectors, SLERP (including a near-`180°` case), and the
  WGS84→ECEF→ENU geodetic chain.
- `clock_oracle.py` — affine clock-domain mapping (`t_to = to_ref +
  rate_ratio·(t_from − from_ref)`) with bounded uncertainty growth, plus
  unresolved/out-of-validity rejection (work-plan M19W2-E4-T1).
- `conventions_oracle.py` — boundary axis conversions for NED, OpenCV optical,
  Unity, and Unreal, applied to points and rigid transforms
  (work-plan M19W3-E4-T1). `spatial_oracle.py` additionally carries a
  `geodetic_precision` section (small local offsets from an anchor, I-S5).
- `query_oracle.py` — query-surface ground truth: linear translation + SLERP
  rotation interpolation between bracketing history samples, the SE(3)
  exponential map (`T(t+Δ) = T(t)·Exp(ξ_b Δ)`) used by `predict`, transactional
  reparent-mode math (Preserve World/Local/Snap), and relocalize/reparent/
  calibration discontinuity fixtures (work-plan M20W1-E4-T1, M20W2-E4-T1).
- `../fixtures/spatial/oracle_vectors.json`,
  `../fixtures/spatial/clock_vectors.json`,
  `../fixtures/spatial/convention_vectors.json`,
  `../fixtures/spatial/query_vectors.json` — committed vectors the Rust
  tests compare against. Regenerate with:

```text
python3 tests/oracle/spatial_oracle.py
python3 tests/oracle/clock_oracle.py
python3 tests/oracle/conventions_oracle.py
python3 tests/oracle/query_oracle.py
```

The Rust consumers are `crates/drm-spatial/tests/math.rs` (`cargo test -p
drm-spatial math`), `crates/drm-spatial/tests/clocks.rs` (`cargo test -p
drm-spatial clocks`), `crates/drm-spatial/tests/conventions.rs` (`cargo test
-p drm-spatial conventions`), `crates/drm-spatial/tests/queries.rs`
(`cargo test -p drm-spatial queries`), and
`crates/drm-spatial/tests/reparent.rs` (`cargo test -p drm-spatial interp_guard`
/ `reparent`), which read the committed JSON — CI does not need Python.

## Quaternion convention

Quaternions are `X, Y, Z, W`; rotations are right-handed with `+X` forward,
`+Y` left, `+Z` up. Both implementations use the same Hamilton product and the
`v + 2w(q×v) + 2q×(q×v)` rotation formula, so a mismatch is a real
implementation defect, not a convention difference. SLERP results are compared
up to sign (`q` and `-q` are the same rotation).

REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-012, REQ-SPATIAL-007,
REQ-SPATIAL-040, REQ-SPATIAL-004, REQ-SPATIAL-041, REQ-SPATIAL-051,
REQ-SPATIAL-022, REQ-SPATIAL-025, REQ-SPATIAL-016, REQ-SPATIAL-028,
REQ-SPATIAL-029.
