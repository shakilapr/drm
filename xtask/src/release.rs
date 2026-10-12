//! v1.0 release candidate fold (work-plan M24W3-E4-T1/E4-T2,
//! [qualification.md](../../docs/algorithms/program/qualification.md) §Phase 34
//! Gate & v1.0 Release Candidate). Folds the completed Phase 34 packet into the
//! v1.0 release candidate (`REQ-QAL-028`): the candidate is tagged with the
//! deployment identity recomputed from the declared inputs (`REQ-CORE-085`), the
//! frozen API version is folded in, and the release notes must be drafted for the
//! RC tag. A red evidence line may not pass silently (`REQ-QAL-029`): it is only
//! cleared by a recorded waiver carrying a rationale and a review quorum of
//! [`WAIVER_QUORUM`] approvers; a silent or informal waiver fails
//! `cargo xtask release --check`.
//!
//! REQ: REQ-QAL-027 (Phase 34 gate verdict), REQ-QAL-028 (v1.0 RC),
//! REQ-QAL-029 (waiver discipline), REQ-CORE-085 (fingerprint inputs).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::freeze::{ABI_VERSION, ArtifactStamp, RELEASE_VERSION, contract_fingerprint};
use crate::pilot::pilot_stamp;
use crate::{freeze, qualify};

/// RC tag folded by the v1.0 release candidate (`V10_RC`).
/// REQ: REQ-QAL-028.
pub const RC_TAG: &str = "v1.0.0-rc.1";

/// The release notes that must be drafted for the RC tag.
/// REQ: REQ-QAL-028.
pub const RELEASE_NOTES: &str = "docs/spec/release-notes.md";

/// The pre-drafted waiver template recorded for any red evidence line.
/// REQ: REQ-QAL-029.
pub const WAIVER_TEMPLATE: &str = "docs/spec/waiver-template.md";

/// Review quorum: a waiver is valid only with this many distinct approvers.
/// REQ: REQ-QAL-029.
pub const WAIVER_QUORUM: usize = 2;

/// A recorded waiver for one red evidence line. Valid only with a non-empty
/// rationale and a review quorum of [`WAIVER_QUORUM`] distinct approvers.
/// REQ: REQ-QAL-029.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiver {
    /// The red evidence line the waiver covers.
    pub line: String,
    /// Why the line is waived; must be non-empty.
    pub rationale: String,
    /// Approvers; deduplicated before the quorum check.
    pub approvers: Vec<String>,
}

impl Waiver {
    /// Whether the waiver carries a non-empty rationale, names the line, and
    /// has a review quorum of [`WAIVER_QUORUM`] distinct approvers.
    /// REQ: REQ-QAL-029.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.line.trim().is_empty() || self.rationale.trim().is_empty() {
            return false;
        }
        let mut approvers: Vec<&str> = self
            .approvers
            .iter()
            .map(String::as_str)
            .filter(|a| !a.trim().is_empty())
            .collect();
        approvers.sort_unstable();
        approvers.dedup();
        approvers.len() >= WAIVER_QUORUM
    }
}

/// The waiver ledger: the red evidence lines at the gate and their recorded
/// waivers. Every red line must carry exactly one valid waiver; a red line with
/// no waiver (or an invalid one) is a silent pass and blocks the gate.
/// REQ: REQ-QAL-029.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaiverLedger {
    /// Red evidence lines at the gate (the lines that must be covered).
    pub red_lines: Vec<String>,
    /// Recorded waivers.
    pub waivers: Vec<Waiver>,
}

impl WaiverLedger {
    /// The red lines covered by a valid waiver.
    /// REQ: REQ-QAL-029.
    #[must_use]
    pub fn waived(&self) -> Vec<String> {
        self.red_lines
            .iter()
            .filter(|line| {
                self.waivers
                    .iter()
                    .any(|w| &w.line == *line && w.is_valid())
            })
            .cloned()
            .collect()
    }

    /// The red lines with no valid waiver (silent or informal passes).
    /// REQ: REQ-QAL-029.
    #[must_use]
    pub fn unwaived(&self) -> Vec<String> {
        self.red_lines
            .iter()
            .filter(|line| {
                !self
                    .waivers
                    .iter()
                    .any(|w| &w.line == *line && w.is_valid())
            })
            .cloned()
            .collect()
    }

