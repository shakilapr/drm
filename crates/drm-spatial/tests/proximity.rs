//! Two-source proximity union gate and completeness property
//! ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md)
//! §Test Matrix `spatial_proximity_union`, `spatial_proximity_complete`;
//! work-plan M21W2-E1-T2/E4-T1). The union of the dynamic hash grid and the
//! compiled static BVH is a deliberate superset: false positives are allowed,
//! misses are forbidden (`I-X6`). A brute-force oracle over both sources is the
//! ground truth for 10k random scenes; the false-positive factor is measured and
//! documented. `cargo test -p drm-spatial proximity`.
//!
//! REQ: REQ-SPATIAL-006, REQ-SPATIAL-013, REQ-SPATIAL-020, REQ-SPATIAL-043.

use drm_spatial::{
    CandidateSet, GridConfig, GridDeps, HashGrid, Point3, ProximityIndex, STATIC_ENTITY_BASE,
    SpatialError, StaticAabb, StaticObject, TypedAabb, TypedPoint,
};

/// A tiny deterministic PRNG (xorshift) so scenes reproduce byte-for-byte.
/// REQ: REQ-SPATIAL-020.
struct Lcg(u64);

impl Lcg {
    /// Next pseudo-random `u64`. REQ: REQ-SPATIAL-020.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A pseudo-random coordinate in `[0.0, span)`. REQ: REQ-SPATIAL-020.
    fn coord(&mut self, span: f64) -> f64 {
        f64::from(u32::try_from(self.next() % 1_000_000).unwrap_or(0)) / 1_000_000.0 * span
    }

    /// A pseudo-random `f64` in `[0.0, span)`. REQ: REQ-SPATIAL-020.
    fn f(&mut self, span: f64) -> f64 {
        self.coord(span)
    }
}

/// Deterministic dependency vector. REQ: REQ-SPATIAL-013.
fn deps(v: u64) -> GridDeps {
    GridDeps {
        own: v,
        ancestor: v,
        calibration: v,
        topology: v,
    }
}

/// Grid config for union scenes. REQ: REQ-SPATIAL-013.
fn config(objects: u32) -> GridConfig {
    GridConfig {
        cell_size: 0.5,
        max_objects: objects,
        max_cells_per_object: 27,
        max_query_cells: 4096,
        slack: 0.1,
    }
}

/// Entity-id base for static primitives so grid object indices never collide
/// with static entity ids. REQ: REQ-SPATIAL-006.
const STATIC_BASE: u32 = STATIC_ENTITY_BASE;

/// Brute-force oracle over grid objects: every indexed object within `radius`.
/// REQ: REQ-SPATIAL-020.
fn oracle_dynamic(points: &[Point3], q: Point3, radius: f64) -> Vec<u32> {
    let r2 = radius * radius;
    let mut out = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let d2 = (p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2);
        if d2 <= r2 {
            out.push(u32::try_from(i).unwrap_or(u32::MAX));
        }
    }
    out
}

/// Brute-force oracle over static primitives: every box within `radius`.
/// REQ: REQ-SPATIAL-006.
fn oracle_static(statics: &[StaticObject], q: Point3, radius: f64) -> Vec<u32> {
    let sphere = StaticAabb {
        min: Point3 {
            x: q.x - radius,
            y: q.y - radius,
            z: q.z - radius,
        },
        max: Point3 {
            x: q.x + radius,
            y: q.y + radius,
            z: q.z + radius,
        },
    };
    let mut out: Vec<u32> = statics
        .iter()
        .filter(|s| s.aabb.overlaps(&sphere))
        .map(|s| s.entity + STATIC_BASE)
        .collect();
    out.sort_unstable();
    out
}

/// Generates a random scene: `n` dynamic points and `m` static boxes in a
/// `span`-wide cube. REQ: REQ-SPATIAL-020.
fn scene(rng: &mut Lcg, n: u32, m: u32, span: f64) -> (Vec<Point3>, Vec<StaticObject>) {
    let dynamic = (0..n)
        .map(|_| Point3 {
            x: rng.f(span),
            y: rng.f(span),
            z: rng.f(span),
        })
        .collect();
    let statics = (0..m)
        .map(|i| {
            let c = [rng.f(span), rng.f(span), rng.f(span)];
            let h = 0.05 + rng.f(0.4);
            StaticObject {
                entity: i,
                aabb: StaticAabb {
                    min: Point3 {
                        x: c[0] - h,
                        y: c[1] - h,
                        z: c[2] - h,
                    },
                    max: Point3 {
                        x: c[0] + h,
                        y: c[1] + h,
                        z: c[2] + h,
                    },
                },
            }
        })
        .collect();
    (dynamic, statics)
}

