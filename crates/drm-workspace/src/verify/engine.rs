//! Static verification engine ([verify-engine.md](../../../docs/algorithms/toolchain/verify-engine.md)
//! §Functions `verify`): the deterministic fact catalogue — `V-SCH` schemas,
//! `V-SW` single writer, `V-PLC` placement, `V-RT` route existence + framing
//! fit, `V-REL` reliability capability, `V-MEM` budgets, `V-CMP` consumer
//! compatibility, `V-ID` identity, `V-SPAT` spatial contracts, `V-RPC` RPC
//! route reliability + shape, `V-REC` recorder backpressure declaration. No
//! short-circuit: every check contributes all its failures (I-V5); the report
//! never labels an estimate a fact (I-V4).
//! REQ: REQ-TOOL-057, REQ-TOOL-058, REQ-TOOL-059, REQ-TOOL-060,
//! REQ-TOOL-061, REQ-TOOL-062, REQ-TOOL-063, REQ-TOOL-064, REQ-SPATIAL-030,
//! REQ-SPATIAL-018, REQ-SPATIAL-037, REQ-OPS-024, REQ-OPS-008.

#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::fmt;

use crate::budget::{self, DeploymentCost};
use crate::model::handles::HandleTable;
use crate::model::runtime::channel_expectations;
use crate::model::{self, WorkspaceModel};
use crate::topology::{self, Link, LinkId, Overrides, RouteGraph, VertexId};

/// Stable check identifier cited in diagnostics, e.g. `V-SW` (verify-engine.md
/// §Data Structures). REQ: REQ-TOOL-057.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CheckId(
    /// The stable check id text.
    pub &'static str,
);

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `V-SCH` — schemas valid and consistent. REQ: REQ-TOOL-063.
pub const V_SCH: CheckId = CheckId("V-SCH");
/// `V-SW` — every consumed topic has exactly one publisher. REQ: REQ-TOOL-064.
pub const V_SW: CheckId = CheckId("V-SW");
/// `V-PLC` — placements/computes/links valid. REQ: REQ-TOOL-060.
pub const V_PLC: CheckId = CheckId("V-PLC");
/// `V-RT` — routes exist; encoded frames fit `max_datagram_payload`.
/// REQ: REQ-TOOL-062.
pub const V_RT: CheckId = CheckId("V-RT");
/// `V-REL` — `reliable` only on reliable-capability transports.
/// REQ: REQ-TOOL-061.
pub const V_REL: CheckId = CheckId("V-REL");
/// `V-MEM` — channels fit the deployment memory budget. REQ: REQ-TOOL-057.
pub const V_MEM: CheckId = CheckId("V-MEM");
/// `V-CMP` — consumer requirements on shared topics mutually compatible.
/// REQ: REQ-TOOL-058.
pub const V_CMP: CheckId = CheckId("V-CMP");
/// `V-ID` — duplicate topic/node/compute names; handle table conflicts.
/// REQ: REQ-TOOL-060.
pub const V_ID: CheckId = CheckId("V-ID");
/// `V-SPAT` — spatial contracts: single writer per dynamic edge, declared
/// reads, index capacity within budget, satisfiable history bounds.
/// REQ: REQ-SPATIAL-030.
pub const V_SPAT: CheckId = CheckId("V-SPAT");
/// `V-RPC` — generated RPC topics route reliably, event topic single-writer
/// server-owned, concurrency within capacity. REQ: REQ-OPS-024.
pub const V_RPC: CheckId = CheckId("V-RPC");
/// `V-REC` — recorder `on_full = backpressure` on a `reject` topic declares a
/// required sequential consumer. REQ: REQ-OPS-008.
pub const V_REC: CheckId = CheckId("V-REC");

/// Every catalogue check id, in report order (I-V1 catalogue completeness).
/// REQ: REQ-TOOL-057.
pub const ALL_CHECKS: [CheckId; 11] = [
    V_SCH, V_SW, V_PLC, V_RT, V_REL, V_MEM, V_CMP, V_ID, V_SPAT, V_RPC, V_REC,
];

/// One deterministic fact failure: check id + preformatted message carrying a
/// REQ citation (I-V3). REQ: REQ-TOOL-059.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    /// Check that produced the failure.
    pub check: CheckId,
    /// `"{check}: {message} [REQ: ...]"`.
    pub message: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A single fact: the check passed, or it failed with a cited message.
