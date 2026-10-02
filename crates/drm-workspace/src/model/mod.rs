//! `WorkspaceModel` construction — start slice (work-plan M02W1-E2-T3): topics
//! parse→model, sorted discovery, error collection. Structural integrity only;
//! policy checks belong to `drm verify` (REQ-TOOL-070). Total-reload build
//! ([workspace-model.md](../../docs/algorithms/toolchain/workspace-model.md)):
//! every parse rebuilds the full model; no incremental caches.
//! REQ: REQ-TOOL-045, REQ-TOOL-047, REQ-TOOL-013.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use crate::canon::{self, StructLayout};
use crate::parser::{self, TopicFile};

/// Canonical topic path: `package/stem`, lowercase ASCII (REQ-TOOL-056).
pub type TopicName = String;
/// Package directory identity.
pub type PackageName = String;
/// Node manifest identity.
pub type NodeName = String;

/// A parsed topic: canonical AST, schema content hash, canonical layout.
/// REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicEntry {
    /// Parsed definition (semantic AST, declaration order preserved).
    pub schema: TopicFile,
    /// Schema content hash over the canonical render (REQ-TOOL-023);
    /// placeholder FNV-1a dual-lane, pinned to BLAKE3 at the CCB phase
    /// (`contract_fingerprint`, workspace-model.md).
    pub schema_hash: [u8; 16],
    /// Canonical two-region layout ([canon-codec.md](../../docs/algorithms/canon-codec.md)).
    pub layout: StructLayout,
}

/// Package-level metadata: the sorted set of its topics (I-D1).
/// REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageMeta {
    /// Canonical topic paths declared by this package.
    pub topics: BTreeSet<TopicName>,
}

/// Node interface declaration (start slice: struct only; node parsing lands
/// M02W2). REQ: REQ-TOOL-047.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeEntry {
    /// Topics this node publishes (single-writer is a verify check, not here).
    pub publishes: BTreeSet<TopicName>,
    /// Topics this node consumes.
    pub consumes: BTreeSet<TopicName>,
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
    /// Topic file failed schema parsing or layout derivation; carries every
    /// diagnostic.
    Parse {
        /// Canonical topic path of the failing file.
        topic: TopicName,
        /// Closed-vocabulary diagnostics with spans.
        diagnostics: Vec<parser::Diagnostic>,
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
            Self::Parse { topic, diagnostics } => {
                write!(f, "topic `{topic}`: {} defect(s)", diagnostics.len())
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

/// Whole-tree input: packages in arbitrary filesystem order. REQ: REQ-TOOL-045.
#[derive(Debug, Clone, Default)]
pub struct TreeInput {
    /// Package inputs; order carries no meaning.
    pub packages: Vec<PackageInput>,
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
            stems.insert(stem.as_str(), src);
        }
        for (stem, src) in &stems {
            let topic: TopicName = format!("{name}/{stem}");
            match parser::parse_bytes(src) {
                Ok(schema) => match canon::layout_struct(&schema.fields) {
                    Ok(layout) => {
                        model.topics.insert(
                            topic.clone(),
                            TopicEntry {
                                schema_hash: schema_hash(&schema),
                                schema,
                                layout,
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

    if errs.is_empty() {
        Ok(model)
    } else {
        Err(errs)
    }
}

/// Deterministic model render (I-D1): packages in sorted order, per-package
/// topics sorted, schema hash + canonical schema render per topic. The exact
/// format is internal; the contract is that identical trees render to
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
        }
    }
    out
}

/// Schema content hash over the canonical render text. Placeholder dual-lane
/// FNV-1a (zero-dep crate policy); BLAKE3 pinning arrives with
/// `contract_fingerprint` (workspace-model.md, REQ-TOOL-023).
/// REQ: REQ-TOOL-023.
#[must_use]
pub fn schema_hash(schema: &TopicFile) -> [u8; 16] {
    let text = parser::render_canonical(schema);
    let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h2: u64 = 0x27d4_eb2f_1656_67c5;
    for b in text.as_bytes() {
        h1 = (h1 ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        h2 = (h2 ^ u64::from(*b)).wrapping_mul(0x0100_0000_0000_01b3);
    }
    let mut out = [0_u8; 16];
    out[..8].copy_from_slice(&h1.to_le_bytes());
    out[8..].copy_from_slice(&h2.to_le_bytes());
    out
}
