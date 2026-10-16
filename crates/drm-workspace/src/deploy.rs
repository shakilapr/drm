//! Deployment-artifact generation ([build-orchestration.md](../../docs/algorithms/toolchain/build-orchestration.md)
//! `deploy::generate`; [deployment-identity.md](../../docs/algorithms/deployment-identity.md)
//! §Protocols): emits the runtime connection config consumed at startup — the
//! deployment instance id, the ABI version, per-node fingerprint expectations,
//! and the per-topic connection data (slot, contract hashes, activation sizing)
//! a process needs in order to attach without hand-built expectations.
//! Deployment artifacts may embed runtime numeric handles; generated bindings
//! never do (codegen I-G9).
//!
//! The deployment identity is REUSED while the deployment inputs are unchanged
//! and minted fresh (CSPRNG) when they move. Minting on every rebuild would
//! orphan a live namespace: the segments of the previous id still exist while
//! nodes refuse (`WrongAbi`) against the new one.
//! REQ: REQ-TOOL-012, REQ-TOOL-083, REQ-TOOL-084.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use drm_types::{ABI_VERSION, Status, blake3_128};

use crate::canon::json_escape;
use crate::model::WorkspaceModel;
use crate::model::fingerprint::contract_fingerprint;
use crate::model::handles::HandleTable;
use crate::model::runtime::{RuntimeOverflow, runtime_plan};
use crate::orchestrator::DEPLOY_DIR_REL;

/// Deployment-artifact file name under `.drm/deploy`. REQ: REQ-TOOL-012.
pub const DEPLOYMENT_FILE: &str = "deployment.json";

/// Emitted `format_version`. Bumped when the artifact shape changes: a consumer
/// refuses a version it does not know rather than interpreting it loosely, and
/// the emitter never reuses an identity recorded under another version.
/// REQ: REQ-TOOL-083.
pub const FORMAT_VERSION: u64 = 2;

/// Path of the deployment artifact for a workspace root.
/// REQ: REQ-TOOL-012.
#[must_use]
pub fn deployment_path(root: &Path) -> PathBuf {
    root.join(DEPLOY_DIR_REL).join(DEPLOYMENT_FILE)
}

/// Writes the deployment artifact for `model` and `handles`, returning the number
/// of topics emitted.
///
/// The recorded identity is reused when the inputs digest and `format_version`
/// still match (REQ-TOOL-084); otherwise a fresh CSPRNG identity is minted. The
/// render is sorted by canonical topic path and carries no timestamps.
///
/// # Errors
/// [`Status::InvalidConfig`] when the directory or file cannot be written, or
/// when the system CSPRNG is unavailable.
/// REQ: REQ-TOOL-083, REQ-TOOL-084.
pub fn generate(
    model: &WorkspaceModel,
    handles: &HandleTable,
    root: &Path,
) -> Result<usize, Status> {
    let path = deployment_path(root);
    let Some(parent) = path.parent() else {
        return Err(Status::InvalidConfig);
    };
    fs::create_dir_all(parent).map_err(|_| Status::InvalidConfig)?;
    let digest = inputs_digest(model, handles);
    // Generated names derive from the slot alone, so the topic-slot table must be
    // injective: two topics sharing a slot would attach to one segment
    // (REQ-CORE-088). A workspace without an assignment table (every topic at the
    // default slot) is refused rather than silently aliased.
    let mut seen: BTreeMap<u32, &str> = BTreeMap::new();
    for topic in runtime_plan(model).topics.keys() {
        if seen
            .insert(slot_of(handles, topic), topic.as_str())
            .is_some()
        {
            return Err(Status::InvalidConfig);
        }
    }
    let id = fs::read_to_string(&path)
        .ok()
        .and_then(|text| reuse_id(&text, &digest))
        .map_or_else(mint_id, Ok)?;
    let body = render_with(model, handles, &id, &digest);
    fs::write(&path, body.as_bytes()).map_err(|_| Status::InvalidConfig)?;
    Ok(model.topics.len())
}

