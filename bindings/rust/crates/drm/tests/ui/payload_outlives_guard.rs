//! Compile-fail fixture (I-B4): a payload slice must not outlive its guard.
//! `payload(&self)` borrows the guard, so returning the bytes out of the guard's
//! scope is rejected.
//! REQ: REQ-BIND-016.
use drm::BorrowGuard;

fn escape<'a>(guard: BorrowGuard<'a>) -> &'a [u8] {
    guard.payload()
}

fn main() {}
