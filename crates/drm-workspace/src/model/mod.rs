//! `WorkspaceModel` construction — start slice (work-plan M02W1-E2-T3): topics
//! parse→model, sorted discovery, error collection; node manifests and the
//! topology manifest follow (M02W3-E2-T2, M02W4-E2-T2). Structural integrity
//! only; policy checks belong to `drm verify` (REQ-TOOL-070). Total-reload
//! build ([workspace-model.md](../../docs/algorithms/toolchain/workspace-model.md)):
//! every parse rebuilds the full model; no incremental caches.
//! REQ: REQ-TOOL-045, REQ-TOOL-047, REQ-TOOL-013.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use crate::canon::{self, StructLayout};
use crate::parser::{self, TopicFile};

pub mod fingerprint;
pub mod handles;
pub mod nodes;
pub mod runtime;
pub mod topology;

pub use fingerprint::{
    ContractFingerprint, FINGERPRINT_FORMAT_VERSION, conflicting_publishers, contract_fingerprint,
    layout_hash,
};
pub use nodes::{NodeManifest, parse_node_manifest};
pub use runtime::{RuntimeOverflow, RuntimePlan, TopicRuntime, runtime_plan, topic_runtime};
pub use topology::{ComputeEntry, LinkEntry, TopologyManifest, parse_topology_manifest};

/// Canonical topic path: `package/stem`, lowercase ASCII (REQ-TOOL-056).
pub type TopicName = String;
/// Package directory identity.
pub type PackageName = String;
/// Node manifest identity.
pub type NodeName = String;

/// Topic attributes extracted into typed model data (work-plan M02W2-E2-T3).
/// Grammar-level validation happened in the parser; the model carries the
/// normalized values. REQ: REQ-TOOL-031, REQ-TOOL-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TopicAttrs {
    /// `@history: <depth>` sample count; `None` when absent or `latest`.
    pub history_depth: Option<u64>,
    /// `@history: latest` (unbounded-latest retention).
    pub history_latest: bool,
    /// `@pool_size: <n>` payload-pool slots.
    pub pool_size: Option<u64>,
    /// `@overflow: overwrite` policy.
    pub overflow_overwrite: bool,
    /// `@overflow: reject` policy.
    pub overflow_reject: bool,
}

/// A parsed topic: canonical AST, schema content hash, canonical layout.
/// REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicEntry {
    /// Parsed definition (semantic AST, declaration order preserved).
    pub schema: TopicFile,
    /// Schema content hash over the canonical render — BLAKE3 truncated to
    /// 128 bits (pinned at the contract-fingerprint phase,
    /// workspace-model.md). REQ: REQ-TOOL-023.
    pub schema_hash: [u8; 16],
    /// Canonical two-region layout ([canon-codec.md](../../docs/algorithms/canon-codec.md)).
    pub layout: StructLayout,
    /// Normalized topic attributes (REQ-TOOL-031).
    pub attrs: TopicAttrs,
}

/// Normalized topic attributes from `@history`/`@pool_size`/`@overflow`
/// (the parser enforces one value per key and closed vocabularies).
/// REQ: REQ-TOOL-031.
/// Package-level metadata: the sorted set of its topics (I-D1).
/// REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageMeta {
    /// Canonical topic paths declared by this package.
    pub topics: BTreeSet<TopicName>,
}

/// Node interface declaration (structural slice: publish/consume sets plus the
/// optional declared language and native build adapter; timing metadata and
/// spatial contracts land with manifest parsing growth).
/// REQ: REQ-TOOL-047, REQ-TOOL-012.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeEntry {
    /// Topics this node publishes (single-writer is a verify check, not here).
    pub publishes: BTreeSet<TopicName>,
    /// Topics this node consumes.
    pub consumes: BTreeSet<TopicName>,
    /// Declared implementation language, when present.
    pub language: Option<crate::orchestrator::Language>,
    /// Declared native build adapter (`[build]`), when present.
    pub build: Option<crate::orchestrator::BuildAdapter>,
}

