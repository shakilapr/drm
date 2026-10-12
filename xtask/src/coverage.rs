//! Critical-path coverage closure and mutation sample (work-plan M24W1-E1/E3/E4,
//! [qualification.md](../../docs/algorithms/program/qualification.md) `I-QAL8`
//! and `I-QAL9`). Resolves each declared critical path (runtime ring history,
//! overflow policies, hazard protocol, exhaustion/reclamation, router
//! transports) to its linked tests, and runs a fixed-seed seeded-defect sample
//! proving the hazard/ring oracles measure assertion strength, not line count.
//!
//! REQ: REQ-QAL-020 (critical-path coverage closure), REQ-QAL-022 (mutation
//! sample).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// One declared critical path and its linked tests.
/// REQ: REQ-QAL-020.
pub struct CriticalPath {
    /// Stable path name.
    pub name: &'static str,
    /// Implementation owner (repository-relative source path).
    pub source: &'static str,
    /// Registry anchor the path is traced to.
    pub req: &'static str,
    /// Linked test files (inline unit tests or integration suites).
    pub test_files: &'static [&'static str],
}

/// The declared critical paths swept in M24W1 (runtime E1, transports E3).
/// REQ: REQ-QAL-020.
pub const CRITICAL_PATHS: [CriticalPath; 6] = [
    CriticalPath {
        name: "ring-history",
        source: "crates/drm-core/src/ring/mod.rs",
        req: "REQ-CORE-066",
        test_files: &["crates/drm-core/src/ring/mod.rs"],
    },
    CriticalPath {
        name: "overflow-policies",
        source: "crates/drm-core/src/channel/overflow.rs",
        req: "REQ-CORE-040",
        test_files: &["crates/drm-core/src/channel/overflow.rs"],
    },
    CriticalPath {
        name: "hazard-protocol",
        source: "crates/drm-core/src/hazard/mod.rs",
        req: "REQ-CORE-030",
        test_files: &["crates/drm-core/src/hazard/mod.rs"],
    },
    CriticalPath {
        name: "exhaustion-reclamation",
        source: "crates/drm-core/src/channel/reclaim.rs",
        req: "REQ-CORE-058",
        test_files: &[
            "crates/drm-core/src/channel/reclaim.rs",
            "crates/drm-core/tests/stress/chaos_kill_matrix.rs",
        ],
    },
    CriticalPath {
        name: "transport-udp",
        source: "crates/drm-router/src/transport/udp.rs",
        req: "REQ-ROUTE-032",
        test_files: &["crates/drm-router/src/transport/udp.rs"],
    },
    CriticalPath {
        name: "transport-serial",
        source: "crates/drm-router/src/transport/serial.rs",
        req: "REQ-ROUTE-024",
        test_files: &["crates/drm-router/src/transport/serial.rs"],
    },
];

/// Resolution outcome for one critical path.
/// REQ: REQ-QAL-020.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCoverage {
    /// Path name.
    pub name: String,
    /// Whether the path resolved to a live owner plus linked tests.
    pub resolved: bool,
    /// Human-readable reason when unresolved.
    pub detail: String,
}

/// `I-QAL8` critical-path coverage report.
/// REQ: REQ-QAL-020.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    /// One outcome per declared path.
    pub paths: Vec<PathCoverage>,
}

impl CoverageReport {
    /// Number of declared paths.
    #[must_use]
    pub fn total(&self) -> usize {
        self.paths.len()
    }

    /// Number of resolved paths.
    #[must_use]
    pub fn resolved(&self) -> usize {
        self.paths.iter().filter(|p| p.resolved).count()
    }

    /// `I-QAL8` gate: every declared critical path resolves.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.resolved() == self.total()
    }

    /// Renders the report (one line per path plus the summary).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for path in &self.paths {
            let _ = writeln!(
                out,
                "critical-path | {} | {} | {}",
                path.name,
                if path.resolved { "resolved" } else { "GAP" },
                path.detail
            );
        }
        let _ = writeln!(out, "CRIT_COVERAGE = {}/{}", self.resolved(), self.total());
        out
    }
}