/// REQ: REQ-TOOL-059.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactResult {
    /// The check found no negative.
    Pass(CheckId),
    /// The check rejected the workspace.
    Fail(VerifyError),
}

/// A non-fatal diagnostic (printed, never gating). REQ: REQ-TOOL-059.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Check that emitted the warning.
    pub check: CheckId,
    /// Human-readable message.
    pub message: String,
}

/// An explicitly labeled engineering estimate — never a guarantee (I-V4).
/// REQ: REQ-TOOL-059.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Estimate {
    /// Check that emitted the estimate.
    pub check: CheckId,
    /// Message prefixed `ESTIMATE`.
    pub message: String,
}

/// Full verification report: facts (deterministic pass/fail), warnings, and
/// estimates. One run reports every check (I-V5). REQ: REQ-TOOL-059.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Deterministic facts, in catalogue order.
    pub facts: Vec<FactResult>,
    /// Non-fatal diagnostics.
    pub warnings: Vec<Warning>,
    /// Labeled estimates (never facts).
    pub estimates: Vec<Estimate>,
}

impl VerifyReport {
    /// True iff no fact failed; `drm verify` exits non-zero otherwise.
    /// REQ: REQ-TOOL-059.
    #[must_use]
    pub fn passes(&self) -> bool {
        !self.facts.iter().any(|f| matches!(f, FactResult::Fail(_)))
    }

    /// All failures in report order. REQ: REQ-TOOL-059.
    #[must_use]
    pub fn failures(&self) -> Vec<&VerifyError> {
        self.facts
            .iter()
            .filter_map(|f| match f {
                FactResult::Fail(e) => Some(e),
                FactResult::Pass(_) => None,
            })
            .collect()
    }

    /// Records a pass for `check`. REQ: REQ-TOOL-059.
    pub fn pass(&mut self, check: CheckId) {
        self.facts.push(FactResult::Pass(check));
    }

    /// Records a failure, formatting `"{check}: {msg} [REQ: {reqs}]"` (I-V3).
    /// REQ: REQ-TOOL-059.
    pub fn fail(&mut self, check: CheckId, msg: impl fmt::Display, reqs: &[&str]) {
        self.facts.push(FactResult::Fail(fail(check, msg, reqs)));
    }

    /// Records a preformatted failure (the message already carries the check id
    /// and a REQ citation). REQ: REQ-TOOL-059.
    pub fn fail_raw(&mut self, check: CheckId, message: String) {
        self.facts
            .push(FactResult::Fail(VerifyError { check, message }));
    }

    /// Records a warning. REQ: REQ-TOOL-059.
    pub fn warn(&mut self, check: CheckId, msg: impl fmt::Display) {
        self.warnings.push(Warning {
            check,
            message: format!("{check}: {msg}"),
        });
    }

    /// Records a preformatted warning. REQ: REQ-TOOL-059.
    pub fn warn_raw(&mut self, check: CheckId, message: String) {
        self.warnings.push(Warning { check, message });
    }

    /// Records a labeled estimate (I-V4). REQ: REQ-TOOL-059.
    pub fn estimate(&mut self, check: CheckId, msg: impl fmt::Display) {
        self.estimates.push(Estimate {
            check,
            message: format!("ESTIMATE: {check}: {msg}"),
        });
    }
}

/// Formats a cited failure (verify-engine.md §Functions `fail`).
/// REQ: REQ-TOOL-059.
fn fail(check: CheckId, msg: impl fmt::Display, reqs: &[&str]) -> VerifyError {
    VerifyError {
        check,
        message: format!("{check}: {msg} [REQ: {}]", reqs.join(", ")),
    }
}

/// Transport capability class of a link, used by `V-REL`/`V-RT`.
/// REQ: REQ-TOOL-061.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Datagram best-effort (UDP).
    Udp,
    /// Ordered reliable stream (TCP).
    Tcp,
    /// Serial best-effort (COBS-framed).
    Serial,
    /// Same-compute local delivery.
    Local,
}

impl Transport {
    /// True iff the transport can carry `reliable` traffic (verify-engine.md
    /// V-REL). REQ: REQ-TOOL-061.
    #[must_use]
    pub const fn reliable_capable(self) -> bool {
        matches!(self, Self::Tcp | Self::Local)
    }

    /// True iff the transport has a datagram payload bound (V-RT framing fit).
    /// REQ: REQ-TOOL-062.
    #[must_use]
    pub const fn datagram(self) -> bool {
        matches!(self, Self::Udp | Self::Serial)
    }
}

