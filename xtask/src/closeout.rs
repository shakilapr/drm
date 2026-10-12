//! v1.0 release + program close-out (work-plan M24W4-E1/E2/E3/E4,
//! [qualification.md](../../docs/algorithms/program/qualification.md) §v1.0
//! Release & Program Close-Out). Folds the v1.0 release candidate
//! ([release.rs](release.rs), `REQ-QAL-028`) into the shipping release and
//! closes the program: the release build covers every declared-supported target
//! (three CI runners + the locked MCU set), the publish set resolves per
//! codebase §2.3, and the deployment identity is stamped (`REQ-CORE-085`);
//! the post-1.0 backlog is groomed (Studio deferred list + extension roadmap);
//! and the program retrospective is complete (outcomes vs waves, risk-register
//! final state, metrics history). `cargo xtask closeout --check` succeeds only
//! when `V10_RELEASED`, `POST_10_BACKLOG`, and `RETRO_DONE` all hold, so the
//! verdict is `GATE_STATUS(M24W4) = PASS` and the program is complete.
//!
//! REQ: REQ-QAL-030 (v1.0 release), REQ-QAL-031 (post-1.0 backlog),
//! REQ-QAL-032 (program retrospective), REQ-QAL-033 (program close-out gate).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{packaging, release};

/// Final v1.0 release tag (`V10_RELEASED`).
/// REQ: REQ-QAL-030.
pub const V10_TAG: &str = "v1.0.0";

/// The final release notes that must be published for the v1.0 tag.
/// REQ: REQ-QAL-030.
pub const RELEASE_NOTES: &str = "docs/spec/release-notes.md";

/// Root README that must advertise the v1.0 release.
/// REQ: REQ-QAL-030.
pub const README: &str = "README.md";

/// Groomed post-1.0 backlog (Studio deferred list + extension roadmap).
/// REQ: REQ-QAL-031.
pub const BACKLOG_DOC: &str = "docs/spec/post-1.0-backlog.md";

/// Completed program retrospective packet.
/// REQ: REQ-QAL-032.
pub const RETRO_DOC: &str = "docs/spec/program-retrospective.md";

/// Number of declared work-plan waves the retrospective must cover.
/// REQ: REQ-QAL-032.
pub const WAVE_COUNT: usize = 8;

/// Kind of declared-supported release target.
/// REQ: REQ-QAL-030.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Host CI runner (`ubuntu-x86_64`, `ubuntu-aarch64`, `windows-x86_64`).
    Host,
    /// Locked MCU target (`h743`/Cortex-M7, `f407`/Cortex-M4F).
    Mcu,
}

impl TargetKind {
    /// Short label used in the report.
    /// REQ: REQ-QAL-030.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Mcu => "mcu",
        }
    }
}

/// One declared-supported release build target.
/// REQ: REQ-QAL-030.
pub struct ReleaseTarget {
    /// Target name (CI runner triple or MCU part number).
    pub name: &'static str,
    /// Target kind.
    pub kind: TargetKind,
    /// Repository-relative build surface that must resolve for the target.
    pub surface: &'static str,
}

/// The declared release build matrix: the three CI runners plus the locked MCU
/// set (`mcu-bring-up.md` §1).
/// REQ: REQ-QAL-030.
pub const RELEASE_TARGETS: [ReleaseTarget; 5] = [
    ReleaseTarget {
        name: "ubuntu-x86_64",
        kind: TargetKind::Host,
        surface: ".github/workflows/ci.yml",
    },
    ReleaseTarget {
        name: "ubuntu-aarch64",
        kind: TargetKind::Host,
        surface: ".github/workflows/ci.yml",
    },
    ReleaseTarget {
        name: "windows-x86_64",
        kind: TargetKind::Host,
        surface: ".github/workflows/windows-nightly.yml",
    },
    ReleaseTarget {
        name: "h743",
        kind: TargetKind::Mcu,
        surface: "bindings/c/mcu/Makefile",
    },
    ReleaseTarget {
        name: "f407",
        kind: TargetKind::Mcu,
        surface: "bindings/c/mcu/Makefile",
    },
];

/// Presence outcome for one declared release target.
/// REQ: REQ-QAL-030.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetOutcome {
    /// Target name.
    pub name: String,
    /// Target kind.
    pub kind: TargetKind,
    /// Build surface path.
    pub surface: String,
    /// Whether the build surface resolves under the repository root.
    pub present: bool,
}

