//! Soak escalation harness (work-plan M18W4-E1-T1/E4-T1;
//! [qualification.md](../../docs/algorithms/program/qualification.md) I-QAL2,
//! `REQ-QAL-005`, `REQ-QAL-013`, `REQ-QAL-014`). Repeats the workspace suite
//! (`cargo test --workspace`) for a wall-clock budget over rotating topologies,
//! appends each run to the committed I-QAL2 evidence ledger, and renders the
//! per-run anomaly dashboard. Format: `docs/spec/soak-ledger.md`.
//! REQ: REQ-QAL-005, REQ-QAL-013, REQ-QAL-014.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Trailing clean runs required by the I-QAL2 regression gate.
/// REQ: REQ-QAL-013.
pub const REQUIRED_CLEAN_RUNS: u64 = 3;

/// Default committed evidence ledger path.
/// REQ: REQ-QAL-014.
const DEFAULT_LEDGER: &str = "docs/metrics/soak-ledger.json";

/// One I-QAL2 soak topology; consecutive daily runs rotate through
/// [`Topology::ALL`].
/// REQ: REQ-QAL-013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    /// All nodes on one host, shared-memory only.
    SingleMachine,
    /// Two-netns simulation of a host pair over the router.
    TwoMachineSim,
    /// Two physical hosts joined by the router transport.
    PhysicalPair,
}

impl Topology {
    /// Every topology in rotation order.
    /// REQ: REQ-QAL-013.
    pub const ALL: [Self; 3] = [Self::SingleMachine, Self::TwoMachineSim, Self::PhysicalPair];

    /// Stable kebab-case name used in flags and reports.
    /// REQ: REQ-QAL-013.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SingleMachine => "single-machine",
            Self::TwoMachineSim => "two-machine-sim",
            Self::PhysicalPair => "physical-pair",
        }
    }

    /// Topology for the run after `completed_runs` earlier runs (round-robin).
    /// REQ: REQ-QAL-013.
    #[must_use]
    pub fn next_after(completed_runs: usize) -> Self {
        Self::ALL[completed_runs % Self::ALL.len()]
    }

    /// Parses a `--topology` value.
    /// REQ: REQ-QAL-013.
    ///
    /// # Errors
    /// [`anyhow::Error`] for any value other than the three topology names.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "single-machine" => Ok(Self::SingleMachine),
            "two-machine-sim" => Ok(Self::TwoMachineSim),
            "physical-pair" => Ok(Self::PhysicalPair),
            other => bail!(
                "unknown soak topology `{other}` \
                 (expected single-machine, two-machine-sim, or physical-pair)"
            ),
        }
    }
}

/// One completed soak run recorded in the I-QAL2 ledger.
/// REQ: REQ-QAL-014.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoakRun {
    /// Topology exercised by this run.
    pub topology: Topology,
    /// Requested wall-clock budget in whole hours (`0` for an `--iterations` run).
    pub hours: u64,
    /// Completed suite iterations.
    pub iterations: u64,
    /// Failed suite iterations (anomaly counter); `0` for a clean run.
    pub anomalies: u64,
    /// `true` iff no anomaly was observed.
    pub clean: bool,
    /// Run completion time (Unix seconds).
    pub finished_unix_s: u64,
}

impl SoakRun {
    /// Builds a run record from its outcome.
    /// REQ: REQ-QAL-014.
    #[must_use]
    pub fn new(topology: Topology, hours: u64, iterations: u64, anomalies: u64) -> Self {
        Self {
            topology,
            hours,
            iterations,
            anomalies,
            clean: anomalies == 0,
            finished_unix_s: unix_now(),
        }
    }
}

/// Append-only I-QAL2 soak evidence ledger.
/// REQ: REQ-QAL-014.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoakLedger {
    /// One record per completed run, in completion order.
    pub runs: Vec<SoakRun>,
}

impl SoakLedger {
    /// Trailing consecutive clean runs.
    /// REQ: REQ-QAL-013.
    #[must_use]
    pub fn consecutive_clean(&self) -> u64 {
        self.runs.iter().rev().take_while(|run| run.clean).count() as u64
    }

    /// I-QAL2 regression gate: [`REQUIRED_CLEAN_RUNS`] consecutive clean runs.
    /// REQ: REQ-QAL-013.
    #[must_use]
    pub fn gate_pass(&self) -> bool {
        self.consecutive_clean() >= REQUIRED_CLEAN_RUNS
    }

