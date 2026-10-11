//! Fault-injection campaigns and deterministic fault scenarios (work-plan
//! M23W2-E1-T1/E2-T1). `cargo xtask fault` executes the five runtime/transport
//! fault campaigns — publisher-crash, exhaustion-burst, subscriber-kill,
//! serial-bomb, link-storm — and reports `5/5` green; a red campaign fails the
//! command (triage-first). The deterministic [`Scenario`] engine records a
//! seeded fault schedule and replays it to a stable [`Replay`] digest so a
//! campaign failure is reproducible from its seed. Normative sources:
//! [qualification.md](../../docs/algorithms/program/qualification.md) I-QAL2,
//! [exhaustion-reclamation.md](../../docs/algorithms/runtime/exhaustion-reclamation.md)
//! (I-E1..I-E4), and
//! [hazard-protocol.md](../../docs/algorithms/runtime/hazard-protocol.md)
//! (I-HZ1/I-HZ7). Campaign/scenario detail: `docs/spec/fault-injection.md`.
//! REQ: REQ-QAL-010, REQ-QAL-015, REQ-QAL-016.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Number of M23W2 fault campaigns; the `FAULT_CAMPAIGNS` target.
/// REQ: REQ-QAL-015.
pub const CAMPAIGN_COUNT: usize = 5;

/// One M23W2 fault campaign.
/// REQ: REQ-QAL-015.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaultCampaign {
    /// Publisher `SIGKILL`/restart at every publish stage.
    PublisherCrash,
    /// Pool-exhaustion bursts under the reclamation policies.
    ExhaustionBurst,
    /// Reader kill at every hazard-borrow stage.
    SubscriberKill,
    /// Serial framing bomb: corruption, truncation, resync.
    SerialBomb,
    /// Router link storm: flap and reconnect under load.
    LinkStorm,
}

impl FaultCampaign {
    /// Every campaign in report order.
    /// REQ: REQ-QAL-015.
    pub const ALL: [Self; CAMPAIGN_COUNT] = [
        Self::PublisherCrash,
        Self::ExhaustionBurst,
        Self::SubscriberKill,
        Self::SerialBomb,
        Self::LinkStorm,
    ];

    /// Stable kebab-case name used in flags, reports, and evidence.
    /// REQ: REQ-QAL-015.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::PublisherCrash => "publisher-crash",
            Self::ExhaustionBurst => "exhaustion-burst",
            Self::SubscriberKill => "subscriber-kill",
            Self::SerialBomb => "serial-bomb",
            Self::LinkStorm => "link-storm",
        }
    }

    /// One-line description of the fault exercised.
    /// REQ: REQ-QAL-015.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::PublisherCrash => "publisher SIGKILL/restart at every publish stage",
            Self::ExhaustionBurst => "pool exhaustion under the reclamation policies",
            Self::SubscriberKill => "reader kill at every hazard-borrow stage",
            Self::SerialBomb => "serial framing corruption, truncation, and resync",
            Self::LinkStorm => "router link flap and reconnect under load",
        }
    }

    /// Parses a `--campaign` value.
    /// REQ: REQ-QAL-015.
    ///
    /// # Errors
    /// [`anyhow::Error`] for any value other than the five campaign names.
    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == value)
            .ok_or_else(|| {
                anyhow!(
                    "unknown fault campaign `{value}` (expected one of publisher-crash, \
                     exhaustion-burst, subscriber-kill, serial-bomb, link-storm)"
                )
            })
    }

    /// Cargo arguments that execute this campaign's fault suite.
    /// REQ: REQ-QAL-010, REQ-QAL-015.
    #[must_use]
    pub fn command(self) -> Vec<&'static str> {
        match self {
            Self::PublisherCrash => vec!["test", "-p", "drm-core", "publisher_kill"],
            Self::ExhaustionBurst => vec![
                "test",
                "-p",
                "drm-core",
                "--test",
                "stress",
                "--",
                "--ignored",
                "stress_chaos_exhaustion_storm",
            ],
            Self::SubscriberKill => vec![
                "test",
                "-p",
                "drm-core",
                "--test",
                "shm_kill",
                "--",
                "--ignored",
                "--skip",
                "stress_shm_kill_child_driver",
            ],
            Self::SerialBomb => vec!["test", "-p", "drm-router", "serial"],
            Self::LinkStorm => vec![
                "test",
                "-p",
                "drm-router",
                "--test",
                "link_storm",
                "--",
                "--ignored",
            ],
        }
    }
}