/// Declared capability of one physical link. REQ: REQ-TOOL-062.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkCapability {
    /// Transport class.
    pub transport: Transport,
    /// Datagram payload bound, or `None` for stream transports.
    pub max_datagram_payload: Option<u64>,
    /// Declared physical line capacity in bits/s (feasibility; informational
    /// here). REQ: REQ-TOOL-062.
    pub capacity_bps: u64,
}

/// Consumer reliability requirement (verify-engine.md V-REL).
/// REQ: REQ-TOOL-061.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reliability {
    /// Best-effort delivery accepted.
    #[default]
    BestEffort,
    /// Ordered reliable delivery required.
    Reliable,
}

/// Consumer read mode (workspace-model.md I-D11). REQ: REQ-TOOL-058.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadMode {
    /// Newest retained sample.
    #[default]
    Latest,
    /// Sequential ring read.
    Sequential,
}

/// One consumer's declared requirement on a topic (V-CMP). REQ: REQ-TOOL-058.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerDecl {
    /// Consuming node.
    pub node: String,
    /// Declared read mode.
    pub read_mode: ReadMode,
    /// Required sequential consumer (overflow-policies.md registry sizing).
    pub required: bool,
    /// Declared deadline in nanoseconds, if any.
    pub deadline_ns: Option<u64>,
}

/// Deployment-declared metadata for one topic: reliability requirement, nominal
/// publisher period, and consumer requirements. REQ: REQ-TOOL-058.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TopicDecl {
    /// Reliability requirement routed against link capability (V-REL).
    pub reliability: Reliability,
    /// Publisher nominal period in nanoseconds (feasibility estimate only).
    pub expected_period_ns: Option<u64>,
    /// Consumer requirements (V-CMP).
    pub consumers: Vec<ConsumerDecl>,
}

/// Declared dynamic spatial edge (V-SPAT): writers, transform, and reads.
/// REQ: REQ-SPATIAL-030.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpatialEdgeDecl {
    /// Edge identity.
    pub edge: String,
    /// Writers on this dynamic edge (single-writer requires 0 or 1).
    pub writers: u32,
    /// Whether a transform is declared directly on the same edge.
    pub transform: bool,
    /// Referenced `[spatial] read`s of this edge.
    pub reads: u32,
    /// Whether the reading nodes declare the read.
    pub declared_read: bool,
}

/// Declared spatial index capacity vs deployment budget (V-SPAT, I-MB9).
/// REQ: REQ-SPATIAL-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpatialIndexDecl {
    /// `index.drm` capacity in bytes.
    pub index_bytes: u64,
    /// Memory budget available to the index in bytes.
    pub budget_bytes: u64,
}

/// Declared transform-history requirement (V-SPAT structural bound).
/// REQ: REQ-SPATIAL-037.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoryDecl {
    /// Edge identity.
    pub edge: String,
    /// Declared history window `H` in nanoseconds.
    pub history_ns: u64,
    /// Declared minimum sample period `P` in nanoseconds (`0` treated as `1`).
    pub min_sample_period_ns: u64,
    /// Declared fixed memory capacity `N`.
    pub max_samples: u64,
}

/// Declared spatial contract surface (V-SPAT). REQ: REQ-SPATIAL-030.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpatialDecl {
    /// Dynamic edges.
    pub edges: Vec<SpatialEdgeDecl>,
    /// Index capacity vs budget, when an index is declared.
    pub index: Option<SpatialIndexDecl>,
    /// History requirements.
    pub histories: Vec<HistoryDecl>,
    /// Spatial-typed schema fields carrying naked coordinates.
    pub naked_coordinate_fields: Vec<String>,
}

/// Declared RPC service surface (V-RPC). REQ: REQ-OPS-024.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RpcDecl {
    /// Service identity.
    pub service: String,
    /// Generated request topic.
    pub request_topic: String,
    /// Generated event topic (single-writer server-owned).
    pub event_topic: String,
    /// Declared concurrency.
    pub concurrency: u32,
    /// Capacity-model bound.
    pub capacity: u32,
}

/// Recorder `on_full` policy (V-REC). REQ: REQ-OPS-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecorderFull {
    /// Drop samples (loss ledger).
    #[default]
    Drop,
    /// Stop recording.
    Stop,
    /// Publisher-gating backpressure.
    Backpressure,
}