/// REQ: REQ-SPATIAL-020 — `spatial_proximity_union`: the union returns the grid
/// hit, the static hit, and both; a foreign frame is rejected (`I-X5`).
#[test]
fn proximity_union_overlap() {
    let statics = vec![StaticObject {
        entity: 7,
        aabb: StaticAabb {
            min: Point3 {
                x: 1.0,
                y: -0.2,
                z: -0.2,
            },
            max: Point3 {
                x: 1.3,
                y: 0.2,
                z: 0.2,
            },
        },
    }];
    let mut idx =
        ProximityIndex::activate(config(8), 0, &statics).unwrap_or_else(|e| unreachable!("{e}"));
    idx.on_sample(
        0,
        Point3 {
            x: 1.1,
            y: 0.0,
            z: 0.0,
        },
        deps(1),
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    let mut scratch = CandidateSet::with_capacity(8).unwrap_or_else(|e| unreachable!("{e}"));
    let outcome = idx
        .nearby(
            TypedPoint {
                point: Point3 {
                    x: 1.1,
                    y: 0.0,
                    z: 0.0,
                },
                frame: 0,
            },
            0.5,
            &mut scratch,
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert!(outcome.is_complete());
    let mut indices: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
    indices.sort_unstable();
    assert!(indices.contains(&0), "grid candidate present: {indices:?}");
    assert!(
        indices.contains(&(7 + STATIC_BASE)),
        "static candidate present: {indices:?}"
    );

    // A foreign query frame is rejected through the standard path (I-X5).
    assert_eq!(
        idx.nearby(
            TypedPoint {
                point: Point3::default(),
                frame: 9,
            },
            0.5,
            &mut scratch,
        ),
        Err(SpatialError::PathNotFound)
    );
    assert_eq!(
        idx.region(
            TypedAabb {
                min: Point3::default(),
                max: Point3::default(),
                frame: 9,
            },
            &mut scratch,
        ),
        Err(SpatialError::PathNotFound)
    );
}

/// REQ: REQ-SPATIAL-006 — the compiled static BVH over-approximates: an internal
/// box is the union of its children and a query never misses an overlap.
#[test]
fn static_bvh_over_approximation() {
    let statics: Vec<StaticObject> = (0..64u32)
        .map(|i| {
            let c = f64::from(i) * 0.7;
            StaticObject {
                entity: i,
                aabb: StaticAabb {
                    min: Point3 {
                        x: c - 0.3,
                        y: -0.3,
                        z: -0.3,
                    },
                    max: Point3 {
                        x: c + 0.3,
                        y: 0.3,
                        z: 0.3,
                    },
                },
            }
        })
        .collect();
    let bvh = drm_spatial::compile_static_bvh(&statics);
    assert_eq!(bvh.object_count(), 64);
    // A binary tree over n leaves holds exactly 2·leaves − 1 nodes, and leaves
    // never exceed primitives, so the compiled node count is bounded by the
    // 2n−1 ceiling the build-time budget mirror also uses (I-X3/I-X9). The
    // per-primitive overlap loop below proves every primitive is reachable.
    assert!(
        bvh.node_count() <= 127,
        "compiled nodes exceed the 2n−1 ceiling: {}",
        bvh.node_count()
    );
    let mut scratch = CandidateSet::with_capacity(256).unwrap_or_else(|e| unreachable!("{e}"));
    for i in 0..64u32 {
        scratch.clear();
        let c = f64::from(i) * 0.7;
        bvh.merge_overlap(
            &StaticAabb {
                min: Point3 {
                    x: c - 0.01,
                    y: -0.01,
                    z: -0.01,
                },
                max: Point3 {
                    x: c + 0.01,
                    y: 0.01,
                    z: 0.01,
                },
            },
            &mut scratch,
        );
        assert!(
            scratch.handles().iter().any(|h| h.index == i + STATIC_BASE),
            "primitive {i} missed"
        );
    }
}

/// REQ: REQ-SPATIAL-020 — `spatial_proximity_complete`: 10k random scenes never
/// miss an oracle hit across the grid ∪ BVH union; the false-positive factor is
/// measured and printed (`I-X6`, `PROX_COMPLETE`).
#[test]
#[allow(clippy::cast_precision_loss)]
fn spatial_proximity_complete() {
    let mut rng = Lcg(0x0BAD_F00D_1234_5678);
    let (n, m, span) = (24u32, 24u32, 6.0);
    let mut total_hits = 0u64;
    let mut total_candidates = 0u64;
    let mut misses = 0u64;

    for _ in 0..10_000u32 {
        let (dynamic, statics) = scene(&mut rng, n, m, span);
        let mut idx = ProximityIndex::activate(config(n), 0, &statics)
            .unwrap_or_else(|e| unreachable!("{e}"));
        for (i, p) in dynamic.iter().enumerate() {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            idx.on_sample(i, *p, deps(1))
                .unwrap_or_else(|e| unreachable!("{e}"));
        }
        let q = Point3 {
            x: rng.f(span),
            y: rng.f(span),
            z: rng.f(span),
        };
        let radius = 0.05 + rng.f(1.0);
        let mut scratch = CandidateSet::with_capacity(256).unwrap_or_else(|e| unreachable!("{e}"));
        let outcome = idx
            .nearby(TypedPoint { point: q, frame: 0 }, radius, &mut scratch)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert!(outcome.is_complete(), "scratch must hold the superset");

        let found: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
        total_candidates += found.len() as u64;
        let mut expect = oracle_dynamic(&dynamic, q, radius);
        expect.extend(oracle_static(&statics, q, radius));
        for hit in expect {
            total_hits += 1;
            if !found.contains(&hit) {
                misses += 1;
            }
        }
    }
    assert_eq!(misses, 0, "I-X6: misses forbidden over 10k scenes");
    assert!(total_hits > 0, "oracle produced hits to compare");
    assert!(total_candidates > 0, "union produced candidates");
    // False-positive factor: candidates delivered / true hits. Documented, not
    // gated — a candidate superset legitimately over-approximates (`I-X6`).
    let factor = total_candidates as f64 / total_hits as f64;
    println!(
        "spatial_proximity_complete: 0 misses; candidates={total_candidates} hits={total_hits} false_positive_factor={factor:.3}"
    );
}

/// REQ: REQ-SPATIAL-020 — `proximity_false_positive_factor`: on an adversarial
/// dense scene the union still never misses, and the factor is reported
/// (documented risk, never a gate). `I-X6`.
#[test]
#[allow(clippy::cast_precision_loss)]
fn proximity_false_positive_factor() {
    let mut rng = Lcg(0x00C0_FFEE_DEAD_BEEF);
    let n = 64u32;
    let dynamic: Vec<Point3> = (0..n)
        .map(|_| Point3 {
            x: rng.f(1.0),
            y: rng.f(1.0),
            z: rng.f(1.0),
        })
        .collect();
    let mut idx =
        ProximityIndex::activate(config(n), 0, &[]).unwrap_or_else(|e| unreachable!("{e}"));
    for (i, p) in dynamic.iter().enumerate() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        idx.on_sample(i, *p, deps(1))
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let mut scratch = CandidateSet::with_capacity(256).unwrap_or_else(|e| unreachable!("{e}"));
    let mut hits = 0u64;
    let mut candidates = 0u64;
    for _ in 0..2_000u32 {
        let q = Point3 {
            x: rng.f(1.0),
            y: rng.f(1.0),
            z: rng.f(1.0),
        };
        let radius = 0.02 + rng.f(0.3);
        idx.nearby(TypedPoint { point: q, frame: 0 }, radius, &mut scratch)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let found: Vec<u32> = scratch.handles().iter().map(|h| h.index).collect();
        candidates += found.len() as u64;
        for hit in oracle_dynamic(&dynamic, q, radius) {
            hits += 1;
            assert!(found.contains(&hit), "miss: {hit}");
        }
    }
    assert!(hits > 0);
    let factor = candidates as f64 / hits as f64;
    // False positives are allowed (`I-X6`); the factor is documented, not gated.
    // A generous adversarial bound only catches pathological blowup.
    assert!(factor < 64.0, "false-positive factor blowup: {factor}");
    println!("proximity_false_positive_factor: {factor:.3}");
}

/// REQ: REQ-SPATIAL-013 — the grid remains usable standalone alongside the
/// union (no behavior regression from the union additions). The scratch must
/// hold the candidate superset: `max_objects` dynamic handles plus the static
/// namespace is not involved here, so the declared object bound suffices
/// (`I-X6`); the candidate count is bounded by `max_objects`, never by the
/// query's cell span.
#[test]
fn grid_still_queryable() {
    let mut grid = HashGrid::activate(config(4)).unwrap_or_else(|e| unreachable!("{e}"));
    grid.on_sample(0, Point3::default(), deps(1))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let mut scratch = CandidateSet::with_capacity(4).unwrap_or_else(|e| unreachable!("{e}"));
    let outcome = grid
        .nearby(
            TypedPoint {
                point: Point3::default(),
                frame: 0,
            },
            0.5,
            &mut scratch,
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    // The single live object is the only candidate; deduplication across the
    // envelope's cells collapses it to one handle, within the declared bound.
    assert_eq!(
        scratch.handles().len(),
        1,
        "one live object ⇒ one candidate"
    );
    assert!(outcome.is_complete());
}
