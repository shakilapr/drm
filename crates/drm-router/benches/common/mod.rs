//! Shared benchmark harness support for the router benches: a counting
//! allocator and the zero-allocation / bounded-time invariant assertions that
//! run inside the benches. Local to each bench binary; no feature flag needed.
//!
//! REQ: REQ-QAL-012 (zero-alloc + bounded-time assertions inside benches).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Allocator entry events.
pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Deallocation events.
pub static DEALLOCS: AtomicU64 = AtomicU64::new(0);

/// `GlobalAlloc` wrapper over `System` counting every allocator entry.
pub struct CountingAlloc;

// SAFETY: delegates every method to `System`; only the counters are added.
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

/// Process-wide counting allocator for the bench binary.
#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Allocator counters at one instant.
#[derive(Debug, Clone, Copy)]
pub struct AllocSnapshot {
    /// Total allocator entries observed.
    pub allocs: u64,
    /// Total deallocations observed.
    pub deallocs: u64,
}

/// Reads the current allocator counters.
#[must_use]
pub fn snapshot() -> AllocSnapshot {
    AllocSnapshot {
        allocs: ALLOCS.load(Ordering::Relaxed),
        deallocs: DEALLOCS.load(Ordering::Relaxed),
    }
}

/// Runs `block` and asserts it performed zero allocations and zero
/// deallocations (I-P/I-4 style steady-state zero allocation).
pub fn assert_zero_alloc(label: &str, block: impl FnOnce()) {
    let before = snapshot();
    block();
    let after = snapshot();
    assert_eq!(
        after.allocs, before.allocs,
        "{label}: allocations on the data path"
    );
    assert_eq!(
        after.deallocs, before.deallocs,
        "{label}: deallocations on the data path"
    );
}

/// Runs `block` `iters` times and returns the sorted per-run latencies.
#[must_use]
pub fn measure(iters: usize, mut block: impl FnMut()) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        block();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples
}

/// Nearest-rank percentile (`pct` in 0..=100) of sorted `samples`.
#[must_use]
pub fn percentile(samples: &[Duration], pct: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let idx = (samples.len().saturating_sub(1) * pct) / 100;
    samples[idx.min(samples.len() - 1)]
}