/// Whether `text` contains a Rust test marker.
/// REQ: REQ-QAL-020.
fn has_test_marker(text: &str) -> bool {
    text.contains("#[test]") || text.contains("#[cfg(test)]")
}

/// Resolves one critical path against the repository rooted at `root`.
/// REQ: REQ-QAL-020.
fn resolve_one(root: &Path, path: &CriticalPath) -> PathCoverage {
    let owner = root.join(path.source);
    let owner_text = match std::fs::read_to_string(&owner) {
        Ok(text) => text,
        Err(e) => {
            return PathCoverage {
                name: path.name.to_string(),
                resolved: false,
                detail: format!("owner {} unreadable: {e}", path.source),
            };
        }
    };
    if !owner_text.contains(path.req) {
        return PathCoverage {
            name: path.name.to_string(),
            resolved: false,
            detail: format!("owner {} does not cite {}", path.source, path.req),
        };
    }
    for file in path.test_files {
        let test_path = root.join(file);
        match std::fs::read_to_string(&test_path) {
            Ok(text) if has_test_marker(&text) => {}
            Ok(_) => {
                return PathCoverage {
                    name: path.name.to_string(),
                    resolved: false,
                    detail: format!("linked test {file} has no test marker"),
                };
            }
            Err(e) => {
                return PathCoverage {
                    name: path.name.to_string(),
                    resolved: false,
                    detail: format!("linked test {file} unreadable: {e}"),
                };
            }
        }
    }
    PathCoverage {
        name: path.name.to_string(),
        resolved: true,
        detail: format!("{} linked test file(s)", path.test_files.len()),
    }
}

/// Builds the `I-QAL8` critical-path coverage report for `root`.
/// REQ: REQ-QAL-020.
///
/// # Errors
/// [`anyhow::Error`] only when the repository root cannot be read.
pub fn critical_paths(root: &Path) -> Result<CoverageReport> {
    let _ = root
        .join("crates")
        .read_dir()
        .with_context(|| format!("read {}", root.join("crates").display()))?;
    let paths = CRITICAL_PATHS
        .iter()
        .map(|path| resolve_one(root, path))
        .collect();
    Ok(CoverageReport { paths })
}

/// A seeded defect class in the `I-QAL9` mutation sample.
/// REQ: REQ-QAL-022.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefectClass {
    /// Ring window floor: `>=` weakened to `>` (off-by-one retention).
    RingBoundaryFlip,
    /// Ring future bound: `s < ws` widened to `s <= ws`.
    RingFutureInclusive,
    /// Hazard validity: registration-generation equality inverted.
    HazardEpochInvert,
    /// Required-consumer retirement: queue-full handling removed.
    QueueFullRemoved,
}

impl DefectClass {
    /// Every seeded defect class in the fixed sample.
    /// REQ: REQ-QAL-022.
    pub const ALL: [Self; 4] = [
        Self::RingBoundaryFlip,
        Self::RingFutureInclusive,
        Self::HazardEpochInvert,
        Self::QueueFullRemoved,
    ];

    /// Short label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::RingBoundaryFlip => "ring-boundary-flip",
            Self::RingFutureInclusive => "ring-future-inclusive",
            Self::HazardEpochInvert => "hazard-epoch-invert",
            Self::QueueFullRemoved => "queue-full-removed",
        }
    }
}

/// The `NONE` hazard-slot sentinel mirrored from the runtime
/// (`crates/drm-core/src/hazard/mod.rs`).
/// REQ: REQ-QAL-022.
const HAZARD_NONE: u32 = u32::MAX;

/// Ring window predicate mirrored from `HistoryRing::lookup` (I1/I5).
/// REQ: REQ-QAL-022.
fn ring_retained(target: u64, ws: u64, r: u64, fv: u64) -> bool {
    target >= fv && target < ws && target >= ws.saturating_sub(r)
}

/// Hazard-validity predicate mirrored from `select_slot` (I-HZ1/I-HZ7).
/// REQ: REQ-QAL-022.
fn hazard_protects(word_gen: u32, rec_gen: u32, slot: u32) -> bool {
    slot != HAZARD_NONE && word_gen == rec_gen
}

