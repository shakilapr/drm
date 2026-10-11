#!/usr/bin/env python3
"""Independent spatial math oracle (E4, work-plan M19W1-E4-T1).

Numeric ground truth for the Rust `drm-spatial` implementation: rigid SE(3)
composition, quaternion rotation of points/directions/vectors, SLERP, and the
WGS84 -> ECEF -> ENU geodetic chain. This is a clean-room NumPy implementation
derived directly from docs/spatial_architecture.md; it shares no code with the
Rust crate under test.

Usage:
    python3 tests/oracle/spatial_oracle.py [out.json]

Default output: tests/fixtures/spatial/oracle_vectors.json

REQ: REQ-SPATIAL-036, REQ-SPATIAL-038, REQ-SPATIAL-012.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import numpy as np

WGS84_A = 6378137.0
WGS84_F = 1.0 / 298.257223563
WGS84_E2 = WGS84_F * (2.0 - WGS84_F)


def q_normalize(q: np.ndarray) -> np.ndarray:
    return q / np.linalg.norm(q)


def q_mul(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return q_normalize(
        np.array(
            [
                aw * bx + ax * bw + ay * bz - az * by,
                aw * by - ax * bz + ay * bw + az * bx,
                aw * bz + ax * by - ay * bx + az * bw,
                aw * bw - ax * bx - ay * by - az * bz,
            ]
        )
    )


def q_rotate(q: np.ndarray, v: np.ndarray) -> np.ndarray:
    qv = q[:3]
    t = 2.0 * np.cross(qv, v)
    return v + q[3] * t + np.cross(qv, t)


def q_slerp(a: np.ndarray, b: np.ndarray, t: float) -> np.ndarray:
    d = float(np.dot(a, b))
    end = b
    if d < 0.0:
        d = -d
        end = -b
    if d > 0.9995:
        return q_normalize(a + (end - a) * t)
    theta0 = math.acos(max(-1.0, min(1.0, d)))
    theta = theta0 * t
    s0 = math.sin((1.0 - t) * theta0) / math.sin(theta0)
    s1 = math.sin(theta) / math.sin(theta0)
    return q_normalize(a * s0 + end * s1)


def t_compose(a: dict, b: dict) -> dict:
    ta = np.array(a["t"])
    tb = np.array(b["t"])
    qa = np.array(a["q"])
    qb = np.array(b["q"])
    return {
        "t": (ta + q_rotate(qa, tb)).tolist(),
        "q": q_mul(qa, qb).tolist(),
    }


def wgs84_to_ecef(lat: float, lon: float, alt: float) -> np.ndarray:
    n = WGS84_A / math.sqrt(1.0 - WGS84_E2 * math.sin(lat) ** 2)
    return np.array(
        [
            (n + alt) * math.cos(lat) * math.cos(lon),
            (n + alt) * math.cos(lat) * math.sin(lon),
            (n * (1.0 - WGS84_E2) + alt) * math.sin(lat),
        ]
    )


def tangent_plane_enu(ecef: np.ndarray, anchor_lat: float, anchor_lon: float, anchor_alt: float) -> np.ndarray:
    origin = wgs84_to_ecef(anchor_lat, anchor_lon, anchor_alt)
    d = ecef - origin
    sl, cl = math.sin(anchor_lat), math.cos(anchor_lat)
    so, co = math.sin(anchor_lon), math.cos(anchor_lon)
    return np.array(
        [
            -so * d[0] + co * d[1],
            -sl * co * d[0] - sl * so * d[1] + cl * d[2],
            cl * co * d[0] + cl * so * d[1] + sl * d[2],
        ]
    )


def axis_angle(axis: list[float], angle: float) -> np.ndarray:
    a = np.array(axis, dtype=float)
    a = a / np.linalg.norm(a)
    return q_normalize(np.array([*(a * math.sin(angle / 2.0)), math.cos(angle / 2.0)]))


def build_vectors() -> list[dict]:
    rng = np.random.default_rng(20260511)
    vectors = []
    for _ in range(24):
        ta = rng.uniform(-5.0, 5.0, 3)
        tb = rng.uniform(-5.0, 5.0, 3)
        qa = q_normalize(rng.uniform(-1.0, 1.0, 4))
        qb = q_normalize(rng.uniform(-1.0, 1.0, 4))
        p = rng.uniform(-10.0, 10.0, 3)
        d = q_normalize(rng.uniform(-1.0, 1.0, 3))
        v = rng.uniform(-10.0, 10.0, 3)
        a = {"t": ta.tolist(), "q": qa.tolist()}
        b = {"t": tb.tolist(), "q": qb.tolist()}
        vectors.append(
            {
                "a": a,
                "b": b,
                "p": p.tolist(),
                "d": d.tolist(),
                "v": v.tolist(),
                "transform_a_point": (ta + q_rotate(qa, p)).tolist(),
                "transform_a_direction": q_rotate(qa, d).tolist(),
                "transform_a_vector": q_rotate(qa, v).tolist(),
                "compose_ab": t_compose(a, b),
                "slerp_a_b_half": q_slerp(qa, qb, 0.5).tolist(),
            }
        )
    # Near-180 degree SLERP edge case (quaternion edge-case risk in the plan).
    qa = axis_angle([0.0, 0.0, 1.0], math.pi - 1e-6)
    qb = axis_angle([1.0, 0.0, 0.0], math.pi - 1e-6)
    vectors.append(
        {
            "a": {"t": [0.0, 0.0, 0.0], "q": qa.tolist()},
            "b": {"t": [0.0, 0.0, 0.0], "q": qb.tolist()},
            "p": [1.0, 0.0, 0.0],
            "d": [0.0, 1.0, 0.0],
            "v": [0.0, 0.0, 1.0],
            "transform_a_point": q_rotate(qa, np.array([1.0, 0.0, 0.0])).tolist(),
            "transform_a_direction": q_rotate(qa, np.array([0.0, 1.0, 0.0])).tolist(),
            "transform_a_vector": q_rotate(qa, np.array([0.0, 0.0, 1.0])).tolist(),
            "compose_ab": t_compose({"t": [0.0, 0.0, 0.0], "q": qa.tolist()},
                                    {"t": [0.0, 0.0, 0.0], "q": qb.tolist()}),
            "slerp_a_b_half": q_slerp(qa, qb, 0.5).tolist(),
        }
    )
    return vectors


def build_geodetic() -> list[dict]:
    cases = [
        ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
        ((0.5, 0.2, 100.0), (0.5, 0.2, 100.0)),
        ((0.5, 0.2, 101.0), (0.5, 0.2, 100.0)),
        ((-0.3, 1.1, 42.0), (-0.3, 1.1, 42.0)),
        ((0.7853981633974483, -1.2, 250.0), (0.7853981633974483, -1.2, 240.0)),
    ]
    out = []
    for lla, anchor in cases:
        ecef = wgs84_to_ecef(*lla)
        enu = tangent_plane_enu(ecef, *anchor)
        out.append(
            {
                "lla": list(lla),
                "anchor": list(anchor),
                "ecef": ecef.tolist(),
                "enu": enu.tolist(),
            }
        )
    return out


def build_geodetic_precision() -> list[dict]:
    """Small local offsets from an anchor — the I-S5 local-precision boundary."""
    anchors = [
        (0.0, 0.0, 0.0),
        (0.5, 0.2, 100.0),
        (-0.3, 1.1, 42.0),
        (1.2, -2.0, 1500.0),
    ]
    offsets = [(4.2, 0.0, 0.0), (0.0, -7.5, 0.0), (0.0, 0.0, 12.0), (3.0, 4.0, -1.5)]
    out = []
    for anchor in anchors:
        for east, north, up in offsets:
            m = meridional_radius(anchor[0]) + anchor[2]
            n = prime_vertical_radius(anchor[0]) + anchor[2]
            lat = anchor[0] + north / m
            lon = anchor[1] + east / (n * math.cos(anchor[0]))
            alt = anchor[2] + up
            ecef = wgs84_to_ecef(lat, lon, alt)
            enu = tangent_plane_enu(ecef, *anchor)
            out.append(
                {
                    "anchor": list(anchor),
                    "target": [lat, lon, alt],
                    "offset": [east, north, up],
                    "enu": enu.tolist(),
                }
            )
    return out


def meridional_radius(lat: float) -> float:
    s = math.sin(lat)
    return WGS84_A * (1.0 - WGS84_E2) / (1.0 - WGS84_E2 * s * s) ** 1.5


def prime_vertical_radius(lat: float) -> float:
    s = math.sin(lat)
    return WGS84_A / math.sqrt(1.0 - WGS84_E2 * s * s)



def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else (
        Path(__file__).resolve().parents[1] / "fixtures" / "spatial" / "oracle_vectors.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_by": "tests/oracle/spatial_oracle.py",
        "vectors": build_vectors(),
        "geodetic": build_geodetic(),
        "geodetic_precision": build_geodetic_precision(),
    }
    out.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"wrote {out} ({len(payload['vectors'])} vectors, {len(payload['geodetic'])} geodetic)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
