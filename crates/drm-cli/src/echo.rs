//! `drm echo <topic>` — read-only single-sample observer
//! (`docs/architecture.md` §Tooling and Introspection; observability-plane.md
//! I-OB5). Momentary borrow → copy → release within the call; the tool never
//! publishes and holds no hazard slot between calls.
//! REQ: REQ-OPS-003.

use std::path::Path;

use drm_core::channel::{Channel, ReadOutcome};
use drm_diag::Observer;

use crate::attach;
use crate::error::CliError;

/// Private copy buffer cap for one momentary introspection read (`I-OB5`); the
/// effective length is the channel's declared payload bound.
const ECHO_BUF_CAP: usize = 1 << 20;

/// Renders a copied sample: raw contiguous hex, or a human length + hex line.
/// REQ: REQ-OPS-003.
#[must_use]
pub fn render(sample: &[u8], raw: bool) -> String {
    let hex = sample
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if raw {
        hex
    } else {
        format!("{} bytes: {hex}", sample.len())
    }
}

/// Copies and renders the newest sample of `ch` through a diagnostic consumer
/// record; the borrow is released before returning (`I-OB5`).
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when no sample is published;
/// [`CliError::Io`] on a transient or channel failure.
pub fn echo_once(ch: &Channel, rec_index: usize, raw: bool) -> Result<String, CliError> {
    let bound = usize::try_from(ch.sample_max())
        .unwrap_or(ECHO_BUF_CAP)
        .min(ECHO_BUF_CAP);
    let mut obs = Observer::attach(ch, rec_index, bound);
    match obs.sample_latest(ch) {
        Ok(ReadOutcome::Sample) => Ok(render(obs.sample(), raw)),
        Ok(ReadOutcome::Empty) => Err(CliError::NotFound(
            "topic has no published sample".to_string(),
        )),
        Ok(_) => Err(CliError::Io("echo: transient read outcome".to_string())),
        Err(e) => Err(CliError::Io(format!("echo: {e:?}"))),
    }
}

/// Resolves the topic, attaches read-only, and prints one sample.
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when the deployment/topic/live channel is absent.
pub fn run(workspace: &Path, topic: &str, raw: bool) -> Result<(), CliError> {
    let _handle = attach::topic_handle(workspace, topic)?;
    let ch = attach::open_channel(workspace, topic)?;
    println!("{}", echo_once(&ch, 0, raw)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{echo_once, render};
    use drm_core::channel::{Channel, OverflowPolicy};

    fn ch() -> Channel {
        Channel::activate(2, 3, 16, 8, OverflowPolicy::Overwrite, 0, 2, None)
            .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
    }

    /// REQ: REQ-OPS-003 — raw and human renderings are deterministic.
    #[test]
    fn render_modes() {
        assert_eq!(render(&[0x01, 0xAB], true), "01 ab");
        assert_eq!(render(&[0x01, 0xAB], false), "2 bytes: 01 ab");
        assert_eq!(render(&[], true), "");
    }

    /// REQ: REQ-OPS-003 — `echo_once` copies the newest sample and releases the
    /// borrow; an empty topic reports `NotFound`.
    #[test]
    fn echo_copies_and_reports_empty() {
        let c = ch();
        assert!(matches!(
            echo_once(&c, 0, true),
            Err(super::CliError::NotFound(_))
        ));
        assert!(c.publish(&[0x2A; 16]).is_ok());
        let line = echo_once(&c, 0, true).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(line, "2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a 2a");
        let rec = c.rec(0).unwrap_or_else(|| unreachable!("rec"));
        assert_eq!(
            rec.hazard.load(std::sync::atomic::Ordering::SeqCst).1,
            drm_core::hazard::HAZARD_NONE
        );
    }
}
