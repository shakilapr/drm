//! Spatial-under-router soak regression (work-plan M21W4-E3-T1): the spatial
//! proximity index and the boundary router run together under sustained load —
//! router egress across several links interleaved with index rebuckets and
//! proximity queries — and neither domain's invariant is disturbed by the other.
//! The router must emit exactly one frame per routable sample (no silent drop or
//! duplication) and the grid must return a candidate set that still contains
//! every brute-force hit after the load (`I-X6`, `I-X11`). `cargo test -p
//! drm-spatial --test soak_regression`.
//!
//! REQ: REQ-SPATIAL-020, REQ-SPATIAL-042, REQ-SPATIAL-043, REQ-ROUTE-022.

use drm_router::data_path::{EgressRoute, Router};
use drm_router::transport::{Reliability, TransportAdapter};
use drm_router::wire::WireHeader;
use drm_spatial::{CandidateSet, GridConfig, GridDeps, HashGrid, Point3, TypedPoint};
use drm_types::Status;

/// In-memory counting transport: every accepted frame increments the counter and
/// nothing is buffered (the load generator for the soak).
/// REQ: REQ-ROUTE-022.
#[derive(Default)]
struct CountingTransport {
    sent: u64,
}

impl TransportAdapter for CountingTransport {
    fn send_frame(&mut self, _header: &WireHeader, _payload: &[u8]) -> Result<(), Status> {
        self.sent += 1;
        Ok(())
    }

    fn recv_frame<'a>(
        &mut self,
        _buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
        Ok(None)
    }

    fn capabilities(&self) -> Reliability {
        Reliability::BestEffort
    }
}

/// A routable link/topic pair with a remote consumer (so egress is not filtered).
/// REQ: REQ-ROUTE-022.
fn route(link: u32, topic: u32, wire: u32) -> EgressRoute {
    EgressRoute {
        topic,
        link,
        wire_channel_id: wire,
        remote_consumers: true,
        max_datagram_payload: 1400,
        max_record_bytes: 4096,
    }
}

/// The anchor-pose dependency vector for `step`: a fresh own+topology epoch, so
/// moving objects invalidate their cached location without touching siblings.
/// REQ: REQ-SPATIAL-043.
fn deps(step: u64) -> GridDeps {
    GridDeps {
        own: step,
        ancestor: 1,
        calibration: 1,
        topology: 1,
    }
}

/// A deterministic object position for `id` at `step`, kept inside a bounded
/// region so the grid stays within its declared cell span.
/// REQ: REQ-SPATIAL-042.
#[allow(clippy::cast_precision_loss)]
fn position(id: u32, step: u64) -> Point3 {
    let phase = step as f64 * 0.05 + f64::from(id) * 0.37;
    Point3 {
        x: (phase.sin() * 2.0) + f64::from(id % 8) * 0.25,
        y: phase.cos() * 2.0,
        z: (phase * 0.5).sin(),
    }
}

/// Brute-force reference: every object whose current position lies within
/// `radius` of `q`. REQ: REQ-SPATIAL-020.
#[allow(clippy::cast_possible_truncation)]
fn brute_force(positions: &[Point3], q: Point3, radius: f64) -> Vec<u32> {
    let mut hits = Vec::new();
    for (i, p) in positions.iter().enumerate() {
        let d = ((p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2)).sqrt();
        if d <= radius {
            hits.push(i as u32);
        }
    }
    hits
}

/// REQ: REQ-SPATIAL-020, REQ-SPATIAL-042, REQ-SPATIAL-043, REQ-ROUTE-022 —
/// sustained router load interleaved with index movement cannot make a proximity
/// query miss a brute-force hit, and the router emits exactly one frame per
/// routable sample across the rotating links (the soak-topology shapes).
#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn spatial_under_router_soak() {
    const OBJECTS: u32 = 48;
    const STEPS: u64 = 4_000;
    const RADIUS: f64 = 3.0;

    let links = [0_u32, 1, 2]; // single-machine / two-machine / physical-pair shapes
    let route_table: Vec<EgressRoute> = links.iter().map(|&l| route(l, 7, 100 + l)).collect();
    let mut router = Router::new(route_table);
    let mut transport = CountingTransport::default();

    let cfg = GridConfig {
        cell_size: 1.0,
        max_objects: OBJECTS,
        max_cells_per_object: 16,
        max_query_cells: 256,
        slack: 0.5,
    };
    let mut grid = HashGrid::activate(cfg).unwrap_or_else(|e| unreachable!("{e:?}"));
    let mut positions: Vec<Point3> = (0..OBJECTS).map(|id| position(id, 0)).collect();
    for (id, p) in positions.iter().enumerate() {
        grid.on_sample(id as u32, *p, deps(0))
            .unwrap_or_else(|e| unreachable!("seed {id}: {e:?}"));
    }

    let mut emit_ok = 0_u64;
    let mut queries = 0_u64;
    for step in 1..=STEPS {
        // Router load: one frame per step, round-robin across the three links.
        let link = links[(step % links.len() as u64) as usize];
        match router.egress_frame(link, 7, 1, step, b"spatial-soak", &mut transport) {
            Ok(true) => emit_ok += 1,
            other => unreachable!("egress step {step}: {other:?}"),
        }

        // Spatial load: one object moves, then a periodic proximity query.
        let id = (step % u64::from(OBJECTS)) as u32;
        let p = position(id, step);
        positions[id as usize] = p;
        grid.on_sample(id, p, deps(step))
            .unwrap_or_else(|e| unreachable!("move {id}@{step}: {e:?}"));

        if step % 8 == 0 {
            let q = position((step % u64::from(OBJECTS)) as u32, step);
            let mut scratch =
                CandidateSet::with_capacity(OBJECTS).unwrap_or_else(|e| unreachable!("{e:?}"));
            let outcome = grid
                .nearby(TypedPoint { point: q, frame: 0 }, RADIUS, &mut scratch)
                .unwrap_or_else(|e| unreachable!("query {step}: {e:?}"));
            assert!(
                outcome.is_complete(),
                "query {step} overflowed its declared scratch"
            );
            let hits: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
            for expected in brute_force(&positions, q, RADIUS) {
                assert!(
                    hits.contains(&expected),
                    "step {step}: object {expected} within {RADIUS} missing from candidates"
                );
            }
            queries += 1;
        }
    }

    assert_eq!(
        emit_ok, STEPS,
        "router must emit exactly one frame per step"
    );
    assert_eq!(transport.sent, STEPS, "transport saw every emitted frame");
    assert!(queries > 0);
    // The grid is stable (even epoch) once the sustained load drains.
    assert_eq!(grid.epoch() % 2, 0, "grid latched an odd epoch");
}
