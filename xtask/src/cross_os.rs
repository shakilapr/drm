//! Cross-OS conformance subset (work-plan M12W4-E4-T1, closed out M13W1-E1/E3;
//! windows-backends.md §Test Matrix). The Linux conformance suite rows that must
//! run green on the Windows runner, or carry an explicit port note when a
//! platform fixture is still scheduled. `cargo xtask test --cross-os` executes
//! every green row on the current host — Windows-only rows are `cfg`-gated and
//! no-op elsewhere — so the same manifest defines the nightly Windows subset and
//! the local smoke, with no divergent lists. `PORT_GAPS_CLOSED` holds when no
//! row is silently missing: each is either green or scheduled with a note. The
//! M12W4 `win-race-create` and `win-init-reinit` carry-overs are green here.
//! REQ: REQ-QAL-008, REQ-CORE-101, REQ-ROUTE-025, REQ-ROUTE-026.

use anyhow::{Result, bail};

/// Returns `true` when every [`REQUIRED_INVARIANTS`] entry is exercised by at
/// least one [`SUBSET`] row.
/// REQ: REQ-QAL-008.
#[must_use]
pub fn covers_required_invariants() -> bool {
    REQUIRED_INVARIANTS
        .iter()
        .all(|required| SUBSET.iter().any(|row| row.invariants.contains(required)))
}

/// Windows execution state of one subset row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsState {
    /// The suite runs on the Windows runner and must stay green.
    Green,
    /// Port gap explicitly scheduled (carry-over); the note names the owner and
    /// week so the row can never be silently absent.
    Scheduled(&'static str),
}

/// One Linux conformance suite row that must run on Windows (or be scheduled).
#[derive(Debug, Clone, Copy)]
pub struct SubsetRow {
    /// Stable row id used in reports and `tests/conformance/README.md`.
    pub id: &'static str,
    /// Invariants the row exercises (`I-Wx`, `I-LSx`, `I-RTx`, ...).
    pub invariants: &'static [&'static str],
    /// Cargo test arguments (`package`, target or filter) run for the row.
    pub args: &'static [&'static str],
    /// Windows execution state.
    pub windows: WindowsState,
}

/// Invariants the subset must cover (windows-backends.md §Invariants).
/// REQ: REQ-CORE-101.
pub const REQUIRED_INVARIANTS: &[&str] = &[
    "I-W1", "I-W2", "I-W3", "I-W4", "I-W4b", "I-W5", "I-W6", "I-W7", "I-W8",
];

/// The Windows conformance subset: every row is green on the Windows runner or
/// scheduled with a port note (windows-backends.md §Test Matrix; work-plan
/// M12W4 E1/E3/E4).
/// REQ: REQ-QAL-008.
pub const SUBSET: &[SubsetRow] = &[
    SubsetRow {
        id: "win-shm-locks-identity",
        invariants: &["I-W1", "I-W2", "I-W3", "I-W5", "I-W6", "I-W7", "I-W8"],
        args: &["test", "-p", "drm-core", "--lib", "win_"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-cleanup",
        invariants: &["I-W4"],
        args: &["test", "-p", "drm-core", "--lib", "win_cleanup"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-lifecycle-parity",
        invariants: &["I-W4b", "I-W6", "I-W7"],
        args: &["test", "-p", "drm-core", "--test", "platform_windows"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-transport",
        invariants: &["I-RT5"],
        args: &["test", "-p", "drm-router", "--lib", "transport"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-framing",
        invariants: &["I-RT9"],
        args: &["test", "-p", "drm-router", "--test", "framing_record"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-lanes",
        invariants: &["I-LS1", "I-LS2"],
        args: &["test", "-p", "drm-router", "--test", "lane"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-shaping",
        invariants: &["I-LS3", "I-LS4"],
        args: &["test", "-p", "drm-router", "shaping"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "verify-engine",
        invariants: &["I-V1", "I-V6"],
        args: &["test", "-p", "drm-workspace", "--test", "verify_engine"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-race-create",
        invariants: &["I-W1", "I-L1"],
        args: &["test", "-p", "drm-core", "--lib", "win_race_create"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-init-reinit",
        invariants: &["I-W1", "I-L8"],
        args: &["test", "-p", "drm-core", "--lib", "win_init_reinit"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "serial-codec",
        invariants: &["I-SR1", "I-SR2", "I-SR3", "I-SR4", "I-SR6"],
        args: &["test", "-p", "drm-router", "--lib", "transport::serial"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "serial-loopback",
        invariants: &["I-SR4", "I-SR6"],
        args: &[
            "test",
            "-p",
            "drm-router",
            "--lib",
            "transport::serial::tests::serial_loopback_x10k",
        ],
        windows: WindowsState::Scheduled(
            "socketpair loopback is unix-only; Windows COM/named-pipe fixture M14",
        ),
    },
];

/// `cargo xtask test --cross-os`: runs every green subset row on the current
/// host and reports the scheduled rows; fails on the first failing row.
/// REQ: REQ-QAL-008.
///
/// # Errors
/// [`anyhow::Error`] when a green row's cargo test invocation exits nonzero.
pub fn run() -> Result<()> {
    if !covers_required_invariants() {
        bail!("cross-os subset does not cover every required I-W invariant");
    }
    let mut green = 0_usize;
    let mut scheduled = 0_usize;
    for row in SUBSET {
        match row.windows {
            WindowsState::Green => {
                crate::spawn("cargo", row.args)
                    .map_err(|e| e.context(format!("cross-os row `{}`", row.id)))?;
                green += 1;
            }
            WindowsState::Scheduled(note) => {
                eprintln!("cross-os: schedule `{}` ({note})", row.id);
                scheduled += 1;
            }
        }
    }
    println!("cross-os subset: {green} green row(s), {scheduled} scheduled");
    Ok(())
}

/// REQ: REQ-QAL-008 — the subset covers every required `I-W` invariant, so a
/// green Windows run proves the windows-backends.md §Invariants set.
#[cfg(test)]
mod tests {
    use super::{REQUIRED_INVARIANTS, SUBSET, WindowsState};

    /// REQ: REQ-QAL-008.
    #[test]
    fn subset_covers_required_invariants() {
        for required in REQUIRED_INVARIANTS {
            assert!(
                SUBSET.iter().any(|r| r.invariants.contains(required)),
                "subset misses invariant {required}"
            );
        }
    }

    /// REQ: REQ-QAL-008 — no port gap is silently missing: scheduled rows carry
    /// a note and green rows carry a cargo invocation.
    #[test]
    fn every_row_is_classified() {
        for row in SUBSET {
            assert!(!row.id.is_empty(), "row id must be non-empty");
            match row.windows {
                WindowsState::Green => assert!(
                    !row.args.is_empty(),
                    "green row `{}` needs a cargo invocation",
                    row.id
                ),
                WindowsState::Scheduled(note) => assert!(
                    !note.is_empty(),
                    "scheduled row `{}` needs a port note",
                    row.id
                ),
            }
        }
    }
}
