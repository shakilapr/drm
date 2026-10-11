//! Node manifest parsing — TOML with a closed surface (`name`, `publishes`,
//! `consumes`, optional `language`, optional `[build]`); unknown keys, empty
//! names, non-string entries, and duplicate entries are rejected
//! ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)
//! `parse_node_manifest`). REQ: REQ-TOOL-047, REQ-TOOL-013, REQ-TOOL-012.

use std::collections::BTreeSet;

use toml::Value;

use crate::orchestrator::{BuildAdapter, Language};

/// Parsed node manifest: declared identity + interface sets (sorted) plus the
/// optional language and native build adapter.
/// REQ: REQ-TOOL-047, REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeManifest {
    /// Declared node identity (unique across the tree; duplicates are a
    /// build conflict, I-D2).
    pub name: String,
    /// Canonical topic paths published.
    pub publishes: BTreeSet<String>,
    /// Canonical topic paths consumed.
    pub consumes: BTreeSet<String>,
    /// Declared implementation language (`[node] language`), when present.
    pub language: Option<Language>,
    /// Native build adapter (`[build]`), when present (REQ-TOOL-012).
    pub build: Option<BuildAdapter>,
}

/// Parses a node manifest from TOML bytes. The file stem is carried into the
/// error for location context. REQ: REQ-TOOL-047, REQ-TOOL-013, REQ-TOOL-012.
///
/// # Errors
/// A message describing the defect when the TOML is malformed, a key is
/// unknown, a value has the wrong type, or an entry list repeats a topic.
pub fn parse_node_manifest(stem: &str, src: &[u8]) -> Result<NodeManifest, String> {
    let table: toml::Table =
        toml::from_slice(src).map_err(|e| format!("node manifest `{stem}`: {e}"))?;
    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "name" | "publishes" | "consumes" | "language" | "build"
        ) {
            return Err(format!("node manifest `{stem}`: unknown key `{key}`"));
        }
    }
    let name = table
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("node manifest `{stem}`: `name` must be a non-empty string"))?
        .to_string();
    let publishes = topic_list(stem, "publishes", table.get("publishes"))?;
    let consumes = topic_list(stem, "consumes", table.get("consumes"))?;
    let language = match table.get("language") {
        None => None,
        Some(Value::String(s)) => Some(
            Language::from_tag(s)
                .ok_or_else(|| format!("node manifest `{stem}`: unknown language `{s}`"))?,
        ),
        Some(_) => {
            return Err(format!(
                "node manifest `{stem}`: `language` must be a string"
            ));
        }
    };
    let build = match table.get("build") {
        None => None,
        Some(value) => Some(parse_build(stem, value)?),
    };
    Ok(NodeManifest {
        name,
        publishes,
        consumes,
        language,
        build,
    })
}