    /// Renders the per-run anomaly trend and the gate verdict.
    /// REQ: REQ-QAL-014.
    #[must_use]
    pub fn dashboard(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "soak dashboard — {} run(s), clean streak {}/{}, gate {}",
            self.runs.len(),
            self.consecutive_clean(),
            REQUIRED_CLEAN_RUNS,
            if self.gate_pass() { "PASS" } else { "PENDING" }
        );
        let _ = writeln!(
            out,
            "run  topology         hours  iters  anomalies  clean  finished_unix_s"
        );
        for (i, run) in self.runs.iter().enumerate() {
            let _ = writeln!(
                out,
                "{:<4} {:<16} {:<6} {:<6} {:<10} {:<6} {}",
                i + 1,
                run.topology.name(),
                run.hours,
                run.iterations,
                run.anomalies,
                if run.clean { "yes" } else { "no" },
                run.finished_unix_s
            );
        }
        out
    }
}

/// Current Unix time in whole seconds (`0` before the epoch).
/// REQ: REQ-QAL-014.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Loads the ledger at `path` (empty when absent).
/// REQ: REQ-QAL-014.
///
/// # Errors
/// [`anyhow::Error`] when the file exists but cannot be read or parsed.
pub fn load(path: &Path) -> Result<SoakLedger> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| anyhow!("parse soak ledger {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SoakLedger::default()),
        Err(e) => Err(anyhow!("read soak ledger {}: {e}", path.display())),
    }
}

/// Writes `ledger` to `path` as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-014.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
pub fn save(path: &Path, ledger: &SoakLedger) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| anyhow!("create {}: {e}", parent.display()))?;
    }
    let text =
        serde_json::to_string_pretty(ledger).map_err(|e| anyhow!("serialize ledger: {e}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| anyhow!("write soak ledger {}: {e}", path.display()))?;
    Ok(())
}

/// Parsed `xtask soak` flags.
#[derive(Debug)]
struct Options {
    hours: Option<u64>,
    iterations: Option<u64>,
    topology: Option<Topology>,
    ledger: PathBuf,
    dashboard: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hours: None,
            iterations: None,
            topology: None,
            ledger: PathBuf::from(DEFAULT_LEDGER),
            dashboard: false,
        }
    }
}

/// Returns the value following a value-taking flag.
/// REQ: REQ-QAL-013.
///
/// # Errors
/// [`anyhow::Error`] when the flag has no following value.
fn value<'a>(flags: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    flags
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("soak flag `{flag}` needs a value"))
}

/// Parses `xtask soak` flags.
/// REQ: REQ-QAL-013.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags or missing flag values.
fn parse_options(flags: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut i = 0;
    while i < flags.len() {
        let flag = flags[i].as_str();
        match flag {
            "--dashboard" => options.dashboard = true,
            "--duration" => {
                i += 1;
                options.hours = Some(parse_hours(value(flags, i, flag)?)?);
            }
            "--iterations" => {
                i += 1;
                options.iterations = Some(parse_count(value(flags, i, flag)?)?);
            }
            "--topology" => {
                i += 1;
                options.topology = Some(Topology::parse(value(flags, i, flag)?)?);
            }
            "--ledger" => {
                i += 1;
                options.ledger = PathBuf::from(value(flags, i, flag)?);
            }
            other => bail!("unknown soak flag `{other}`; see docs/spec/soak-ledger.md"),
        }
        i += 1;
    }
    Ok(options)
}

/// Parses the `--duration` value as whole hours (at least 1).
/// REQ: REQ-QAL-005, REQ-QAL-013.
///
/// # Errors
/// [`anyhow::Error`] when the value is not an integer hour or is zero.
fn parse_hours(raw: &str) -> Result<u64> {
    let hours: u64 = raw
        .parse()
        .map_err(|_| anyhow!("soak duration `{raw}` is not an integer hour"))?;
    if hours == 0 {
        bail!("soak duration must be at least 1 hour");
    }
    Ok(hours)
}

/// Parses the `--iterations` value (at least 1).
/// REQ: REQ-QAL-013.
///
/// # Errors
/// [`anyhow::Error`] when the value is not a positive integer.
fn parse_count(raw: &str) -> Result<u64> {
    let count: u64 = raw
        .parse()
        .map_err(|_| anyhow!("soak iterations `{raw}` is not an integer"))?;
    if count == 0 {
        bail!("soak iterations must be at least 1");
    }
    Ok(count)
}

/// Soak run budget.
#[derive(Debug, Clone, Copy)]
enum Budget {
    /// Run until the wall-clock duration elapses (at least one iteration).
    Duration(Duration),
    /// Run exactly this many iterations.
    Iterations(u64),
}

/// Result of executing a soak budget.
struct Outcome {
    iterations: u64,
    anomalies: u64,
    error: Option<String>,
}

