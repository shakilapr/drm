//! Topology manifest parsing ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)
//! `parse_topology`): computes, undirected links, node placement. Closed TOML
//! surface; referential validation happens at model build — structural
//! integrity only, policy belongs to `drm verify` (I-D12).
//! REQ: REQ-TOOL-047, REQ-TOOL-070, REQ-TOOL-013.

use std::collections::{BTreeMap, BTreeSet};

use toml::Value;

/// A compute vertex: identity only for this slice; capabilities land with
/// later work-plan steps. REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeEntry;

/// Undirected link between two compute vertices. REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEntry {
    /// Unique link id (repeats are a build conflict, I-D2).
    pub id: String,
    /// One endpoint compute name.
    pub a: String,
    /// The other endpoint compute name.
    pub b: String,
}

/// Parsed topology manifest. REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TopologyManifest {
    /// Declared compute vertex names.
    pub computes: BTreeSet<String>,
    /// Declared links in input order; the build sorts by id and reports
    /// duplicate ids (I-D2).
    pub links: Vec<LinkEntry>,
    /// Node name -> compute name.
    pub placement: BTreeMap<String, String>,
}

/// Parses a topology manifest from TOML bytes. Surface: `[computes]` with
/// `name = true` or empty-table values, `[[links]]` entries with non-empty
/// `id`/`a`/`b` strings and `a != b`, `[placement]` with
/// `node = "compute"`. REQ: REQ-TOOL-047, REQ-TOOL-013.
///
/// # Errors
/// A message describing the defect when the TOML is malformed, a top-level
/// key is unknown, a value has the wrong type, a name is empty, or a link is
/// a self-loop. Duplicate link ids and dangling references are build-level
/// conflicts, collected with all other defects (I-D2).
pub fn parse_topology_manifest(src: &[u8]) -> Result<TopologyManifest, String> {
    let table: toml::Table =
        toml::from_slice(src).map_err(|e| format!("topology manifest: {e}"))?;
    for key in table.keys() {
        if !matches!(key.as_str(), "computes" | "links" | "placement") {
            return Err(format!("topology manifest: unknown key `{key}`"));
        }
    }
    Ok(TopologyManifest {
        computes: computes_of(table.get("computes"))?,
        links: links_of(table.get("links"))?,
        placement: placement_of(table.get("placement"))?,
    })
}

/// Reads `[computes]`: each value must be `true` or an empty table.
fn computes_of(v: Option<&Value>) -> Result<BTreeSet<String>, String> {
    let Some(v) = v else {
        return Ok(BTreeSet::new());
    };
    let table = v
        .as_table()
        .ok_or_else(|| "topology manifest: `computes` must be a table".to_string())?;
    let mut out = BTreeSet::new();
    for (name, body) in table {
        if name.is_empty() {
            return Err("topology manifest: compute names must be non-empty".to_string());
        }
        match body {
            Value::Boolean(true) => {}
            Value::Table(t) if t.is_empty() => {}
            _ => {
                return Err(format!(
                    "topology manifest: `computes.{name}` must be `true` or an empty table"
                ));
            }
        }
        out.insert(name.clone());
    }
    Ok(out)
}

/// Reads `[[links]]`: each entry needs non-empty `id`/`a`/`b` and `a != b`.
fn links_of(v: Option<&Value>) -> Result<Vec<LinkEntry>, String> {
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    let arr = v
        .as_array()
        .ok_or_else(|| "topology manifest: `links` must be an array of tables".to_string())?;
    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let table = item
            .as_table()
            .ok_or_else(|| format!("topology manifest: `links[{i}]` must be a table"))?;
        for key in table.keys() {
            if !matches!(key.as_str(), "id" | "a" | "b") {
                return Err(format!(
                    "topology manifest: `links[{i}]`: unknown key `{key}`"
                ));
            }
        }
        let id = link_field(table, i, "id")?;
        let a = link_field(table, i, "a")?;
        let b = link_field(table, i, "b")?;
        if a == b {
            return Err(format!("topology manifest: link `{id}` is a self-loop"));
        }
        out.push(LinkEntry {
            id: id.to_string(),
            a: a.to_string(),
            b: b.to_string(),
        });
    }
    Ok(out)
}

/// Reads one required non-empty string field of a link entry.
fn link_field<'a>(table: &'a toml::Table, i: usize, key: &str) -> Result<&'a str, String> {
    table
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("topology manifest: `links[{i}].{key}` must be a non-empty string"))
}

/// Reads `[placement]`: node names -> non-empty compute names.
fn placement_of(v: Option<&Value>) -> Result<BTreeMap<String, String>, String> {
    let Some(v) = v else {
        return Ok(BTreeMap::new());
    };
    let table = v
        .as_table()
        .ok_or_else(|| "topology manifest: `placement` must be a table".to_string())?;
    let mut out = BTreeMap::new();
    for (node, compute) in table {
        if node.is_empty() {
            return Err("topology manifest: placement node names must be non-empty".to_string());
        }
        let compute = compute.as_str().filter(|c| !c.is_empty()).ok_or_else(|| {
            format!("topology manifest: `placement.{node}` must be a non-empty string")
        })?;
        out.insert(node.clone(), compute.to_string());
    }
    Ok(out)
}