/// In-memory workspace model. `BTree*` everywhere so filesystem read order can
/// never leak into model output (I-D1). REQ: REQ-TOOL-045, REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkspaceModel {
    /// Canonical path → definition.
    pub topics: BTreeMap<TopicName, TopicEntry>,
    /// Package identity → metadata.
    pub packages: BTreeMap<PackageName, PackageMeta>,
    /// Node identity → interfaces.
    pub nodes: BTreeMap<NodeName, NodeEntry>,
    /// Compute vertices: name → identity (placement targets). A map, not a
    /// set, so per-compute data can be added without changing the type; the
    /// value is zero-sized for now.
    #[allow(clippy::zero_sized_map_values)]
    pub computes: BTreeMap<String, ComputeEntry>,
    /// Undirected compute↔compute links, sorted by id after build (I-D1);
    /// `Vec` because duplicate ids are reported, not keyed (I-D2).
    pub links: Vec<LinkEntry>,
    /// Node placement: node → compute.
    pub placement: BTreeMap<String, String>,
}

/// Build failure with location context. REQ: REQ-TOOL-045.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// Same canonical topic path defined twice (both sites named).
    DuplicateTopic {
        /// Canonical topic path.
        topic: TopicName,
        /// First defining package.
        first: PackageName,
        /// Second defining package.
        second: PackageName,
    },
    /// Same node identity declared twice (both file stems named).
    DuplicateNode {
        /// Declared node name.
        node: NodeName,
        /// First defining file stem.
        first: String,
        /// Second defining file stem.
        second: String,
    },
    /// Two node directories share a name. Discovery keys nodes by directory
    /// name, so the collision would otherwise silently drop one node's
    /// manifest before any identity check runs.
    DuplicateNodeDir {
        /// The shared directory name.
        stem: String,
    },
    /// Topic file failed schema parsing or layout derivation; carries every
    /// diagnostic.
    Parse {
        /// Canonical topic path of the failing file.
        topic: TopicName,
        /// Closed-vocabulary diagnostics with spans.
        diagnostics: Vec<parser::Diagnostic>,
    },
    /// Node manifest failed TOML parsing or surface validation.
    NodeManifest {
        /// File stem of the failing manifest.
        stem: String,
        /// Defect description with location from the TOML parser.
        message: String,
    },
    /// Node references a topic that no package defines.
    UnknownTopic {
        /// Referencing node.
        node: NodeName,
        /// Referenced topic path.
        topic: TopicName,
    },
    /// Topology manifest failed TOML parsing or surface validation.
    Topology {
        /// Defect description with location from the TOML parser.
        message: String,
    },
    /// A link endpoint references a compute that `[computes]` does not declare.
    UnknownLinkEndpoint {
        /// Link id.
        link: String,
        /// Referenced, undeclared compute name.
        vertex: String,
    },
    /// The same link id is declared twice.
    DuplicateLink {
        /// Repeated link id.
        link: String,
    },
    /// A placement names a node that no manifest declares.
    DanglingPlacement {
        /// Placed node name.
        node: NodeName,
    },
    /// A placement targets a compute that `[computes]` does not declare.
    PlacementUnknownCompute {
        /// Placed node name.
        node: NodeName,
        /// Referenced, undeclared compute name.
        compute: String,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTopic {
                topic,
                first,
                second,
            } => {
                write!(f, "duplicate topic `{topic}` in `{first}` and `{second}`")
            }
            Self::DuplicateNode {
                node,
                first,
                second,
            } => {
                write!(f, "duplicate node `{node}` in `{first}` and `{second}`")
            }
            Self::DuplicateNodeDir { stem } => {
                write!(
                    f,
                    "duplicate node directory `{stem}`: node directories are unique by name across packages"
                )
            }
            Self::Parse { topic, diagnostics } => {
                // Render the reasons, never just a count: a build failure says
                // what is wrong with the schema (`unknown-type` at 1:4), which is
                // the only thing the author can act on. The diagnostic text is
                // `<line>:<col>: error: <reason>` — the file is the topic's own
                // `.drm`, already named on the left.
                write!(f, "topic `{topic}`: {} defect(s)", diagnostics.len())?;
                for diagnostic in diagnostics {
                    write!(
                        f,
                        "\n  {}:{}: error: {}",
                        diagnostic.span.line, diagnostic.span.col, diagnostic.reason
                    )?;
                }
                Ok(())
            }
            Self::NodeManifest { stem, message } => write!(f, "node `{stem}`: {message}"),
            Self::UnknownTopic { node, topic } => {
                write!(f, "node `{node}` references undefined topic `{topic}`")
            }
            Self::Topology { message } => write!(f, "topology manifest: {message}"),
            Self::UnknownLinkEndpoint { link, vertex } => {
                write!(f, "link `{link}` references undefined compute `{vertex}`")
            }
            Self::DuplicateLink { link } => write!(f, "duplicate link `{link}`"),
            Self::DanglingPlacement { node } => {
                write!(f, "placement references undefined node `{node}`")
            }
            Self::PlacementUnknownCompute { node, compute } => {
                write!(
                    f,
                    "placement of node `{node}` references undefined compute `{compute}`"
                )
            }
        }
    }
}