/// Outcome of one executed campaign.
/// REQ: REQ-QAL-015.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignResult {
    /// Campaign that ran.
    pub campaign: FaultCampaign,
    /// `true` iff the campaign's fault suite finished clean.
    pub passed: bool,
}

/// Aggregate result of a fault-injection run (`FAULT_CAMPAIGNS`) plus the
/// deterministic scenario and its replay digest (reproducibility evidence).
/// REQ: REQ-QAL-015, REQ-QAL-016.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultReport {
    /// Deterministic fault scenario recorded for the run.
    pub scenario: Scenario,
    /// Replay of [`FaultReport::scenario`] over the pool model.
    pub replay: Replay,
    /// One result per executed campaign, in [`FaultCampaign::ALL`] order.
    pub results: Vec<CampaignResult>,
}

impl FaultReport {
    /// Campaigns that finished clean.
    /// REQ: REQ-QAL-015.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    /// `FAULT_CAMPAIGNS` target: every campaign green.
    /// REQ: REQ-QAL-015.
    #[must_use]
    pub fn green(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.passed)
    }

    /// Renders `passed/total` per campaign and the aggregate verdict.
    /// REQ: REQ-QAL-015.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "scenario seed={} steps={} digest={:#018x} published={} dropped={} reclaimed={}",
            self.scenario.seed,
            self.scenario.steps,
            self.replay.digest,
            self.replay.published,
            self.replay.dropped,
            self.replay.reclaimed
        );
        for result in &self.results {
            let _ = writeln!(
                out,
                "{:<8} {:<16} {}",
                if result.passed { "PASS" } else { "FAIL" },
                result.campaign.name(),
                result.campaign.description()
            );
        }
        let _ = writeln!(
            out,
            "fault campaigns: {}/{} green ({})",
            self.passed(),
            self.results.len(),
            if self.green() { "PASS" } else { "FAIL" }
        );
        out
    }
}

/// One fault event in a deterministic [`Scenario`] schedule.
/// REQ: REQ-QAL-016.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaultEvent {
    /// Publisher restart mid-publish.
    PublisherCrash,
    /// Pool-exhaustion burst.
    ExhaustionBurst,
    /// Subscriber kill.
    SubscriberKill,
    /// Serial framing bomb.
    SerialBomb,
    /// Link flap.
    LinkStorm,
}

impl FaultEvent {
    /// Every event kind, indexed by [`FaultEvent::code`].
    /// REQ: REQ-QAL-016.
    pub const ALL: [Self; CAMPAIGN_COUNT] = [
        Self::PublisherCrash,
        Self::ExhaustionBurst,
        Self::SubscriberKill,
        Self::SerialBomb,
        Self::LinkStorm,
    ];

    /// Dense index into [`FaultEvent::ALL`].
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::PublisherCrash => 0,
            Self::ExhaustionBurst => 1,
            Self::SubscriberKill => 2,
            Self::SerialBomb => 3,
            Self::LinkStorm => 4,
        }
    }
}

/// A seeded fault scenario: a reproducible schedule of [`FaultEvent`]s and a
/// stable replay digest (`determinism of replay after fault`).
/// REQ: REQ-QAL-016.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// Schedule seed; the same seed always replays identically.
    pub seed: u64,
    /// Number of scheduled steps.
    pub steps: u64,
}

impl Scenario {
    /// Builds a scenario from a seed and step count.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn new(seed: u64, steps: u64) -> Self {
        Self { seed, steps }
    }

    /// The deterministic fault schedule for this scenario.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn schedule(&self) -> Vec<FaultEvent> {
        let mut rng = Xorshift::new(self.seed);
        (0..self.steps)
            .map(|_| {
                let index =
                    usize::try_from(rng.next() % u64::try_from(CAMPAIGN_COUNT).unwrap_or(1))
                        .unwrap_or(0);
                FaultEvent::ALL[index]
            })
            .collect()
    }

    /// Stable digest of the schedule; identical seeds yield identical digests,
    /// so a failed campaign replays from its recorded seed.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn replay_digest(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (step, event) in self.schedule().iter().enumerate() {
            hash ^= u64::try_from(step)
                .unwrap_or(0)
                .wrapping_mul(0x0100_0001_0000_0001);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
            hash ^= u64::from(event.code());
        }
        hash
    }

    /// Deterministically replays the schedule over a [`PoolModel`], returning
    /// the observable counters (published/dropped/reclaimed) and the digest.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn replay(&self) -> Replay {
        let mut model = PoolModel::new(POOL_SLOTS);
        for event in self.schedule() {
            match event {
                FaultEvent::PublisherCrash => {
                    // Restart adopts the channel; one publish attempt.
                    let _ = model.publish();
                }
                FaultEvent::ExhaustionBurst => {
                    // Protect every slot, attempt a publish (drop), then run the
                    // maintenance reclaim pass (I-E1/I-E4).
                    while model.protect_next() {}
                    let _ = model.publish();
                    model.reclaim_dead();
                }
                FaultEvent::SubscriberKill => {
                    // A killed subscriber abandons its protected slot; the
                    // lock-proven-dead reclaim pass frees it.
                    let _ = model.protect_next();
                    model.reclaim_dead();
                }
                FaultEvent::SerialBomb | FaultEvent::LinkStorm => {
                    // Transport faults do not touch the pool: a publish attempt
                    // still exercises the drop-free path.
                    let _ = model.publish();
                }
            }
        }
        Replay {
            published: model.published(),
            dropped: model.pool_exhausted(),
            reclaimed: model.reclaims(),
            digest: self.replay_digest(),
        }
    }
}