/// Declared recorder (V-REC). REQ: REQ-OPS-008.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecorderDecl {
    /// Recorded topic.
    pub topic: String,
    /// `on_full` policy.
    pub on_full: RecorderFull,
    /// Whether the recorder is declared as a required sequential consumer.
    pub required_sequential: bool,
}

/// Deployment configuration consumed by `drm verify`: memory budgets, link
/// capabilities, per-topic requirements, spatial/RPC/recorder declarations, and
/// the optional committed handle table. REQ: REQ-TOOL-057.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentConfig {
    /// Per-compute memory budgets and channel costs (V-MEM).
    pub budgets: DeploymentCost,
    /// Per-link transport capability (V-RT/V-REL).
    pub links: BTreeMap<String, LinkCapability>,
    /// Per-topic requirements (V-CMP/V-REL).
    pub topics: BTreeMap<String, TopicDecl>,
    /// Committed handle table, when verifying a build with prior assignments
    /// (V-ID handle conflicts). REQ: REQ-TOOL-060.
    pub handles: Option<HandleTable>,
    /// Spatial contract declarations (V-SPAT). REQ: REQ-SPATIAL-030.
    pub spatial: SpatialDecl,
    /// RPC service declarations (V-RPC). REQ: REQ-OPS-024.
    pub rpc: Vec<RpcDecl>,
    /// Recorder declarations (V-REC). REQ: REQ-OPS-008.
    pub recorders: Vec<RecorderDecl>,
}

/// Builds the route-derivation graph from the model: computes, links (both
/// endpoints as computes), placement, and node publish/consume sets.
/// REQ: REQ-TOOL-062.
#[must_use]
pub fn route_graph(model: &WorkspaceModel) -> RouteGraph {
    let mut graph = RouteGraph {
        computes: model.computes.keys().cloned().collect(),
        links: model
            .links
            .iter()
            .map(|l| Link {
                id: LinkId(l.id.clone()),
                a: VertexId::Compute(l.a.clone()),
                b: VertexId::Compute(l.b.clone()),
            })
            .collect(),
        placement: model.placement.clone(),
        publishes: BTreeMap::new(),
        consumes: BTreeMap::new(),
        external: BTreeMap::new(),
        overrides: Overrides::default(),
    };
    for (node, entry) in &model.nodes {
        graph
            .publishes
            .insert(node.clone(), entry.publishes.clone());
        graph.consumes.insert(node.clone(), entry.consumes.clone());
    }
    graph
}

/// Runs the full deterministic catalogue over `model` (verify-engine.md
/// §Functions `verify`): every check contributes all failures, in catalogue
/// order, with no short-circuit (I-V5). REQ: REQ-TOOL-057, REQ-TOOL-063,
/// REQ-TOOL-064, REQ-TOOL-060, REQ-TOOL-062, REQ-TOOL-061, REQ-TOOL-058,
/// REQ-SPATIAL-030, REQ-OPS-024, REQ-OPS-008.
#[must_use]
pub fn verify(model: &WorkspaceModel, deploy: &DeploymentConfig) -> VerifyReport {
    let mut report = VerifyReport::default();
    let routes = topology::derive(&route_graph(model));
    check_schemas(model, &mut report);
    check_single_writer(model, &mut report);
    check_placement(model, &mut report);
    check_routes(model, &routes, deploy, &mut report);
    check_reliability(&routes, deploy, &mut report);
    check_memory(deploy, &mut report);
    check_compatibility(model, deploy, &mut report);
    check_identity(model, deploy, &mut report);
    check_spatial(deploy, &mut report);
    check_rpc(model, deploy, &mut report);
    check_recorder(model, deploy, &mut report);
    report
}

/// `V-SCH`: the stored schema hash of every topic equals the hash recomputed
/// from its canonical schema (cross-package consistency). REQ: REQ-TOOL-063.
fn check_schemas(model: &WorkspaceModel, report: &mut VerifyReport) {
    let mut failed = false;
    for (topic, entry) in &model.topics {
        if model::schema_hash(&entry.schema) != entry.schema_hash {
            report.fail(
                V_SCH,
                format!("topic `{topic}` stored schema hash does not match its canonical schema"),
                &["REQ-TOOL-063"],
            );
            failed = true;
        }
    }
    if !failed {
        report.pass(V_SCH);
    }
}

