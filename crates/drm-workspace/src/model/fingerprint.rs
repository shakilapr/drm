//! Contract fingerprints and the canonical layout hash
//! ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)
//! §Functions `contract_fingerprint`; canon-codec.md `layout_hash`).
//! Fingerprints cover exactly the pinned interface inputs — schemas AND
//! layouts AND declared directions — so an unrelated schema edit cannot move
//! an unaffected node (I-D4). REQ: REQ-TOOL-023, REQ-TOOL-013.

use std::collections::BTreeMap;

use drm_types::blake3_128;

use super::{NodeEntry, TopicName, WorkspaceModel};
use crate::canon::{Region, StructLayout};

/// Serialization format of the fingerprint input. Frozen (work-plan risk
/// register: change requires an ADR — Phase 18 gate depends on it).
/// REQ: REQ-TOOL-023.
pub const FINGERPRINT_FORMAT_VERSION: u32 = 1;

/// Contract fingerprint: BLAKE3 over the pinned interface serialization,
/// truncated to 128 bits. Width parity with `schema_hash`/`layout_hash`
/// ([u8; 16]); a freak collision is still caught by per-segment validation at
/// attach (channel-metadata.md). REQ: REQ-TOOL-023.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractFingerprint(
    /// 128-bit truncated digest.
    pub [u8; 16],
);

/// Canonical layout hash: BLAKE3 over the pinned two-region-layout
/// serialization (field offsets + descriptor offsets + tail packing, per
/// canon-codec.md). REQ: REQ-TOOL-023, REQ-TOOL-042.
#[must_use]
pub fn layout_hash(layout: &StructLayout) -> [u8; 16] {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"DRM-LAYOUT\0");
    buf.extend_from_slice(&layout.header_end.to_le_bytes());
    buf.extend_from_slice(&layout.sample_max.to_le_bytes());
    buf.extend_from_slice(&layout.struct_align.to_le_bytes());
    buf.extend_from_slice(
        &u32::try_from(layout.members.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for m in &layout.members {
        buf.push(region_code(m.region));
        buf.extend_from_slice(
            &u32::try_from(m.name.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        buf.extend_from_slice(m.name.as_bytes());
        buf.extend_from_slice(&m.offset.to_le_bytes());
        buf.extend_from_slice(&m.size.to_le_bytes());
        buf.extend_from_slice(&m.align.to_le_bytes());
    }
    blake3_128(&buf)
}

/// Region encoding inside the pinned layout serialization. REQ: REQ-TOOL-042.
fn region_code(r: Region) -> u8 {
    match r {
        Region::Header => 0,
        Region::Descriptor => 1,
        Region::Body => 2,
    }
}

/// Computes the contract fingerprint of `node` against model `m`. Interfaces
/// are sorted by (canonical topic path, direction) so map iteration order can
/// never leak in; topics absent from the model (possible only outside
/// `build`) are omitted from both the list and the count.
/// REQ: REQ-TOOL-023, REQ-TOOL-013.
#[must_use]
pub fn contract_fingerprint(node: &NodeEntry, m: &WorkspaceModel) -> ContractFingerprint {
    let mut interfaces: Vec<(&TopicName, u8)> = Vec::new();
    for t in &node.publishes {
        interfaces.push((t, 0));
    }
    for t in &node.consumes {
        interfaces.push((t, 1));
    }
    interfaces.sort_unstable();
    interfaces.retain(|(t, _)| m.topics.contains_key(*t));

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"DRM-CONTRACT\0");
    buf.extend_from_slice(&FINGERPRINT_FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&drm_types::ABI_VERSION.to_le_bytes());
    buf.extend_from_slice(
        &u32::try_from(interfaces.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for (topic, direction) in interfaces {
        // Indexing is safe: `contains_key` retained this exact key.
        let entry = &m.topics[topic.as_str()];
        buf.push(direction);
        buf.extend_from_slice(&u32::try_from(topic.len()).unwrap_or(u32::MAX).to_le_bytes());
        buf.extend_from_slice(topic.as_bytes());
        buf.extend_from_slice(&entry.schema_hash);
        buf.extend_from_slice(&layout_hash(&entry.layout));
    }
    ContractFingerprint(blake3_128(&buf))
}

/// Index of every topic published by more than one node — the verify-lite
/// query backing `drm verify` V-SW (I-D12: the model itself never rejects).
/// REQ: REQ-TOOL-047, REQ-TOOL-070.
#[must_use]
pub fn conflicting_publishers(m: &WorkspaceModel) -> Vec<(TopicName, Vec<String>)> {
    let mut by_topic: BTreeMap<&TopicName, Vec<&String>> = BTreeMap::new();
    for (name, node) in &m.nodes {
        for t in &node.publishes {
            by_topic.entry(t).or_default().push(name);
        }
    }
    by_topic
        .into_iter()
        .filter(|(_, nodes)| nodes.len() > 1)
        .map(|(t, nodes)| (t.clone(), nodes.into_iter().cloned().collect()))
        .collect()
}