/// Slots in the scenario replay model (matches the default `@history` pool of 3).
/// REQ: REQ-QAL-016.
const POOL_SLOTS: usize = 3;

/// Deterministic xorshift64 RNG (same construction as the stress suites).
/// REQ: REQ-QAL-016.
struct Xorshift(u64);

impl Xorshift {
    /// Seeds the generator (never zero).
    /// REQ: REQ-QAL-016.
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// Next value in the sequence.
    /// REQ: REQ-QAL-016.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Discrete pool model mirroring
/// [exhaustion-reclamation.md](../../docs/algorithms/runtime/exhaustion-reclamation.md):
/// a protected slot is never reused, a full pool drops and counts (`I-E1`), and
/// the maintenance pass reclaims lock-proven-dead slots (`I-E4`).
/// REQ: REQ-QAL-016.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolModel {
    protected: Vec<bool>,
    published: u64,
    pool_exhausted: u64,
    reclaims: u64,
}

impl PoolModel {
    /// Builds a model with `slots` payload slots.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self {
            protected: vec![false; slots.max(1)],
            published: 0,
            pool_exhausted: 0,
            reclaims: 0,
        }
    }

    /// Publishes one sample: occupies the first free slot, or drops and counts
    /// (`I-E1`); returns whether a slot was available.
    /// REQ: REQ-QAL-016.
    pub fn publish(&mut self) -> bool {
        if let Some(slot) = self.protected.iter().position(|p| !p) {
            self.protected[slot] = true;
            self.published += 1;
            true
        } else {
            self.pool_exhausted += 1;
            false
        }
    }

    /// Protects the first free slot; `false` when every slot is protected.
    /// REQ: REQ-QAL-016.
    pub fn protect_next(&mut self) -> bool {
        match self.protected.iter().position(|p| !p) {
            Some(slot) => {
                self.protected[slot] = true;
                true
            }
            None => false,
        }
    }

    /// Lock-proven-dead reclamation (`I-E4`): frees every protected slot and
    /// returns how many were recovered.
    /// REQ: REQ-QAL-016.
    pub fn reclaim_dead(&mut self) -> usize {
        let freed = self.protected.iter().filter(|p| **p).count();
        self.protected.fill(false);
        self.reclaims += u64::try_from(freed).unwrap_or(0);
        freed
    }

    /// Successful publishes.
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published
    }

    /// Dropped publishes (`pool_exhausted` counter, `I-E1`/`I-E3`).
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn pool_exhausted(&self) -> u64 {
        self.pool_exhausted
    }

    /// Slots recovered by the maintenance pass (`I-E4`).
    /// REQ: REQ-QAL-016.
    #[must_use]
    pub fn reclaims(&self) -> u64 {
        self.reclaims
    }
}

/// Observable result of a scenario replay.
/// REQ: REQ-QAL-016.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    /// Successful publishes.
    pub published: u64,
    /// Dropped publishes.
    pub dropped: u64,
    /// Slots recovered by reclamation.
    pub reclaimed: u64,
    /// Stable schedule digest ([`Scenario::replay_digest`]).
    pub digest: u64,
}

/// Parsed `xtask fault` flags.
#[derive(Debug)]
struct Options {
    campaigns: Vec<FaultCampaign>,
    seed: u64,
    steps: u64,
    dry_run: bool,
    list: bool,
    json: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            campaigns: FaultCampaign::ALL.to_vec(),
            seed: 0,
            steps: 64,
            dry_run: false,
            list: false,
            json: None,
        }
    }
}