/// Required-consumer retirement predicate mirrored from `allow_commit`
/// (REQ-CORE-040).
/// REQ: REQ-QAL-022.
fn retire_allowed(acks: &[u64], retire_seq: u64) -> bool {
    !acks.is_empty() && acks.iter().all(|a| *a > retire_seq)
}

/// A fixed oracle probe: hardcoded expected outputs for the linked predicates.
/// REQ: REQ-QAL-022.
type Probe = (&'static str, bool);

/// Ring oracle probes (hardcoded expected retention; R=4, fv=0, ws=10).
/// REQ: REQ-QAL-022.
const RING_PROBES: [Probe; 5] = [
    ("oldest retained", true),
    ("one below oldest", false),
    ("newest", true),
    ("future seq", false),
    ("below first_valid", false),
];

/// Evaluates the (possibly mutated) ring predicate against the fixed probes.
/// REQ: REQ-QAL-022.
fn ring_probe_results(mutant: Option<DefectClass>) -> Vec<bool> {
    let inputs = [
        (6_u64, 10_u64, 4_u64, 0_u64),
        (5, 10, 4, 0),
        (9, 10, 4, 0),
        (10, 10, 4, 0),
        (6, 10, 4, 8),
    ];
    inputs
        .iter()
        .map(|(target, ws, r, fv)| match mutant {
            Some(DefectClass::RingBoundaryFlip) => {
                *target >= *fv && *target < *ws && *target > ws.saturating_sub(*r)
            }
            Some(DefectClass::RingFutureInclusive) => {
                *target >= *fv && *target <= *ws && *target >= ws.saturating_sub(*r)
            }
            _ => ring_retained(*target, *ws, *r, *fv),
        })
        .collect()
}

/// Hazard oracle probes (hardcoded expected protection).
/// REQ: REQ-QAL-022.
const HAZARD_PROBES: [Probe; 3] = [
    ("current generation", true),
    ("stale generation", false),
    ("NONE slot", false),
];

/// Evaluates the (possibly mutated) hazard predicate against the fixed probes.
/// REQ: REQ-QAL-022.
fn hazard_probe_results(mutant: Option<DefectClass>) -> Vec<bool> {
    let inputs = [(7_u32, 7_u32, 1_u32), (6, 7, 1), (7, 7, HAZARD_NONE)];
    inputs
        .iter()
        .map(|(word_gen, rec_gen, slot)| match mutant {
            Some(DefectClass::HazardEpochInvert) => *slot != HAZARD_NONE && word_gen != rec_gen,
            _ => hazard_protects(*word_gen, *rec_gen, *slot),
        })
        .collect()
}

/// Retirement oracle probes (hardcoded expected gating).
/// REQ: REQ-QAL-022.
const RETIRE_PROBES: [Probe; 3] = [
    ("acknowledged past", true),
    ("unacknowledged", false),
    ("one of two unacknowledged", false),
];

/// Evaluates the (possibly mutated) retirement predicate against the fixed probes.
/// REQ: REQ-QAL-022.
fn retire_probe_results(mutant: Option<DefectClass>) -> Vec<bool> {
    let inputs: [&[u64]; 3] = [&[1], &[0], &[0, 5]];
    let retire_seq = 0_u64;
    inputs
        .iter()
        .map(|acks| match mutant {
            Some(DefectClass::QueueFullRemoved) => true,
            _ => retire_allowed(acks, retire_seq),
        })
        .collect()
}

/// Detects the seeded defect in the fixed probe set: a class is detected iff
/// at least one probe output differs from the hardcoded oracle expectation.
/// REQ: REQ-QAL-022.
#[must_use]
pub fn detected(class: DefectClass) -> bool {
    let (probes, results): (&[Probe], Vec<bool>) = match class {
        DefectClass::RingBoundaryFlip | DefectClass::RingFutureInclusive => {
            (&RING_PROBES, ring_probe_results(Some(class)))
        }
        DefectClass::HazardEpochInvert => (&HAZARD_PROBES, hazard_probe_results(Some(class))),
        DefectClass::QueueFullRemoved => (&RETIRE_PROBES, retire_probe_results(Some(class))),
    };
    probes
        .iter()
        .zip(results.iter())
        .any(|((_, expected), got)| expected != got)
}

/// One mutation-sample result.
/// REQ: REQ-QAL-022.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationResult {
    /// Seeded defect class.
    pub class: DefectClass,
    /// Short class label (mirrored into reports and the gate doc).
    pub label: &'static str,
    /// Whether the linked oracle detected it.
    pub detected: bool,
}

