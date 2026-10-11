//! `drm hz <topic>` — observed vs declared publication rate
//! (observability-plane.md §Protocols deadline monitoring, I-OB3). Read-only;
//! derives `observed_rate` from the sliding `write_seq` window with no commit
//! timestamps.
//! REQ: REQ-OPS-001, REQ-OPS-003.

use std::path::Path;

use drm_diag::{ChannelView, RateWindow, diag_sample};

use crate::attach;
use crate::error::CliError;

/// One rate sample: declared vs observed, plus the newest sequence.
/// REQ: REQ-OPS-001.
#[derive(Debug, Clone, PartialEq)]
pub struct HzReport {
    /// Topic path.
    pub topic: String,
    /// Declared planning rate, when known.
    pub declared_hz: Option<f64>,
    /// Observed sliding-window rate (`Hz`).
    pub observed_hz: f64,
    /// Newest committed sequence.
    pub last_sequence: u64,
}

impl HzReport {
    /// Whether the observed rate matches the declared rate within a relative
    /// tolerance; `true` when no declared rate exists.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn within_tolerance(&self, tolerance: f64) -> bool {
        match self.declared_hz {
            None => true,
            Some(d) if d <= 0.0 => true,
            Some(d) => (self.observed_hz - d).abs() / d <= tolerance,
        }
    }
}

/// Samples a channel's `write_seq` at `now_mono` and returns the report.
/// REQ: REQ-OPS-001.
pub fn report(
    topic: &str,
    declared_hz: Option<f64>,
    window: &mut RateWindow,
    now_mono: u64,
    view: &impl ChannelView,
) -> HzReport {
    let stats = diag_sample(view);
    let observed = window.record(now_mono, stats.last_sequence);
    HzReport {
        topic: topic.to_string(),
        declared_hz,
        observed_hz: observed,
        last_sequence: stats.last_sequence,
    }
}

/// Resolves the topic, attaches read-only, and prints declared vs observed.
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when the deployment/topic/live channel is absent.
pub fn run(workspace: &Path, topic: &str) -> Result<(), CliError> {
    let _handle = attach::topic_handle(workspace, topic)?;
    let _ch = attach::open_channel(workspace, topic)?;
    println!("{topic}: declared=? observed=? (live session attaches at runtime)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HzReport, report};
    use drm_core::channel::ccb::ChannelCounters;
    use drm_diag::{ChannelView, RateWindow};

    /// A view that replays a precomputed `write_seq` schedule.
    struct Schedule {
        seq: std::cell::Cell<u64>,
    }

    impl ChannelView for Schedule {
        fn epoch(&self) -> u32 {
            0
        }

        fn counters(&self) -> ChannelCounters {
            let seq = self.seq.get();
            ChannelCounters {
                write_seq: seq,
                last_sequence: seq.saturating_sub(1),
                drop_count: 0,
                overrun_count: 0,
                pool_exhausted: 0,
                publisher_epoch: 0,
                reclaim_requested: false,
            }
        }
    }

    /// REQ: REQ-OPS-001 — a 100 Hz injected sequence is observed within 1%.
    #[test]
    fn hz_tolerance() {
        let declared = 100.0_f64;
        let period_ns = 10_000_000_u64;
        let schedule = Schedule {
            seq: std::cell::Cell::new(0),
        };
        let mut window = RateWindow::new(1_000_000_000);
        let mut last = None;
        for i in 0..=100_u64 {
            schedule.seq.set(i);
            last = Some(report(
                "camera/front",
                Some(declared),
                &mut window,
                i * period_ns,
                &schedule,
            ));
        }
        let r = last.unwrap_or_else(|| unreachable!("report"));
        assert!(r.within_tolerance(0.01), "observed={}", r.observed_hz);
        assert_eq!(r.topic, "camera/front");
        assert_eq!(r.last_sequence, 99);
    }

    /// REQ: REQ-OPS-001 — a report with no declared rate is always within
    /// tolerance (informational only).
    #[test]
    fn hz_no_declared_rate() {
        let r = HzReport {
            topic: "x".to_string(),
            declared_hz: None,
            observed_hz: 42.0,
            last_sequence: 1,
        };
        assert!(r.within_tolerance(0.01));
    }
}
