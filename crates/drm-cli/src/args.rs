//! Hand-written argument parsing over `std::env::args` — no parser
//! dependency (facade minimalism). Recognized shapes:
//!
//! ```text
//! drm [--workspace <dir>] ids
//! drm [--workspace <dir>] ids compact [--yes]
//! drm [--workspace <dir>] assign <topic> (--consume <node> | --publish <node>)
//! drm [--workspace <dir>] help
//! ```
//!
//! REQ: REQ-TOOL-080 (deterministic tooling), REQ-TOOL-038 (compaction is an
//! explicit acknowledged action).

use std::path::PathBuf;

/// Direction of an `drm assign` manifest mutation.
/// REQ: REQ-TOOL-037 (identity is the canonical topic path, never the handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Add the topic to the node's `consumes` list.
    Consume,
    /// Add the topic to the node's `publishes` list.
    Publish,
}

/// Which `drm ids` operation to perform.
/// REQ: REQ-TOOL-038.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdsAction {
    /// Print the committed lockfile contents (read-only).
    Report,
    /// Re-pack handles dense; `yes` mirrors the mandatory `--yes` gate (I-H4).
    Compact {
        /// Whether the operator acknowledged with `--yes`.
        yes: bool,
    },
}

/// The parsed command to execute.
/// REQ: REQ-TOOL-037.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `drm ids ...`.
    Ids(IdsAction),
    /// `drm assign <topic> (--consume|--publish) <node>`.
    Assign {
        /// Canonical topic path to assign.
        topic: String,
        /// Which manifest list to mutate.
        direction: Direction,
        /// Target node identity.
        node: String,
    },
    /// `drm help` / `-h` / `--help` / no arguments.
    Help,
}

/// A fully parsed invocation.
/// REQ: REQ-TOOL-037.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Workspace root (defaults to the current directory).
    pub workspace: PathBuf,
    /// Command to run.
    pub command: Command,
}

/// Usage text printed for `drm help`. REQ: REQ-TOOL-080.
pub const HELP: &str = "\
drm — DRM build-time engine (facade).

Usage:
  drm [--workspace <dir>] ids               Report handle assignments (.drm/ids.json)
  drm [--workspace <dir>] ids compact --yes Re-pack handles dense (explicit; I-H4)
  drm [--workspace <dir>] assign <topic> --consume <node>
  drm [--workspace <dir>] assign <topic> --publish <node>
  drm [--workspace <dir>] help

Options:
  --workspace, -C <dir>   Workspace root (default: current directory)
  --yes                   Acknowledge handle compaction (required)
  -h, --help              Print this help
";

/// Parses a raw argument vector (program name already stripped).
/// REQ: REQ-TOOL-038.
///
/// # Errors
/// A human-readable message when an option lacks its value, an option is
/// unknown, or the positional shape is not one of the recognized commands.
pub fn parse(raw: &[String]) -> Result<Args, String> {
    let mut workspace = PathBuf::from(".");
    let mut positionals: Vec<String> = Vec::new();
    let mut yes = false;
    let mut consume: Option<String> = None;
    let mut publish: Option<String> = None;

    let mut i = 0;
    while i < raw.len() {
        let arg = raw[i].as_str();
        match arg {
            "--workspace" | "-C" => {
                let v = raw
                    .get(i + 1)
                    .ok_or_else(|| format!("`{arg}` requires a path"))?;
                workspace = PathBuf::from(v);
                i += 2;
            }
            "--consume" => {
                let v = raw
                    .get(i + 1)
                    .ok_or_else(|| "`--consume` requires a node".to_string())?;
                consume = Some(v.clone());
                i += 2;
            }
            "--publish" => {
                let v = raw
                    .get(i + 1)
                    .ok_or_else(|| "`--publish` requires a node".to_string())?;
                publish = Some(v.clone());
                i += 2;
            }
            "--yes" => {
                yes = true;
                i += 1;
            }
            "--help" | "-h" => {
                return Ok(Args {
                    workspace,
                    command: Command::Help,
                });
            }
            opt if opt.starts_with('-') => return Err(format!("unknown option `{opt}`")),
            _ => {
                positionals.push(raw[i].clone());
                i += 1;
            }
        }
    }

    let command = build_command(&positionals, yes, consume, publish)?;
    Ok(Args { workspace, command })
}