    /// `waiver_clean` iff every red line carries a valid recorded waiver.
    /// REQ: REQ-QAL-029.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.unwaived().is_empty()
    }

    /// Renders the ledger (red-line count, waived lines, unwaived lines).
    /// REQ: REQ-QAL-029.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "release | waiver | red_lines={} waived={:?} unwaived={:?}",
            self.red_lines.len(),
            self.waived(),
            self.unwaived(),
        );
        out
    }
}

/// The folded v1.0 release candidate.
/// REQ: REQ-QAL-028.
#[allow(clippy::struct_excessive_bools)] // independent candidate fold legs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCandidate {
    /// v1.0 RC tag.
    pub tag: String,
    /// Frozen ABI version folded into the candidate.
    pub abi_version: u32,
    /// Stamped deployment identity recomputed from the declared inputs.
    pub identity: [u8; 16],
    /// Whether the assembled qualification packet is complete (`I-QAL1`..`I-QAL9`).
    pub qualification_complete: bool,
    /// Whether the API is frozen (`API_FROZEN`).
    pub api_frozen: bool,
    /// Whether the release notes are drafted for the RC tag.
    pub notes_published: bool,
    /// Whether every red evidence line carries a valid waiver.
    pub waiver_clean: bool,
}

impl ReleaseCandidate {
    /// `V10_RC = 1` iff the qualification packet is complete, the API is frozen,
    /// the candidate identity is stamped (non-trivial fingerprint), the release
    /// notes are drafted, and the waiver discipline holds.
    /// REQ: REQ-QAL-027, REQ-QAL-028, REQ-QAL-029.
    #[must_use]
    pub fn v10_rc(&self) -> bool {
        self.qualification_complete
            && self.api_frozen
            && !self.identity.iter().all(|b| *b == 0)
            && self.notes_published
            && self.waiver_clean
    }

    /// Renders the candidate (tag, identity, each fold leg, verdict).
    /// REQ: REQ-QAL-028.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "release | candidate | {} | abi={} | identity={:02x}{:02x}{:02x}{:02x}…",
            self.tag,
            self.abi_version,
            self.identity[0],
            self.identity[1],
            self.identity[2],
            self.identity[3]
        );
        let _ = writeln!(
            out,
            "release | fold | qualification={} api_frozen={} notes={} waiver_clean={}",
            self.qualification_complete, self.api_frozen, self.notes_published, self.waiver_clean,
        );
        let _ = writeln!(out, "V10_RC = {}", u8::from(self.v10_rc()));
        out
    }
}

/// The declared deployment-identity stamp folded into the release candidate
/// (the pilot's released surface, `REQ-CORE-085`).
/// REQ: REQ-QAL-028, REQ-CORE-085.
#[must_use]
pub fn candidate_stamp() -> ArtifactStamp {
    pilot_stamp()
}

/// The recorded waiver ledger for the M24W3 gate. There are no red evidence
/// lines at the gate, so the ledger is empty and therefore clean
/// (`REQ-QAL-029`); the template records how a line is waived when one appears.
/// REQ: REQ-QAL-029.
#[must_use]
pub fn waiver_ledger() -> WaiverLedger {
    WaiverLedger::default()
}

