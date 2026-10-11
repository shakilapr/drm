//! Geodetic boundary chain: `WGS84 → ECEF → tangent-plane → ENU`
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Functions; [`spatial_architecture.md`](../../docs/spatial_architecture.md)
//! §Canonical Conventions and Geodetic Boundaries). Local operations stay at
//! small metric offsets around an anchor instead of million-meter `ECEF` deltas
//! (`I-S5`).
//!
//! REQ: REQ-SPATIAL-012, REQ-SPATIAL-007.

/// WGS84 semi-major axis, meters.
const WGS84_A: f64 = 6_378_137.0;
/// WGS84 flattening.
const WGS84_F: f64 = 1.0 / 298.257_223_563;
/// WGS84 first eccentricity squared.
const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);

/// Geodetic latitude/longitude/altitude (`WGS84`); radians and meters.
/// REQ: REQ-SPATIAL-012.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lla {
    /// Geodetic latitude, radians.
    pub lat_rad: f64,
    /// Longitude, radians.
    pub lon_rad: f64,
    /// Ellipsoidal height, meters.
    pub alt_m: f64,
}

/// Earth-centered, Earth-fixed Cartesian coordinates, meters.
/// REQ: REQ-SPATIAL-012.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ecef {
    /// `X`, meters.
    pub x: f64,
    /// `Y`, meters.
    pub y: f64,
    /// `Z`, meters.
    pub z: f64,
}

/// East-North-Up local tangent-plane coordinates, meters.
/// REQ: REQ-SPATIAL-012.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Enu {
    /// East offset, meters.
    pub east: f64,
    /// North offset, meters.
    pub north: f64,
    /// Up offset, meters.
    pub up: f64,
}

/// Converts `WGS84` geodetic coordinates to `ECEF`.
///
/// REQ: REQ-SPATIAL-012.
#[must_use]
pub fn wgs84_to_ecef(lla: Lla) -> Ecef {
    let (sin_lat, cos_lat) = lla.lat_rad.sin_cos();
    let (sin_lon, cos_lon) = lla.lon_rad.sin_cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    Ecef {
        x: (n + lla.alt_m) * cos_lat * cos_lon,
        y: (n + lla.alt_m) * cos_lat * sin_lon,
        z: (n * (1.0 - WGS84_E2) + lla.alt_m) * sin_lat,
    }
}

/// Projects an `ECEF` point onto the local East-North-Up tangent plane anchored
/// at `anchor` (`I-S5`: small-offset local frame).
///
/// REQ: REQ-SPATIAL-012.
#[must_use]
pub fn tangent_plane_enu(ecef: Ecef, anchor: Lla) -> Enu {
    let origin = wgs84_to_ecef(anchor);
    let dx = ecef.x - origin.x;
    let dy = ecef.y - origin.y;
    let dz = ecef.z - origin.z;
    let (sin_lat, cos_lat) = anchor.lat_rad.sin_cos();
    let (sin_lon, cos_lon) = anchor.lon_rad.sin_cos();
    Enu {
        east: -sin_lon * dx + cos_lon * dy,
        north: -sin_lat * cos_lon * dx - sin_lat * sin_lon * dy + cos_lat * dz,
        up: cos_lat * cos_lon * dx + cos_lat * sin_lon * dy + sin_lat * dz,
    }
}

/// The full boundary chain `WGS84 → ECEF → tangent-plane → ENU`.
///
/// REQ: REQ-SPATIAL-012.
#[must_use]
pub fn geodetic_to_enu(wgs84: Lla, anchor: Lla) -> Enu {
    tangent_plane_enu(wgs84_to_ecef(wgs84), anchor)
}

#[cfg(test)]
mod tests {
    use crate::geodetic::{Lla, geodetic_to_enu, wgs84_to_ecef};

    /// REQ: REQ-SPATIAL-012 — the equator/prime-meridian origin maps to
    /// `(a, 0, 0)` in `ECEF`.
    #[test]
    fn ecef_origin() {
        let e = wgs84_to_ecef(Lla {
            lat_rad: 0.0,
            lon_rad: 0.0,
            alt_m: 0.0,
        });
        assert!((e.x - 6_378_137.0).abs() < 1e-6);
        assert!(e.y.abs() < 1e-6);
        assert!(e.z.abs() < 1e-6);
    }

    /// REQ: REQ-SPATIAL-012 — a point 1 m above the anchor projects to `up = 1`
    /// (small-offset local frame, `I-S5`).
    #[test]
    fn enu_local_offset() {
        let anchor = Lla {
            lat_rad: 0.5,
            lon_rad: 0.2,
            alt_m: 100.0,
        };
        let above = Lla {
            alt_m: 101.0,
            ..anchor
        };
        let enu = geodetic_to_enu(above, anchor);
        assert!(enu.east.abs() < 1e-6);
        assert!(enu.north.abs() < 1e-6);
        assert!((enu.up - 1.0).abs() < 1e-6);
    }

    /// REQ: REQ-SPATIAL-012 — the anchor itself is the local origin.
    #[test]
    fn anchor_is_origin() {
        let anchor = Lla {
            lat_rad: -0.3,
            lon_rad: 1.1,
            alt_m: 42.0,
        };
        let enu = geodetic_to_enu(anchor, anchor);
        assert!(enu.east.abs() < 1e-9);
        assert!(enu.north.abs() < 1e-9);
        assert!(enu.up.abs() < 1e-9);
    }
}
