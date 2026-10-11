#!/usr/bin/env python3
"""Independent query oracle (E4, work-plan M20W1-E4-T1).

Numeric ground truth for the `drm-spatial` query surface: linear translation +
SLERP rotation interpolation between bracketing history samples, bracket search
with exact-vs-interpolated classification, and the SE(3) exponential map used by
`predict`. Clean-room NumPy, derived directly from
docs/algorithms/spatial/spatial-queries.md; shares no code with the Rust crate.

Usage:
    python3 tests/oracle/query_oracle.py [out.json]

Default output: tests/fixtures/spatial/query_vectors.json

REQ: REQ-SPATIAL-022, REQ-SPATIAL-024, REQ-SPATIAL-025, REQ-SPATIAL-023.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import numpy as np

from spatial_oracle import q_normalize, q_slerp, axis_angle  # clean-room math reuse within the oracle


def interpolate(a: dict, b: dict, frac: float) -> dict:
    ta = np.array(a["t"])
    tb = np.array(b["t"])
    t = ta + (tb - ta) * frac
    q = q_slerp(np.array(a["q"]), np.array(b["q"]), frac)
    return {"t": t.tolist(), "q": q.tolist()}


def exp_twist(twist: dict, dt: float) -> dict:
    v = np.array(twist["linear"])
    w = np.array(twist["angular"])
    omega = w * dt
    u = v * dt
    theta = float(np.linalg.norm(omega))
    if theta < 1e-12:
        return {"t": u.tolist(), "q": [0.0, 0.0, 0.0, 1.0]}
    axis = omega / theta
    q = axis_angle(axis.tolist(), theta)
    c1 = (1.0 - math.cos(theta)) / (theta * theta)
    c2 = (theta - math.sin(theta)) / (theta * theta * theta)
    translation = u + c1 * np.cross(omega, u) + c2 * np.cross(omega, np.cross(omega, u))
    return {"t": translation.tolist(), "q": q.tolist()}


def build_interp() -> list[dict]:
    rng = np.random.default_rng(20260511)
    out = []
    for _ in range(16):
        qa = q_normalize(rng.uniform(-1.0, 1.0, 4))
        qb = q_normalize(rng.uniform(-1.0, 1.0, 4))
        a = {"t": rng.uniform(-5.0, 5.0, 3).tolist(), "q": qa.tolist()}
        b = {"t": rng.uniform(-5.0, 5.0, 3).tolist(), "q": qb.tolist()}
        t0 = float(rng.integers(0, 1000))
        t1 = t0 + float(rng.integers(10, 1000))
        for frac in (0.0, 0.25, 0.5, 0.75, 1.0):
            out.append(
                {
                    "a": a,
                    "b": b,
                    "t0": t0,
                    "t1": t1,
                    "frac": frac,
                    "expected": interpolate(a, b, frac),
                }
            )
    return out


def build_predict() -> list[dict]:
    rng = np.random.default_rng(20260512)
    out = []
    for _ in range(16):
        q0 = q_normalize(rng.uniform(-1.0, 1.0, 4))
        base = {"t": rng.uniform(-3.0, 3.0, 3).tolist(), "q": q0.tolist()}
        twist = {
            "linear": rng.uniform(-2.0, 2.0, 3).tolist(),
            "angular": rng.uniform(-1.5, 1.5, 3).tolist(),
        }
        dt = float(rng.uniform(0.001, 1.0))
        out.append(
            {
                "base": base,
                "twist": twist,
                "dt": dt,
                "delta": exp_twist(twist, dt),
            }
        )
    return out


def build_bracket() -> list[dict]:
    """Bracket search: exact vs interpolated vs out-of-history classification."""
    samples = [{"t": t, "x": float(t) * 0.5} for t in (0, 100, 200, 300)]
    cases = []
    for query_t, kind in [
        (0, "exact"),
        (100, "exact"),
        (150, "interp"),
        (250, "interp"),
        (299, "interp"),
        (-1, "out"),
        (301, "out"),
    ]:
        cases.append({"query_t": query_t, "kind": kind})
    return {"samples": samples, "cases": cases}


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else (
        Path(__file__).resolve().parents[1] / "fixtures" / "spatial" / "query_vectors.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_by": "tests/oracle/query_oracle.py",
        "interp": build_interp(),
        "predict": build_predict(),
        "bracket": build_bracket(),
    }
    out.write_text(json.dumps(payload, indent=2) + "\n")
    print(
        f"wrote {out} ({len(payload['interp'])} interp, "
        f"{len(payload['predict'])} predict)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
