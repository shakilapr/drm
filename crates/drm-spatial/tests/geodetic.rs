//! Geodetic boundary precision suite (`cargo test -p drm-spatial geodetic`):
//! the `WGS84 → ECEF → tangent-plane → ENU` chain keeps local operations at
//! small metric offsets, so a point a few meters from its anchor lands within
//! [`GEODETIC_LOCAL_BOUND_M`] of the independent ellipsoid-radius reference
//! (`I-S5`, [spatial-core.md] §Invariants `I-S5`).
//!
//! REQ: REQ-SPATIAL-012.

use drm_spatial::{GEODETIC_LOCAL_BOUND_M, Lla, geodetic_to_enu};

/// WGS84 semi-major axis, meters.
const A: f64 = 6_378_137.0;
/// WGS84 first eccentricity squared.
const E2: f64 = (1.0 / 298.257_223_563) * (2.0 - 1.0 / 298.257_223_563);

/// Meridional radius of curvature at `lat_rad`, meters (analytic reference).
/// REQ: REQ-SPATIAL-012.
fn meridional_radius(lat_rad: f64) -> f64 {
    let s = lat_rad.sin();
    A * (1.0 - E2) / (1.0 - E2 * s * s).powf(1.5)
}

/// Prime-vertical radius of curvature at `lat_rad`, meters (analytic reference).
/// REQ: REQ-SPATIAL-012.
fn prime_vertical_radius(lat_rad: f64) -> f64 {
    let s = lat_rad.sin();
    A / (1.0 - E2 * s * s).sqrt()
}

/// Builds the LLA reached by moving `(east, north, up)` meters from `anchor`
/// using the analytic ellipsoid radii at the anchor altitude — independent of
/// the ECEF math under test.
/// REQ: REQ-SPATIAL-012.
fn offset_lla(anchor: Lla, east: f64, north: f64, up: f64) -> Lla {
    let m = meridional_radius(anchor.lat_rad) + anchor.alt_m;
    let n = prime_vertical_radius(anchor.lat_rad) + anchor.alt_m;
    Lla {
        lat_rad: anchor.lat_rad + north / m,
        lon_rad: anchor.lon_rad + east / (n * anchor.lat_rad.cos()),
        alt_m: anchor.alt_m + up,
    }
}

/// REQ: REQ-SPATIAL-012 — `spatial_geodetic_precision`: local ENU offsets match
/// the independent ellipsoid reference within the documented bound (`I-S5`).
#[test]
fn spatial_geodetic_precision() {
    let anchors = [
        Lla {
            lat_rad: 0.0,
            lon_rad: 0.0,
            alt_m: 0.0,
        },
        Lla {
            lat_rad: 0.5,
            lon_rad: 0.2,
            alt_m: 100.0,
        },
        Lla {
            lat_rad: -0.3,
            lon_rad: 1.1,
            alt_m: 42.0,
        },
        Lla {
            lat_rad: 1.2,
            lon_rad: -2.0,
            alt_m: 1500.0,
        },
    ];
    for anchor in anchors {
        for (east, north, up) in [
            (4.2, 0.0, 0.0),
            (0.0, -7.5, 0.0),
            (0.0, 0.0, 12.0),
            (3.0, 4.0, -1.5),
        ] {
            let target = offset_lla(anchor, east, north, up);
            let enu = geodetic_to_enu(target, anchor);
            assert!(
                (enu.east - east).abs() < GEODETIC_LOCAL_BOUND_M,
                "east: got {} want {east} at {anchor:?}",
                enu.east
            );
            assert!(
                (enu.north - north).abs() < GEODETIC_LOCAL_BOUND_M,
                "north: got {} want {north} at {anchor:?}",
                enu.north
            );
            assert!(
                (enu.up - up).abs() < GEODETIC_LOCAL_BOUND_M,
                "up: got {} want {up} at {anchor:?}",
                enu.up
            );
        }
    }
}

/// REQ: REQ-SPATIAL-012 — the anchor itself is exactly the local origin (no
/// million-meter `ECEF` residual leaks into the local frame, `I-S5`).
#[test]
fn geodetic_anchor_is_local_origin() {
    let anchor = Lla {
        lat_rad: 0.9,
        lon_rad: 0.4,
        alt_m: 300.0,
    };
    let enu = geodetic_to_enu(anchor, anchor);
    assert!(enu.east.abs() < 1e-9);
    assert!(enu.north.abs() < 1e-9);
    assert!(enu.up.abs() < 1e-9);
}

/// REQ: REQ-SPATIAL-012 — `spatial_geodetic_oracle`: the local chain matches the
/// independent E4 `NumPy` oracle (`tests/oracle/spatial_oracle.py`
/// `geodetic_precision`) within the documented bound (`I-S5`).
#[test]
fn geodetic_precision_oracle() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/spatial/oracle_vectors.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| unreachable!("oracle: {e}"));
    let doc: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| unreachable!("parse: {e}"));
    let cases = doc["geodetic_precision"]
        .as_array()
        .unwrap_or_else(|| unreachable!("geodetic_precision"));
    assert!(!cases.is_empty());
    for case in cases {
        let anchor = Lla {
            lat_rad: case["anchor"][0].as_f64().unwrap_or_default(),
            lon_rad: case["anchor"][1].as_f64().unwrap_or_default(),
            alt_m: case["anchor"][2].as_f64().unwrap_or_default(),
        };
        let target = Lla {
            lat_rad: case["target"][0].as_f64().unwrap_or_default(),
            lon_rad: case["target"][1].as_f64().unwrap_or_default(),
            alt_m: case["target"][2].as_f64().unwrap_or_default(),
        };
        let got = geodetic_to_enu(target, anchor);
        let want = [
            case["enu"][0].as_f64().unwrap_or_default(),
            case["enu"][1].as_f64().unwrap_or_default(),
            case["enu"][2].as_f64().unwrap_or_default(),
        ];
        assert!(
            (got.east - want[0]).abs() < 1e-9,
            "east {:?}",
            case["anchor"]
        );
        assert!(
            (got.north - want[1]).abs() < 1e-9,
            "north {:?}",
            case["anchor"]
        );
        assert!((got.up - want[2]).abs() < 1e-9, "up {:?}", case["anchor"]);
    }
}
