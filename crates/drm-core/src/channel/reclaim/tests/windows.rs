//! Real-Windows reader-kill reclamation matrix (Windows CI). The in-process
//! forcing suite in the parent module proves the protocol against synthetic
//! identities; this proves the reclamation authority against real Win32
//! process identity (`windows-backends.md` I-W5/I-W6: signaled process +
//! creation time) — a killed reader's abandoned hazard word is reclaimed, a
//! live reader's is never mis-reclaimed (I-E5), and a reused PID is a
//! different, dead incarnation.
//! REQ: REQ-CORE-057, REQ-CORE-056, REQ-CORE-101.

use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

use super::{ch, freed, hold, pat, word};
use crate::hazard::{HAZARD_NONE, pack};
use crate::shm::{ProcessIdentity, current_identity, identity_alive};

/// Unwrap-or-abort without `unwrap`/`expect` (workspace deny).
/// REQ: REQ-CORE-101.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("reclaim windows: {what}: {e:?}"),
    }
}

/// Records `id` on record `i` (test instrumentation).
/// REQ: REQ-CORE-057.
fn set_identity(c: &crate::channel::Channel, i: usize, id: ProcessIdentity) {
    c.rec(i)
        .unwrap_or_else(|| unreachable!("rec{i}"))
        .record_identity(id);
}

/// Records a short-lived child's real Windows identity so the matrix can prove
/// a dead incarnation; reuses the platform identity child (`shm::windows`),
/// which writes `pid start_ticks` then exits.
/// REQ: REQ-CORE-057, REQ-CORE-101.
fn dead_identity() -> ProcessIdentity {
    let out = std::env::temp_dir().join(format!("drm-reclaim-id-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exe = ok(std::env::current_exe(), "current_exe");
    let mut child = ok(
        Command::new(exe)
            .args([
                "--exact",
                "shm::windows::tests::win_identity_child",
                "--nocapture",
            ])
            .env("DRM_TEST_IDENTITY_OUT", &out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        "spawn identity child",
    );
    ok(child.wait(), "reap identity child");
    let text = ok(std::fs::read_to_string(&out), "identity file");
    let _ = std::fs::remove_file(&out);
    let mut parts = text.split_ascii_whitespace();
    let pid = parts
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or_else(|| unreachable!("child pid in `{text}`"));
    let start_ticks = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| unreachable!("child start in `{text}`"));
    ProcessIdentity { pid, start_ticks }
}

/// REQ: REQ-CORE-057, REQ-CORE-056, REQ-CORE-101 — I-E5 Windows matrix: kill a
/// reader at every borrow stage and prove reclamation only under lock authority
/// with the real Win32 identity. A live reader (this process) is never
/// reclaimed; a killed reader's abandoned word is; a reused PID (same pid,
/// different creation time) is a different, dead incarnation.
#[test]
fn reclaim_kill_matrix_windows() {
    let _serial = crate::test_guard::acquire();
    let dead = dead_identity();
    assert!(
        !identity_alive(dead.pid, dead.start_ticks),
        "killed child incarnation is dead"
    );

    let mut cells = 0_u32;

    // Stage 1 — pre-intent: registered, no borrow started (word NONE).
    {
        let c = ch(1, 3);
        ok(c.publish(&pat(1)), "publish");
        set_identity(&c, 0, dead);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 0, "pre-intent: nothing protected");
        cells += 1;
    }

    // Stage 2 — post-intent: a killed reader's abandoned word is reclaimed.
    {
        let c = ch(1, 3);
        ok(c.publish(&pat(1)), "publish");
        set_identity(&c, 0, dead);
        let g = hold(&c, 0, 1);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 1, "killed reader reclaimed");
        assert!(freed(&c, 0, g));
        cells += 1;
    }

    // Stage 3 — mid-borrow: the live reader survives, the killed reader is freed.
    {
        let c = ch(1, 3);
        ok(c.publish(&pat(1)), "publish");
        set_identity(&c, 0, current_identity());
        set_identity(&c, 1, dead);
        let g0 = hold(&c, 0, 1);
        let g1 = hold(&c, 1, 2);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 1, "only the killed reader reclaimed");
        assert_eq!(word(&c, 0), (g0, 1), "live reader never mis-reclaimed");
        assert!(freed(&c, 1, g1));
        cells += 1;
    }

    // Stage 4 — post-release: the killed reader released before dying.
    {
        let c = ch(1, 3);
        ok(c.publish(&pat(1)), "publish");
        set_identity(&c, 0, dead);
        let g = hold(&c, 0, 1);
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec0"));
        assert!(
            rec.hazard
                .compare_exchange(pack(g, 1), pack(g, HAZARD_NONE))
        );
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 0, "post-release: nothing protected");
        cells += 1;
    }

    // Stage 5 — lock held: the registration lock outlives the killed reader's
    // hazard word until released, so reclamation waits, then succeeds.
    {
        let c = ch(2, 3);
        ok(c.publish(&pat(1)), "publish");
        ok(c.publish(&pat(2)), "publish");
        set_identity(&c, 0, dead);
        let g = hold(&c, 0, 2);
        let lock = c
            .rec(0)
            .unwrap_or_else(|| unreachable!("rec0"))
            .try_reg_lock()
            .unwrap_or_else(|| unreachable!("lock free"));
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 0, "lock held => live authority => skip");
        assert_eq!(word(&c, 0), (g, 2), "word intact while locked");
        drop(lock);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 1, "reclaimed once the lock is free");
        assert!(freed(&c, 0, g));
        cells += 1;
    }

    // PID reuse: same pid as a live process, different creation time ⇒ dead.
    {
        let me = current_identity();
        let c = ch(1, 3);
        ok(c.publish(&pat(1)), "publish");
        let reused = ProcessIdentity {
            pid: me.pid,
            start_ticks: me.start_ticks.wrapping_add(1),
        };
        set_identity(&c, 0, reused);
        let g = hold(&c, 0, 1);
        c.ccb().reclaim_requested.store(true, Ordering::Release);
        assert_eq!(c.try_reclaim(), 1, "reused PID is a different incarnation");
        assert!(freed(&c, 0, g));
        cells += 1;
    }

    assert_eq!(cells, 6, "Windows kill-matrix cell count");
}
