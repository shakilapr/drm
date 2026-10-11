//! Compile-fail fixture (I-B4): a moved-from guard cannot be used.
//! REQ: REQ-BIND-016.
use drm::BorrowGuard;

fn use_after_move(guard: BorrowGuard<'_>) {
    let moved = guard;
    let _ = moved.payload();
    let _ = guard.payload();
}

fn main() {}
