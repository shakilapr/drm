//! Compile-fail fixture (I-B4): the borrow guard is `!Send + !Sync`, so moving
//! or sharing it across threads does not compile.
//! REQ: REQ-BIND-016.
use drm::BorrowGuard;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

fn main() {
    assert_send::<BorrowGuard<'static>>();
    assert_sync::<BorrowGuard<'static>>();
}
