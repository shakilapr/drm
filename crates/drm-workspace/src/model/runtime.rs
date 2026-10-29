//! Runtime hooks: derive deployment/runtime activation expectations from the
//! in-memory model ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)).
//! The runtime never reads `.drm` schemas directly; it consumes the descriptor
//! expectations and payload-pool sizing this hook produces, so a topic edit and
//! the runtime channel configuration cannot drift (architecture.md §Channel
//! Metadata, §In-Memory Workspace Model).
//! REQ: REQ-TOOL-045, REQ-CORE-045, REQ-CORE-015, REQ-CORE-017.

use std::collections::BTreeMap;

use crate::model::{TopicEntry, WorkspaceModel, layout_hash};

/// Overflow policy a topic's runtime channel activates with: `@overflow:
/// overwrite` (default) drops the oldest retained sample, `@overflow: reject`
/// refuses the commit. REQ: REQ-CORE-040.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOverflow {
    /// Drop-oldest policy (`@overflow: overwrite`, the default).
    Overwrite,
    /// Refuse-on-full policy (`@overflow: reject`).
    Reject,
}

/// Per-topic runtime activation expectations derived from the model.
/// REQ: REQ-CORE-045, REQ-CORE-015.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRuntime {
    /// Canonical topic path (runtime identity, never a numeric handle).
    pub topic: String,
    /// 128-bit canonical schema content hash the descriptor must match.
    pub schema_hash: [u8; 16],
    /// 128-bit canonical layout hash the descriptor must match.
    pub layout_hash: [u8; 16],
    /// Fixed sample size or max bound (`StructLayout::sample_max`).
    pub payload_capacity: u64,
    /// Slot alignment (`StructLayout::struct_align`).
    pub slot_align: u64,
    /// History depth `R` (`@history: latest` ⇒ `1`).
    pub history_depth: u64,
    /// Payload-pool slots: explicit `@pool_size` or the `R + 1` floor.
    pub pool_size: u64,
    /// Resolved overflow policy.
    pub overflow: RuntimeOverflow,
}

/// Whole-model runtime plan: one [`TopicRuntime`] per declared topic, sorted by
/// canonical path (`BTreeMap`, I-D1). REQ: REQ-TOOL-045.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimePlan {
    /// Canonical topic path → runtime activation expectations.
    pub topics: BTreeMap<String, TopicRuntime>,
}

impl RuntimePlan {
    /// The runtime expectations for `topic`, if declared.
    /// REQ: REQ-TOOL-045.
    #[must_use]
    pub fn get(&self, topic: &str) -> Option<&TopicRuntime> {
        self.topics.get(topic)
    }
}

/// Derives the runtime plan from the model. History depth defaults to `latest`
/// (`R = 1`); the payload pool defaults to the activation floor `R + 1` unless
/// the topic declares `@pool_size`. `drm verify` (not this hook) owns the
/// policy check `pool_size >= R + H + 1` (workspace-model.md I-D12).
/// REQ: REQ-TOOL-045, REQ-CORE-045.
#[must_use]
pub fn runtime_plan(model: &WorkspaceModel) -> RuntimePlan {
    let mut topics = BTreeMap::new();
    for (topic, entry) in &model.topics {
        topics.insert(topic.clone(), topic_runtime(topic, entry));
    }
    RuntimePlan { topics }
}

/// Default payload-pool slots when a topic declares no `@pool_size`.
///
/// A `@history: latest` topic gets the documented triple buffer — one sample for
/// the latest commit, one for the in-progress publisher write, one held by an
/// active reader (`architecture.md` §Bulk Latest Topics, `payload-pool.md` I-P6)
/// — which is also the drop-free floor `R + H + 1` for `R = 1, H = 1`. Any other
/// history depth gets `R + 1`, the mandatory floor (I-P4).
///
/// This deliberately mirrors `drm_core::channel::pool::default_pool_size`:
/// `drm-workspace` must never depend on `drm-core` (codebase.md layering), so the
/// rule exists twice by necessity. It drifted once — this function returned
/// `R + 1` for every topic, which silently dropped the triple buffer and left
/// every defaulted topic one slot short of drop-free — so `default_pool_sizes_agree`
/// pins the two implementations together.
/// REQ: REQ-CORE-041, REQ-CORE-045.
#[must_use]
pub const fn default_pool_size(history_is_latest: bool) -> u64 {
    if history_is_latest { 3 } else { 8 }
}

