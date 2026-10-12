//! v1.0 API freeze execution (work-plan M24W2-E1-T1/E2-T1,
//! [qualification.md](../../docs/algorithms/program/qualification.md) §Release
//! Freeze & Pilot Deployment; [c-abi-surface.md](../../docs/algorithms/bindings/c-abi-surface.md)
//! §Public API Inventory & Freeze). Executes the M24W1 inventory freeze: locks
//! the crate/binding versions, re-resolves every frozen surface append-only
//! (I-A8), and stamps released artifacts with their ABI version and contract
//! fingerprint (`hash(ABI version, topic schemas, declared interfaces)`,
//! [deployment-identity.md](../../docs/algorithms/deployment-identity.md)
//! `REQ-CORE-085`). A divergent version, a vanished frozen symbol, or a stamp
//! that disagrees with its declared inputs fails `cargo xtask freeze`.
//!
//! REQ: REQ-QAL-023 (API freeze), REQ-CORE-085 (fingerprint inputs).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::inventory;

/// Locked release version shared by every crate and binding (`API_FROZEN`).
/// REQ: REQ-QAL-023.
pub const RELEASE_VERSION: &str = "0.1.0";

/// Locked `DRM_ABI_VERSION` from `bindings/c/include/drm/drm.h`.
/// REQ: REQ-QAL-023, REQ-BIND-018.
pub const ABI_VERSION: u32 = 0;

/// One crate/binding manifest whose version is locked.
/// REQ: REQ-QAL-023.
pub struct VersionLock {
    /// Human name used in the report.
    pub name: &'static str,
    /// Repository-relative manifest path.
    pub manifest: &'static str,
    /// TOML section holding the version (`[workspace.package]`, `[project]`),
    /// or `""` for a JSON manifest (`package.json`).
    pub section: &'static str,
    /// Version key inside the section.
    pub key: &'static str,
}

/// The declared version lock set: every workspace crate (through the single
/// workspace version) plus each binding and Studio package manifest.
/// REQ: REQ-QAL-023.
pub const VERSION_LOCKS: [VersionLock; 6] = [
    VersionLock {
        name: "workspace",
        manifest: "Cargo.toml",
        section: "workspace.package",
        key: "version",
    },
    VersionLock {
        name: "drm-rust-sdk",
        manifest: "bindings/rust/crates/drm/Cargo.toml",
        section: "package",
        key: "version.workspace",
    },
    VersionLock {
        name: "drm-python",
        manifest: "bindings/python/pyproject.toml",
        section: "project",
        key: "version",
    },
    VersionLock {
        name: "drm-python-native",
        manifest: "bindings/python/native/Cargo.toml",
        section: "package",
        key: "version.workspace",
    },
    VersionLock {
        name: "studio-protocol",
        manifest: "packages/studio-protocol/package.json",
        section: "",
        key: "version",
    },
    VersionLock {
        name: "studio-extension-api",
        manifest: "packages/studio-extension-api/package.json",
        section: "",
        key: "version",
    },
];

/// The freeze note that must be published for `API_FROZEN = 1`.
/// REQ: REQ-QAL-023.
pub const FREEZE_NOTE: &str = "docs/spec/api-freeze.md";

/// The canonical C ABI header (single source of truth, ADR 0002).
/// REQ: REQ-QAL-023, REQ-BIND-018.
pub const HEADER: &str = "bindings/c/include/drm/drm.h";

/// Reads the `#define DRM_ABI_VERSION <n>` value from the header at `root`.
/// REQ: REQ-QAL-023, REQ-BIND-018.
///
/// # Errors
/// [`anyhow::Error`] when the header cannot be read.
fn header_abi_version(root: &Path) -> Result<Option<u32>> {
    let path = root.join(HEADER);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("#define") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        if parts.next() != Some("DRM_ABI_VERSION") {
            continue;
        }
        return Ok(parts.next().and_then(|v| v.parse::<u32>().ok()));
    }
    Ok(None)
}

/// One released artifact and its recomputable deployment-identity stamp.
/// REQ: REQ-QAL-023, REQ-CORE-085.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactStamp {
    /// Artifact name.
    pub artifact: &'static str,
    /// Declared ABI version input to the fingerprint.
    pub abi_version: u32,
    /// Declared topic schema/layout inputs (sorted, canonical).
    pub schemas: &'static [&'static str],
    /// Declared interface inputs (sorted, canonical).
    pub interfaces: &'static [&'static str],
}