/// Parses the `[build]` adapter table into a [`BuildAdapter`] (REQ-TOOL-012):
/// `system = "cmake" | "cargo"` with its required fields, or an explicit
/// `command = [...]` array for custom builders. Unknown keys and systems are
/// rejected so a typo never silently disables a node build.
/// REQ: REQ-TOOL-012.
///
/// # Errors
/// A message naming the manifest stem and the defective field.
fn parse_build(stem: &str, value: &Value) -> Result<BuildAdapter, String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("node manifest `{stem}`: `build` must be a table"))?;
    if let Some(command) = table.get("command") {
        for key in table.keys() {
            if key != "command" {
                return Err(format!(
                    "node manifest `{stem}`: `build.command` excludes key `{key}`"
                ));
            }
        }
        let arr = command.as_array().ok_or_else(|| {
            format!("node manifest `{stem}`: `build.command` must be an array of strings")
        })?;
        let mut argv = Vec::with_capacity(arr.len());
        for item in arr {
            let arg = item.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                format!("node manifest `{stem}`: `build.command` entries must be non-empty strings")
            })?;
            argv.push(arg.to_string());
        }
        if argv.is_empty() {
            return Err(format!(
                "node manifest `{stem}`: `build.command` must not be empty"
            ));
        }
        return Ok(BuildAdapter::Command(argv));
    }
    for key in table.keys() {
        if !matches!(key.as_str(), "system" | "target" | "package" | "dir") {
            return Err(format!(
                "node manifest `{stem}`: unknown `build` key `{key}`"
            ));
        }
    }
    let system = table
        .get("system")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("node manifest `{stem}`: `build.system` must be a string"))?;
    match system {
        "cmake" => {
            let target = table
                .get("target")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    format!("node manifest `{stem}`: `build.target` is required for cmake")
                })?;
            let dir = match table.get("dir") {
                Some(Value::String(s)) if !s.is_empty() => std::path::PathBuf::from(s),
                Some(_) => {
                    return Err(format!(
                        "node manifest `{stem}`: `build.dir` must be a non-empty string"
                    ));
                }
                None => std::path::PathBuf::from("."),
            };
            Ok(BuildAdapter::Cmake {
                dir,
                target: target.to_string(),
            })
        }
        "cargo" => {
            let package = table
                .get("package")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    format!("node manifest `{stem}`: `build.package` is required for cargo")
                })?;
            Ok(BuildAdapter::Cargo {
                package: package.to_string(),
            })
        }
        other => Err(format!(
            "node manifest `{stem}`: unknown `build.system` `{other}` (expected cmake | cargo, or use `build.command`)"
        )),
    }
}

/// Reads a topic list: absent ⇒ empty; array of non-empty, duplicate-free
/// strings otherwise. REQ: REQ-TOOL-047.
fn topic_list(stem: &str, key: &str, v: Option<&Value>) -> Result<BTreeSet<String>, String> {
    let Some(v) = v else {
        return Ok(BTreeSet::new());
    };
    let arr = v
        .as_array()
        .ok_or_else(|| format!("node manifest `{stem}`: `{key}` must be an array of strings"))?;
    let mut out = BTreeSet::new();
    for item in arr {
        let t = item
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                format!("node manifest `{stem}`: `{key}` entries must be non-empty strings")
            })?
            .to_string();
        if !out.insert(t.clone()) {
            return Err(format!("node manifest `{stem}`: `{key}` repeats `{t}`"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{BuildAdapter, Language, parse_node_manifest};

    /// REQ: REQ-TOOL-012 — a cargo adapter parses from `[build] system`/`package`.
    #[test]
    fn parses_cargo_adapter() {
        let src = b"name = \"driver\"\npublishes = []\nconsumes = []\nlanguage = \"rust\"\n[build]\nsystem = \"cargo\"\npackage = \"driver\"\n";
        let m = parse_node_manifest("driver", src).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(m.language, Some(Language::Rust));
        assert_eq!(
            m.build,
            Some(BuildAdapter::Cargo {
                package: "driver".to_string()
            })
        );
    }

    /// REQ: REQ-TOOL-012 — an explicit command adapter is preserved verbatim.
    #[test]
    fn parses_command_adapter() {
        let src = b"name = \"x\"\n[build]\ncommand = [\"sh\", \"-c\", \"true\"]\n";
        let m = parse_node_manifest("x", src).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            m.build,
            Some(BuildAdapter::Command(vec![
                "sh".to_string(),
                "-c".to_string(),
                "true".to_string()
            ]))
        );
    }

    /// REQ: REQ-TOOL-012 — a cmake adapter without `target`, an unknown system,
    /// and a mixed `command` table are all rejected.
    #[test]
    fn rejects_malformed_adapters() {
        assert!(parse_node_manifest("a", b"name=\"a\"\n[build]\nsystem=\"cmake\"\n").is_err());
        assert!(parse_node_manifest("a", b"name=\"a\"\n[build]\nsystem=\"make\"\n").is_err());
        assert!(
            parse_node_manifest(
                "a",
                b"name=\"a\"\n[build]\ncommand=[\"true\"]\ntarget=\"x\"\n"
            )
            .is_err()
        );
    }
}