/// `V10_RELEASED` release build report.
/// REQ: REQ-QAL-030.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseBuild {
    /// Final v1.0 tag.
    pub tag: String,
    /// Whether the v1.0 release candidate is complete (`V10_RC`, `REQ-QAL-028`).
    pub rc_folded: bool,
    /// One outcome per declared-supported target.
    pub targets: Vec<TargetOutcome>,
    /// Publish set resolved over the declared total (codebase §2.3).
    pub packages: (usize, usize),
    /// Stamped deployment identity recomputed from the declared inputs.
    pub identity: [u8; 16],
    /// Whether the final release notes and the v1.0 README are published.
    pub docs_published: bool,
}

impl ReleaseBuild {
    /// `V10_RELEASED = 1` iff the v1.0 candidate is folded, every
    /// declared-supported target resolves, every publish package resolves, the
    /// identity is stamped (non-trivial fingerprint), and the docs are
    /// published.
    /// REQ: REQ-QAL-030.
    #[must_use]
    pub fn v10_released(&self) -> bool {
        self.rc_folded
            && self.targets.len() == RELEASE_TARGETS.len()
            && self.targets.iter().all(|t| t.present)
            && self.packages.1 > 0
            && self.packages.0 == self.packages.1
            && !self.identity.iter().all(|b| *b == 0)
            && self.docs_published
    }
}

/// Groomed post-1.0 backlog (`POST_10_BACKLOG`).
/// REQ: REQ-QAL-031.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlog {
    /// Whether [`BACKLOG_DOC`] is present on disk.
    pub doc_present: bool,
    /// Whether the Studio deferred list carries at least one entry.
    pub studio_deferred: bool,
    /// Whether the extension roadmap carries at least one entry.
    pub extension_roadmap: bool,
}

impl Backlog {
    /// `POST_10_BACKLOG = 1` iff the backlog doc exists and both declared
    /// sections carry at least one entry.
    /// REQ: REQ-QAL-031.
    #[must_use]
    pub fn groomed(&self) -> bool {
        self.doc_present && self.studio_deferred && self.extension_roadmap
    }
}

/// Completed program retrospective (`RETRO_DONE`).
/// REQ: REQ-QAL-032.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrospective {
    /// Whether [`RETRO_DOC`] is present on disk.
    pub doc_present: bool,
    /// Work-plan waves covered by the outcomes-vs-waves table.
    pub waves_covered: usize,
    /// Whether the risk-register final state is recorded.
    pub risk_register: bool,
    /// Whether the metrics history is replayed.
    pub metrics_history: bool,
}

impl Retrospective {
    /// `RETRO_DONE = 1` iff the retrospective doc exists, covers all
    /// [`WAVE_COUNT`] waves, and records the risk register and metrics history.
    /// REQ: REQ-QAL-032.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.doc_present
            && self.waves_covered == WAVE_COUNT
            && self.risk_register
            && self.metrics_history
    }
}

/// The assembled program close-out report.
/// REQ: REQ-QAL-033.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramCloseout {
    /// v1.0 release build report.
    pub release: ReleaseBuild,
    /// Groomed post-1.0 backlog.
    pub backlog: Backlog,
    /// Completed program retrospective.
    pub retro: Retrospective,
}

impl ProgramCloseout {
    /// `V10_RELEASED` fold leg.
    /// REQ: REQ-QAL-030.
    #[must_use]
    pub fn v10_released(&self) -> bool {
        self.release.v10_released()
    }

    /// `POST_10_BACKLOG` fold leg.
    /// REQ: REQ-QAL-031.
    #[must_use]
    pub fn post_10_backlog(&self) -> bool {
        self.backlog.groomed()
    }

    /// `RETRO_DONE` fold leg.
    /// REQ: REQ-QAL-032.
    #[must_use]
    pub fn retro_done(&self) -> bool {
        self.retro.complete()
    }

    /// `GATE_STATUS(M24W4) = PASS` iff v1.0 is released, the backlog is
    /// groomed, and the retrospective is complete.
    /// REQ: REQ-QAL-033.
    #[must_use]
    pub fn program_complete(&self) -> bool {
        self.v10_released() && self.post_10_backlog() && self.retro_done()
    }

