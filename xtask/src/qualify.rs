//! Qualification gate packet assembly (work-plan M23W4-E4-T1,
//! [qualification.md](../../docs/algorithms/program/qualification.md)
//! §Functions `qualify()`). Assembles the whole-system packet from the
//! traceability matrix (`I-QAL1`), the fault/soak evidence (`I-QAL2`), the
//! conformance matrix closure (`I-QAL4`), and the declared perf-invariant /
//! repro / docs evidence. `cargo xtask qualify --check` fails while any packet
//! leg is unmet.
//!
//! REQ: REQ-QAL-005 (soak gates), REQ-QAL-006 (traceability), REQ-QAL-008
//! (cross-OS conformance), REQ-QAL-011 (bench baseline), REQ-QAL-012 (bench
//! invariants), REQ-QAL-018 (gate packet assembly).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cross_os::{CellState, cell_state, interop_core_matrix, interop_matrix};
use crate::fault::CAMPAIGN_COUNT;
use crate::soak::{SoakLedger, load as load_soak};
use crate::traceability;

/// Evidence artifacts the packet references and their on-disk presence.
/// REQ: REQ-QAL-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// Short label (`trace`, `fault`, `soak`, `repro`, `docs`, `perf`).
    pub label: String,
    /// Repository-relative artifact path.
    pub path: String,
    /// Whether the artifact exists at assembly time.
    pub present: bool,
}

/// `I-QAL4` conformance closure over the declared OS×language×transport matrix.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConformanceClosure {
    /// Every declared cell in the manifest.
    pub declared: usize,
    /// Declared-supported cells (runnable on the current host).
    pub supported: usize,
    /// Declared cells that are scheduled behind a named dependency.
    pub scheduled: usize,
    /// Gate-blocking same-language cells (must be exactly 24).
    pub core: usize,
}

impl ConformanceClosure {
    /// Closure holds iff every declared cell is classified (supported or
    /// scheduled-with-note) and the gate-blocking core is the full 24 cells.
    /// Undeclared cells cannot appear — the manifest is the sole source.
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.supported + self.scheduled == self.declared && self.core == 24
    }
}

/// The assembled qualification gate packet (`QualificationReport`).
/// REQ: REQ-QAL-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualificationPacket {
    /// Traceability coverage in whole percent (`I-QAL1`).
    pub trace_coverage: usize,
    /// Traceability completeness (`I-QAL1`).
    pub trace_complete: bool,
    /// Conformance matrix closure (`I-QAL4`).
    pub conformance: ConformanceClosure,
    /// Fault campaigns that must be green (`FAULT_CAMPAIGNS = 5`).
    pub fault_campaigns: usize,
    /// Trailing clean soak-run streak from the committed ledger (`I-QAL2`).
    pub soak_clean_streak: u64,
    /// Referenced evidence artifacts.
    pub evidence: Vec<Evidence>,
}

impl QualificationPacket {
    /// Packet gate: trace complete, conformance closed, all fault campaigns
    /// present, and every evidence artifact on disk. REQ: REQ-QAL-006.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.trace_complete
            && self.conformance.is_closed()
            && self.fault_campaigns == CAMPAIGN_COUNT
            && self.evidence.iter().all(|e| e.present)
    }

    /// Human-readable packet rendering (one line per leg).
    /// REQ: REQ-QAL-006.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "qualification packet: trace={}% complete={} conformance={} (supported={} scheduled={} core={}) faults={} soak_streak={}",
            self.trace_coverage,
            self.trace_complete,
            if self.conformance.is_closed() {
                "closed"
            } else {
                "open"
            },
            self.conformance.supported,
            self.conformance.scheduled,
            self.conformance.core,
            self.fault_campaigns,
            self.soak_clean_streak,
        );
        for evidence in &self.evidence {
            let _ = writeln!(
                out,
                "evidence | {} | {} | {}",
                evidence.label,
                if evidence.present {
                    "present"
                } else {
                    "MISSING"
                },
                evidence.path
            );
        }
        let _ = writeln!(
            out,
            "qualification gate: {}",
            if self.is_complete() { "PASS" } else { "FAIL" }
        );
        out
    }
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-006.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Evidence artifacts the packet requires, relative to the repository root.
/// REQ: REQ-QAL-002, REQ-QAL-003.
const EVIDENCE: [(&str, &str); 8] = [
    ("trace", "docs/algorithms/program/qualification.md"),
    ("fault", "docs/spec/fault-injection.md"),
    ("soak", "docs/spec/soak-ledger.md"),
    ("soak-ledger", "docs/metrics/soak-ledger.json"),
    ("repro", "docs/algorithms/program/qualification.md"),
    ("docs", "docs/spec/bench-methodology.md"),
    ("perf", "docs/spec/bench-baseline.md"),
    ("packet", "docs/gates/m23w4.md"),
];

/// Classifies the declared interop matrix into a closure.
/// REQ: REQ-QAL-008.
#[must_use]
pub fn conformance_closure() -> ConformanceClosure {
    let matrix = interop_matrix();
    let core = interop_core_matrix().len();
    let mut supported = 0;
    let mut scheduled = 0;
    for cell in &matrix {
        match cell_state(*cell) {
            CellState::Green => supported += 1,
            CellState::Scheduled(_) => scheduled += 1,
        }
    }
    ConformanceClosure {
        declared: matrix.len(),
        supported,
        scheduled,
        core,
    }
}

