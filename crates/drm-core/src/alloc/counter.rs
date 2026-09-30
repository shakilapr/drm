//! Counting allocator harness (canon-codec I4; work-plan M01W4-E1-T2).
//! Enabled only with the `test-count-allocs` feature on a test build: the
//! `GlobalAlloc` wrapper totals every allocator entry and `tests/alloc_counter.rs`
//! (a `harness = false` binary) asserts the codec block is allocation-free.
//! REQ: REQ-CORE-082.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

/// Total allocator entry events (alloc, realloc, alloc_zeroed).
pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Total deallocation events.
pub static DEALLOCS: AtomicU64 = AtomicU64::new(0);

/// Wrapper over `System` counting every allocator entry.
pub struct CountingAlloc;

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

/// Installs the counting allocator when the feature is on.
/// REQ: REQ-CORE-082.
#[cfg(feature = "test-count-allocs")]
#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;
