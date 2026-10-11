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
- `../fixtures/spatial/oracle_vectors.json` — committed vectors the Rust tests
  compare against. Regenerate with:

```text
python3 tests/oracle/spatial_oracle.py
```

The Rust consumer is `crates/drm-spatial/tests/math.rs` (`cargo test -p
drm-spatial math`), which reads the committed JSON — CI does not need Python.

## Quaternion convention

Quaternions are `X, Y, Z, W`; rotations are right-handed with `+X` forward,
`+Y` left, `+Z` up. Both implementations use the same Hamilton product and the
`v + 2w(q×v) + 2q×(q×v)` rotation formula, so a mismatch is a real
implementation defect, not a convention difference. SLERP results are compared
up to sign (`q` and `-q` are the same rotation).

REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-012.