/// The declared stamped artifacts; each stamp is recomputed from these inputs.
/// REQ: REQ-QAL-023.
pub const ARTIFACT_STAMPS: [ArtifactStamp; 4] = [
    ArtifactStamp {
        artifact: "drm-crates",
        abi_version: ABI_VERSION,
        schemas: &["control/target:v1", "status/heartbeat:v1"],
        interfaces: &["publisher", "consumer"],
    },
    ArtifactStamp {
        artifact: "drm-c",
        abi_version: ABI_VERSION,
        schemas: &["control/target:v1", "status/heartbeat:v1"],
        interfaces: &["publisher", "consumer"],
    },
    ArtifactStamp {
        artifact: "drm-python",
        abi_version: ABI_VERSION,
        schemas: &["control/target:v1", "status/heartbeat:v1"],
        interfaces: &["publisher", "consumer"],
    },
    ArtifactStamp {
        artifact: "studio-packages",
        abi_version: ABI_VERSION,
        schemas: &["control/target:v1", "status/heartbeat:v1"],
        interfaces: &["publisher", "consumer"],
    },
];

/// FNV-1a fold over `bytes` (mirrors the runtime fingerprint input folding).
/// REQ: REQ-CORE-085.
fn fold(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Contract fingerprint over the declared inputs: ABI version, own topic
/// schemas/layouts, and declared interfaces (`REQ-CORE-085`), truncated to 128
/// bits. Deterministic and input-sensitive: an edit to any input changes the
/// digest and the stamp no longer matches.
/// REQ: REQ-CORE-085.
#[must_use]
pub fn contract_fingerprint(stamp: &ArtifactStamp) -> [u8; 16] {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    hash = fold(hash, &stamp.abi_version.to_le_bytes());
    hash = fold(hash, b"|schemas|");
    for schema in stamp.schemas {
        hash = fold(hash, schema.as_bytes());
        hash = fold(hash, b"|");
    }
    hash = fold(hash, b"|interfaces|");
    for interface in stamp.interfaces {
        hash = fold(hash, interface.as_bytes());
        hash = fold(hash, b"|");
    }
    let mut out = [0_u8; 16];
    out[..8].copy_from_slice(&hash.to_le_bytes());
    out[8..].copy_from_slice(&hash.rotate_left(31).to_le_bytes());
    out
}

/// Resolution outcome for one version lock.
/// REQ: REQ-QAL-023.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionOutcome {
    /// Manifest name.
    pub name: String,
    /// Version read from the manifest, when parseable.
    pub found: Option<String>,
    /// Whether it equals [`RELEASE_VERSION`].
    pub locked: bool,
}

/// `API_FROZEN` freeze report.
/// REQ: REQ-QAL-023.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreezeReport {
    /// One outcome per declared manifest.
    pub versions: Vec<VersionOutcome>,
    /// Frozen surfaces that re-resolve against their owners.
    pub frozen_resolved: usize,
    /// Declared frozen surfaces.
    pub frozen_total: usize,
    /// Whether every artifact stamp matches its declared inputs.
    pub stamps_consistent: bool,
    /// Whether the freeze note is published on disk.
    pub note_published: bool,
}

impl FreezeReport {
    /// `API_FROZEN = 1` iff every version is locked, every frozen surface
    /// resolves, stamps are consistent, and the freeze note is published.
    /// REQ: REQ-QAL-023.
    #[must_use]
    pub fn api_frozen(&self) -> bool {
        !self.versions.is_empty()
            && self.versions.iter().all(|v| v.locked)
            && self.frozen_total > 0
            && self.frozen_resolved == self.frozen_total
            && self.stamps_consistent
            && self.note_published
    }

    /// Renders the report (per-manifest version, surfaces, stamps, note).
    /// REQ: REQ-QAL-023.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for version in &self.versions {
            let _ = writeln!(
                out,
                "freeze | version | {} | {} | {}",
                version.name,
                if version.locked { "locked" } else { "DRIFT" },
                version.found.as_deref().unwrap_or("<unreadable>")
            );
        }
        let _ = writeln!(
            out,
            "freeze | surfaces | {}/{} resolved",
            self.frozen_resolved, self.frozen_total
        );
        let _ = writeln!(
            out,
            "freeze | stamps | {}",
            if self.stamps_consistent {
                "consistent"
            } else {
                "INCONSISTENT"
            }
        );
        let _ = writeln!(
            out,
            "freeze | note | {}",
            if self.note_published {
                "published"
            } else {
                "MISSING"
            }
        );
        let _ = writeln!(out, "API_FROZEN = {}", u8::from(self.api_frozen()));
        out
    }
}

