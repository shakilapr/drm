#!/usr/bin/env python3
"""Independent boundary-convention oracle (E4, work-plan M19W3-E4-T1).

Numeric ground truth for the Rust `drm-spatial` boundary conversions
(`BoundaryConvention`): the canonical convention is right-handed SI with
+X forward / +Y left / +Z up; NED, OpenCV optical, Unity, and Unreal each
declare an explicit axis permutation at the boundary (docs/spatial_architecture.md
§Canonical Conventions and Geodetic Boundaries; spatial-core.md I-S4). This is a
clean-room NumPy implementation derived directly from the architecture doc; it
shares no code with the Rust crate under test.

Usage:
    python3 tests/oracle/conventions_oracle.py [out.json]

Default output: tests/fixtures/spatial/convention_vectors.json

REQ: REQ-SPATIAL-007, REQ-SPATIAL-040.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np

# Canonical basis vectors (+X forward, +Y left, +Z up) expressed in each external
# frame — i.e. the external -> canonical rotation matrix rows.
AXES = {
    # NED: x=North(fwd), y=East(=left? no, East is right), z=Down.
    # Canonical +X (fwd) = NED +x; +Y (left) = -NED y; +Z (up) = -NED z.
    "ned": np.array([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]]),
    # OpenCV: x=right, y=down, z=forward. fwd=+z, left=-x, up=-y.
    "opencv_optical": np.array([[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]]),
    # Unity (left-handed): x=right, y=up, z=forward. fwd=+z, left=-x, up=+y.
    "unity": np.array([[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
    # Unreal (left-handed): x=forward, y=right, z=up. fwd=+x, left=-y, up=+z.
    "unreal": np.array([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]]),
}


def q_rotmat(q: np.ndarray) -> np.ndarray:
    x, y, z, w = q / np.linalg.norm(q)
    return np.array(
        [
            [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
        ]
    )


def mat_quat(m: np.ndarray) -> np.ndarray:
    """Rotation matrix -> quaternion [x, y, z, w] (Shepperd)."""
    trace = m[0, 0] + m[1, 1] + m[2, 2]
    if trace > 0.0:
        s = np.sqrt(trace + 1.0) * 2.0
        q = np.array(
            [
                (m[2, 1] - m[1, 2]) / s,
                (m[0, 2] - m[2, 0]) / s,
                (m[1, 0] - m[0, 1]) / s,
                0.25 * s,
            ]
        )
    elif m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
        s = np.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2]) * 2.0
        q = np.array(
            [
                0.25 * s,
                (m[0, 1] + m[1, 0]) / s,
                (m[0, 2] + m[2, 0]) / s,
                (m[2, 1] - m[1, 2]) / s,
            ]
        )
    elif m[1, 1] > m[2, 2]:
        s = np.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2]) * 2.0
        q = np.array(
            [
                (m[0, 1] + m[1, 0]) / s,
                0.25 * s,
                (m[1, 2] + m[2, 1]) / s,
                (m[0, 2] - m[2, 0]) / s,
            ]
        )
    else:
        s = np.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1]) * 2.0
        q = np.array(
            [
                (m[0, 2] + m[2, 0]) / s,
                (m[1, 2] + m[2, 1]) / s,
                0.25 * s,
                (m[1, 0] - m[0, 1]) / s,
            ]
        )
    return q / np.linalg.norm(q)


def build() -> dict:
    rng = np.random.default_rng(20260615)
    points = [rng.uniform(-5.0, 5.0, 3).tolist() for _ in range(5)]
    # A nontrivial rigid transform (90 deg about +Z).
    q = np.array([0.0, 0.0, np.sin(np.pi / 4.0), np.cos(np.pi / 4.0)])
    tt = np.array([1.0, -2.0, 0.5])
    cases = []
    for name, m in AXES.items():
        to_canon_points = [ (m @ np.array(p)).tolist() for p in points ]
        r = q_rotmat(q)
        r2 = m @ r @ m.T
        t2 = m @ tt
        cases.append(
            {
                "convention": name,
                "points": points,
                "to_canonical": to_canon_points,
                "transform_t": tt.tolist(),
                "transform_q": (q / np.linalg.norm(q)).tolist(),
                "transform_to_canonical_t": t2.tolist(),
                "transform_to_canonical_q": mat_quat(r2).tolist(),
            }
        )
    return {"generated_by": "tests/oracle/conventions_oracle.py", "cases": cases}


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else (
        Path(__file__).resolve().parents[1] / "fixtures" / "spatial" / "convention_vectors.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    payload = build()
    out.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"wrote {out} ({len(payload['cases'])} conventions)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