/// Returns the value following a value-taking flag.
/// REQ: REQ-QAL-015.
///
/// # Errors
/// [`anyhow::Error`] when the flag has no following value.
fn value<'a>(flags: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    flags
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("fault flag `{flag}` needs a value"))
}

/// Parses `xtask fault` flags.
/// REQ: REQ-QAL-015.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags, missing values, or an unknown campaign.
fn parse_options(flags: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut selected: Vec<FaultCampaign> = Vec::new();
    let mut i = 0;
    while i < flags.len() {
        let flag = flags[i].as_str();
        match flag {
            "--dry-run" => options.dry_run = true,
            "--list" => options.list = true,
            "--seed" => {
                i += 1;
                options.seed = parse_u64(value(flags, i, flag)?, flag)?;
            }
            "--steps" => {
                i += 1;
                options.steps = parse_u64(value(flags, i, flag)?, flag)?;
            }
            "--campaign" => {
                i += 1;
                selected.push(FaultCampaign::parse(value(flags, i, flag)?)?);
            }
            "--json" => {
                i += 1;
                options.json = Some(PathBuf::from(value(flags, i, flag)?));
            }
            other => bail!("unknown fault flag `{other}`; see docs/spec/fault-injection.md"),
        }
        i += 1;
    }
    if !selected.is_empty() {
        options.campaigns = selected;
    }
    Ok(options)
}

/// Parses an unsigned integer flag value.
/// REQ: REQ-QAL-016.
///
/// # Errors
/// [`anyhow::Error`] when the value is not an unsigned integer.
fn parse_u64(raw: &str, flag: &str) -> Result<u64> {
    raw.parse()
        .map_err(|_| anyhow!("fault flag `{flag}` value `{raw}` is not an integer"))
}

/// `cargo xtask fault`: runs the five fault campaigns (or a selected subset)
/// and reports `FAULT_CAMPAIGNS = n/5`.
/// REQ: REQ-QAL-010, REQ-QAL-015.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags or when any campaign finishes red.
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    if options.list {
        for campaign in FaultCampaign::ALL {
            println!("{:<16} {}", campaign.name(), campaign.description());
        }
        return Ok(());
    }
    let scenario = Scenario::new(options.seed, options.steps);
    let mut report = FaultReport {
        scenario,
        replay: scenario.replay(),
        results: Vec::new(),
    };
    for campaign in &options.campaigns {
        let args = campaign.command();
        if options.dry_run {
            println!("{}: cargo {}", campaign.name(), args.join(" "));
            continue;
        }
        println!(
            "fault campaign {}: cargo {}",
            campaign.name(),
            args.join(" ")
        );
        let passed = crate::spawn("cargo", &args).is_ok();
        report.results.push(CampaignResult {
            campaign: *campaign,
            passed,
        });
    }
    if options.dry_run {
        return Ok(());
    }
    print!("{}", report.render());
    if let Some(path) = &options.json {
        save(path, &report)?;
    }
    if report.green() {
        Ok(())
    } else {
        bail!(
            "FAULT_CAMPAIGNS = {}/{} (red campaign: triage-first)",
            report.passed(),
            report.results.len()
        )
    }
}

