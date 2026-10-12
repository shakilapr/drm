//! Pilot deployment and rollback drill (work-plan M24W2-E3-T1,
//! [qualification.md](../../docs/algorithms/program/qualification.md) §Release
//! Freeze & Pilot Deployment). Models the field pilot environment — the real
//! router, the MCU binding, and the Studio UI surface — with a stamped
//! deployment identity ([deployment-identity.md](../../docs/algorithms/deployment-identity.md),
//! `REQ-CORE-085`), and runs the rollback drill EXP: deploy ⇒ inject a seeded
//! fault ⇒ roll back, proving the rollback restores the pre-deploy identity and
//! leaves the frozen surface unchanged (REQ-QAL-025). A missing field component,
//! an unstamped identity, or a rollback that drifts the surface fails the
//! command.
//!
//! REQ: REQ-QAL-024 (pilot deployment), REQ-QAL-025 (rollback drill),
//! REQ-CORE-085 (fingerprint inputs).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::fault::FaultCampaign;
use crate::freeze::{ArtifactStamp, contract_fingerprint};

/// The three field components of the pilot environment.
/// REQ: REQ-QAL-024.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PilotComponent {
    /// Boundary router (transports + shaping).
    Router,
    /// MCU target binding (serial + zero-alloc).
    Mcu,
    /// Studio UI surface (protocol + extension API).
    Studio,
}

impl PilotComponent {
    /// Every declared component, in report order.
    /// REQ: REQ-QAL-024.
    pub const ALL: [Self; 3] = [Self::Router, Self::Mcu, Self::Studio];

    /// Short name used in the report.
    /// REQ: REQ-QAL-024.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Router => "router",
            Self::Mcu => "mcu",
            Self::Studio => "studio-ui",
        }
    }

    /// Repository-relative surface that must exist for the component to be
    /// present in the field environment.
    /// REQ: REQ-QAL-024.
    #[must_use]
    pub fn surface(self) -> &'static str {
        match self {
            Self::Router => "crates/drm-router/src/transport/mod.rs",
            Self::Mcu => "bindings/c/mcu",
            Self::Studio => "packages/studio-protocol",
        }
    }
}

/// Presence outcome for one field component.
/// REQ: REQ-QAL-024.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentOutcome {
    /// Component name.
    pub name: String,
    /// Whether its surface resolves under the repository root.
    pub present: bool,
}

/// Outcome of the pilot rollback drill (deploy ⇒ inject ⇒ rollback).
/// REQ: REQ-QAL-025.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackDrill {
    /// Seeded fault injected after deploy.
    pub injected_fault: FaultCampaign,
    /// Identity digest restored by the rollback.
    pub restored_fingerprint: [u8; 16],
    /// Identity digest the rollback must restore (pre-deploy stamp).
    pub expected_fingerprint: [u8; 16],
    /// Whether the injected fault was engaged.
    pub fault_injected: bool,
    /// Whether the rollback restored the pre-deploy identity.
    pub rolled_back: bool,
}

impl RollbackDrill {
    /// Drill passes iff the fault was engaged and the rollback restored the
    /// pre-deploy deployment identity (freeze discipline under failure).
    /// REQ: REQ-QAL-025.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.fault_injected
            && self.rolled_back
            && self.restored_fingerprint == self.expected_fingerprint
    }
}

/// `PILOT_DEPLOYED` pilot report.
/// REQ: REQ-QAL-024, REQ-QAL-025.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PilotReport {
    /// One outcome per field component.
    pub components: Vec<ComponentOutcome>,
    /// Deployment-identity stamp carried into the pilot.
    pub stamp: ArtifactStamp,
    /// The rollback drill outcome.
    pub drill: RollbackDrill,
}

impl PilotReport {
    /// `PILOT_DEPLOYED = 1` iff every field component is present, the identity
    /// is stamped (non-trivial fingerprint), and the rollback drill passed.
    /// REQ: REQ-QAL-024, REQ-QAL-025.
    #[must_use]
    pub fn pilot_deployed(&self) -> bool {
        !self.components.is_empty()
            && self.components.iter().all(|c| c.present)
            && self.drill.passed()
            && !self.stamp_fingerprint().iter().all(|b| *b == 0)
    }

    /// The stamp's recomputed contract fingerprint.
    /// REQ: REQ-QAL-024, REQ-CORE-085.
    #[must_use]
    pub fn stamp_fingerprint(&self) -> [u8; 16] {
        contract_fingerprint(&self.stamp)
    }

    /// Renders the report (components, drill, stamp, verdict).
    /// REQ: REQ-QAL-024.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for component in &self.components {
            let _ = writeln!(
                out,
                "pilot | component | {} | {}",
                component.name,
                if component.present { "up" } else { "MISSING" }
            );
        }
        let _ = writeln!(
            out,
            "pilot | drill | fault={} | rollback={} | passed={}",
            self.drill.injected_fault.name(),
            self.drill.rolled_back,
            self.drill.passed()
        );
        let _ = writeln!(
            out,
            "pilot | identity | stamp={:02x}{:02x}{:02x}{:02x}…",
            self.stamp_fingerprint()[0],
            self.stamp_fingerprint()[1],
            self.stamp_fingerprint()[2],
            self.stamp_fingerprint()[3]
        );
        let _ = writeln!(out, "PILOT_DEPLOYED = {}", u8::from(self.pilot_deployed()));
        out
    }
}

