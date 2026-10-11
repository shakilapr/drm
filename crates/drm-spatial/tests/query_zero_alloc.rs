//! Zero-allocation query-execution gate (`spatial_query_zero_alloc`, `I-Q15`).
//! `harness = false` binary: single-threaded, a counting allocator observes the
//! allocator deltas around `execute_steps` over a fixed compiled plan. Query
//! execution uses generated plans, a fixed transform stack, and preallocated
//! histories — the shared executor performs no query-time heap allocation.
//!
//! Run: `cargo test -p drm-spatial --test query_zero_alloc`.
//! REQ: REQ-SPATIAL-034, REQ-SPATIAL-050.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use drm_spatial::{PlanStep, Transform3, execute_steps};

/// Total allocator entry events (alloc, realloc, `alloc_zeroed`).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Total deallocation events.
static DEALLOCS: AtomicU64 = AtomicU64::new(0);

/// A system allocator that counts every entry, installed only in this harness.
/// REQ: REQ-SPATIAL-034.
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

/// A fixed generated-plan step slice (edge read, invert, constant, fold).
/// REQ: REQ-SPATIAL-034.
static STEPS: [PlanStep; 4] = [
    PlanStep::DynamicEdge(0),
    PlanStep::Invert,
    PlanStep::Constant(Transform3::IDENTITY),
    PlanStep::Multiply,
];

/// REQ: REQ-SPATIAL-034, REQ-SPATIAL-050 — `spatial_query_zero_alloc`:
/// `execute_steps` over a fixed plan performs zero query-time heap allocations
/// (`I-Q15`).
fn main() {
    let sample = Transform3 {
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    // Warm-up once so any one-time lazy initialization is not attributed to the
    // measured steady-state block.
    let _ = execute_steps(&STEPS, |_e| Ok(sample), |_s| Ok(Transform3::IDENTITY))
        .unwrap_or_else(|e| unreachable!("warmup: {e:?}"));

    let before = (
        ALLOCS.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
    );
    for _ in 0..1_000 {
        let out = execute_steps(&STEPS, |_e| Ok(sample), |_s| Ok(Transform3::IDENTITY))
            .unwrap_or_else(|e| unreachable!("exec: {e:?}"));
        // Keep the value observed so the loop is not optimized away.
        assert!(out.translation[0].is_finite());
    }
    let after = (
        ALLOCS.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
    );
    assert_eq!(after.0 - before.0, 0, "query-path allocations");
    assert_eq!(after.1 - before.1, 0, "query-path deallocations");
    println!("spatial_query_zero_alloc: query execution allocation-free (0/0)");
}
