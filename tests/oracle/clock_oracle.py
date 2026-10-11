#!/usr/bin/env python3
"""Independent clock-domain mapping oracle (E4, work-plan M19W2-E4-T1).

Numeric ground truth for the Rust `drm-spatial` affine clock mapping and its
bounded uncertainty growth. A clean-room implementation derived directly from
docs/algorithms/spatial/spatial-core.md (§Temporal Precision) and
docs/spatial_architecture.md; it shares no code with the Rust crate under test.

Model (per mapping):
    t_to = to_ref + rate_ratio * (t_from - from_ref)
    uncertainty(t_from) = uncertainty_at_ref + growth * |t_from - from_ref|

Usage:
    python3 tests/oracle/clock_oracle.py [out.json]

Default output: tests/fixtures/spatial/clock_vectors.json

REQ: REQ-SPATIAL-004, REQ-SPATIAL-041, REQ-SPATIAL-051.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def map_uncertainty(mapping: dict, t_from: int) -> tuple[int, int] | None:
    """Returns (mapped_ns, uncertainty_ns), or None when unresolved.

    Truncates toward zero exactly like the Rust `as u64` cast: the affine result
    must be non-negative and representable, otherwise unresolved.
    """
    lo = mapping["valid_from_ns"]
    hi = mapping["valid_until_ns"]
    if t_from < lo or t_from > hi:
        return None
    r = mapping["rate_ratio"]
    mapped = mapping["to_ref_ns"] + r * (t_from - mapping["from_ref_ns"])
    if mapped < 0.0:
        return None
    mapped_ns = int(mapped)
    elapsed = abs(t_from - mapping["from_ref_ns"])
    uncertainty = mapping["uncertainty_at_ref_ns"] + int(
        mapping["uncertainty_growth"] * elapsed
    )
    return mapped_ns, uncertainty


def build_cases() -> list[dict]:
    mappings = [
        {
            "to_ref_ns": 2_000_000,
            "from_ref_ns": 1_000_000,
            "rate_ratio": 1.000_002,
            "uncertainty_at_ref_ns": 50,
            "uncertainty_growth": 0.00001,
            "valid_from_ns": 0,
            "valid_until_ns": 10_000_000,
        },
        {
            "to_ref_ns": 0,
            "from_ref_ns": 0,
            "rate_ratio": 0.999_998,
            "uncertainty_at_ref_ns": 0,
            "uncertainty_growth": 0.0,
            "valid_from_ns": 0,
            "valid_until_ns": 1_000_000_000,
        },
        {
            "to_ref_ns": 500,
            "from_ref_ns": 1_000,
            "rate_ratio": 1.0,
            "uncertainty_at_ref_ns": 25,
            "uncertainty_growth": 0.001,
            "valid_from_ns": 500,
            "valid_until_ns": 5_000,
        },
    ]
    offsets = [0, 1, 10_000, 100_000, 1_000_000, -100_000]
    cases = []
    for m in mappings:
        for off in offsets:
            t = m["from_ref_ns"] + off
            got = map_uncertainty(m, t)
            cases.append(
                {
                    "mapping": m,
                    "t_from_ns": t,
                    "resolved": got is not None,
                    "t_to_ns": None if got is None else got[0],
                    "uncertainty_ns": None if got is None else got[1],
                }
            )
    return cases


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else (
        Path(__file__).resolve().parents[1] / "fixtures" / "spatial" / "clock_vectors.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_by": "tests/oracle/clock_oracle.py",
        "cases": build_cases(),
    }
    out.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"wrote {out} ({len(payload['cases'])} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
