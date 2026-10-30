//! Verify — `drm verify` static checks
//! ([verify-engine.md](../../docs/algorithms/toolchain/verify-engine.md)):
//! the V-SW single-writer slice ([`verify_lite`]) plus the full deterministic
//! engine ([`engine`]) covering V-SCH, V-SW, V-PLC, V-RT, V-REL, V-MEM, V-CMP,
//! and V-ID. Model construction enforces structural/referential integrity only;
//! architectural policy is a verify check and therefore lives here
//! (workspace-model.md I-D12). REQ: REQ-TOOL-064, REQ-TOOL-070.

pub mod engine;
pub mod from_model;

use std::collections::BTreeMap;
use std::fmt;

use crate::model::{NodeName, TopicName, WorkspaceModel};

/// Stable check id for "consumed topic has no publisher" (verify-engine.md I-V3).
pub const V_SW_NO_PUBLISHER: &str = "V-SW-1";
/// Stable check id for "topic has more than one publisher".
pub const V_SW_MULTI_PUBLISHER: &str = "V-SW-2";

/// One V-SW negative with its location context. Every variant names the topic
/// (and the involved nodes) so a caller can report it without a source map;
/// the [`fmt::Display`] form cites the check id and REQ ID (I-V3).
/// REQ: REQ-TOOL-064, REQ-TOOL-070.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyLiteFinding {
    /// A topic is consumed by one or more nodes but no node publishes it.
    NoPublisher {
        /// Offending topic path.
        topic: TopicName,
        /// Consuming nodes, sorted by name.
        consumers: Vec<NodeName>,
    },
    /// A topic is published by more than one node (single-writer violation).
    MultiplePublishers {
        /// Offending topic path.
        topic: TopicName,
        /// Publishing nodes, sorted by name.
        publishers: Vec<NodeName>,
    },
}

impl fmt::Display for VerifyLiteFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPublisher { topic, consumers } => write!(
                f,
                "{V_SW_NO_PUBLISHER}: topic `{topic}` is consumed by [{}] but has no publisher (REQ-TOOL-064)",
                consumers.join(", ")
            ),
            Self::MultiplePublishers { topic, publishers } => write!(
                f,
                "{V_SW_MULTI_PUBLISHER}: topic `{topic}` has multiple publishers [{}] (REQ-TOOL-064)",
                publishers.join(", ")
            ),
        }
    }
}

/// Runs the V-SW checks over `m`. Findings are deterministically ordered
/// (`BTree` iteration only); an empty result means every consumed topic has
/// exactly one publisher (workspace-model.md I-D1). REQ: REQ-TOOL-064, REQ-TOOL-070.
#[must_use]
pub fn verify_lite(m: &WorkspaceModel) -> Vec<VerifyLiteFinding> {
    let mut publishers: BTreeMap<&TopicName, Vec<&NodeName>> = BTreeMap::new();
    let mut consumers: BTreeMap<&TopicName, Vec<&NodeName>> = BTreeMap::new();
    for (node, entry) in &m.nodes {
        for topic in &entry.publishes {
            publishers.entry(topic).or_default().push(node);
        }
        for topic in &entry.consumes {
            consumers.entry(topic).or_default().push(node);
        }
    }

    let mut out = Vec::new();
    for (topic, nodes) in &publishers {
        if nodes.len() > 1 {
            out.push(VerifyLiteFinding::MultiplePublishers {
                topic: (*topic).clone(),
                publishers: nodes.iter().map(|n| (*n).clone()).collect(),
            });
        }
    }
    for (topic, nodes) in &consumers {
        if !publishers.contains_key(topic) {
            out.push(VerifyLiteFinding::NoPublisher {
                topic: (*topic).clone(),
                consumers: nodes.iter().map(|n| (*n).clone()).collect(),
            });
        }
    }
    out
}