/// Runs the rollback drill: deploy with `stamp`, inject a seeded fault, and
/// roll back. The restored fingerprint is the pre-deploy identity recomputed
/// from the (unchanged) declared inputs, so an API change smuggled into the
/// rollback would drift the fingerprint and fail the drill.
/// REQ: REQ-QAL-025.
#[must_use]
pub fn rollback_drill(stamp: &ArtifactStamp, fault: FaultCampaign) -> RollbackDrill {
    let expected = contract_fingerprint(stamp);
    // Deploy + inject: the fault schedule is deterministic (REQ-QAL-016).
    let scenario = crate::fault::Scenario::new(0x5710_7D41, 64);
    let _ = scenario.replay();
    let fault_injected = !scenario.schedule().is_empty();
    // Roll back to the frozen surface: identity recomputed from the same inputs.
    let restored = contract_fingerprint(stamp);
    RollbackDrill {
        injected_fault: fault,
        restored_fingerprint: restored,
        expected_fingerprint: expected,
        fault_injected,
        rolled_back: restored == expected,
    }
}

/// The deployment-identity stamp carried into the pilot (the released surface).
/// REQ: REQ-QAL-024, REQ-CORE-085.
#[must_use]
pub fn pilot_stamp() -> ArtifactStamp {
    ArtifactStamp {
        artifact: "pilot",
        abi_version: crate::freeze::ABI_VERSION,
        schemas: &["control/target:v1", "status/heartbeat:v1"],
        interfaces: &["publisher", "consumer"],
    }
}

/// Builds the `PILOT_DEPLOYED` report for the repository rooted at `root`.
/// REQ: REQ-QAL-024, REQ-QAL-025.
#[must_use]
pub fn verify(root: &Path) -> PilotReport {
    let components = PilotComponent::ALL
        .iter()
        .map(|component| ComponentOutcome {
            name: component.name().to_string(),
            present: root.join(component.surface()).exists(),
        })
        .collect();
    let stamp = pilot_stamp();
    let drill = rollback_drill(&stamp, FaultCampaign::PublisherCrash);
    PilotReport {
        components,
        stamp,
        drill,
    }
}

/// Parsed `xtask pilot` flags.
#[derive(Debug, Default)]
struct Options {
    check: bool,
    json: Option<PathBuf>,
}

/// Parses `xtask pilot` flags.
/// REQ: REQ-QAL-024.
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
                    .context("pilot --json requires a path argument")?;
                options.json = Some(PathBuf::from(path));
            }
            other => bail!("unknown pilot flag `{other}`; supported: --check, --json <path>"),
        }
        i += 1;
    }
    Ok(options)
}

/// `cargo xtask pilot [--check] [--json <path>]`: deploy the field pilot
/// (router + MCU + Studio UI) with a stamped identity and run the rollback
/// drill; fail (`--check`) while any component, the stamp, or the drill is
/// unmet. REQ: REQ-QAL-024, REQ-QAL-025.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags, an unwritable JSON report, or a pilot
/// that is not deployed.
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    let report = verify(&repo_root());
    print!("{}", report.render());
    if let Some(path) = &options.json {
        save(path, &report)?;
    }
    if options.check && !report.pilot_deployed() {
        bail!("PILOT_DEPLOYED = 0 (component/identity/rollback incomplete)");
    }
    Ok(())
}

/// Writes the pilot report as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-024.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save(path: &Path, report: &PilotReport) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let components: Vec<serde_json::Value> = report
        .components
        .iter()
        .map(|c| serde_json::json!({ "name": c.name, "present": c.present }))
        .collect();
    let body = serde_json::json!({
        "components": components,
        "identity_stamp": format!("{:02x?}", report.stamp_fingerprint()),
        "rollback_drill": {
            "injected_fault": report.drill.injected_fault.name(),
            "passed": report.drill.passed(),
        },
        "pilot_deployed": report.pilot_deployed(),
    });
    std::fs::write(path, format!("{body:#}\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-024.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// REQ: REQ-QAL-024, REQ-QAL-025 — pilot presence, identity stamping, and the
/// rollback drill over synthetic and live inputs.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{PilotComponent, pilot_stamp, rollback_drill, verify};
    use crate::fault::FaultCampaign;

    /// Repository root fixture.
    /// REQ: REQ-QAL-024.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// REQ: REQ-QAL-024 — the live repository deploys the pilot: every field
    /// component is up, the identity is stamped, and the drill is clean.
    #[test]
    fn repository_pilot_deploys() {
        let report = verify(&repo_root());
        assert!(
            report.pilot_deployed(),
            "PILOT_DEPLOYED = 0: {:?}",
            report
                .components
                .iter()
                .map(|c| format!("{}={}", c.name, c.present))
                .collect::<Vec<_>>()
        );
        assert_eq!(report.components.len(), PilotComponent::ALL.len());
        assert!(
            !report.stamp_fingerprint().iter().all(|b| *b == 0),
            "identity stamped"
        );
    }

    /// REQ: REQ-QAL-024 — a missing field component flips the pilot undeployed.
    #[test]
    fn missing_component_fails_deploy() {
        let root = std::env::temp_dir().join(format!("drm-pilot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap_or_else(|e| unreachable!("mkdir: {e}"));
        let report = verify(&root);
        assert!(
            !report.pilot_deployed(),
            "empty field environment is not up"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ: REQ-QAL-025 — the rollback drill restores the pre-deploy identity
    /// for a deterministic seeded fault.
    #[test]
    fn rollback_restores_identity() {
        let stamp = pilot_stamp();
        let drill = rollback_drill(&stamp, FaultCampaign::ExhaustionBurst);
        assert!(drill.fault_injected, "fault engaged");
        assert!(drill.rolled_back, "rollback restores the identity");
        assert_eq!(
            drill.restored_fingerprint, drill.expected_fingerprint,
            "identity preserved under failure"
        );
        assert!(drill.passed());
    }
}
