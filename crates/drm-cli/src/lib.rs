//! DRM CLI facade (codebase.md 2.1 layer `facade`): `drm` executable plus the
//! `drm studio --stdio` host; composition of ops crates happens only here.
//! REQ: REQ-CORE-080 (drm clean unlinks only when the exclusive deployment active
//! lock proves no nodes), REQ-STUDIO-003 (every mutation executed and validated by
//! the Rust engine).
//!
//! Implemented slice: `drm ids` (committed assignment-lockfile report +
//! explicit compaction), `drm assign` (manifest mutation + verify), `drm build`
//! (orchestrator, REQ-TOOL-012), `drm info` (read-only model projection,
//! REQ-TOOL-045), `drm dev` (watch skeleton, REQ-TOOL-016), and the read-only
//! introspection tools `drm echo`/`hz`/`log` (observability-plane I-OB5,
//! REQ-OPS-003). Argument parsing is hand-written over
//! [`std::env::args`](std::env::args) — no parser dependency.
//! `#![forbid(unsafe_code)]` (facade).

#![forbid(unsafe_code)]

pub mod args;
pub mod assign;
pub mod attach;
pub mod build;
pub mod dev;
pub mod echo;
pub mod error;
pub mod hz;
pub mod ids;
pub mod info;
pub mod log;
pub mod ws;

pub use args::{Args, Command, Direction, IdsAction};
pub use error::CliError;

/// Dispatch a raw argument vector (everything after the program name).
/// REQ: REQ-TOOL-038 (compaction is reachable only through the explicit
/// `drm ids compact` path), REQ-TOOL-080 (lockfile tooling).
///
/// # Errors
/// [`CliError::Usage`] on a malformed invocation; any command-specific failure
/// (IO, lockfile, refusal, verify) otherwise.
pub fn run(raw: &[String]) -> Result<(), CliError> {
    let args = args::parse(raw).map_err(CliError::Usage)?;
    match args.command {
        Command::Help => {
            print!("{}", args::HELP);
            Ok(())
        }
        Command::Ids(action) => ids::run(&args.workspace, action),
        Command::Assign {
            topic,
            direction,
            node,
        } => assign::run(&args.workspace, &topic, direction, &node),
        Command::Build {
            targets,
            workspace: path,
        } => {
            let root = path.map_or_else(|| args.workspace.clone(), std::path::PathBuf::from);
            build::run(&root, &targets)
        }
        Command::Info => info::run(&args.workspace),
        Command::Dev { once } => dev::run(&args.workspace, once),
        Command::Echo { topic, raw } => echo::run(&args.workspace, &topic, raw),
        Command::Hz { topic } => hz::run(&args.workspace, &topic),
        Command::Log { topic, level } => log::run(&args.workspace, topic.as_deref(), &level),
    }
}

#[cfg(test)]
mod tests {
    /// REQ: REQ-TOOL-038 — the `ids compact` path without `--yes` is refused at
    /// the dispatch boundary (no workspace needed; refusal precedes IO).
    #[test]
    fn compact_without_yes_refused() {
        let raw = vec![
            "--workspace".to_string(),
            "does-not-exist".to_string(),
            "ids".to_string(),
            "compact".to_string(),
        ];
        match super::run(&raw) {
            Err(super::CliError::Refused(_)) => {}
            other => unreachable!("expected refusal, got {other:?}"),
        }
    }
}
