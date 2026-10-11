//! Compiled static BVH oracle and determinism gate
//! ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md)
//! §Test Matrix `I-X3`, §Functions `compile_bvh`; work-plan M21W1-E2-T1).
//! A brute-force AABB overlap reference is the ground truth for the compiled
//! tree: every primitive whose box overlaps the query must appear in the tree
//! result — a compiled node box over-approximates its subtree, so false
//! positives are allowed and misses are forbidden. `cargo test -p drm-workspace
//! bvh`.
//!
//! REQ: REQ-SPATIAL-006.

use drm_workspace::spatial::{Aabb, StaticPrimitive, compile_bvh};

/// A tiny deterministic PRNG (xorshift) so scenes reproduce byte-for-byte.
/// REQ: REQ-SPATIAL-006.
struct Lcg(u64);

impl Lcg {
    /// Next pseudo-random `u64`. REQ: REQ-SPATIAL-006.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A pseudo-random coordinate in `[0.0, span)`. REQ: REQ-SPATIAL-006.
    fn coord(&mut self, span: f64) -> f64 {
        f64::from(u32::try_from(self.next() % 1_000_000).unwrap_or(0)) / 1_000_000.0 * span
    }
}

/// Brute-force reference: every primitive overlapping `query`.
/// REQ: REQ-SPATIAL-006.
fn oracle(prims: &[StaticPrimitive], query: &Aabb) -> Vec<u32> {
    let mut out: Vec<u32> = prims
        .iter()
        .filter(|p| p.aabb.overlaps(query))
        .map(|p| p.entity)
        .collect();
    out.sort_unstable();
    out
}

/// REQ: REQ-SPATIAL-006 — a random scene (1k primitives) matches the brute-force
/// overlap oracle over 5k queries: no misses, false positives allowed (`I-X3`).
#[test]
fn bvh_aabb_reference() {
    let mut rng = Lcg(0xDEAD_BEEF_CAFE_1234);
    let prims: Vec<StaticPrimitive> = (0..1_000u32)
        .map(|e| {
            let c = [rng.coord(20.0), rng.coord(20.0), rng.coord(20.0)];
            let h = 0.1 + rng.coord(0.5);
            StaticPrimitive {
                entity: e,
                aabb: Aabb {
                    min: [c[0] - h, c[1] - h, c[2] - h],
                    max: [c[0] + h, c[1] + h, c[2] + h],
                },
            }
        })
        .collect();
    let bvh = compile_bvh(&prims);
    for _ in 0..5_000 {
        let c = [rng.coord(20.0), rng.coord(20.0), rng.coord(20.0)];
        let h = 0.2 + rng.coord(2.0);
        let query = Aabb {
            min: [c[0] - h, c[1] - h, c[2] - h],
            max: [c[0] + h, c[1] + h, c[2] + h],
        };
        let got: Vec<u32> = bvh.query(&query).iter().map(|p| p.entity).collect();
        for hit in oracle(&prims, &query) {
            assert!(got.contains(&hit), "miss: entity {hit} overlaps but absent");
        }
    }
}

/// REQ: REQ-SPATIAL-006 — double compile is identical, including shuffled input
/// (`I-D1` determinism).
#[test]
fn bvh_deterministic_compile() {
    let prims: Vec<StaticPrimitive> = (0..200u32)
        .map(|e| {
            let x = f64::from(e) * 0.37;
            StaticPrimitive {
                entity: e,
                aabb: Aabb {
                    min: [x - 0.2, -0.2, -0.2],
                    max: [x + 0.2, 0.2, 0.2],
                },
            }
        })
        .collect();
    let a = compile_bvh(&prims);
    let b = compile_bvh(&prims);
    assert_eq!(a, b, "double compile identical");
    let mut shuffled = prims.clone();
    shuffled.reverse();
    assert_eq!(a, compile_bvh(&shuffled), "input order independent");
}

/// REQ: REQ-SPATIAL-006 — internal node boxes are the union of their children,
/// and the root box contains every primitive (`I-X3`).
#[test]
fn bvh_node_aabb_union() {
    let prims: Vec<StaticPrimitive> = (0..64u32)
        .map(|e| {
            let c = [f64::from(e), f64::from(e % 5), 0.0];
            StaticPrimitive {
                entity: e,
                aabb: Aabb {
                    min: [c[0] - 0.5, c[1] - 0.5, -0.5],
                    max: [c[0] + 0.5, c[1] + 0.5, 0.5],
                },
            }
        })
        .collect();
    let bvh = compile_bvh(&prims);
    for node in &bvh.nodes {
        if !node.is_leaf() {
            let l = &bvh.nodes[node.left as usize];
            let r = &bvh.nodes[node.right as usize];
            assert_eq!(node.aabb, l.aabb.union(r.aabb));
        }
    }
    for p in &prims {
        assert!(bvh.root_aabb.overlaps(&p.aabb));
    }
}