/// Per-topic runtime expectations (I-D10: typed values parsed once at build).
/// REQ: REQ-TOOL-045, REQ-CORE-045.
#[must_use]
pub fn topic_runtime(topic: &str, entry: &TopicEntry) -> TopicRuntime {
    let history_depth = entry.attrs.history_depth.unwrap_or(1);
    let pool_size = entry.attrs.pool_size.unwrap_or_else(|| {
        // `latest` is either declared (`@history: latest`) or implied by the
        // absence of a depth: the runtime resolves an undeclared depth to 1, which
        // *is* the latest case, so both spell the same storage class.
        let latest = entry.attrs.history_latest || entry.attrs.history_depth.is_none();
        // The default is a *floor*, never a ceiling: `pool_size >= R + 1` is
        // mandatory (payload-pool.md I-P4), so a deep history such as
        // `@history: 128` must not be given the smaller 8-slot default.
        default_pool_size(latest).max(history_depth.saturating_add(1))
    });
    let overflow = if entry.attrs.overflow_reject {
        RuntimeOverflow::Reject
    } else {
        RuntimeOverflow::Overwrite
    };
    TopicRuntime {
        topic: topic.to_string(),
        schema_hash: entry.schema_hash,
        layout_hash: layout_hash(&entry.layout),
        payload_capacity: entry.layout.sample_max,
        slot_align: u64::from(entry.layout.struct_align),
        history_depth,
        pool_size,
        overflow,
    }
}

/// Per-channel verification expectations exposed from the model for the
/// verifier's `V-MEM`/`V-RT` checks (interfaces only — no runtime behavior
/// change). REQ: REQ-TOOL-057, REQ-TOOL-062.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelExpectation {
    /// Canonical topic path.
    pub topic: String,
    /// Payload-pool slots the model derives (explicit `@pool_size` or `R + 1`).
    pub pool_size: u64,
    /// Max sample size in bytes (`StructLayout::sample_max`) — the framing-fit
    /// input for `V-RT`. REQ: REQ-TOOL-062.
    pub payload_capacity: u64,
    /// History depth `R` (`@history: latest` ⇒ `1`).
    pub history_depth: u64,
    /// Slot alignment in bytes.
    pub slot_align: u64,
}