/// Maps positionals + collected flags into a [`Command`].
/// REQ: REQ-TOOL-037, REQ-TOOL-038.
fn build_command(
    positionals: &[String],
    yes: bool,
    consume: Option<String>,
    publish: Option<String>,
) -> Result<Command, String> {
    match positionals.first().map(String::as_str) {
        Some("ids") => match positionals.get(1).map(String::as_str) {
            None => Ok(Command::Ids(IdsAction::Report)),
            Some("compact") => Ok(Command::Ids(IdsAction::Compact { yes })),
            Some(other) => Err(format!("unknown `ids` subcommand `{other}`")),
        },
        Some("assign") => {
            let topic = positionals
                .get(1)
                .cloned()
                .ok_or_else(|| "`assign` requires a <topic>".to_string())?;
            let (direction, node) = match (consume, publish) {
                (Some(n), None) => (Direction::Consume, n),
                (None, Some(n)) => (Direction::Publish, n),
                _ => {
                    return Err(
                        "`assign` requires exactly one of --consume <node> / --publish <node>"
                            .to_string(),
                    );
                }
            };
            Ok(Command::Assign {
                topic,
                direction,
                node,
            })
        }
        Some("help") | None => Ok(Command::Help),
        Some(other) => Err(format!("unknown command `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    /// REQ: REQ-TOOL-080 — a bare `ids` invocation reports (read-only).
    #[test]
    fn ids_report_default() {
        let a = super::parse(&["ids".to_string()]).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(a.command, super::Command::Ids(super::IdsAction::Report));
        assert_eq!(a.workspace, std::path::PathBuf::from("."));
    }

    /// REQ: REQ-TOOL-038 — `ids compact` without `--yes` parses with the gate
    /// closed; with `--yes` it is open (the refusal/acceptance decision is
    /// deferred to the command body).
    #[test]
    fn ids_compact_gate_flag() {
        let mut raw = vec!["ids".to_string(), "compact".to_string()];
        let a = super::parse(&raw).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            a.command,
            super::Command::Ids(super::IdsAction::Compact { yes: false })
        );
        raw.push("--yes".to_string());
        let a = super::parse(&raw).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            a.command,
            super::Command::Ids(super::IdsAction::Compact { yes: true })
        );
    }

    /// REQ: REQ-TOOL-037 — an `assign` requires exactly one direction flag and
    /// a topic, and the workspace flag is order-independent.
    #[test]
    fn assign_shapes() {
        let a = super::parse(&[
            "--workspace".to_string(),
            "ws".to_string(),
            "assign".to_string(),
            "control/target".to_string(),
            "--consume".to_string(),
            "controller".to_string(),
        ])
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(a.workspace, std::path::PathBuf::from("ws"));
        assert_eq!(
            a.command,
            super::Command::Assign {
                topic: "control/target".to_string(),
                direction: super::Direction::Consume,
                node: "controller".to_string(),
            }
        );
        assert!(super::parse(&["assign".to_string(), "control/target".to_string()]).is_err());
        assert!(
            super::parse(&[
                "assign".to_string(),
                "control/target".to_string(),
                "--consume".to_string(),
                "a".to_string(),
                "--publish".to_string(),
                "b".to_string(),
            ])
            .is_err()
        );
    }

    /// REQ: REQ-TOOL-080 — an unknown option is a usage error.
    #[test]
    fn unknown_option_rejected() {
        assert!(super::parse(&["--nope".to_string()]).is_err());
    }
}
