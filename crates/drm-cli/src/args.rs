//! Hand-written argument parsing over `std::env::args` — no parser
//! dependency (facade minimalism). Recognized shapes:
//!
//! ```text
//! drm [--workspace <dir>] ids
//! drm [--workspace <dir>] ids compact [--yes]
//! drm [--workspace <dir>] assign <topic> (--consume <node> | --publish <node>)
//! drm [--workspace <dir>] echo <topic> [--raw]
//! drm [--workspace <dir>] hz <topic>
//! drm [--workspace <dir>] log [<topic>] [--level <level>]
//! drm [--workspace <dir>] help
//! ```
//!
//! REQ: REQ-TOOL-080 (deterministic tooling), REQ-TOOL-038 (compaction is an
//! explicit acknowledged action), REQ-OPS-003 (read-only introspection).

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
/// REQ: REQ-TOOL-037, REQ-TOOL-012.
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
    /// `drm build [<workspace>] [--target <node>]...` — orchestrator
    /// (REQ-TOOL-012). An optional positional workspace mirrors
    /// `drm build examples/mixed_robot`.
    Build {
        /// Optional target node subset.
        targets: Vec<String>,
        /// Optional positional workspace (overrides `--workspace`).
        workspace: Option<String>,
    },
    /// `drm info` — read-only model + verify projection (REQ-TOOL-045).
    Info,
    /// `drm dev [--once]` — watch skeleton (REQ-TOOL-016).
    Dev {
        /// Stop after the initial build (smoke mode).
        once: bool,
    },
    /// `drm echo <topic> [--raw]` — read-only momentary borrow+copy observer
    /// (observability-plane.md I-OB5, REQ-OPS-003).
    Echo {
        /// Canonical topic path.
        topic: String,
        /// Emit raw hex instead of the human rendering.
        raw: bool,
    },
    /// `drm hz <topic>` — observed vs declared rate (REQ-OPS-001).
    Hz {
        /// Canonical topic path.
        topic: String,
    },
    /// `drm log [<topic>] [--level <level>]` — read-only application log view
    /// over the trace ring (REQ-OPS-003, REQ-OPS-014).
    Log {
        /// Optional topic filter.
        topic: Option<String>,
        /// Minimum severity to display.
        level: String,
    },
    /// `drm studio --stdio [--trust]` — long-lived JSON-RPC studio host.
    /// REQ: REQ-STUDIO-011.
    Studio {
        /// Whether the workspace is trusted (unlocks Build/Run/mutations).
        trusted: bool,
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
  drm [--workspace <dir>] build [--target <node>]...
  drm [--workspace <dir>] info
  drm [--workspace <dir>] dev [--once]
  drm [--workspace <dir>] echo <topic> [--raw]
  drm [--workspace <dir>] hz <topic>
  drm [--workspace <dir>] log [<topic>] [--level <level>]
  drm [--workspace <dir>] studio --stdio [--trust]
  drm [--workspace <dir>] help

Options:
  --workspace, -C <dir>   Workspace root (default: current directory)
  --yes                   Acknowledge handle compaction (required)
  --target <node>         Restrict `build` to a node (repeatable)
  --once                  `dev`: run the initial build only
  --raw                   `echo`: print raw hex bytes
  --level <level>         `log`: error|warn|info|debug|trace (default info)
  --stdio                 `studio`: speak JSON-RPC over stdin/stdout
  --trust                 `studio`: mark the workspace trusted (unlocks writes)
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
    let mut once = false;
    let mut targets: Vec<String> = Vec::new();
    let mut consume: Option<String> = None;
    let mut publish: Option<String> = None;
    let mut raw_hex = false;
    let mut stdio = false;
    let mut trust = false;
    let mut level = String::from("info");

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
            "--target" => {
                let v = raw
                    .get(i + 1)
                    .ok_or_else(|| "`--target` requires a node".to_string())?;
                targets.push(v.clone());
                i += 2;
            }
            "--once" => {
                once = true;
                i += 1;
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
            "--raw" => {
                raw_hex = true;
                i += 1;
            }
            "--stdio" => {
                stdio = true;
                i += 1;
            }
            "--trust" => {
                trust = true;
                i += 1;
            }
            "--level" => {
                let v = raw
                    .get(i + 1)
                    .ok_or_else(|| "`--level` requires a level".to_string())?;
                level.clone_from(v);
                i += 2;
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

    let command = build_command(
        &positionals,
        Flags {
            yes,
            once,
            raw_hex,
            stdio,
            trust,
            level,
            targets,
            consume,
            publish,
        },
    )?;
    Ok(Args { workspace, command })
}

/// Collected flag values for [`build_command`] (keeps the arity bounded).
/// REQ: REQ-TOOL-012.
// CLI flag bag: each boolean is an independent toggle.
#[allow(clippy::struct_excessive_bools)]
struct Flags {
    yes: bool,
    once: bool,
    raw_hex: bool,
    stdio: bool,
    trust: bool,
    level: String,
    targets: Vec<String>,
    consume: Option<String>,
    publish: Option<String>,
}

/// Maps positionals + collected flags into a [`Command`].
/// REQ: REQ-TOOL-037, REQ-TOOL-038, REQ-TOOL-012, REQ-OPS-003.
fn build_command(positionals: &[String], flags: Flags) -> Result<Command, String> {
    let Flags {
        yes,
        once,
        raw_hex,
        stdio,
        trust,
        level,
        targets,
        consume,
        publish,
    } = flags;
    match positionals.first().map(String::as_str) {
        Some("ids") => match positionals.get(1).map(String::as_str) {
            None => Ok(Command::Ids(IdsAction::Report)),
            Some("compact") => Ok(Command::Ids(IdsAction::Compact { yes })),
            Some(other) => Err(format!("unknown `ids` subcommand `{other}`")),
        },
        Some("build") => match positionals.len() {
            0 | 1 => Ok(Command::Build {
                targets,
                workspace: None,
            }),
            2 => Ok(Command::Build {
                targets,
                workspace: Some(positionals[1].clone()),
            }),
            _ => Err(format!("unexpected `build` argument `{}`", positionals[2])),
        },
        Some("info") => Ok(Command::Info),
        Some("dev") => Ok(Command::Dev { once }),
        Some("echo") => {
            let topic = positionals
                .get(1)
                .cloned()
                .ok_or_else(|| "`echo` requires a <topic>".to_string())?;
            Ok(Command::Echo {
                topic,
                raw: raw_hex,
            })
        }
        Some("hz") => {
            let topic = positionals
                .get(1)
                .cloned()
                .ok_or_else(|| "`hz` requires a <topic>".to_string())?;
            Ok(Command::Hz { topic })
        }
        Some("log") => Ok(Command::Log {
            topic: positionals.get(1).cloned(),
            level,
        }),
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
        Some("studio") => {
            if stdio {
                Ok(Command::Studio { trusted: trust })
            } else {
                Err("`studio` requires `--stdio` (no HTTP server, no ports)".to_string())
            }
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

    /// REQ: REQ-OPS-003 — the read-only introspection tools parse their topic,
    /// `--raw`, and `--level` shapes.
    #[test]
    fn introspection_shapes() {
        let echo = super::parse(&[
            "echo".to_string(),
            "camera/front".to_string(),
            "--raw".to_string(),
        ])
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            echo.command,
            super::Command::Echo {
                topic: "camera/front".to_string(),
                raw: true,
            }
        );
        let hz = super::parse(&["hz".to_string(), "camera/front".to_string()])
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            hz.command,
            super::Command::Hz {
                topic: "camera/front".to_string(),
            }
        );
        let log = super::parse(&[
            "log".to_string(),
            "--level".to_string(),
            "debug".to_string(),
        ])
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            log.command,
            super::Command::Log {
                topic: None,
                level: "debug".to_string(),
            }
        );
        assert!(super::parse(&["echo".to_string()]).is_err());
        assert!(super::parse(&["hz".to_string()]).is_err());
    }

    /// REQ: REQ-TOOL-012 — `build` accepts a positional workspace, `--target`
    /// repeats, `info` takes nothing, and `dev --once` sets the smoke flag.
    #[test]
    fn build_info_dev_shapes() {
        let a = super::parse(&[
            "build".to_string(),
            "examples/mixed_robot".to_string(),
            "--target".to_string(),
            "controller".to_string(),
        ])
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            a.command,
            super::Command::Build {
                targets: vec!["controller".to_string()],
                workspace: Some("examples/mixed_robot".to_string()),
            }
        );
        assert_eq!(
            super::parse(&["info".to_string()])
                .unwrap_or_else(|e| unreachable!("{e}"))
                .command,
            super::Command::Info
        );
        assert_eq!(
            super::parse(&["dev".to_string(), "--once".to_string()])
                .unwrap_or_else(|e| unreachable!("{e}"))
                .command,
            super::Command::Dev { once: true }
        );
    }
}