/// Repeats the workspace suite until the budget elapses; the first failing
/// iteration is recorded as an anomaly and returned without further runs
/// (triage-first).
/// REQ: REQ-QAL-013.
fn execute(budget: Budget) -> Outcome {
    let start = Instant::now();
    let mut iterations = 0_u64;
    loop {
        if let Err(e) = crate::spawn("cargo", &["test", "--workspace"]) {
            return Outcome {
                iterations: iterations + 1,
                anomalies: 1,
                error: Some(format!("{e:#}")),
            };
        }
        iterations += 1;
        let done = match budget {
            Budget::Iterations(n) => iterations >= n,
            Budget::Duration(d) => start.elapsed() >= d,
        };
        if done {
            return Outcome {
                iterations,
                anomalies: 0,
                error: None,
            };
        }
    }
}

/// `cargo xtask soak`: runs the escalation harness or renders the dashboard.
/// REQ: REQ-QAL-005, REQ-QAL-013, REQ-QAL-014.
///
/// # Errors
/// [`anyhow::Error`] for unknown flags, a missing or duplicate budget, an
/// unreadable ledger, a write failure, or a recorded anomaly (triage-first).
pub fn run(flags: &[String]) -> Result<()> {
    let options = parse_options(flags)?;
    if options.dashboard {
        let ledger = load(&options.ledger)?;
        print!("{}", ledger.dashboard());
        return Ok(());
    }
    let budget = match (options.hours, options.iterations) {
        (Some(hours), None) => {
            let seconds = hours
                .checked_mul(3600)
                .ok_or_else(|| anyhow!("soak duration {hours} h overflows"))?;
            Budget::Duration(Duration::from_secs(seconds))
        }
        (None, Some(iterations)) => Budget::Iterations(iterations),
        _ => bail!("soak: pass exactly one of --duration <h> or --iterations <n>"),
    };
    let mut ledger = load(&options.ledger)?;
    let topology = options
        .topology
        .unwrap_or_else(|| Topology::next_after(ledger.runs.len()));
    let outcome = execute(budget);
    ledger.runs.push(SoakRun::new(
        topology,
        options.hours.unwrap_or(0),
        outcome.iterations,
        outcome.anomalies,
    ));
    save(&options.ledger, &ledger)?;
    print!("{}", ledger.dashboard());
    if let Some(error) = outcome.error {
        bail!(
            "soak anomaly on {} (triage-first): {error}",
            topology.name()
        );
    }
    if ledger.gate_pass() {
        eprintln!(
            "soak: {REQUIRED_CLEAN_RUNS} consecutive clean runs — I-QAL2 regression gate PASS"
        );
    }
    Ok(())
}

/// REQ: REQ-QAL-013, REQ-QAL-014 — topology rotation, the three-consecutive-
/// clean gate rule, ledger JSON round-trip, and the dashboard trend.
#[cfg(test)]
mod tests {
    use super::{
        Options, REQUIRED_CLEAN_RUNS, SoakLedger, SoakRun, Topology, parse_count, parse_hours,
        parse_options,
    };

    /// A synthetic run record with the given topology and cleanliness.
    /// REQ: REQ-QAL-014.
    fn run(topology: Topology, clean: bool) -> SoakRun {
        SoakRun {
            topology,
            hours: 24,
            iterations: 10,
            anomalies: u64::from(!clean),
            clean,
            finished_unix_s: 0,
        }
    }

    /// REQ: REQ-QAL-013 — rotation covers all three topologies in order and
    /// then repeats; every name round-trips through `Topology::parse`.
    #[test]
    fn topology_rotation_covers_all() {
        assert_eq!(
            [
                Topology::next_after(0),
                Topology::next_after(1),
                Topology::next_after(2),
                Topology::next_after(3),
            ],
            [
                Topology::SingleMachine,
                Topology::TwoMachineSim,
                Topology::PhysicalPair,
                Topology::SingleMachine,
            ]
        );
        for topology in Topology::ALL {
            let parsed = Topology::parse(topology.name())
                .unwrap_or_else(|e| unreachable!("parse {}: {e}", topology.name()));
            assert_eq!(parsed, topology, "name round-trip");
        }
    }

    /// REQ: REQ-QAL-013 — an unknown topology is a typed error, never a silent
    /// default.
    #[test]
    fn topology_parse_rejects_unknown() {
        for bad in ["", "single", "two-machine", "physical", "SINGLE-MACHINE"] {
            assert!(Topology::parse(bad).is_err(), "bad topology: {bad}");
        }
    }

