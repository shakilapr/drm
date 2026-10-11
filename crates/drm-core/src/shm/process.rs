//! Process identity diagnostics, verbatim from
//! `docs/algorithms/runtime/shm-backend.md` §Functions (`current_identity`) and
//! §Protocols (consumer-registration identity). DIAGNOSTIC ONLY: reclamation
//! authority is the registration lock; identity never gates a reclaim decision
//! (`docs/algorithms/runtime/exhaustion-reclamation.md`).
//! REQ: REQ-CORE-036.

use super::ProcessIdentity;

/// Reads this process's `(pid, start_ticks)` pair. The start time disambiguates
/// PID reuse within one boot on diagnostic surfaces; an unreadable `/proc` yields
/// start ticks `0` rather than a fabricated value.
/// REQ: REQ-CORE-036.
#[must_use]
pub fn current_identity() -> ProcessIdentity {
    let pid = {
        // SAFETY: `getpid` has no preconditions and cannot fail.
        unsafe { libc::getpid() }
    };
    let raw = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let start_ticks = parse_stat_starttime(&raw).unwrap_or(0);
    ProcessIdentity { pid, start_ticks }
}

/// Extracts field 22 (`starttime`) from a `/proc/<pid>/stat` line. Defensive per
/// proc(5): field 2 (`comm`) may contain spaces and parentheses, so the token
/// scan starts after the LAST `)` on the line.
/// REQ: REQ-CORE-036.
#[must_use]
pub(crate) fn parse_stat_starttime(raw: &str) -> Option<u64> {
    let after_comm = raw.rfind(')')? + 1;
    // Fields 1 (`pid`) and 2 (`comm`) precede the closing `)`; the first token
    // after it is field 3, so field 22 is token `22 - 3`.
    raw[after_comm..]
        .split_ascii_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// `kill(pid, 0)` existence probe per the work-plan pseudocode: `ESRCH` ⇒ false;
/// success or `EPERM` ⇒ true (the process exists even when owned by another
/// user). Non-positive `pid` would address a process group and reports false.
/// Diagnostic only — never consulted by reclamation.
/// REQ: REQ-CORE-036.
#[must_use]
pub fn is_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 performs no delivery; the call is a pure existence probe.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Incarnation-safe liveness proof for a recorded [`ProcessIdentity`] snapshot:
/// true iff `pid` is alive AND its current start ticks equal the recorded
/// `start_ticks`. A PID reused by a newer process fails the start-ticks guard,
/// so the recorded (older) incarnation is reported dead; a genuinely live
/// same-incarnation owner is reported alive. An unreadable `/proc/<pid>/stat`
/// is treated as alive so a live reader is never mis-reclaimed (I-E5).
/// REQ: REQ-CORE-008, REQ-CORE-057.
#[must_use]
pub fn identity_alive(pid: i32, start_ticks: u64) -> bool {
    if !is_alive(pid) {
        return false;
    }
    match read_start_ticks(pid) {
        Some(current) => current == start_ticks,
        None => true,
    }
}

/// Field 22 (`starttime`) of `/proc/<pid>/stat`, or `None` when unreadable.
/// REQ: REQ-CORE-036.
fn read_start_ticks(pid: i32) -> Option<u64> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat_starttime(&raw)
}

#[cfg(test)]
mod tests {
    use super::{current_identity, identity_alive, is_alive, parse_stat_starttime};

    /// REQ: REQ-CORE-036 — field 22 lands on the 19th token after `comm`.
    #[test]
    fn stat_parse_field22() {
        let mut line = String::from("42 (drm-node)");
        for field in 3..=21 {
            line.push(' ');
            line.push_str(&field.to_string());
        }
        line.push_str(" 123456789");
        line.push_str(" 0 0 0 0 0 0 0");
        assert_eq!(parse_stat_starttime(&line), Some(1_2345_6789));
    }

    /// REQ: REQ-CORE-036 — defensive parse: `comm` containing spaces and
    /// parentheses must not shift the field index (proc(5) anomaly).
    #[test]
    fn stat_parse_hostile_comm() {
        let mut line = String::from("7 ((we)rd (name))");
        for field in 3..=21 {
            line.push(' ');
            line.push_str(&field.to_string());
        }
        line.push_str(" 987654321");
        assert_eq!(parse_stat_starttime(&line), Some(987_654_321));
    }

    /// REQ: REQ-CORE-036 — malformed fixtures parse to `None`, never a number.
    #[test]
    fn stat_parse_malformed() {
        assert_eq!(parse_stat_starttime("no closing paren"), None);
        assert_eq!(parse_stat_starttime("1 (x)"), None);
        assert_eq!(parse_stat_starttime("1 (x) S a b c"), None);
        let mut short = String::from("1 (x) S");
        for _ in 0..19 {
            short.push_str(" 2");
        }
        assert_eq!(parse_stat_starttime(&short), Some(2));
        let mut nonnum = String::from("1 (x) S");
        for _ in 0..18 {
            nonnum.push_str(" 2");
        }
        nonnum.push_str(" zz");
        assert_eq!(parse_stat_starttime(&nonnum), None);
    }

    /// REQ: REQ-CORE-036 — identity stable across reads; pid positive.
    #[test]
    fn identity_stable() {
        let a = current_identity();
        let b = current_identity();
        assert_eq!(a, b);
        assert!(a.pid > 0);
    }

    /// REQ: REQ-CORE-036 — live probe: self alive, group addresses false,
    /// reaped child false (dead-child; PID reuse within one tick is the
    /// documented diagnostics-only limitation).
    #[test]
    fn alive_probe() {
        let _serial = crate::test_guard::acquire();
        let me = current_identity().pid;
        assert!(is_alive(me));
        assert!(!is_alive(0));
        assert!(!is_alive(-1));

        let mut child = match std::process::Command::new("true").spawn() {
            Ok(c) => c,
            Err(e) => unreachable!("spawn true failed: {e:?}"),
        };
        let pid = i32::try_from(child.id()).unwrap_or(0);
        match child.wait() {
            Ok(_) => {}
            Err(e) => unreachable!("wait failed: {e:?}"),
        }
        assert!(!is_alive(pid));
    }

    /// REQ: REQ-CORE-008, REQ-CORE-057 — the start-ticks guard makes the proof
    /// incarnation-safe: the live self-identity is reported alive, a
    /// same-PID/different-start snapshot (PID reuse) is reported dead, and a
    /// non-positive/absent pid is always dead.
    #[test]
    fn identity_alive_incarnation() {
        let me = current_identity();
        assert!(identity_alive(me.pid, me.start_ticks), "live self is alive");
        assert!(
            !identity_alive(me.pid, me.start_ticks.wrapping_add(1)),
            "a reused PID is a different incarnation"
        );
        assert!(!identity_alive(0, 0));
        assert!(!identity_alive(-1, 0));
    }
}