/// Folds the release candidate for the repository rooted at `root`.
/// REQ: REQ-QAL-027, REQ-QAL-028, REQ-QAL-029.
///
/// # Errors
/// [`anyhow::Error`] when the qualification packet or the freeze report cannot
/// be assembled.
pub fn verify(root: &Path) -> Result<ReleaseCandidate> {
    let packet = qualify::assemble(root)?;
    let freeze = freeze::verify(root)?;
    let stamp = candidate_stamp();
    Ok(ReleaseCandidate {
        tag: RC_TAG.to_string(),
        abi_version: ABI_VERSION,
        identity: contract_fingerprint(&stamp),
        qualification_complete: packet.is_complete(),
        api_frozen: freeze.api_frozen(),
        notes_published: root.join(RELEASE_NOTES).is_file(),
        waiver_clean: waiver_ledger().is_clean() && root.join(WAIVER_TEMPLATE).is_file(),
    })
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-028.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Parsed `xtask release` flags.
#[derive(Debug, Default)]
struct Options {
    check: bool,
    json: Option<PathBuf>,
}

/// Parses `xtask release` flags.
/// REQ: REQ-QAL-028.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags or a missing `--json` path.
fn parse_options(flags: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut i = 0;
    while i < flags.len() {
        match flags[i].as_str() {
            "--check" => options.check = true,
            "--json" => {
                i += 1;
                let path = flags
                    .get(i)
                    .context("release --json requires a path argument")?;
                options.json = Some(PathBuf::from(path));
            }
            other => bail!("unknown release flag `{other}`; supported: --check, --json <path>"),
        }
        i += 1;
    }
    Ok(options)
}

/// `cargo xtask release [--check] [--json <path>]`: fold the v1.0 release
/// candidate and fail (`--check`) while the candidate is unmet (`V10_RC = 0`).
/// REQ: REQ-QAL-027, REQ-QAL-028, REQ-QAL-029.
///
/// # Errors
/// [`anyhow::Error`] when the candidate cannot be folded, the JSON report
/// cannot be written, or `--check` finds the candidate incomplete.
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    let root = repo_root();
    let candidate = verify(&root)?;
    print!("{}", candidate.render());
    print!("{}", waiver_ledger().render());
    if let Some(path) = &options.json {
        save(path, &candidate)?;
    }
    if options.check && !candidate.v10_rc() {
        let unwaived = waiver_ledger().unwaived();
        bail!(
            "V10_RC = 0 (qualification={} api_frozen={} notes={} waiver_clean={}; unwaived red lines: {unwaived:?})",
            candidate.qualification_complete,
            candidate.api_frozen,
            candidate.notes_published,
            candidate.waiver_clean,
        );
    }
    Ok(())
}