/// Writes the report to `path` as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-015.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save(path: &std::path::Path, report: &FaultReport) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| anyhow!("create {}: {e}", parent.display()))?;
    }
    let text =
        serde_json::to_string_pretty(report).map_err(|e| anyhow!("serialize report: {e}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| anyhow!("write fault report {}: {e}", path.display()))?;
    Ok(())
}

/// REQ: REQ-QAL-010, REQ-QAL-015, REQ-QAL-016 — campaign registry, command
/// construction, flag parsing, report rendering, and deterministic replay.
#[cfg(test)]
mod tests {
    use super::{
        CAMPAIGN_COUNT, FaultCampaign, FaultEvent, FaultReport, Options, PoolModel, Scenario,
        parse_options,
    };

    /// REQ: REQ-QAL-015 — the campaign registry is complete: five unique names,
    /// each parseable, each with a non-empty cargo command.
    #[test]
    fn campaign_registry_complete() {
        assert_eq!(FaultCampaign::ALL.len(), CAMPAIGN_COUNT);
        assert_eq!(CAMPAIGN_COUNT, 5);
        for campaign in FaultCampaign::ALL {
            let parsed = FaultCampaign::parse(campaign.name())
                .unwrap_or_else(|e| unreachable!("parse {}: {e}", campaign.name()));
            assert_eq!(parsed, campaign, "name round-trip");
            let args = campaign.command();
            assert_eq!(args.first().copied(), Some("test"), "cargo test command");
            assert!(args.len() > 1, "command has arguments");
        }
        assert!(FaultCampaign::parse("not-a-campaign").is_err());
    }

    /// REQ: REQ-QAL-015 — the report is green only when every campaign passed,
    /// and rendering names each campaign.
    #[test]
    fn report_green_and_render() {
        let mut report = FaultReport::default();
        assert!(!report.green(), "an empty report is not green");
        for campaign in FaultCampaign::ALL {
            report.results.push(super::CampaignResult {
                campaign,
                passed: true,
            });
        }
        assert_eq!(report.passed(), CAMPAIGN_COUNT);
        assert!(report.green());
        let rendered = report.render();
        for campaign in FaultCampaign::ALL {
            assert!(rendered.contains(campaign.name()), "render names campaign");
        }
        assert!(
            rendered.contains("5/5 green"),
            "aggregate verdict: {rendered}"
        );

        if let Some(first) = report.results.first_mut() {
            first.passed = false;
        }
        assert!(!report.green(), "a red campaign fails the report");
        assert_eq!(report.passed(), CAMPAIGN_COUNT - 1);
    }

    /// REQ: REQ-QAL-016 — the same seed replays to the same schedule and digest;
    /// different seeds diverge; schedule length matches the step count.
    #[test]
    fn scenario_replay_is_deterministic() {
        let a = Scenario::new(0xDEAD_BEEF, 64);
        let b = Scenario::new(0xDEAD_BEEF, 64);
        assert_eq!(a.schedule(), b.schedule(), "same seed, same schedule");
        assert_eq!(a.replay_digest(), b.replay_digest(), "same digest");
        assert_eq!(a.schedule().len(), 64, "schedule honours step count");
        assert_eq!(a.replay(), b.replay(), "same replay");

        let c = Scenario::new(0x0000_0001, 64);
        assert_ne!(a.replay_digest(), c.replay_digest(), "seeds diverge");

        let empty = Scenario::new(7, 0);
        assert!(empty.schedule().is_empty());
    }

    /// REQ: REQ-QAL-016 — the pool model never blocks: a full pool drops and
    /// counts exactly, and the maintenance pass frees protected slots.
    #[test]
    fn pool_model_drops_and_reclaims() {
        let mut model = PoolModel::new(2);
        assert!(model.publish(), "first slot free");
        assert!(model.publish(), "second slot free");
        assert!(!model.publish(), "full pool drops (I-E1)");
        assert_eq!(model.pool_exhausted(), 1, "exact drop count (I-E3)");
        assert_eq!(model.published(), 2);

        assert_eq!(model.reclaim_dead(), 2, "both protected slots recovered");
        assert_eq!(model.reclaims(), 2);
        assert!(
            model.protect_next(),
            "a free slot is protectable after reclaim"
        );
        assert!(model.publish(), "publishing resumes after reclaim (I-E4)");
    }

    /// REQ: REQ-QAL-016 — every event kind is reachable and codes are dense.
    #[test]
    fn fault_event_codes_dense() {
        assert_eq!(FaultEvent::ALL.len(), CAMPAIGN_COUNT);
        for (i, event) in FaultEvent::ALL.iter().enumerate() {
            assert_eq!(usize::from(event.code()), i, "dense event code");
        }
    }

    /// REQ: REQ-QAL-015 — the flag parser selects campaigns, paths, and modes,
    /// and rejects unknown flags and campaigns.
    #[test]
    fn options_parse_flags() {
        let flags: Vec<String> = [
            "--campaign",
            "link-storm",
            "--dry-run",
            "--json",
            "target/f.json",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert_eq!(options.campaigns, vec![FaultCampaign::LinkStorm]);
        assert!(options.dry_run);
        assert_eq!(
            options.json,
            Some(std::path::PathBuf::from("target/f.json"))
        );
        assert_eq!(Options::default().campaigns.len(), CAMPAIGN_COUNT);

        assert!(parse_options(&["--bogus".to_string()]).is_err());
        assert!(parse_options(&["--campaign".to_string()]).is_err());
        assert!(parse_options(&["--campaign".to_string(), "nope".to_string()]).is_err());
    }
}