/// Deployment inputs digest — the identity reuse key (REQ-TOOL-084). Covers the
/// handle table, every topic's contract hashes and activation sizing, and every
/// node's contract fingerprint: exactly the inputs whose change requires a new
/// namespace. Public so tooling (`drm deploy`) can report whether an identity is
/// still current.
#[must_use]
pub fn inputs_digest(model: &WorkspaceModel, handles: &HandleTable) -> [u8; 16] {
    let plan = runtime_plan(model);
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"DRM-DEPLOYMENT-INPUTS\0");
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&ABI_VERSION.to_le_bytes());
    for (topic, runtime) in &plan.topics {
        push_str(&mut buf, topic);
        buf.extend_from_slice(&slot_of(handles, topic).to_le_bytes());
        buf.extend_from_slice(&runtime.schema_hash);
        buf.extend_from_slice(&runtime.layout_hash);
        buf.extend_from_slice(&runtime.payload_capacity.to_le_bytes());
        buf.extend_from_slice(&runtime.slot_align.to_le_bytes());
        buf.extend_from_slice(&runtime.history_depth.to_le_bytes());
        buf.extend_from_slice(&runtime.pool_size.to_le_bytes());
        buf.push(u8::from(matches!(
            runtime.overflow,
            RuntimeOverflow::Reject
        )));
    }
    for (name, node) in &model.nodes {
        push_str(&mut buf, name);
        buf.extend_from_slice(&contract_fingerprint(node, model).0);
    }
    blake3_128(&buf)
}

/// The identity recorded in `text`, when it is still valid for `digest`: same
/// `format_version` and same inputs digest. `None` ⇒ mint fresh.
#[must_use]
fn reuse_id(text: &str, digest: &[u8; 16]) -> Option<String> {
    if field(text, "format_version")?.parse::<u64>().ok()? != FORMAT_VERSION {
        return None;
    }
    if field(text, "inputs_digest")? != hex(digest) {
        return None;
    }
    field(text, "deployment_id")
}

/// Reads a top-level field rendered by [`render_with`] (`"key": "value"` or
/// `"key": 12`). Keys are unique per level, so a targeted scan suffices and the
/// toolchain layer needs no JSON parser.
fn field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let rest = text.get(text.find(&needle)? + needle.len()..)?.trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return Some(quoted.get(..quoted.find('"')?)?.to_string());
    }
    let end = rest.find([',', '\n', '}']).unwrap_or(rest.len());
    Some(rest.get(..end)?.trim().to_string())
}

/// A fresh 128-bit deployment instance id, lowercase hex (REQ-TOOL-084).
///
/// # Errors
/// [`Status::InvalidConfig`] when the system CSPRNG cannot be read.
fn mint_id() -> Result<String, Status> {
    let mut raw = [0_u8; 16];
    getrandom::fill(&mut raw).map_err(|_| Status::InvalidConfig)?;
    Ok(hex(&raw))
}

/// Canonical deployment render: identity and contract metadata, per-node
/// expectations, then the sorted per-topic connection table.
/// REQ: REQ-TOOL-083.
#[must_use]
pub fn render_with(
    model: &WorkspaceModel,
    handles: &HandleTable,
    deployment_id: &str,
    inputs_digest: &[u8; 16],
) -> String {
    let plan = runtime_plan(model);
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"format_version\": {FORMAT_VERSION},");
    let _ = writeln!(out, "  \"abi_version\": {ABI_VERSION},");
    let _ = writeln!(
        out,
        "  \"deployment_id\": \"{}\",",
        json_escape(deployment_id)
    );
    let _ = writeln!(out, "  \"inputs_digest\": \"{}\",", hex(inputs_digest));

    out.push_str("  \"fingerprint_expectations\": ");
    if model.nodes.is_empty() {
        out.push_str("{},\n");
    } else {
        let entries: Vec<String> = model
            .nodes
            .iter()
            .map(|(name, node)| {
                format!(
                    "    \"{}\": \"{}\"",
                    json_escape(name),
                    hex(&contract_fingerprint(node, model).0)
                )
            })
            .collect();
        out.push_str("{\n");
        out.push_str(&entries.join(",\n"));
        out.push_str("\n  },\n");
    }

    out.push_str("  \"topics\": ");
    if plan.topics.is_empty() {
        out.push_str("{}\n");
    } else {
        let entries: Vec<String> = plan
            .topics
            .iter()
            .map(|(topic, runtime)| {
                let mut e = String::new();
                let _ = writeln!(e, "    \"{}\": {{", json_escape(topic));
                let _ = writeln!(e, "      \"slot\": {},", slot_of(handles, topic));
                let _ = writeln!(
                    e,
                    "      \"schema_hash\": \"{}\",",
                    hex(&runtime.schema_hash)
                );
                let _ = writeln!(
                    e,
                    "      \"layout_hash\": \"{}\",",
                    hex(&runtime.layout_hash)
                );
                let _ = writeln!(
                    e,
                    "      \"payload_capacity\": {},",
                    runtime.payload_capacity
                );
                let _ = writeln!(e, "      \"slot_align\": {},", runtime.slot_align);
                let _ = writeln!(e, "      \"history_depth\": {},", runtime.history_depth);
                let _ = writeln!(e, "      \"pool_size\": {},", runtime.pool_size);
                let _ = writeln!(
                    e,
                    "      \"overflow\": \"{}\"",
                    overflow_name(runtime.overflow)
                );
                e.push_str("    }");
                e
            })
            .collect();
        out.push_str("{\n");
        out.push_str(&entries.join(",\n"));
        out.push_str("\n  }\n");
    }

    out.push_str("}\n");
    out
}