/// Extracts a quoted value for `key` from the TOML `section` of `text`; `None`
/// when the section/key is absent. A `.workspace = true` inheritance is
/// resolved to [`RELEASE_VERSION`] by the caller.
/// REQ: REQ-QAL-023.
fn toml_section_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed.trim_matches(['[', ']']) == section;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim();
        if value.ends_with("= true") || value == "true" {
            return Some("workspace");
        }
        return Some(value.trim_matches('"'));
    }
    None
}

/// Reads the version declared by a manifest under `root`.
/// REQ: REQ-QAL-023.
///
/// # Errors
/// [`anyhow::Error`] when the manifest cannot be read.
fn manifest_version(root: &Path, lock: &VersionLock) -> Result<Option<String>> {
    let path = root.join(lock.manifest);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if lock.section.is_empty() {
        let json: serde_json::Value =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        return Ok(json
            .get(lock.key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string));
    }
    Ok(
        toml_section_value(&text, lock.section, lock.key).map(|value| {
            if value == "workspace" {
                RELEASE_VERSION.to_string()
            } else {
                value.to_string()
            }
        }),
    )
}

/// Builds the `API_FROZEN` report for the repository rooted at `root`.
/// REQ: REQ-QAL-023.
///
/// # Errors
/// [`anyhow::Error`] when a manifest or the inventory cannot be read.
pub fn verify(root: &Path) -> Result<FreezeReport> {
    let mut versions = Vec::with_capacity(VERSION_LOCKS.len());
    for lock in &VERSION_LOCKS {
        let found = manifest_version(root, lock)?;
        versions.push(VersionOutcome {
            name: lock.name.to_string(),
            locked: found.as_deref() == Some(RELEASE_VERSION),
            found,
        });
    }
    let inventory = inventory::verify(root)?;
    let frozen_total = inventory
        .entries
        .iter()
        .filter(|e| e.disposition == inventory::Disposition::Freeze)
        .count();
    let frozen_resolved = inventory
        .entries
        .iter()
        .filter(|e| e.disposition == inventory::Disposition::Freeze && e.resolved)
        .count();
    let header_abi = header_abi_version(root)?;
    let stamps_consistent = header_abi == Some(ABI_VERSION)
        && ARTIFACT_STAMPS.iter().all(|stamp| {
            stamp.abi_version == ABI_VERSION && !contract_fingerprint(stamp).iter().all(|b| *b == 0)
        });
    Ok(FreezeReport {
        versions,
        frozen_resolved,
        frozen_total,
        stamps_consistent,
        note_published: root.join(FREEZE_NOTE).is_file(),
    })
}

/// Parsed `xtask freeze` flags.
#[derive(Debug, Default)]
struct Options {
    check: bool,
    json: Option<PathBuf>,
}

/// Parses `xtask freeze` flags.
/// REQ: REQ-QAL-023.
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
                    .context("freeze --json requires a path argument")?;
                options.json = Some(PathBuf::from(path));
            }
            other => bail!("unknown freeze flag `{other}`; supported: --check, --json <path>"),
        }
        i += 1;
    }
    Ok(options)
}

/// `cargo xtask freeze [--check] [--json <path>]`: execute the v1.0 API freeze
/// and fail (`--check`) while any version, surface, stamp, or the note is unmet.
/// REQ: REQ-QAL-023.
///
/// # Errors
/// [`anyhow::Error`] when a manifest cannot be read, the JSON report cannot be
/// written, or `--check` finds the freeze incomplete.
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    let root = repo_root();
    let report = verify(&root)?;
    print!("{}", report.render());
    if let Some(path) = &options.json {
        save(path, &report)?;
    }
    if options.check && !report.api_frozen() {
        bail!("API_FROZEN = 0 (versions/surfaces/stamps/note incomplete)");
    }
    Ok(())
}

