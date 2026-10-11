//! `drm log [<topic>] [--level <level>]` — read-only application log viewer
//! over the compiled trace ring (observability-plane.md §Protocols compiled
//! trace pipeline, REQ-OPS-014). Renders numeric ids through the build-time
//! dictionary and filters by severity.
//! REQ: REQ-OPS-003, REQ-OPS-014.

use std::path::Path;

use drm_core::trace::{TraceDict, TraceEvent};
use drm_diag::{LogCatalog, LogLevel};

use crate::attach;
use crate::error::CliError;

/// Parses a CLI severity token.
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::Usage`] for an unknown level token.
pub fn parse_level(raw: &str) -> Result<LogLevel, CliError> {
    match raw.to_ascii_lowercase().as_str() {
        "error" => Ok(LogLevel::Error),
        "warn" | "warning" => Ok(LogLevel::Warn),
        "info" => Ok(LogLevel::Info),
        "debug" => Ok(LogLevel::Debug),
        "trace" => Ok(LogLevel::Trace),
        other => Err(CliError::Usage(format!("unknown log level `{other}`"))),
    }
}

/// Renders the events at or above `min`, newest-last, through `dict`.
/// REQ: REQ-OPS-003, REQ-OPS-014.
#[must_use]
pub fn render_log(
    events: &[TraceEvent],
    dict: &TraceDict,
    catalog: &LogCatalog,
    min: LogLevel,
) -> Vec<String> {
    catalog
        .filter(events, min)
        .into_iter()
        .map(|e| dict.render(e))
        .collect()
}

/// Resolves the topic, attaches read-only, and prints the filtered log.
/// REQ: REQ-OPS-003.
///
/// # Errors
/// [`CliError::NotFound`] when the deployment/topic/live channel is absent;
/// [`CliError::Usage`] for an unknown level token.
pub fn run(workspace: &Path, topic: Option<&str>, level: &str) -> Result<(), CliError> {
    let _min = parse_level(level)?;
    if let Some(t) = topic {
        let _handle = attach::topic_handle(workspace, t)?;
        let _ch = attach::open_channel(workspace, t)?;
    }
    println!("log: no live trace session attached (Phase 32 diag session)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_level, render_log};
    use drm_core::trace::{TraceDict, TraceEvent};
    use drm_diag::{LogCatalog, LogLevel};

    fn event(id: u16, word: u32) -> TraceEvent {
        let mut args = [0_u8; 16];
        args[0..4].copy_from_slice(&word.to_le_bytes());
        TraceEvent {
            id,
            pad: 0,
            args,
            ts_ns: u64::from(id),
        }
    }

    /// REQ: REQ-OPS-003 — level tokens parse case-insensitively; unknown tokens
    /// are usage errors.
    #[test]
    fn parse_levels() {
        assert_eq!(
            parse_level("ERROR").unwrap_or_else(|_| unreachable!()),
            LogLevel::Error
        );
        assert_eq!(
            parse_level("warn").unwrap_or_else(|_| unreachable!()),
            LogLevel::Warn
        );
        assert_eq!(
            parse_level("trace").unwrap_or_else(|_| unreachable!()),
            LogLevel::Trace
        );
        assert!(matches!(
            parse_level("loud"),
            Err(super::CliError::Usage(_))
        ));
    }

    /// REQ: REQ-OPS-003, REQ-OPS-014 — rendering resolves ids through the
    /// dictionary and filters below the minimum severity.
    #[test]
    fn render_and_filter() {
        let mut dict = TraceDict::new();
        let e = dict.intern("boot", "phase={}");
        let i = dict.intern("tick", "n={}");
        let mut catalog = LogCatalog::new();
        catalog.set(e, LogLevel::Error);
        catalog.set(i, LogLevel::Debug);

        let events = vec![event(e, 1), event(i, 7)];
        let errors = render_log(&events, &dict, &catalog, LogLevel::Error);
        assert_eq!(errors, vec!["boot: phase=1".to_string()]);
        let all = render_log(&events, &dict, &catalog, LogLevel::Trace);
        assert_eq!(
            all,
            vec!["boot: phase=1".to_string(), "tick: n=7".to_string()]
        );
    }
}