/// One package's input: name + topic files (stem → source bytes), in arbitrary
/// filesystem order. REQ: REQ-TOOL-045.
#[derive(Debug, Clone)]
pub struct PackageInput {
    /// Package directory name.
    pub name: PackageName,
    /// `(file stem, source bytes)` pairs; order carries no meaning.
    pub topics: Vec<(String, Vec<u8>)>,
}

/// Whole-tree input: packages and node manifests in arbitrary filesystem
/// order. REQ: REQ-TOOL-045.
#[derive(Debug, Clone, Default)]
pub struct TreeInput {
    /// Package inputs; order carries no meaning.
    pub packages: Vec<PackageInput>,
    /// `(file stem, TOML manifest bytes)` pairs; order carries no meaning.
    pub nodes: Vec<(String, Vec<u8>)>,
    /// Topology manifest TOML bytes; `None` ⇒ no computes/links/placement.
    pub topology: Option<Vec<u8>>,
}

/// Total-reload build: parse every topic file, collect every failure (I-D2),
/// populate only sorted maps (I-D1). REQ: REQ-TOOL-045, REQ-TOOL-047.
///
/// # Errors
/// `Vec<ModelError>` — every duplicate site and every parse/layout defect.
pub fn build(tree: &TreeInput) -> Result<WorkspaceModel, Vec<ModelError>> {
    let mut model = WorkspaceModel::default();
    let mut errs: Vec<ModelError> = Vec::new();

    // Sorted discovery (I-D1): filesystem order never reaches the model.
    // Package identity is a directory name ⇒ unique by construction.
    let mut packages: BTreeMap<&PackageName, &PackageInput> = BTreeMap::new();
    for pkg in &tree.packages {
        packages.insert(&pkg.name, pkg);
    }

    for (name, pkg) in &packages {
        let mut stems: BTreeMap<&str, &Vec<u8>> = BTreeMap::new();
        for (stem, src) in &pkg.topics {
            if stems.insert(stem.as_str(), src).is_some() {
                // Same file stem twice in one package ⇒ same topic path
                // defined twice; both sites are the same directory (I-D2).
                errs.push(ModelError::DuplicateTopic {
                    topic: format!("{name}/{stem}"),
                    first: (*name).clone(),
                    second: (*name).clone(),
                });
            }
        }
        for (stem, src) in &stems {
            let topic: TopicName = format!("{name}/{stem}");
            match parser::parse_bytes(src) {
                Ok(schema) => match canon::layout_struct(&schema.fields) {
                    Ok(layout) => {
                        let attrs = attrs_of(&schema);
                        model.topics.insert(
                            topic.clone(),
                            TopicEntry {
                                schema_hash: schema_hash(&schema),
                                schema,
                                layout,
                                attrs,
                            },
                        );
                        model
                            .packages
                            .entry((*name).clone())
                            .or_default()
                            .topics
                            .insert(topic);
                    }
                    Err(_) => {
                        // Layout cap overflow (sample_max > u32::MAX): the
                        // closed-vocabulary match is size-overflow.
                        errs.push(ModelError::Parse {
                            topic,
                            diagnostics: vec![parser::Diagnostic {
                                span: parser::Span {
                                    line: 1,
                                    col: 1,
                                    offset: 0,
                                },
                                reason: parser::Reason::SizeOverflow,
                            }],
                        });
                    }
                },
                Err(diagnostics) => errs.push(ModelError::Parse { topic, diagnostics }),
            }
        }
    }

    // Node manifests: sorted by file stem (I-D1); declared identity is
    // unique (I-D2); references checked against the parsed topics.
    // Structural integrity only — policy checks stay in `drm verify` (I-D12).
    build_nodes(&tree.nodes, &mut model, &mut errs);
    build_topology(tree, &mut model, &mut errs);

    if errs.is_empty() {
        Ok(model)
    } else {
        Err(errs)
    }
}