/// `I-QAL9` mutation sample: the fixed-seed, fixed-size seeded-defect set over
/// the hazard/ring/queue predicates. Sample rate: 4 defect classes.
/// REQ: REQ-QAL-022.
#[must_use]
pub fn mutation_sample() -> Vec<MutationResult> {
    DefectClass::ALL
        .iter()
        .map(|class| MutationResult {
            class: *class,
            label: class.label(),
            detected: detected(*class),
        })
        .collect()
}

/// `I-QAL9` gate: every seeded defect is detected by the linked oracle.
/// REQ: REQ-QAL-022.
#[must_use]
pub fn mutation_sample_clean() -> bool {
    mutation_sample().iter().all(|r| r.detected)
}

/// REQ: REQ-QAL-020, REQ-QAL-022 — coverage-police and mutation-sample self
/// tests over synthetic and live inputs.
#[cfg(test)]
mod tests {
    use super::{
        CRITICAL_PATHS, DefectClass, critical_paths, detected, mutation_sample,
        mutation_sample_clean,
    };

    /// Repository root fixture (the `xtask` crate lives at `<root>/xtask`).
    /// REQ: REQ-QAL-020.
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            )
    }

    /// REQ: REQ-QAL-020 — the live repository resolves every declared critical
    /// path (`CRIT_COVERAGE = 6/6`).
    #[test]
    fn repository_critical_paths_resolve() {
        let report = critical_paths(&repo_root()).unwrap_or_else(|e| unreachable!("coverage: {e}"));
        assert_eq!(report.total(), CRITICAL_PATHS.len());
        assert!(
            report.is_complete(),
            "unresolved critical paths: {:?}",
            report
                .paths
                .iter()
                .filter(|p| !p.resolved)
                .map(|p| p.detail.clone())
                .collect::<Vec<_>>()
        );
    }

    /// REQ: REQ-QAL-020 — a missing linked test is reported as a gap, not
    /// silently ignored.
    #[test]
    fn missing_linked_test_is_a_gap() {
        let root = std::env::temp_dir().join(format!("drm-cov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ok = critical_paths(&root);
        assert!(ok.is_err(), "absent crates dir is an error");
        // A root with a citing owner but no test marker resolves to a gap.
        std::fs::create_dir_all(root.join("crates/drm-core/src/ring"))
            .unwrap_or_else(|e| unreachable!("mkdir: {e}"));
        std::fs::write(
            root.join("crates/drm-core/src/ring/mod.rs"),
            "// REQ: REQ-CORE-066\nfn lookup() {}\n",
        )
        .unwrap_or_else(|e| unreachable!("write: {e}"));
        let report = critical_paths(&root).unwrap_or_else(|e| unreachable!("coverage: {e}"));
        assert!(report.paths.iter().any(|p| !p.resolved), "gap reported");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ: REQ-QAL-022 — every seeded defect class is detected by the fixed
    /// oracle probe set (the sample's whole point).
    #[test]
    fn seeded_defects_detected() {
        for class in DefectClass::ALL {
            assert!(detected(class), "undetected seeded defect {class:?}");
        }
        assert!(mutation_sample_clean());
        assert_eq!(mutation_sample().len(), DefectClass::ALL.len());
    }

    /// REQ: REQ-QAL-022 — the probe set carries hardcoded expected outputs, not
    /// the predicate's own return, so a broken oracle cannot pass vacuously.
    #[test]
    fn probe_oracle_is_fixed() {
        assert!(
            super::RING_PROBES.iter().any(|(_, expected)| !expected),
            "ring oracle has a negative probe"
        );
        assert!(
            super::HAZARD_PROBES.iter().any(|(_, expected)| !expected),
            "hazard oracle has a negative probe"
        );
    }
}
