//! `drm assign <topic> --consume|--publish <node>` — manifest-mutation CLI.
//! Adds a canonical topic to the named node's `consumes`/`publishes` list in
//! its `drm.toml`, then re-runs the workspace verify and reports green/red
//! (architecture.md §Workspace Layout: "Developers can modify these manifests
//! by hand or through the CLI (`drm assign ...`)").
//!
//! Manifest surface: the implemented [`drm_workspace`] node parser reads
//! top-level `name`/`publishes`/`consumes` (workspace-model.md
//! `parse_node_manifest`); this command targets that surface so a mutated
//! manifest re-parses and the round-trip verify stays meaningful. The writer
//! is value-level TOML (`toml` crate), not comment-preserving —
//! `toml_edit` is not vendored.
//! REQ: REQ-TOOL-013 (manifest is the single source of truth), REQ-TOOL-047
//! (publishes/consumes contracts), REQ-TOOL-035 (adding never renumbers — the
//! handle lockfile is untouched here).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use toml::Value;

use crate::args::Direction;
use crate::error::CliError;
use crate::ws;

/// Adds `topic` to the node `node`'s list for `direction`, writes the manifest
/// back, then re-runs verify. REQ: REQ-TOOL-013, REQ-TOOL-047.
///
/// # Errors
/// [`CliError::Usage`] for an undeclared topic or node; [`CliError::Manifest`]
/// when the manifest cannot be parsed or written; [`CliError::Verify`] when the
/// post-edit workspace does not verify green.
pub fn run(root: &Path, topic: &str, direction: Direction, node: &str) -> Result<(), CliError> {
    let discovered = ws::discover(root)?;

    if !discovered.topics.contains(topic) {
        return Err(CliError::Usage(format!(
            "undeclared topic `{topic}`: no `packages/*/topics/{}.drm` defines it; \
             add the topic first (REQ-TOOL-013)",
            topic.rsplit('/').next().unwrap_or(topic)
        )));
    }
    let manifest = discovered.nodes.get(node).ok_or_else(|| {
        CliError::Usage(format!(
            "undeclared node `{node}`: no `packages/*/nodes/{node}/drm.toml` in `{}`",
            root.display()
        ))
    })?;

    let bytes = fs::read(manifest)
        .map_err(|e| CliError::Io(format!("cannot read `{}` ({e})", manifest.display())))?;
    let key = list_key(direction);
    let updated = add_to_list(&bytes, key, topic)
        .map_err(|m| CliError::Manifest(format!("`{}`: {m}", manifest.display())))?;
    fs::write(manifest, updated)
        .map_err(|e| CliError::Io(format!("cannot write `{}` ({e})", manifest.display())))?;

    // Re-discover so the model sees the edited manifest, then verify.
    let after = ws::discover(root)?;
    match ws::verify(&after) {
        Ok(findings) if findings.is_empty() => {
            println!("assigned `{topic}` as {key} of node `{node}`.");
            println!("verify: green (single-writer + referential checks OK).");
            Ok(())
        }
        Ok(findings) => Err(CliError::Verify(format!(
            "verify: red — {}",
            join_findings(&findings)
        ))),
        Err(errors) => Err(CliError::Verify(format!(
            "verify: red — model build failed: {}",
            errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

/// Manifest list key for a direction. REQ: REQ-TOOL-047.
#[must_use]
fn list_key(direction: Direction) -> &'static str {
    match direction {
        Direction::Consume => "consumes",
        Direction::Publish => "publishes",
    }
}

/// Renders verify findings, one per clause. REQ: REQ-TOOL-047.
fn join_findings(findings: &[drm_workspace::verify::VerifyLiteFinding]) -> String {
    findings
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Pure manifest mutation: parse `bytes` as TOML, add `topic` to the string
/// array at `key` (creating it if absent), keep the array sorted and
/// duplicate-free, and serialize back. REQ: REQ-TOOL-013, REQ-TOOL-047.
///
/// # Errors
/// A message when the TOML is malformed, `key` holds a non-array/non-string
/// value, or serialization fails.
pub fn add_to_list(bytes: &[u8], key: &str, topic: &str) -> Result<Vec<u8>, String> {
    let mut table: toml::Table =
        toml::from_slice(bytes).map_err(|e| format!("not valid TOML: {e}"))?;

    let mut set: BTreeSet<String> = match table.get(key) {
        None => BTreeSet::new(),
        Some(Value::Array(items)) => {
            let mut set = BTreeSet::new();
            for item in items {
                let existing = item
                    .as_str()
                    .ok_or_else(|| format!("`{key}` entries must be non-empty strings"))?;
                set.insert(existing.to_string());
            }
            set
        }
        Some(_) => return Err(format!("`{key}` must be an array of strings")),
    };
    set.insert(topic.to_string());

    let array = Value::Array(set.into_iter().map(Value::String).collect());
    table.insert(key.to_string(), array);

    let body = toml::to_string(&table).map_err(|e| format!("cannot serialize manifest: {e}"))?;
    Ok(body.into_bytes())
}

#[cfg(test)]
mod tests {
    /// REQ: REQ-TOOL-013 — adding a topic to a missing list creates it, sorted.
    #[test]
    fn add_creates_list() {
        let out = super::add_to_list(b"name = \"controller\"\n", "consumes", "control/target")
            .unwrap_or_else(|e| unreachable!("{e}"));
        let text = String::from_utf8(out).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(text.contains("name = \"controller\""));
        assert!(text.contains("consumes = [\"control/target\"]"));
    }

    /// REQ: REQ-TOOL-047 — re-adding is idempotent; the list stays sorted and
    /// duplicate-free.
    #[test]
    fn add_is_idempotent_and_sorted() {
        let first = super::add_to_list(b"name = \"c\"\n", "consumes", "b/t")
            .unwrap_or_else(|e| unreachable!("{e}"));
        let twice =
            super::add_to_list(&first, "consumes", "b/t").unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(first, twice, "re-adding an existing topic is a no-op");
        let out =
            super::add_to_list(&first, "consumes", "a/t").unwrap_or_else(|e| unreachable!("{e}"));
        let text = String::from_utf8(out).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(
            text.contains("consumes = [\"a/t\", \"b/t\"]"),
            "got: {text}"
        );
    }

    /// REQ: REQ-TOOL-013 — a non-array list is rejected, not silently replaced.
    #[test]
    fn non_array_list_rejected() {
        assert!(super::add_to_list(b"name = \"c\"\nconsumes = 3\n", "consumes", "a/t").is_err());
    }
}
