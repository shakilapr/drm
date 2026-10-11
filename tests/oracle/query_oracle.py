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

REQ: REQ-SPATIAL-022, REQ-SPATIAL-024, REQ-SPATIAL-025, REQ-SPATIAL-023,
REQ-SPATIAL-016, REQ-SPATIAL-028, REQ-SPATIAL-029, REQ-SPATIAL-003,
REQ-SPATIAL-011.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import numpy as np

from spatial_oracle import (  # clean-room math reuse within the oracle
    axis_angle,
    q_normalize,
    q_rotate,
    q_slerp,
    t_compose,
)


def t_invert(a: dict) -> dict:
    q = np.array(a["q"])
    q_inv = q_normalize(np.array([-q[0], -q[1], -q[2], q[3]]))
    t = np.array(a["t"])
    return {"t": (-q_rotate(q_inv, t)).tolist(), "q": q_inv.tolist()}


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


def build_reparent() -> list[dict]:
    """Transactional reparenting ground truth (I-Q8/I-Q9).

    Composes in the compiled-plan fold order: `compile_plan` folds the path
    segments from the reference side toward the target side as
    `e0 ∘ e1 ∘ … ∘ en` ([spatial-queries.md] §Functions `compile_plan`). With
    `world` as the anchor, `e_old`/`e_new` are the parents' stored offsets and
    `e_obj` the object's local offset. The object's pre-reparent pose in the
    anchor is `e_obj ∘ e_old`; the object's pose relative to the new parent is
    `e_new ∘ e_old⁻¹ ∘ e_obj⁻¹`. Preserve World recomputes the local offset as
    the inverse of that pose, so the anchor pose stays continuous; Preserve Local
    keeps `e_obj`; Snap pins to identity.
    """
    rng = np.random.default_rng(20260513)
    out = []
    for _ in range(12):
        q_old = q_normalize(rng.uniform(-1.0, 1.0, 4))
        q_new = q_normalize(rng.uniform(-1.0, 1.0, 4))
        q_local = q_normalize(rng.uniform(-1.0, 1.0, 4))
        e_old = {"t": rng.uniform(-5.0, 5.0, 3).tolist(), "q": q_old.tolist()}
        e_new = {"t": rng.uniform(-5.0, 5.0, 3).tolist(), "q": q_new.tolist()}
        e_obj = {"t": rng.uniform(-3.0, 3.0, 3).tolist(), "q": q_local.tolist()}
        anchor_pose_before = t_compose(e_obj, e_old)
        obj_in_new = t_compose(t_compose(e_new, t_invert(e_old)), t_invert(e_obj))
        identity = {"t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0]}
        modes = {
            "preserve_world": t_invert(obj_in_new),
            "preserve_local": e_obj,
            "snap": identity,
        }
        cases = {}
        for name, local in modes.items():
            cases[name] = {
                "local": local,
                "anchor_pose": t_compose(local, e_new),
            }
        out.append(
            {
                "e_old": e_old,
                "e_new": e_new,
                "e_obj": e_obj,
                "anchor_pose_before": anchor_pose_before,
                "modes": cases,
            }
        )
    return out


def build_discontinuity() -> list[dict]:
    """Discontinuity fixtures (I-Q7): relocalize, reparent, and calibration jumps.

    Each fixture is a sample-time-ordered edge with the newer sample flagged
    discontinuous. Interpolation strictly between the brackets must be refused;
    the exact endpoints stay valid.
    """
    return {
        "cases": [
            {
                "kind": "relocalize",
                "samples": [{"t": 0, "x": 0.0}, {"t": 100, "x": 100.0}],
                "jump_at": 100,
                "query_t": 50,
                "expect": "refused",
                "endpoints_valid": [0, 100],
            },
            {
                "kind": "reparent",
                "samples": [{"t": 200, "x": 1.0}, {"t": 300, "x": -3.0}],
                "jump_at": 300,
                "query_t": 250,
                "expect": "refused",
                "endpoints_valid": [200, 300],
            },
            {
                "kind": "calibration_jump",
                "samples": [{"t": 400, "x": 2.0}, {"t": 500, "x": 7.5}],
                "jump_at": 500,
                "query_t": 450,
                "expect": "refused",
                "endpoints_valid": [400, 500],
            },
        ]
    }


def build_calibration() -> list[dict]:
    """Live calibration ground truth (I-Q11, I-Q7 tie-in).

    `Effective = Nominal x Correction`; each case carries the nominal transform,
    a sequence of corrections with their effective instants, and the jump
    magnitude (translation-delta norm + rotation-delta radians) between the
    previous and new correction. The jump-threshold discontinuity marker is set
    when the magnitude exceeds the case threshold.
    """
    rng = np.random.default_rng(20260514)
    out = []
    for _ in range(8):
        q_nom = q_normalize(rng.uniform(-1.0, 1.0, 4))
        nominal = {"t": rng.uniform(-3.0, 3.0, 3).tolist(), "q": q_nom.tolist()}
        q_c0 = q_normalize(rng.uniform(-1.0, 1.0, 4))
        q_c1 = q_normalize(rng.uniform(-1.0, 1.0, 4))
        c0 = {"t": rng.uniform(-2.0, 2.0, 3).tolist(), "q": q_c0.tolist()}
        c1 = {"t": rng.uniform(-2.0, 2.0, 3).tolist(), "q": q_c1.tolist()}
        delta_t = float(np.linalg.norm(np.array(c1["t"]) - np.array(c0["t"])))
        dot = abs(float(np.dot(np.array(c0["q"]), np.array(c1["q"]))))
        angle = 2.0 * math.acos(min(1.0, dot))
        jump = delta_t + angle
        threshold = jump * 0.5
        effective = t_compose(nominal, c1)
        out.append(
            {
                "nominal": nominal,
                "corrections": [
                    {"effective_ns": 100, "correction": c0},
                    {"effective_ns": 300, "correction": c1},
                ],
                "jump": jump,
                "threshold": threshold,
                "discontinuous": jump > threshold,
                "effective": effective,
            }
        )
    return out


def build_generational() -> list[dict]:
    """Generational-handle ground truth (I-Q12/I-S12).

    Each case names a frame, its parent, a reuse frame, and how many
    create/delete cycles precede the final allocation. The generation of an index
    equals its delete count, so `expected_generation = cycles` after `cycles`
    deletes; every handle from a previous incarnation must be dead.
    """
    return {
        "cases": [
            {
                "frame": "object/42",
                "parent": "map",
                "reuse_frame": "object/43",
                "cycles": 1,
                "expected_generation": 1,
            },
            {
                "frame": "obstacle/7",
                "parent": "odom",
                "reuse_frame": "obstacle/8",
                "cycles": 2,
                "expected_generation": 2,
            },
            {
                "frame": "track/9",
                "parent": "odom",
                "reuse_frame": "track/10",
                "cycles": 3,
                "expected_generation": 3,
            },
        ]
    }


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
        "reparent": build_reparent(),
        "discontinuity": build_discontinuity(),
        "calibration": build_calibration(),
        "generational": build_generational(),
    }
    out.write_text(json.dumps(payload, indent=2) + "\n")
    print(
        f"wrote {out} ({len(payload['interp'])} interp, "
        f"{len(payload['predict'])} predict, "
        f"{len(payload['reparent'])} reparent, "
        f"{len(payload['calibration'])} calibration)"
    )
    return 0



if __name__ == "__main__":
    raise SystemExit(main())
