//! Node manifest parsing — TOML with a closed surface (`name`, `publishes`,
//! `consumes`); unknown keys, empty names, non-string entries, and duplicate
//! entries are rejected ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)
//! `parse_node_manifest`). REQ: REQ-TOOL-047, REQ-TOOL-013.

use std::collections::BTreeSet;

use toml::Value;

/// Parsed node manifest: declared identity + interface sets (sorted).
/// REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeManifest {
    /// Declared node identity (unique across the tree; duplicates are a
    /// build conflict, I-D2).
    pub name: String,
    /// Canonical topic paths published.
    pub publishes: BTreeSet<String>,
    /// Canonical topic paths consumed.
    pub consumes: BTreeSet<String>,
}

/// Parses a node manifest from TOML bytes. The file stem is carried into the
/// error for location context. REQ: REQ-TOOL-047, REQ-TOOL-013.
///
/// # Errors
/// A message describing the defect when the TOML is malformed, a key is
/// unknown, a value has the wrong type, or an entry list repeats a topic.
pub fn parse_node_manifest(stem: &str, src: &[u8]) -> Result<NodeManifest, String> {
    let table: toml::Table =
        toml::from_slice(src).map_err(|e| format!("node manifest `{stem}`: {e}"))?;
    for key in table.keys() {
        if !matches!(key.as_str(), "name" | "publishes" | "consumes") {
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
    Ok(NodeManifest {
        name,
        publishes,
        consumes,
    })
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
