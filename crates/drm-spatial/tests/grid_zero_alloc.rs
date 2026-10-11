//! Zero-allocation steady-state gate (`spatial_grid_zero_alloc`, `I-X1`,
//! `REQ-SPATIAL-013`) plus the incremental re-bucket scaling evidence
//! (`spatial_rebucket_scaling`, `I-X12`, `REQ-SPATIAL-042`). `harness = false`
//! binary: single-threaded, a counting allocator observes allocator deltas
//! around steady-state `on_sample` + `nearby` once every capacity is declared.
//!
//! Run: `cargo test -p drm-spatial --test grid_zero_alloc`.
//! REQ: REQ-SPATIAL-013, REQ-SPATIAL-042.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use drm_spatial::{CandidateSet, GridConfig, GridDeps, HashGrid, Point3, TypedPoint};

/// Total allocator entry events (alloc, realloc, `alloc_zeroed`).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Total deallocation events.
static DEALLOCS: AtomicU64 = AtomicU64::new(0);

/// A system allocator that counts every entry, installed only in this harness.
/// REQ: REQ-SPATIAL-013.
struct CountingAlloc;

// SAFETY: the wrapper forwards every operation to `System` unchanged, only
// incrementing counters; the returned pointers and layouts are the system's.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Deterministic dependency vector. REQ: REQ-SPATIAL-013.
fn deps(v: u64) -> GridDeps {
    GridDeps {
        own: v,
        ancestor: v,
        calibration: v,
        topology: v,
    }
}

/// A grid config with all capacities declared up front.
/// REQ: REQ-SPATIAL-013.
fn config(objects: u32) -> GridConfig {
    GridConfig {
        cell_size: 0.5,
        max_objects: objects,
        max_cells_per_object: 27,
        max_query_cells: 512,
        slack: 0.25,
    }
}

/// REQ: REQ-SPATIAL-013 — `spatial_grid_zero_alloc`: steady-state indexing and
/// queries perform zero heap allocations once capacities are declared (`I-X1`).
fn main() {
    let n = 128u32;
    let mut grid = HashGrid::activate(config(n)).unwrap_or_else(|e| unreachable!("{e}"));
    let mut scratch = CandidateSet::with_capacity(n).unwrap_or_else(|e| unreachable!("{e}"));
    // Fill every object once (activation/insert may allocate membership; the
    // measured block below excludes it).
    for i in 0..n {
        grid.on_sample(
            i,
            Point3 {
                x: f64::from(i) * 0.5,
                y: 0.0,
                z: 0.0,
            },
            deps(1),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
    }

    let before = (
        ALLOCS.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
    );
    // Steady state: dependency-only updates and queries — no new cells.
    for v in 2..1_000u64 {
        let i = u32::try_from(v % u64::from(n)).unwrap_or(0);
        grid.on_sample(
            i,
            Point3 {
                x: f64::from(i) * 0.5,
                y: 0.0,
                z: 0.0,
            },
            deps(v),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        let _ = grid
            .nearby(
                TypedPoint {
                    point: Point3 {
                        x: f64::from(i) * 0.5,
                        y: 0.0,
                        z: 0.0,
                    },
                    frame: 0,
                },
                0.6,
                &mut scratch,
            )
            .unwrap_or_else(|e| unreachable!("{e:?}"));
    }
    let after = (
        ALLOCS.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
    );
    assert_eq!(after.0 - before.0, 0, "steady-state allocations");
    assert_eq!(after.1 - before.1, 0, "steady-state deallocations");
    println!("spatial_grid_zero_alloc: steady-state allocation-free (0/0)");

    // REQ: REQ-SPATIAL-042 — `spatial_rebucket_scaling`: one mover's re-bucket
    // work is independent of total capacity (O(k) cell edits, `I-X12`).
    for cap in [256u32, 8192] {
        let mut g = HashGrid::activate(config(cap)).unwrap_or_else(|e| unreachable!("{e}"));
        for i in 0..cap {
            g.on_sample(
                i,
                Point3 {
                    x: f64::from(i) * 2.0,
                    y: 0.0,
                    z: 0.0,
                },
                deps(1),
            )
            .unwrap_or_else(|e| unreachable!("{e}"));
        }
        let base = g.rebucket_count();
        // A single far mover re-buckets exactly once regardless of capacity.
        g.on_sample(
            0,
            Point3 {
                x: 1_000_000.0,
                y: 0.0,
                z: 0.0,
            },
            deps(2),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            g.rebucket_count() - base,
            1,
            "one mover ⇒ one re-bucket at capacity {cap}"
        );
    }
    println!("spatial_rebucket_scaling: one mover ⇒ O(k) edits at both capacities");
}