/// The topic's assigned slot, or `0` when the workspace has no assignment for it
/// (pre-`drm assign` state).
fn slot_of(handles: &HandleTable, topic: &str) -> u32 {
    handles.assignments.get(topic).copied().unwrap_or(0)
}

/// Overflow policy as rendered. REQ: REQ-TOOL-083.
fn overflow_name(overflow: RuntimeOverflow) -> &'static str {
    match overflow {
        RuntimeOverflow::Overwrite => "overwrite",
        RuntimeOverflow::Reject => "reject",
    }
}

/// Lowercase hex of a 16-byte digest.
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().fold(String::with_capacity(32), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

/// Length-prefixed UTF-8 push, so no two distinct input sequences can produce
/// the same digest buffer.
fn push_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{FORMAT_VERSION, deployment_path, field, generate, render_with};
    use crate::model::handles::HandleTable;
    use crate::model::{PackageInput, TreeInput, WorkspaceModel, build};

    /// Topic source with attributes, one `u64` field.
    const TOPIC: &str = "@history: 64\n@pool_size: 128\n@overflow: overwrite\nvalue: u64\n";
    /// Minimal producer manifest.
    const PRODUCER: &str = "name = \"producer\"\npublishes = [\"counter/value\"]\nconsumes = []\n";

    /// A one-topic, one-node workspace built from `topic`.
    fn model(topic: &str) -> WorkspaceModel {
        let tree = TreeInput {
            packages: vec![PackageInput {
                name: "counter".to_string(),
                topics: vec![("value".to_string(), topic.as_bytes().to_vec())],
            }],
            nodes: vec![("producer".to_string(), PRODUCER.as_bytes().to_vec())],
            topology: None,
        };
        build(&tree).unwrap_or_else(|e| unreachable!("fixture model must build: {e:?}"))
    }

    /// Handle table assigning slot 5 to the fixture topic.
    fn handles() -> HandleTable {
        HandleTable {
            assignments: BTreeMap::from([("counter/value".to_string(), 5_u32)]),
            next_free: 6,
            ..HandleTable::default()
        }
    }

    /// A fresh writable scratch workspace root, per tag.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("drm-deploy-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("scratch: {e}"));
        dir
    }

    /// REQ: REQ-TOOL-083 — the render is deterministic and sorted.
    #[test]
    fn render_is_deterministic_and_sorted() {
        let tree = TreeInput {
            packages: vec![PackageInput {
                name: "pkg".to_string(),
                topics: vec![
                    ("two".to_string(), b"v: u32\n".to_vec()),
                    ("one".to_string(), b"v: u32\n".to_vec()),
                ],
            }],
            nodes: Vec::new(),
            topology: None,
        };
        let model = build(&tree).unwrap_or_else(|e| unreachable!("{e:?}"));
        let handles = HandleTable {
            assignments: BTreeMap::from([
                ("pkg/one".to_string(), 0_u32),
                ("pkg/two".to_string(), 1_u32),
            ]),
            next_free: 2,
            ..HandleTable::default()
        };
        let digest = [7_u8; 16];
        let id = "0123456789abcdef0123456789abcdef";
        let body = render_with(&model, &handles, id, &digest);
        assert!(body.contains(&format!("\"format_version\": {FORMAT_VERSION}")));
        let one = body
            .find("pkg/one")
            .unwrap_or_else(|| unreachable!("pkg/one"));
        let two = body
            .find("pkg/two")
            .unwrap_or_else(|| unreachable!("pkg/two"));
        assert!(one < two, "sorted: {body}");
        assert_eq!(render_with(&model, &handles, id, &digest), body);
    }

    /// REQ: REQ-TOOL-083 — the artifact carries the complete connection data a
    /// process needs to attach: identity, ABI, expectations, and per topic its
    /// slot, contract hashes and activation sizing.
    #[test]
    fn config_carries_connection_data() {
        let root = scratch("connection");
        let model = model(TOPIC);
        let handles = handles();
        generate(&model, &handles, &root).unwrap_or_else(|e| unreachable!("generate: {e:?}"));
        let body = std::fs::read_to_string(deployment_path(&root))
            .unwrap_or_else(|e| unreachable!("read: {e}"));

        let id = field(&body, "deployment_id").unwrap_or_else(|| unreachable!("id"));
        assert_eq!(id.len(), 32, "128-bit hex id: {id}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
        assert_eq!(field(&body, "format_version").as_deref(), Some("2"));
        assert_eq!(
            field(&body, "abi_version").as_deref(),
            Some(drm_types::ABI_VERSION.to_string().as_str())
        );
        assert!(
            body.contains("\"producer\": "),
            "per-node fingerprint expectation: {body}"
        );
        for needle in [
            "\"counter/value\": {",
            "\"slot\": 5",
            "\"schema_hash\": \"",
            "\"layout_hash\": \"",
            "\"payload_capacity\": 8",
            "\"slot_align\": 8",
            "\"history_depth\": 64",
            "\"pool_size\": 128",
            "\"overflow\": \"overwrite\"",
        ] {
            assert!(body.contains(needle), "missing {needle} in {body}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-084 — an unchanged rebuild keeps the identity; a contract
    /// change mints a new one.
    #[test]
    fn deployment_id_reused_until_inputs_change() {
        let root = scratch("identity");
        let handles = handles();
        generate(&model(TOPIC), &handles, &root).unwrap_or_else(|e| unreachable!("first: {e:?}"));
        let first = std::fs::read_to_string(deployment_path(&root))
            .unwrap_or_else(|e| unreachable!("read: {e}"));
        let first_id = field(&first, "deployment_id").unwrap_or_else(|| unreachable!("id"));

        generate(&model(TOPIC), &handles, &root).unwrap_or_else(|e| unreachable!("again: {e:?}"));
        let again = std::fs::read_to_string(deployment_path(&root))
            .unwrap_or_else(|e| unreachable!("read: {e}"));
        assert_eq!(
            field(&again, "deployment_id").as_deref(),
            Some(first_id.as_str()),
            "an unchanged rebuild must reuse the deployment identity"
        );
        assert_eq!(again, first, "and re-render identically");

        // A schema change moves the identity: the old namespace must not be reused.
        let changed = "@history: 64\n@pool_size: 128\n@overflow: overwrite\nvalue: u32\n";
        generate(&model(changed), &handles, &root)
            .unwrap_or_else(|e| unreachable!("changed: {e:?}"));
        let moved = std::fs::read_to_string(deployment_path(&root))
            .unwrap_or_else(|e| unreachable!("read: {e}"));
        assert_ne!(
            field(&moved, "deployment_id").as_deref(),
            Some(first_id.as_str()),
            "a contract change must mint a fresh deployment identity"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-083, REQ-CORE-088 — a topic table whose slots collide is
    /// refused: generated names derive from the slot, so two topics sharing one
    /// would attach to the same segment.
    #[test]
    fn colliding_slots_refused() {
        let root = scratch("collide");
        // Two topics with no assignment table: both default to slot 0, which
        // would alias one segment.
        let two = TreeInput {
            packages: vec![PackageInput {
                name: "counter".to_string(),
                topics: vec![
                    ("value".to_string(), TOPIC.as_bytes().to_vec()),
                    ("other".to_string(), b"v: u64\n".to_vec()),
                ],
            }],
            nodes: Vec::new(),
            topology: None,
        };
        let two = build(&two).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(
            generate(&two, &HandleTable::default(), &root).err(),
            Some(drm_types::Status::InvalidConfig),
            "an unassigned multi-topic workspace must not alias one segment"
        );
        // A single topic at slot 0 is legitimate (handles start at zero).
        assert!(generate(&model(TOPIC), &HandleTable::default(), &root).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-083 — an artifact recorded under another `format_version` is
    /// superseded, never reinterpreted.
    #[test]
    fn foreign_format_version_is_not_reused() {
        let root = scratch("version");
        let handles = handles();
        generate(&model(TOPIC), &handles, &root)
            .unwrap_or_else(|e| unreachable!("generate: {e:?}"));
        let path = deployment_path(&root);
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| unreachable!("read: {e}"));
        let id = field(&body, "deployment_id").unwrap_or_else(|| unreachable!("id"));
        let bumped = body.replacen(
            &format!("\"format_version\": {FORMAT_VERSION}"),
            &format!("\"format_version\": {}", FORMAT_VERSION + 1),
            1,
        );
        std::fs::write(&path, bumped).unwrap_or_else(|e| unreachable!("write: {e}"));

        generate(&model(TOPIC), &handles, &root).unwrap_or_else(|e| unreachable!("regen: {e:?}"));
        let after = std::fs::read_to_string(&path).unwrap_or_else(|e| unreachable!("read: {e}"));
        assert_ne!(
            field(&after, "deployment_id").as_deref(),
            Some(id.as_str()),
            "an artifact from another format version must be superseded"
        );
        assert_eq!(field(&after, "format_version").as_deref(), Some("2"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