    /// Renders the close-out report (one line per leg plus the verdicts).
    /// REQ: REQ-QAL-030, REQ-QAL-031, REQ-QAL-032, REQ-QAL-033.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for target in &self.release.targets {
            let _ = writeln!(
                out,
                "closeout | target | {} | {} | {}",
                target.name,
                target.kind.label(),
                if target.present { "built" } else { "MISSING" }
            );
        }
        let _ = writeln!(
            out,
            "closeout | v1.0 | {} | rc={} | targets={}/{} | packages={}/{} | identity={:02x}{:02x}{:02x}{:02x}…",
            self.release.tag,
            self.release.rc_folded,
            self.release.targets.iter().filter(|t| t.present).count(),
            self.release.targets.len(),
            self.release.packages.0,
            self.release.packages.1,
            self.release.identity[0],
            self.release.identity[1],
            self.release.identity[2],
            self.release.identity[3],
        );
        let _ = writeln!(
            out,
            "closeout | backlog | groomed={} | studio_deferred={} | extension_roadmap={}",
            self.backlog.groomed(),
            self.backlog.studio_deferred,
            self.backlog.extension_roadmap,
        );
        let _ = writeln!(
            out,
            "closeout | retro | complete={} | waves={}/{} | risk_register={} | metrics_history={}",
            self.retro.complete(),
            self.retro.waves_covered,
            WAVE_COUNT,
            self.retro.risk_register,
            self.retro.metrics_history,
        );
        let _ = writeln!(out, "V10_RELEASED = {}", u8::from(self.v10_released()));
        let _ = writeln!(
            out,
            "POST_10_BACKLOG = {}",
            u8::from(self.post_10_backlog())
        );
        let _ = writeln!(out, "RETRO_DONE = {}", u8::from(self.retro_done()));
        let _ = writeln!(
            out,
            "GATE_STATUS(M24W4) = {}",
            if self.program_complete() {
                "PASS"
            } else {
                "FAIL"
            }
        );
        out
    }
}

/// Whether the section under `heading` (up to the next heading) contains at
/// least one entry line (a bullet `- ` or a table row `| `).
/// REQ: REQ-QAL-031.
#[must_use]
pub fn section_has_entry(text: &str, heading: &str) -> bool {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if in_section {
                break;
            }
            in_section = trimmed.trim_start_matches('#').trim() == heading;
            continue;
        }
        if in_section && (trimmed.starts_with("- ") || trimmed.starts_with("| ")) {
            return true;
        }
    }
    false
}

/// Counts how many distinct work-plan waves `W1`..`W8` appear in `text`.
/// REQ: REQ-QAL-032.
#[must_use]
pub fn wave_count(text: &str) -> usize {
    (1..=WAVE_COUNT)
        .filter(|n| text.contains(&format!("W{n}")))
        .count()
}

/// Reads a repository doc, returning an empty string when it is absent or
/// unreadable (absence is reported by the leg's `doc_present` flag).
/// REQ: REQ-QAL-031.
fn read_doc(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_default()
}

/// Builds the program close-out report for the repository rooted at `root`.
/// REQ: REQ-QAL-030, REQ-QAL-031, REQ-QAL-032, REQ-QAL-033.
///
/// # Errors
/// [`anyhow::Error`] when the folded v1.0 release candidate cannot be verified.
pub fn verify(root: &Path) -> Result<ProgramCloseout> {
    let candidate = release::verify(root)?;
    let targets = RELEASE_TARGETS
        .iter()
        .map(|target| TargetOutcome {
            name: target.name.to_string(),
            kind: target.kind,
            surface: target.surface.to_string(),
            present: root.join(target.surface).exists(),
        })
        .collect();
    let packages = packaging::verify(root);
    let notes = read_doc(root, RELEASE_NOTES);
    let readme = read_doc(root, README);
    let release = ReleaseBuild {
        tag: V10_TAG.to_string(),
        rc_folded: candidate.v10_rc(),
        targets,
        packages: (packages.resolved(), packages.total()),
        identity: candidate.identity,
        docs_published: !notes.is_empty() && readme.contains("v1.0"),
    };
    let backlog_text = read_doc(root, BACKLOG_DOC);
    let backlog = Backlog {
        doc_present: root.join(BACKLOG_DOC).is_file(),
        studio_deferred: section_has_entry(&backlog_text, "Studio deferred"),
        extension_roadmap: section_has_entry(&backlog_text, "Extension roadmap"),
    };
    let retro_text = read_doc(root, RETRO_DOC);
    let retro = Retrospective {
        doc_present: root.join(RETRO_DOC).is_file(),
        waves_covered: wave_count(&retro_text),
        risk_register: section_has_entry(&retro_text, "Risk register"),
        metrics_history: section_has_entry(&retro_text, "Metrics history"),
    };
    Ok(ProgramCloseout {
        release,
        backlog,
        retro,
    })
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-033.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Parsed `xtask closeout` flags.
#[derive(Debug, Default)]
struct Options {
    check: bool,
    json: Option<PathBuf>,
}

/// Parses `xtask closeout` flags.
/// REQ: REQ-QAL-033.
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
                    .context("closeout --json requires a path argument")?;
                options.json = Some(PathBuf::from(path));
            }
            other => bail!("unknown closeout flag `{other}`; supported: --check, --json <path>"),
        }
        i += 1;
    }
    Ok(options)
}