/// Parses node manifests into the model: sorted by file stem (I-D1), declared
/// identity unique (I-D2), publish/consume references checked against the
/// parsed topics, and every defect collected. REQ: REQ-TOOL-047, REQ-TOOL-045.
fn build_nodes(
    sources: &[(String, Vec<u8>)],
    model: &mut WorkspaceModel,
    errs: &mut Vec<ModelError>,
) {
    let mut stems: BTreeMap<&str, &Vec<u8>> = BTreeMap::new();
    for (stem, src) in sources {
        // A repeated directory name would otherwise overwrite the earlier
        // manifest here, silently dropping that node before the identity check
        // below can see it. REQ: REQ-TOOL-045.
        if stems.insert(stem.as_str(), src).is_some() {
            errs.push(ModelError::DuplicateNodeDir { stem: stem.clone() });
        }
    }
    let mut declared: BTreeMap<NodeName, &str> = BTreeMap::new();
    for (stem, src) in &stems {
        match nodes::parse_node_manifest(stem, src) {
            Ok(manifest) => {
                if let Some(first) = declared.get(&manifest.name) {
                    errs.push(ModelError::DuplicateNode {
                        node: manifest.name.clone(),
                        first: (*first).to_string(),
                        second: (*stem).to_string(),
                    });
                    continue;
                }
                declared.insert(manifest.name.clone(), stem);
                let mut topics = manifest.publishes.clone();
                topics.extend(manifest.consumes.clone());
                for topic in &topics {
                    if !model.topics.contains_key(topic) {
                        errs.push(ModelError::UnknownTopic {
                            node: manifest.name.clone(),
                            topic: topic.clone(),
                        });
                    }
                }
                model.nodes.insert(
                    manifest.name,
                    NodeEntry {
                        publishes: manifest.publishes,
                        consumes: manifest.consumes,
                        language: manifest.language,
                        build: manifest.build,
                    },
                );
            }
            Err(message) => errs.push(ModelError::NodeManifest {
                stem: (*stem).to_string(),
                message,
            }),
        }
    }
}

/// Parses the optional topology manifest into the model: computes, links
/// (sorted by id), placement; every referential defect is collected with the
/// other build defects (I-D2), structural integrity only (I-D12).
/// REQ: REQ-TOOL-047, REQ-TOOL-070.
fn build_topology(tree: &TreeInput, model: &mut WorkspaceModel, errs: &mut Vec<ModelError>) {
    let Some(src) = tree.topology.as_deref() else {
        return;
    };
    let manifest = match topology::parse_topology_manifest(src) {
        Ok(manifest) => manifest,
        Err(message) => {
            errs.push(ModelError::Topology { message });
            return;
        }
    };
    for name in &manifest.computes {
        model.computes.insert(name.clone(), ComputeEntry);
    }
    let mut links = manifest.links;
    links.sort_by(|a, b| a.id.cmp(&b.id));
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for link in &links {
        for vertex in [&link.a, &link.b] {
            if !manifest.computes.contains(vertex) {
                errs.push(ModelError::UnknownLinkEndpoint {
                    link: link.id.clone(),
                    vertex: vertex.clone(),
                });
            }
        }
        if !seen.insert(link.id.as_str()) {
            errs.push(ModelError::DuplicateLink {
                link: link.id.clone(),
            });
        }
    }
    for (node, compute) in &manifest.placement {
        if !model.nodes.contains_key(node) {
            errs.push(ModelError::DanglingPlacement { node: node.clone() });
        }
        if !manifest.computes.contains(compute) {
            errs.push(ModelError::PlacementUnknownCompute {
                node: node.clone(),
                compute: compute.clone(),
            });
        }
    }
    if errs.is_empty() {
        model.links = links;
        model.placement = manifest.placement;
    }
}