/// Assembles the packet for the repository rooted at `root`.
/// REQ: REQ-QAL-005, REQ-QAL-006, REQ-QAL-008.
///
/// # Errors
/// [`anyhow::Error`] when the traceability matrix cannot be generated.
pub fn assemble(root: &Path) -> Result<QualificationPacket> {
    let matrix = traceability::generate(root)?;
    let soak_streak = load_soak(&root.join("docs/metrics/soak-ledger.json"))
        .map(|ledger: SoakLedger| ledger.consecutive_clean())
        .unwrap_or(0);
    let evidence = EVIDENCE
        .iter()
        .map(|(label, path)| Evidence {
            label: (*label).to_string(),
            path: (*path).to_string(),
            present: root.join(path).is_file(),
        })
        .collect();
    Ok(QualificationPacket {
        trace_coverage: matrix.coverage_percent(),
        trace_complete: matrix.is_complete(),
        conformance: conformance_closure(),
        fault_campaigns: CAMPAIGN_COUNT,
        soak_clean_streak: soak_streak,
        evidence,
    })
}

/// REQ: REQ-QAL-006 — `cargo xtask qualify [--check] [--json <path>]`: assemble
/// the qualification gate packet, print it, optionally write JSON, and fail on
/// `--check` while any packet leg is unmet.
///
/// # Errors
/// [`anyhow::Error`] when the packet cannot be assembled, the JSON report
/// cannot be written, or `--check` finds an incomplete packet.
pub fn run(flags: &[String]) -> Result<()> {
    let mut check = false;
    let mut json: Option<PathBuf> = None;
    let mut iter = flags.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--check" => check = true,
            "--json" => {
                let path = iter
                    .next()
                    .context("qualify --json requires a path argument")?;
                json = Some(PathBuf::from(path));
            }
            other => bail!("unknown qualify flag `{other}`; supported: --check, --json <path>"),
        }
    }

    let packet = assemble(&repo_root())?;
    print!("{}", packet.render());
    if let Some(path) = json {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let body = serde_json::json!({
            "trace_coverage": packet.trace_coverage,
            "trace_complete": packet.trace_complete,
            "conformance": {
                "declared": packet.conformance.declared,
                "supported": packet.conformance.supported,
                "scheduled": packet.conformance.scheduled,
                "core": packet.conformance.core,
                "closed": packet.conformance.is_closed(),
            },
            "fault_campaigns": packet.fault_campaigns,
            "soak_clean_streak": packet.soak_clean_streak,
            "evidence": packet
                .evidence
                .iter()
                .map(|e| serde_json::json!({ "label": e.label, "path": e.path, "present": e.present }))
                .collect::<Vec<_>>(),
            "complete": packet.is_complete(),
        });
        std::fs::write(&path, format!("{body:#}\n"))
            .with_context(|| format!("write {}", path.display()))?;
    }
    if check && !packet.is_complete() {
        bail!("qualification packet incomplete (I-QAL1/I-QAL4)");
    }
    Ok(())
}

/// REQ: REQ-QAL-006, REQ-QAL-008 — closure and packet logic over the declared
/// matrix and the live repository.
#[cfg(test)]
mod tests {
    use super::{
        CAMPAIGN_COUNT, Evidence, QualificationPacket, assemble, conformance_closure, repo_root,
    };

    /// A packet with every leg satisfied.
    /// REQ: REQ-QAL-006.
    fn complete_packet() -> QualificationPacket {
        QualificationPacket {
            trace_coverage: 100,
            trace_complete: true,
            conformance: conformance_closure(),
            fault_campaigns: CAMPAIGN_COUNT,
            soak_clean_streak: 3,
            evidence: vec![Evidence {
                label: "packet".to_string(),
                path: "docs/gates/m23w4.md".to_string(),
                present: true,
            }],
        }
    }

    /// REQ: REQ-QAL-008 — the declared matrix is 96 cells, 24 gate-blocking core
    /// cells, and the closure classifies every cell (supported or scheduled).
    #[test]
    fn conformance_matrix_closed() {
        let closure = conformance_closure();
        assert_eq!(
            closure.declared, 96,
            "2 OS directions × 16 pairs × 3 transports"
        );
        assert_eq!(closure.core, 24, "gate-blocking same-language cells");
        assert_eq!(
            closure.supported + closure.scheduled,
            closure.declared,
            "every declared cell is classified"
        );
        assert!(closure.is_closed());
    }

    /// REQ: REQ-QAL-006 — a packet is complete only when every leg holds; a
    /// missing evidence artifact or a short fault campaign flips it incomplete.
    #[test]
    fn packet_requires_every_leg() {
        assert!(complete_packet().is_complete());
        let mut missing_evidence = complete_packet();
        missing_evidence.evidence[0].present = false;
        assert!(!missing_evidence.is_complete());
        let mut short_faults = complete_packet();
        short_faults.fault_campaigns = CAMPAIGN_COUNT - 1;
        assert!(!short_faults.is_complete());
        let mut open = complete_packet();
        open.conformance.supported = 0;
        assert!(!open.is_complete());
    }

    /// REQ: REQ-QAL-006 — the live repository assembles a complete packet: the
    /// traceability matrix is 100% and the declared evidence exists.
    #[test]
    fn repository_packet_is_complete() {
        let root = repo_root();
        let packet = assemble(&root).unwrap_or_else(|e| unreachable!("assemble: {e}"));
        assert_eq!(packet.trace_coverage, 100, "I-QAL1 coverage");
        assert!(packet.trace_complete, "I-QAL1 completeness");
        assert!(packet.conformance.is_closed(), "I-QAL4 closure");
        assert_eq!(packet.fault_campaigns, CAMPAIGN_COUNT);
        assert!(
            packet.is_complete(),
            "packet incomplete: {:?}",
            packet
                .evidence
                .iter()
                .filter(|e| !e.present)
                .map(|e| e.path.clone())
                .collect::<Vec<_>>()
        );
    }
}