/// `V-SW`: every consumed topic has exactly one publisher (reuses the V-SW
/// slice). REQ: REQ-TOOL-064.
fn check_single_writer(model: &WorkspaceModel, report: &mut VerifyReport) {
    let findings = super::verify_lite(model);
    if findings.is_empty() {
        report.pass(V_SW);
    } else {
        for finding in findings {
            report.fail(V_SW, finding, &["REQ-TOOL-064"]);
        }
    }
}

/// `V-PLC`: every placement targets a declared compute and every link endpoint
/// is a declared compute (re-checked here so a hand-built model cannot slip
/// through). REQ: REQ-TOOL-060.
fn check_placement(model: &WorkspaceModel, report: &mut VerifyReport) {
    let mut failed = false;
    for (node, compute) in &model.placement {
        if !model.computes.contains_key(compute) {
            report.fail(
                V_PLC,
                format!("node `{node}` placed on unknown compute `{compute}`"),
                &["REQ-TOOL-060"],
            );
            failed = true;
        }
    }
    for link in &model.links {
        for vertex in [&link.a, &link.b] {
            if !model.computes.contains_key(vertex) {
                report.fail(
                    V_PLC,
                    format!("link `{}` references unknown compute `{vertex}`", link.id),
                    &["REQ-TOOL-060"],
                );
                failed = true;
            }
        }
    }
    if !failed {
        report.pass(V_PLC);
    }
}

/// `V-RT`: route plans exist (derivation errors are collected, I-V5) and every
/// encoded frame fits its datagram link's `max_datagram_payload`.
/// REQ: REQ-TOOL-062.
fn check_routes(
    model: &WorkspaceModel,
    routes: &topology::RouteResult,
    deploy: &DeploymentConfig,
    report: &mut VerifyReport,
) {
    let mut failed = false;
    for err in &routes.errors {
        report.fail(V_RT, err, &["REQ-TOOL-062"]);
        failed = true;
    }
    let expectations = channel_expectations(model);
    for (topic, plan) in &routes.plans {
        let Some(exp) = expectations.get(topic) else {
            continue;
        };
        for (compute, rule) in &plan.forwarding {
            for edge in &rule.egress {
                let Some(cap) = deploy.links.get(&edge.link.0) else {
                    continue; // transport unknown ⇒ cannot decide framing fit
                };
                if !cap.transport.datagram() {
                    continue;
                }
                if let Some(max) = cap.max_datagram_payload
                    && exp.payload_capacity > max
                {
                    report.fail(
                        V_RT,
                        format!(
                            "topic `{topic}` canon-encoded payload {} B exceeds `{}` max_datagram_payload {max} B (route at `{compute}`); move the topic to a stream transport (e.g. TCP) or raise the datagram bound",
                            exp.payload_capacity, edge.link.0
                        ),
                        &["REQ-TOOL-062", "REQ-ROUTE-018"],
                    );
                    failed = true;
                }
            }
        }
    }
    if !failed {
        report.pass(V_RT);
    }
}

/// `V-REL`: a topic requiring `reliable` must route only over reliable-capable
/// transports. REQ: REQ-TOOL-061.
fn check_reliability(
    routes: &topology::RouteResult,
    deploy: &DeploymentConfig,
    report: &mut VerifyReport,
) {
    let mut failed = false;
    for (topic, decl) in &deploy.topics {
        if decl.reliability != Reliability::Reliable {
            continue;
        }
        let Some(plan) = routes.plans.get(topic) else {
            continue;
        };
        for (compute, rule) in &plan.forwarding {
            for edge in &rule.egress {
                if let Some(cap) = deploy.links.get(&edge.link.0)
                    && !cap.transport.reliable_capable()
                {
                    report.fail(
                        V_REL,
                        format!(
                            "topic `{topic}` requires `reliable` but route at `{compute}` uses best-effort link `{}`",
                            edge.link.0
                        ),
                        &["REQ-TOOL-061"],
                    );
                    failed = true;
                }
            }
        }
    }
    if !failed {
        report.pass(V_REL);
    }
}

/// `V-MEM`: the memory-budget verifier's errors become facts; its warnings and
/// estimates are carried through unchanged (memory-budget.md
/// §`verify_memory`).
/// REQ: REQ-TOOL-057.
fn check_memory(deploy: &DeploymentConfig, report: &mut VerifyReport) {
    let budget = budget::verify_deployment(&deploy.budgets);
    let mut failed = false;
    for err in &budget.errors {
        report.fail_raw(V_MEM, err.to_string());
        failed = true;
    }
    for warn in &budget.warnings {
        report.warn_raw(V_MEM, warn.to_string());
    }
    for est in &budget.estimates {
        report.estimate(
            V_MEM,
            format!(
                "compute `{}` worst-case {} B, locked {} B, nominal {} B",
                est.compute, est.worst_case, est.locked, est.nominal
            ),
        );
    }
    if !failed {
        report.pass(V_MEM);
    }
}