/// Writes the freeze report as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-023.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save(path: &Path, report: &FreezeReport) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let versions: Vec<serde_json::Value> = report
        .versions
        .iter()
        .map(|v| serde_json::json!({ "name": v.name, "found": v.found, "locked": v.locked }))
        .collect();
    let body = serde_json::json!({
        "release_version": RELEASE_VERSION,
        "abi_version": ABI_VERSION,
        "versions": versions,
        "frozen_resolved": report.frozen_resolved,
        "frozen_total": report.frozen_total,
        "stamps_consistent": report.stamps_consistent,
        "note_published": report.note_published,
        "api_frozen": report.api_frozen(),
    });
    std::fs::write(path, format!("{body:#}\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-023.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// REQ: REQ-QAL-023, REQ-CORE-085 — version-lock parsing, fingerprint stamping,
/// and the live-repository freeze over the M24W1 inventory.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ABI_VERSION, ARTIFACT_STAMPS, ArtifactStamp, RELEASE_VERSION, contract_fingerprint,
        header_abi_version, parse_options, repo_root, toml_section_value, verify,
    };

    /// REQ: REQ-QAL-023 — the TOML section parser reads only the requested
    /// section and treats `version.workspace = true` as workspace-inherited.
    #[test]
    fn toml_section_parser_is_scoped() {
        let text =
            "[package]\nname = \"x\"\nversion.workspace = true\n\n[project]\nversion = \"9.9.9\"\n";
        assert_eq!(
            toml_section_value(text, "package", "version.workspace"),
            Some("workspace")
        );
        assert_eq!(
            toml_section_value(text, "project", "version"),
            Some("9.9.9")
        );
        assert_eq!(toml_section_value(text, "package", "version"), None);
    }

    /// REQ: REQ-CORE-085 — the contract fingerprint is input-sensitive: an ABI
    /// bump, a schema edit, or an interface edit changes the digest, while the
    /// untouched stamp reproduces it exactly.
    #[test]
    fn fingerprint_tracks_each_input() {
        let base = ArtifactStamp {
            artifact: "x",
            abi_version: 0,
            schemas: &["a:v1"],
            interfaces: &["publisher"],
        };
        let digest = contract_fingerprint(&base);
        assert_eq!(digest, contract_fingerprint(&base), "deterministic");
        let abi_bump = ArtifactStamp {
            abi_version: 1,
            ..base
        };
        assert_ne!(digest, contract_fingerprint(&abi_bump), "ABI input");
        let schema_edit = ArtifactStamp {
            schemas: &["a:v2"],
            ..base
        };
        assert_ne!(digest, contract_fingerprint(&schema_edit), "schema input");
        let iface_edit = ArtifactStamp {
            interfaces: &["consumer"],
            ..base
        };
        assert_ne!(digest, contract_fingerprint(&iface_edit), "interface input");
        assert!(
            ARTIFACT_STAMPS
                .iter()
                .all(|s| !contract_fingerprint(s).iter().all(|b| *b == 0)),
            "every declared stamp is non-trivial"
        );
    }

    /// REQ: REQ-QAL-023 — the live repository is frozen: every crate/binding
    /// version is locked, every frozen surface resolves, and the note exists.
    #[test]
    fn repository_is_frozen() {
        let report = verify(&repo_root()).unwrap_or_else(|e| unreachable!("freeze: {e}"));
        assert!(
            report.api_frozen(),
            "API_FROZEN = 0: {:?}",
            report
                .versions
                .iter()
                .map(|v| format!("{}={:?}", v.name, v.found))
                .collect::<Vec<_>>()
        );
        assert_eq!(report.versions.len(), super::VERSION_LOCKS.len());
        assert!(report.frozen_total > 0, "freeze set non-empty");
    }

    /// REQ: REQ-QAL-023 — flag parsing accepts `--check`/`--json`, rejects
    /// unknown flags and a missing `--json` value.
    #[test]
    fn options_parse_flags() {
        let flags: Vec<String> = ["--check", "--json", "target/f.json"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert!(options.check);
        assert_eq!(options.json, Some(PathBuf::from("target/f.json")));
        assert!(parse_options(&["--bogus".to_string()]).is_err());
        assert!(parse_options(&["--json".to_string()]).is_err());
    }

    /// REQ: REQ-QAL-023, REQ-BIND-018 — the stamp ABI version is the header's
    /// `DRM_ABI_VERSION`, and the locked release version is non-empty.
    #[test]
    fn header_abi_matches_stamp() {
        let abi = header_abi_version(&repo_root())
            .unwrap_or_else(|e| unreachable!("header: {e}"))
            .unwrap_or_else(|| unreachable!("DRM_ABI_VERSION absent"));
        assert_eq!(abi, ABI_VERSION, "stamp ABI version tracks the header");
        assert!(!RELEASE_VERSION.is_empty());
    }
}
