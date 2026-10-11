//! Process-wide test serialization for OS-global state.
//!
//! A `fork()` child inherits the parent's open file descriptions
//! (`O_CLOEXEC` covers only `exec`), so a forked child that parks while
//! holding its resources also keeps a concurrently-running peer test's
//! `flock` alive. Tests that fork or spawn a process
//! (`channel::lifecycle::tests::posix`, `shm::process::tests`,
//! `shm::windows::tests`) and tests that assert on lock acquisition
//! (`shm::lock::tests`, `shm::windows` `LockFileEx`) therefore take this
//! guard so the two never overlap in one test process. The guard is a plain
//! mutex, so it is compiled on every test host (only the tests that need it
//! are platform-gated).
//!
//! REQ: REQ-CORE-073 (lock lifecycle binds to the open file description),
//! REQ-CORE-054 (fork-sanitized authority).

use std::sync::{Mutex, MutexGuard, PoisonError};

fn slot() -> &'static Mutex<()> {
    static GUARD: Mutex<()> = Mutex::new(());
    &GUARD
}

/// Acquire the shared fork/`flock` test guard, tolerating a poisoned lock
/// (a peer test that panicked still releases its hold).
pub(crate) fn acquire() -> MutexGuard<'static, ()> {
    slot().lock().unwrap_or_else(PoisonError::into_inner)
}