/// Deterministic model render (I-D1): packages in sorted order, per-package
/// topics sorted, schema hash + canonical schema render per topic, nodes in
/// sorted order, computes/links/placement sorted after the node sections. The
/// exact format is internal; the contract is that identical trees render to
/// byte-identical text under any input order. REQ: REQ-TOOL-045.
#[must_use]
pub fn render_model(m: &WorkspaceModel) -> String {
    let mut out = String::new();
    for (name, meta) in &m.packages {
        out.push_str("package ");
        out.push_str(name);
        out.push('\n');
        for topic in &meta.topics {
            let Some(entry) = m.topics.get(topic) else {
                unreachable!("package topic set is derived from the model itself");
            };
            out.push_str("topic ");
            out.push_str(topic);
            out.push('\n');
            for b in entry.schema_hash {
                let _ = write!(out, "{b:02x}");
            }
            out.push('\n');
            out.push_str(&parser::render_canonical(&entry.schema));
            out.push('\n');
            out.push_str("end\n");
        }
    }
    for (name, node) in &m.nodes {
        out.push_str("node ");
        out.push_str(name);
        out.push('\n');
        out.push_str("publishes");
        for t in &node.publishes {
            out.push(' ');
            out.push_str(t);
        }
        out.push('\n');
        out.push_str("consumes");
        for t in &node.consumes {
            out.push(' ');
            out.push_str(t);
        }
        out.push('\n');
        out.push_str("end\n");
    }
    for name in m.computes.keys() {
        out.push_str("compute ");
        out.push_str(name);
        out.push('\n');
    }
    for link in &m.links {
        out.push_str("link ");
        out.push_str(&link.id);
        out.push(' ');
        out.push_str(&link.a);
        out.push(' ');
        out.push_str(&link.b);
        out.push('\n');
    }
    for (node, compute) in &m.placement {
        out.push_str("place ");
        out.push_str(node);
        out.push(' ');
        out.push_str(compute);
        out.push('\n');
    }
    out
}

/// Schema content hash over the canonical render text: BLAKE3 truncated to
/// 128 bits (pinned with the contract-fingerprint phase; width parity with
/// `ContractFingerprint`/`layout_hash`). REQ: REQ-TOOL-023.
#[must_use]
pub fn schema_hash(schema: &TopicFile) -> [u8; 16] {
    drm_types::blake3_128(parser::render_canonical(schema).as_bytes())
}

/// Extracts topic attributes into normalized [`TopicAttrs`] (grammar-level
/// validation already happened in the parser: one value per key, closed
/// vocabularies). REQ: REQ-TOOL-031.
#[must_use]
pub fn attrs_of(schema: &TopicFile) -> TopicAttrs {
    let mut a = TopicAttrs::default();
    for attr in &schema.attrs {
        match attr.value {
            parser::AttrValue::Latest => a.history_latest = true,
            parser::AttrValue::Depth(n) => a.history_depth = Some(u64::from(n)),
            parser::AttrValue::Size(n) => a.pool_size = Some(u64::from(n)),
            parser::AttrValue::Overwrite => a.overflow_overwrite = true,
            parser::AttrValue::Reject => a.overflow_reject = true,
            parser::AttrValue::Encoding(_) => {} // carried in the schema AST
        }
    }
    a
}