/// Derives per-topic verification expectations from the model, sorted by
/// canonical path (`BTreeMap`, I-D1). The verifier consumes these so declared
/// pool sizing and payload bounds cannot drift from the model.
/// REQ: REQ-TOOL-057, REQ-TOOL-062.
#[must_use]
pub fn channel_expectations(model: &WorkspaceModel) -> BTreeMap<String, ChannelExpectation> {
    runtime_plan(model)
        .topics
        .into_iter()
        .map(|(topic, rt)| {
            let expectation = ChannelExpectation {
                topic: topic.clone(),
                pool_size: rt.pool_size,
                payload_capacity: rt.payload_capacity,
                history_depth: rt.history_depth,
                slot_align: rt.slot_align,
            };
            (topic, expectation)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::model::{PackageInput, TreeInput, WorkspaceModel, build};

    use super::{RuntimeOverflow, RuntimePlan, TopicRuntime, channel_expectations, runtime_plan};

    /// Builds a one-package model from `(stem, source)` topics.
    fn model(topics: &[(&str, &str)]) -> WorkspaceModel {
        let tree = TreeInput {
            packages: vec![PackageInput {
                name: "control".to_string(),
                topics: topics
                    .iter()
                    .map(|(stem, src)| ((*stem).to_string(), src.as_bytes().to_vec()))
                    .collect(),
            }],
            nodes: Vec::new(),
            topology: None,
        };
        build(&tree).unwrap_or_else(|e| unreachable!("fixture model: {e:?}"))
    }

    /// Looks up a topic's runtime expectations or marks a fixture bug.
    fn rt(plan: &RuntimePlan, topic: &str) -> TopicRuntime {
        plan.get(topic)
            .cloned()
            .unwrap_or_else(|| unreachable!("fixture topic `{topic}` present"))
    }

    /// REQ: REQ-CORE-045, REQ-CORE-041 — `@history: latest` defaults to `R = 1`
    /// with the documented triple-buffer pool (3); a sequential topic defaults to
    /// 8 but never below its own `R + 1` floor; an explicit `@pool_size` wins.
    ///
    /// This test previously asserted 2 and 5 — i.e. it pinned `R + 1` as the
    /// default and so *protected* the drift from `architecture.md` and
    /// `payload-pool.md` I-P6 rather than catching it.
    #[test]
    fn runtime_plan_pool_defaults() {
        let m = model(&[
            ("latest", "@history: latest\nx: u8\n"),
            ("implicit", "x: u8\n"),
            ("depth", "@history: 4\nx: u8\n"),
            ("deep", "@history: 12\nx: u8\n"),
            ("sized", "@pool_size: 9\n@history: 4\nx: u8\n"),
        ]);
        let plan = runtime_plan(&m);
        assert_eq!(rt(&plan, "control/latest").history_depth, 1);
        assert_eq!(
            rt(&plan, "control/latest").pool_size,
            3,
            "the documented triple buffer, not R+1"
        );
        assert_eq!(
            rt(&plan, "control/implicit").pool_size,
            3,
            "an undeclared @history is the `latest` case"
        );
        assert_eq!(rt(&plan, "control/depth").history_depth, 4);
        assert_eq!(rt(&plan, "control/depth").pool_size, 8);
        assert_eq!(
            rt(&plan, "control/deep").pool_size,
            13,
            "the default is a floor: @history: 12 needs R+1 = 13"
        );
        assert_eq!(rt(&plan, "control/sized").pool_size, 9);
    }

    /// REQ: REQ-CORE-040 — `@overflow: reject` resolves to the reject policy;
    /// the default is drop-oldest (`overwrite`).
    #[test]
    fn runtime_plan_overflow_policy() {
        let m = model(&[
            ("plain", "x: u8\n"),
            ("reject", "@overflow: reject\nx: u8\n"),
        ]);
        let plan = runtime_plan(&m);
        assert_eq!(
            rt(&plan, "control/plain").overflow,
            RuntimeOverflow::Overwrite
        );
        assert_eq!(
            rt(&plan, "control/reject").overflow,
            RuntimeOverflow::Reject
        );
    }

    /// REQ: REQ-CORE-015 — descriptor expectations mirror the model's schema and
    /// layout hashes (the runtime validates these once at attach).
    #[test]
    fn runtime_plan_descriptor_hashes_match_model() {
        let m = model(&[("target", "a: u8\nb: u64\n")]);
        let plan = runtime_plan(&m);
        let entry = &m.topics["control/target"];
        let expectations = rt(&plan, "control/target");
        assert_eq!(expectations.schema_hash, entry.schema_hash);
        assert_eq!(
            expectations.layout_hash,
            crate::model::layout_hash(&entry.layout)
        );
        assert_eq!(expectations.payload_capacity, entry.layout.sample_max);
    }

    /// REQ: REQ-TOOL-057 — `channel_expectations` mirrors the runtime plan's
    /// pool/bounds for each topic (the V-MEM/V-RT model hook).
    #[test]
    fn channel_expectations_mirror_runtime_plan() {
        let m = model(&[("sized", "@pool_size: 9\n@history: 4\nx: u8\n")]);
        let expectations = channel_expectations(&m);
        let e = expectations
            .get("control/sized")
            .unwrap_or_else(|| unreachable!("fixture topic present"));
        assert_eq!(e.topic, "control/sized");
        assert_eq!(e.pool_size, 9);
        assert_eq!(e.history_depth, 4);
        assert_eq!(
            e.payload_capacity,
            m.topics["control/sized"].layout.sample_max
        );
    }
}