    /// REQ: REQ-QAL-013 — the gate needs three consecutive clean runs; an
    /// anomaly resets the streak; the three-clean clock restarts only on an
    /// anomaly.
    #[test]
    fn three_consecutive_clean_rule() {
        let mut ledger = SoakLedger::default();
        ledger.runs.push(run(Topology::SingleMachine, true));
        ledger.runs.push(run(Topology::TwoMachineSim, true));
        assert_eq!(ledger.consecutive_clean(), 2);
        assert!(!ledger.gate_pass(), "two clean runs are not enough");
        ledger.runs.push(run(Topology::PhysicalPair, true));
        assert_eq!(ledger.consecutive_clean(), REQUIRED_CLEAN_RUNS);
        assert!(ledger.gate_pass(), "three consecutive clean runs pass");
        ledger.runs.push(run(Topology::SingleMachine, false));
        assert_eq!(ledger.consecutive_clean(), 0, "anomaly resets the streak");
        assert!(!ledger.gate_pass());
        ledger.runs.push(run(Topology::TwoMachineSim, true));
        ledger.runs.push(run(Topology::PhysicalPair, true));
        assert!(
            !ledger.gate_pass(),
            "two clean after an anomaly are not enough"
        );
        ledger.runs.push(run(Topology::SingleMachine, true));
        assert!(ledger.gate_pass(), "three clean after the anomaly restart");
    }

    /// REQ: REQ-QAL-014 — the ledger round-trips through JSON and file I/O,
    /// and an absent ledger loads as empty.
    #[test]
    fn ledger_json_and_file_round_trip() {
        let mut ledger = SoakLedger::default();
        ledger.runs.push(run(Topology::SingleMachine, true));
        ledger.runs.push(run(Topology::TwoMachineSim, false));
        let json = serde_json::to_string(&ledger).unwrap_or_else(|e| unreachable!("ser: {e}"));
        let back: SoakLedger =
            serde_json::from_str(&json).unwrap_or_else(|e| unreachable!("de: {e}"));
        assert_eq!(back, ledger, "JSON round-trip");

        let dir = std::env::temp_dir().join(format!("drm-soak-ledger-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("tmpdir: {e}"));
        let path = dir.join("soak-ledger.json");
        super::save(&path, &ledger).unwrap_or_else(|e| unreachable!("save: {e}"));
        let loaded = super::load(&path).unwrap_or_else(|e| unreachable!("load: {e}"));
        assert_eq!(loaded, ledger, "file round-trip");
        let missing = dir.join("missing.json");
        let empty = super::load(&missing).unwrap_or_else(|e| unreachable!("missing: {e}"));
        assert!(empty.runs.is_empty(), "absent ledger is empty");
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-QAL-014 — the dashboard trends the anomaly counter per run and
    /// reports the gate verdict.
    #[test]
    fn dashboard_reports_trend() {
        let mut ledger = SoakLedger::default();
        ledger.runs.push(run(Topology::SingleMachine, true));
        ledger.runs.push(run(Topology::TwoMachineSim, false));
        let board = ledger.dashboard();
        assert!(board.contains("single-machine"), "topology name: {board}");
        assert!(board.contains("two-machine-sim"), "topology name: {board}");
        assert!(board.contains("anomalies"), "anomaly column: {board}");
        assert!(board.contains("gate PENDING"), "gate verdict: {board}");
        assert!(
            board.contains("1 run") || board.contains("2 run"),
            "run count: {board}"
        );

        let mut clean = SoakLedger::default();
        for topology in Topology::ALL {
            clean.runs.push(run(topology, true));
        }
        assert!(clean.dashboard().contains("gate PASS"));
    }

    /// REQ: REQ-QAL-005, REQ-QAL-013 — duration and iteration parsing: positive
    /// integers only, zero and non-numeric input rejected.
    #[test]
    fn budget_parsing() {
        assert_eq!(
            parse_hours("24").unwrap_or_else(|e| unreachable!("24 h: {e}")),
            24
        );
        assert_eq!(
            parse_count("3").unwrap_or_else(|e| unreachable!("3 iters: {e}")),
            3
        );
        for bad in ["0", "1.5", "h", "", "-1"] {
            assert!(parse_hours(bad).is_err(), "bad duration: {bad}");
            assert!(parse_count(bad).is_err(), "bad iterations: {bad}");
        }
    }

    /// REQ: REQ-QAL-013 — the flag parser fills every option and rejects
    /// unknown flags and missing values.
    #[test]
    fn options_parse_flags() {
        let flags: Vec<String> = [
            "--iterations",
            "2",
            "--topology",
            "physical-pair",
            "--ledger",
            "target/soak.json",
            "--dashboard",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert_eq!(options.iterations, Some(2));
        assert_eq!(options.hours, None);
        assert_eq!(options.topology, Some(Topology::PhysicalPair));
        assert_eq!(options.ledger, std::path::PathBuf::from("target/soak.json"));
        assert!(options.dashboard);
        assert_eq!(
            Options::default().ledger,
            std::path::PathBuf::from("docs/metrics/soak-ledger.json")
        );

        assert!(parse_options(&["--bogus".to_string()]).is_err());
        assert!(parse_options(&["--duration".to_string()]).is_err());
        assert!(parse_options(&["--topology".to_string()]).is_err());
    }
}