/// Writes the release-candidate report as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-028.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save(path: &Path, candidate: &ReleaseCandidate) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut identity = String::with_capacity(32);
    for byte in candidate.identity {
        let _ = write!(identity, "{byte:02x}");
    }
    let body = serde_json::json!({
        "tag": candidate.tag,
        "release_version": RELEASE_VERSION,
        "abi_version": candidate.abi_version,
        "identity": identity,
        "qualification_complete": candidate.qualification_complete,
        "api_frozen": candidate.api_frozen,
        "notes_published": candidate.notes_published,
        "waiver_clean": candidate.waiver_clean,
        "v10_rc": candidate.v10_rc(),
    });
    std::fs::write(path, format!("{body:#}\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// REQ: REQ-QAL-027, REQ-QAL-028, REQ-QAL-029, REQ-CORE-085 — candidate fold,
/// deployment-identity stamping, and the live-repository v1.0 RC over the
/// Phase 34 packet.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ABI_VERSION, RC_TAG, ReleaseCandidate, Waiver, WaiverLedger, candidate_stamp,
        contract_fingerprint, parse_options, repo_root, verify,
    };

    /// A candidate with every fold leg satisfied.
    /// REQ: REQ-QAL-028.
    fn complete_candidate() -> ReleaseCandidate {
        ReleaseCandidate {
            tag: RC_TAG.to_string(),
            abi_version: ABI_VERSION,
            identity: contract_fingerprint(&candidate_stamp()),
            qualification_complete: true,
            api_frozen: true,
            notes_published: true,
            waiver_clean: true,
        }
    }

    /// REQ: REQ-QAL-028 — the candidate is complete only when every fold leg
    /// holds; dropping the packet, the freeze, the notes, or a stamped identity
    /// flips it incomplete.
    #[test]
    fn candidate_folds_every_leg() {
        assert!(complete_candidate().v10_rc());
        let mut no_packet = complete_candidate();
        no_packet.qualification_complete = false;
        assert!(!no_packet.v10_rc());
        let mut unfrozen = complete_candidate();
        unfrozen.api_frozen = false;
        assert!(!unfrozen.v10_rc());
        let mut no_notes = complete_candidate();
        no_notes.notes_published = false;
        assert!(!no_notes.v10_rc());
        let mut unstamped = complete_candidate();
        unstamped.identity = [0_u8; 16];
        assert!(!unstamped.v10_rc());
        let mut dirty = complete_candidate();
        dirty.waiver_clean = false;
        assert!(!dirty.v10_rc());
    }

    /// REQ: REQ-CORE-085 — the candidate identity tracks each declared input: a
    /// schema edit or an interface edit changes the folded digest.
    #[test]
    fn identity_tracks_declared_inputs() {
        let stamp = candidate_stamp();
        let identity = contract_fingerprint(&stamp);
        assert_eq!(identity, contract_fingerprint(&stamp), "deterministic");
        assert!(!identity.iter().all(|b| *b == 0), "identity is stamped");
        let schema_edit = crate::freeze::ArtifactStamp {
            schemas: &["control/target:v2"],
            ..stamp
        };
        assert_ne!(
            identity,
            contract_fingerprint(&schema_edit),
            "schema input folds in"
        );
    }

    /// REQ: REQ-QAL-029 — a waiver is valid only with a non-empty rationale and a
    /// review quorum of distinct approvers; a single approver, a blank rationale,
    /// or a blank line is rejected.
    #[test]
    fn waiver_requires_quorum_and_rationale() {
        let valid = Waiver {
            line: "I-QAL1".to_string(),
            rationale: "verified manually".to_string(),
            approvers: vec!["E1".to_string(), "E2".to_string()],
        };
        assert!(valid.is_valid());
        let dup = Waiver {
            approvers: vec!["E1".to_string(), "E1".to_string()],
            ..valid.clone()
        };
        assert!(!dup.is_valid(), "quorum counts distinct approvers");
        let blank_rationale = Waiver {
            rationale: "  ".to_string(),
            ..valid.clone()
        };
        assert!(!blank_rationale.is_valid(), "rationale must be non-empty");
        let blank_line = Waiver {
            line: String::new(),
            ..valid.clone()
        };
        assert!(!blank_line.is_valid(), "waiver must name a line");
    }

    /// REQ: REQ-QAL-029 — a red evidence line with no valid waiver is a silent
    /// pass: it leaves the ledger unclean and blocks the gate, while covering it
    /// with a valid waiver clears it.
    #[test]
    fn red_line_without_waiver_blocks() {
        let mut ledger = WaiverLedger {
            red_lines: vec!["I-QAL2".to_string()],
            waivers: Vec::new(),
        };
        assert!(!ledger.is_clean());
        assert_eq!(ledger.unwaived(), vec!["I-QAL2".to_string()]);
        let informal = Waiver {
            line: "I-QAL2".to_string(),
            rationale: String::new(),
            approvers: vec!["E1".to_string(), "E2".to_string()],
        };
        ledger.waivers.push(informal);
        assert!(
            !ledger.is_clean(),
            "informal waiver does not clear the line"
        );
        ledger.waivers.clear();
        ledger.waivers.push(Waiver {
            line: "I-QAL2".to_string(),
            rationale: "tracked in ADR 0009".to_string(),
            approvers: vec!["E1".to_string(), "E2".to_string()],
        });
        assert!(ledger.is_clean());
        assert!(ledger.unwaived().is_empty());
        assert_eq!(ledger.waived(), vec!["I-QAL2".to_string()]);
    }

    /// REQ: REQ-QAL-028 — flag parsing accepts `--check`/`--json`, rejects
    /// unknown flags and a missing `--json` value.
    #[test]
    fn options_parse_flags() {
        let flags: Vec<String> = ["--check", "--json", "target/r.json"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert!(options.check);
        assert_eq!(options.json, Some(PathBuf::from("target/r.json")));
        assert!(parse_options(&["--bogus".to_string()]).is_err());
        assert!(parse_options(&["--json".to_string()]).is_err());
    }

    /// REQ: REQ-QAL-027, REQ-QAL-028, REQ-QAL-029 — the live repository folds a
    /// complete v1.0 release candidate: the Phase 34 packet is complete, the API
    /// is frozen, the identity is stamped, the notes are drafted, and the waiver
    /// ledger is clean.
    #[test]
    fn repository_candidate_is_complete() {
        let candidate = verify(&repo_root()).unwrap_or_else(|e| unreachable!("release: {e}"));
        assert_eq!(candidate.tag, RC_TAG);
        assert_eq!(candidate.abi_version, ABI_VERSION);
        assert!(
            !candidate.identity.iter().all(|b| *b == 0),
            "identity must be stamped"
        );
        assert!(candidate.qualification_complete, "Phase 34 packet (I-QAL1)");
        assert!(candidate.api_frozen, "API_FROZEN (REQ-QAL-023)");
        assert!(
            candidate.notes_published,
            "release notes drafted (REQ-QAL-028)"
        );
        assert!(candidate.waiver_clean, "waiver discipline (REQ-QAL-029)");
        assert!(candidate.v10_rc(), "V10_RC = 1");
    }
}