/// `V-CMP`: `reject` topics need a required sequential consumer
/// (overflow-policies degenerate case). Different per-consumer deadlines are
/// independent requirements, never compared (I-V6); a deadline tighter than the
/// publisher's nominal period is a labeled estimate, not a failure.
/// REQ: REQ-TOOL-058.
fn check_compatibility(
    model: &WorkspaceModel,
    deploy: &DeploymentConfig,
    report: &mut VerifyReport,
) {
    let mut failed = false;
    for (topic, entry) in &model.topics {
        let decl = deploy.topics.get(topic);
        let required = decl.map_or(0, |d| d.consumers.iter().filter(|c| c.required).count());
        if entry.attrs.overflow_reject && required == 0 {
            report.fail(
                V_CMP,
                format!("topic `{topic}` uses `@overflow: reject` but declares no required sequential consumer"),
                &["REQ-TOOL-058"],
            );
            failed = true;
        }
        if let Some(decl) = decl
            && let Some(period) = decl.expected_period_ns
        {
            for consumer in &decl.consumers {
                if let Some(deadline) = consumer.deadline_ns
                    && deadline < period
                {
                    report.estimate(
                        V_CMP,
                        format!(
                            "consumer `{}` on `{topic}` declares deadline {deadline} ns tighter than publisher nominal period {period} ns (runtime misses measured by drm-diag)",
                            consumer.node
                        ),
                    );
                }
            }
        }
    }
    if !failed {
        report.pass(V_CMP);
    }
}

/// `V-ID`: topic paths are canonical lowercase ASCII, node names never collide
/// with compute names, and handle-table assignments are conflict-free.
/// REQ: REQ-TOOL-060.
fn check_identity(model: &WorkspaceModel, deploy: &DeploymentConfig, report: &mut VerifyReport) {
    let mut failed = false;
    for topic in model.topics.keys() {
        if !is_canonical_topic(topic) {
            report.fail(
                V_ID,
                format!("topic `{topic}` is not a canonical lowercase ASCII path"),
                &["REQ-TOOL-056"],
            );
            failed = true;
        }
    }
    for node in model.nodes.keys() {
        if model.computes.contains_key(node) {
            report.fail(
                V_ID,
                format!("node `{node}` collides with a compute name"),
                &["REQ-TOOL-060"],
            );
            failed = true;
        }
    }
    if let Some(handles) = &deploy.handles {
        let mut seen: BTreeMap<u32, &String> = BTreeMap::new();
        for (topic, id) in &handles.assignments {
            if let Some(previous) = seen.insert(*id, topic) {
                report.fail(
                    V_ID,
                    format!("handle {id} assigned to both `{previous}` and `{topic}`"),
                    &["REQ-TOOL-060"],
                );
                failed = true;
            }
        }
    }
    if !failed {
        report.pass(V_ID);
    }
}