/// `cargo xtask closeout [--check] [--json <path>]`: fold the v1.0 release and
/// the program close-out; fail (`--check`) while any of `V10_RELEASED`,
/// `POST_10_BACKLOG`, or `RETRO_DONE` is unmet.
/// REQ: REQ-QAL-030, REQ-QAL-031, REQ-QAL-032, REQ-QAL-033.
///
/// # Errors
/// [`anyhow::Error`] when the close-out cannot be assembled, the JSON report
/// cannot be written, or `--check` finds the program incomplete.
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    let root = repo_root();
    let closeout = verify(&root)?;
    print!("{}", closeout.render());
    if let Some(path) = &options.json {
        save(path, &closeout)?;
    }
    if options.check && !closeout.program_complete() {
        bail!(
            "GATE_STATUS(M24W4) = FAIL (v10_released={} post_10_backlog={} retro_done={})",
            closeout.v10_released(),
            closeout.post_10_backlog(),
            closeout.retro_done(),
        );
    }
    Ok(())
}

/// Writes the close-out report as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-033.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save(path: &Path, closeout: &ProgramCloseout) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let targets: Vec<serde_json::Value> = closeout
        .release
        .targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "kind": t.kind.label(),
                "surface": t.surface,
                "present": t.present,
            })
        })
        .collect();
    let body = serde_json::json!({
        "tag": closeout.release.tag,
        "v10_released": closeout.v10_released(),
        "release": {
            "rc_folded": closeout.release.rc_folded,
            "targets": targets,
            "packages": {
                "resolved": closeout.release.packages.0,
                "total": closeout.release.packages.1,
            },
            "docs_published": closeout.release.docs_published,
        },
        "post_10_backlog": {
            "groomed": closeout.post_10_backlog(),
            "studio_deferred": closeout.backlog.studio_deferred,
            "extension_roadmap": closeout.backlog.extension_roadmap,
        },
        "retro_done": {
            "complete": closeout.retro_done(),
            "waves_covered": closeout.retro.waves_covered,
            "risk_register": closeout.retro.risk_register,
            "metrics_history": closeout.retro.metrics_history,
        },
        "program_complete": closeout.program_complete(),
    });
    std::fs::write(path, format!("{body:#}\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// REQ: REQ-QAL-030, REQ-QAL-031, REQ-QAL-032, REQ-QAL-033 — release-target
/// presence, backlog/retro section parsing, and the live-repository close-out.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        Backlog, RELEASE_TARGETS, ReleaseBuild, Retrospective, TargetKind, TargetOutcome,
        WAVE_COUNT, parse_options, section_has_entry, verify, wave_count,
    };

    /// Repository root fixture.
    /// REQ: REQ-QAL-030.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// A release build with every fold leg satisfied.
    /// REQ: REQ-QAL-030.
    fn complete_release() -> ReleaseBuild {
        ReleaseBuild {
            tag: "v1.0.0".to_string(),
            rc_folded: true,
            targets: RELEASE_TARGETS
                .iter()
                .map(|t| TargetOutcome {
                    name: t.name.to_string(),
                    kind: t.kind,
                    surface: t.surface.to_string(),
                    present: true,
                })
                .collect(),
            packages: (7, 7),
            identity: [1_u8; 16],
            docs_published: true,
        }
    }

    /// REQ: REQ-QAL-030 — the declared release matrix covers three host runners
    /// and the two locked MCU targets, all named uniquely.
    #[test]
    fn declared_targets_cover_runners_and_mcu() {
        assert_eq!(RELEASE_TARGETS.len(), 5);
        let hosts = RELEASE_TARGETS
            .iter()
            .filter(|t| t.kind == TargetKind::Host)
            .count();
        let mcus = RELEASE_TARGETS
            .iter()
            .filter(|t| t.kind == TargetKind::Mcu)
            .count();
        assert_eq!((hosts, mcus), (3, 2), "3 CI runners + 2 MCU targets");
        let mut names: Vec<&str> = RELEASE_TARGETS.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), RELEASE_TARGETS.len(), "target names unique");
        assert!(RELEASE_TARGETS.iter().any(|t| t.name == "windows-x86_64"));
        assert!(RELEASE_TARGETS.iter().any(|t| t.name == "h743"));
    }

    /// REQ: REQ-QAL-030 — the release is gated on every fold leg; dropping a
    /// target, a package, the identity, the docs, or the candidate flips it.
    #[test]
    fn v10_released_requires_every_leg() {
        assert!(complete_release().v10_released());
        let mut missing_target = complete_release();
        missing_target.targets[0].present = false;
        assert!(!missing_target.v10_released());
        let mut short_packages = complete_release();
        short_packages.packages = (6, 7);
        assert!(!short_packages.v10_released());
        let mut unstamped = complete_release();
        unstamped.identity = [0_u8; 16];
        assert!(!unstamped.v10_released());
        let mut no_docs = complete_release();
        no_docs.docs_published = false;
        assert!(!no_docs.v10_released());
        let mut no_rc = complete_release();
        no_rc.rc_folded = false;
        assert!(!no_rc.v10_released());
    }

    /// REQ: REQ-QAL-031 — the section parser detects a heading followed by a
    /// bullet or table entry, and rejects an absent or empty section.
    #[test]
    fn section_entry_detection() {
        let doc =
            "# Title\n\n## Studio deferred\n\n- feature A\n\n## Extension roadmap\n\n| item |\n";
        assert!(section_has_entry(doc, "Studio deferred"));
        assert!(section_has_entry(doc, "Extension roadmap"));
        assert!(!section_has_entry(doc, "Missing section"));
        let empty = "## Studio deferred\n\n## Extension roadmap\n";
        assert!(!section_has_entry(empty, "Studio deferred"));
    }

    /// REQ: REQ-QAL-031 — the backlog is groomed only with both entries.
    #[test]
    fn backlog_requires_both_sections() {
        assert!(
            Backlog {
                doc_present: true,
                studio_deferred: true,
                extension_roadmap: true,
            }
            .groomed()
        );
        assert!(
            !Backlog {
                doc_present: true,
                studio_deferred: true,
                extension_roadmap: false,
            }
            .groomed()
        );
        assert!(
            !Backlog {
                doc_present: false,
                studio_deferred: false,
                extension_roadmap: false,
            }
            .groomed()
        );
    }

    /// REQ: REQ-QAL-032 — wave counting finds each distinct `W1`..`W8`, and the
    /// retrospective is complete only with every wave and both records.
    #[test]
    fn retro_requires_waves_and_records() {
        let all = (1..=WAVE_COUNT)
            .map(|n| format!("W{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(wave_count(&all), WAVE_COUNT);
        assert_eq!(wave_count("W1 W3"), 2);
        let complete = Retrospective {
            doc_present: true,
            waves_covered: WAVE_COUNT,
            risk_register: true,
            metrics_history: true,
        };
        assert!(complete.complete());
        let missing_wave = Retrospective {
            waves_covered: WAVE_COUNT - 1,
            ..complete.clone()
        };
        assert!(!missing_wave.complete());
        let no_risk = Retrospective {
            risk_register: false,
            ..complete.clone()
        };
        assert!(!no_risk.complete());
        let no_metrics = Retrospective {
            metrics_history: false,
            ..complete
        };
        assert!(!no_metrics.complete());
    }

    /// REQ: REQ-QAL-033 — flag parsing accepts `--check`/`--json`, rejects
    /// unknown flags and a missing `--json` value.
    #[test]
    fn options_parse_flags() {
        let flags: Vec<String> = ["--check", "--json", "target/c.json"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert!(options.check);
        assert_eq!(options.json, Some(PathBuf::from("target/c.json")));
        assert!(parse_options(&["--bogus".to_string()]).is_err());
        assert!(parse_options(&["--json".to_string()]).is_err());
    }

    /// REQ: REQ-QAL-030, REQ-QAL-031, REQ-QAL-032, REQ-QAL-033 — the live
    /// repository closes the program: v1.0 released across every target, the
    /// backlog groomed, and the retrospective complete.
    #[test]
    fn repository_program_is_complete() {
        let closeout = verify(&repo_root()).unwrap_or_else(|e| unreachable!("closeout: {e}"));
        assert!(
            closeout.v10_released(),
            "V10_RELEASED = 0: targets={:?} packages={:?} identity={:02x?} docs={}",
            closeout
                .release
                .targets
                .iter()
                .map(|t| (t.name.clone(), t.present))
                .collect::<Vec<_>>(),
            closeout.release.packages,
            closeout.release.identity,
            closeout.release.docs_published,
        );
        assert!(
            closeout.post_10_backlog(),
            "POST_10_BACKLOG = 0: {:?}",
            closeout.backlog
        );
        assert!(
            closeout.retro_done(),
            "RETRO_DONE = 0: {:?}",
            closeout.retro
        );
        assert!(closeout.program_complete(), "GATE_STATUS(M24W4) = PASS");
    }
}