/// `V-SPAT`: dynamic-edge single writer, write/transform conflict, declared
/// `[spatial]` reads, `index.drm` capacity within the memory budget, history
/// requirements satisfiable by the structural bound, and no naked-coordinate
/// fields in spatial-typed topics. REQ: REQ-SPATIAL-030, REQ-SPATIAL-018,
/// REQ-SPATIAL-037, REQ-SPATIAL-036.
fn check_spatial(deploy: &DeploymentConfig, report: &mut VerifyReport) {
    let mut failed = false;
    for edge in &deploy.spatial.edges {
        if edge.writers > 1 {
            report.fail(
                V_SPAT,
                format!(
                    "dynamic edge `{}` has {} writers (single-writer required)",
                    edge.edge, edge.writers
                ),
                &["REQ-SPATIAL-030"],
            );
            failed = true;
        }
        if edge.writers > 0 && edge.transform {
            report.fail(
                V_SPAT,
                format!(
                    "dynamic edge `{}` declares both a writer and a transform (write+transform conflict)",
                    edge.edge
                ),
                &["REQ-SPATIAL-030"],
            );
            failed = true;
        }
        if edge.reads > 0 && !edge.declared_read {
            report.fail(
                V_SPAT,
                format!(
                    "edge `{}` is read but no `[spatial] read` is declared",
                    edge.edge
                ),
                &["REQ-SPATIAL-030"],
            );
            failed = true;
        }
    }
    if let Some(index) = &deploy.spatial.index
        && index.index_bytes > index.budget_bytes
    {
        report.fail(
            V_SPAT,
            format!(
                "spatial index capacity {} B exceeds the memory budget {} B (I-MB9)",
                index.index_bytes, index.budget_bytes
            ),
            &["REQ-SPATIAL-018"],
        );
        failed = true;
    }
    for history in &deploy.spatial.histories {
        let period = history.min_sample_period_ns.max(1);
        let required = history.history_ns / period + 2;
        if history.max_samples < required {
            report.fail(
                V_SPAT,
                format!(
                    "edge `{}` history unsatisfiable: max_samples {} < floor(H/P)+2 = {required}",
                    history.edge, history.max_samples
                ),
                &["REQ-SPATIAL-037"],
            );
            failed = true;
        }
    }
    for field in &deploy.spatial.naked_coordinate_fields {
        report.fail(
            V_SPAT,
            format!(
                "spatial topic field `{field}` carries a naked coordinate (typed frame required)"
            ),
            &["REQ-SPATIAL-036"],
        );
        failed = true;
    }
    if !failed {
        report.pass(V_SPAT);
    }
}

/// `V-RPC`: generated request/event topics must be declared `reliable`, the
/// event topic must be single-writer server-owned, and declared concurrency
/// must fit the capacity model. REQ: REQ-OPS-024, REQ-OPS-012, REQ-OPS-021.
fn check_rpc(model: &WorkspaceModel, deploy: &DeploymentConfig, report: &mut VerifyReport) {
    let mut failed = false;
    for rpc in &deploy.rpc {
        for topic in [&rpc.request_topic, &rpc.event_topic] {
            if let Some(decl) = deploy.topics.get(topic)
                && decl.reliability != Reliability::Reliable
            {
                report.fail(
                    V_RPC,
                    format!(
                        "RPC service `{}` topic `{topic}` is routed best-effort-only (reliable required)",
                        rpc.service
                    ),
                    &["REQ-OPS-024"],
                );
                failed = true;
            }
        }
        if model.topics.contains_key(&rpc.event_topic) {
            let publishers = model
                .nodes
                .values()
                .filter(|n| n.publishes.contains(&rpc.event_topic))
                .count();
            if publishers != 1 {
                report.fail(
                    V_RPC,
                    format!(
                        "RPC event topic `{}` has {publishers} publishers (single-writer server-owned required)",
                        rpc.event_topic
                    ),
                    &["REQ-OPS-012"],
                );
                failed = true;
            }
        }
        if rpc.concurrency > rpc.capacity {
            report.fail(
                V_RPC,
                format!(
                    "RPC service `{}` declares concurrency {} above the capacity model {}",
                    rpc.service, rpc.concurrency, rpc.capacity
                ),
                &["REQ-OPS-021"],
            );
            failed = true;
        }
    }
    if !failed {
        report.pass(V_RPC);
    }
}

/// `V-REC`: a recorder declaring `on_full = backpressure` on a `reject` topic
/// gates the publisher and must be declared as a required sequential consumer
/// (record-replay.md). REQ: REQ-OPS-008.
fn check_recorder(model: &WorkspaceModel, deploy: &DeploymentConfig, report: &mut VerifyReport) {
    let mut failed = false;
    for recorder in &deploy.recorders {
        let reject = model
            .topics
            .get(&recorder.topic)
            .is_some_and(|entry| entry.attrs.overflow_reject);
        if recorder.on_full == RecorderFull::Backpressure && reject && !recorder.required_sequential
        {
            report.fail(
                V_REC,
                format!(
                    "recorder on `{}` uses `on_full = backpressure` on a `reject` topic without a declared required sequential consumer",
                    recorder.topic
                ),
                &["REQ-OPS-008"],
            );
            failed = true;
        }
    }
    if !failed {
        report.pass(V_REC);
    }
}

/// Canonical topic path: lowercase ASCII letters/digits and `/`, `_`, `-`, `.`
/// (workspace-model REQ-TOOL-056). REQ: REQ-TOOL-056.
fn is_canonical_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'_' | b'-' | b'.')
        })
}
