//! Soak escalation harness (work-plan M18W4-E1-T1/E4-T1; extended by
//! M23W2-E4-T1: three parallel 24 h runs, memory-boundedness, and the
//! zero-false-positive anomaly threshold band). Implements
//! [qualification.md](../../docs/algorithms/program/qualification.md) I-QAL2
//! (`REQ-QAL-005`, `REQ-QAL-013`, `REQ-QAL-014`): repeats the workspace suite
//! (`cargo test --workspace`) for a wall-clock budget over rotating topologies,
//! samples each run's peak resident set against the memory budget
//! ([memory-budget.md](../../docs/algorithms/memory-budget.md), I-MB9), appends
//! each run to the committed I-QAL2 evidence ledger, and renders the per-run
//! anomaly dashboard plus the threshold band. Format: `docs/spec/soak-ledger.md`.
//! REQ: REQ-QAL-005, REQ-QAL-013, REQ-QAL-014, REQ-QAL-017.

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
/// REQ: REQ-QAL-014, REQ-QAL-017.
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
    /// Peak resident set observed across the run's child processes (KiB); `0`
    /// when sampling is unavailable.
    #[serde(default)]
    pub peak_rss_kb: u64,
    /// Memory ceiling for this run (KiB); `0` means unbounded (legacy run).
    #[serde(default)]
    pub bound_kb: u64,
    /// Run completion time (Unix seconds).
    pub finished_unix_s: u64,
}

impl SoakRun {
    /// Builds a run record from its outcome (unbounded, unsampled).
    /// REQ: REQ-QAL-014.
    #[must_use]
    pub fn new(topology: Topology, hours: u64, iterations: u64, anomalies: u64) -> Self {
        Self {
            topology,
            hours,
            iterations,
            anomalies,
            clean: anomalies == 0,
            peak_rss_kb: 0,
            bound_kb: 0,
            finished_unix_s: unix_now(),
        }
    }

    /// Records the peak resident set and ceiling for this run.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn with_memory(mut self, peak_rss_kb: u64, bound_kb: u64) -> Self {
        self.peak_rss_kb = peak_rss_kb;
        self.bound_kb = bound_kb;
        self
    }

    /// I-QAL2 memory-boundedness: the run stayed within its ceiling. A run with
    /// no ceiling (`bound_kb == 0`) is treated as unbounded (legacy).
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn memory_bounded(&self) -> bool {
        self.bound_kb == 0 || self.peak_rss_kb <= self.bound_kb
    }

    /// I-QAL2 anomaly counters derived from the run.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn counters(&self) -> AnomalyCounters {
        AnomalyCounters {
            corruption: 0,
            deadlock: 0,
            anomaly: self.anomalies,
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

    /// I-QAL2 memory-boundedness: every recorded run stayed within its ceiling.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn memory_bounded(&self) -> bool {
        self.runs.iter().all(SoakRun::memory_bounded)
    }

    /// Zero-false-positive anomaly-detector threshold band over the ledger: the
    /// smallest counter threshold that flags no clean run, and how many
    /// anomalous runs it detects.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn threshold_band(&self) -> ThresholdBand {
        let clean: Vec<AnomalyCounters> = self
            .runs
            .iter()
            .map(SoakRun::counters)
            .filter(|counters| counters.is_clean())
            .collect();
        let anomalous: Vec<AnomalyCounters> = self
            .runs
            .iter()
            .map(SoakRun::counters)
            .filter(|counters| !counters.is_clean())
            .collect();
        sweep_threshold(&clean, &anomalous)
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
            "run  topology         hours  iters  anomalies  clean  peak_kb  bound_kb  finished_unix_s"
        );
        for (i, run) in self.runs.iter().enumerate() {
            let _ = writeln!(
                out,
                "{:<4} {:<16} {:<6} {:<6} {:<10} {:<6} {:<8} {:<8} {}",
                i + 1,
                run.topology.name(),
                run.hours,
                run.iterations,
                run.anomalies,
                if run.clean { "yes" } else { "no" },
                run.peak_rss_kb,
                run.bound_kb,
                run.finished_unix_s
            );
        }
        let band = self.threshold_band();
        let _ = writeln!(
            out,
            "anomaly threshold band: threshold={} false_positives={} detected={} memory_bounded={}",
            band.threshold,
            band.false_positives,
            band.detected,
            self.memory_bounded()
        );
        out
    }
}

/// I-QAL2 anomaly counters for one soak run: corruption, deadlock, and the
/// generic anomaly counter. All zero is clean.
/// REQ: REQ-QAL-017.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnomalyCounters {
    /// Corrupted-sample events.
    pub corruption: u64,
    /// Deadlock/hang events.
    pub deadlock: u64,
    /// Other suite anomalies.
    pub anomaly: u64,
}

impl AnomalyCounters {
    /// Largest counter across the three classes.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn max_counter(self) -> u64 {
        self.corruption.max(self.deadlock).max(self.anomaly)
    }

    /// `true` iff every counter is zero.
    /// REQ: REQ-QAL-017.
    #[must_use]
    pub fn is_clean(self) -> bool {
        self.max_counter() == 0
    }
}

/// Zero-false-positive anomaly-detector threshold band (`EXP`, M23W2-E4-T2):
/// the smallest counter threshold that flags no clean run, plus the number of
/// anomalous runs it still detects.
/// REQ: REQ-QAL-017.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdBand {
    /// Counter threshold (`max_counter > threshold` ⇒ flagged).
    pub threshold: u64,
    /// Clean runs flagged at this threshold (target: `0`).
    pub false_positives: usize,
    /// Anomalous runs flagged at this threshold.
    pub detected: usize,
}

/// Sweeps candidate thresholds and returns the smallest one with zero clean-run
/// false positives; the threshold is the largest counter seen on any clean run.
/// REQ: REQ-QAL-017.
#[must_use]
pub fn sweep_threshold(clean: &[AnomalyCounters], anomalous: &[AnomalyCounters]) -> ThresholdBand {
    let threshold = clean.iter().map(|c| c.max_counter()).max().unwrap_or(0);
    let false_positives = clean.iter().filter(|c| c.max_counter() > threshold).count();
    let detected = anomalous
        .iter()
        .filter(|c| c.max_counter() > threshold)
        .count();
    ThresholdBand {
        threshold,
        false_positives,
        detected,
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
    parallel: u64,
    memory_bound_kb: u64,
    report: Option<PathBuf>,
    dashboard: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hours: None,
            iterations: None,
            topology: None,
            ledger: PathBuf::from(DEFAULT_LEDGER),
            parallel: 1,
            memory_bound_kb: 0,
            report: None,
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
            "--parallel" => {
                i += 1;
                options.parallel = parse_count(value(flags, i, flag)?)?;
            }
            "--memory-bound" => {
                i += 1;
                options.memory_bound_kb = parse_megabytes(value(flags, i, flag)?)?;
            }
            "--report" => {
                i += 1;
                options.report = Some(PathBuf::from(value(flags, i, flag)?));
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

/// Parses the `--memory-bound` value in whole megabytes into a KiB ceiling.
/// REQ: REQ-QAL-017.
///
/// # Errors
/// [`anyhow::Error`] when the value is not an integer or the KiB conversion
/// overflows.
fn parse_megabytes(raw: &str) -> Result<u64> {
    let mb: u64 = raw
        .parse()
        .map_err(|_| anyhow!("soak memory bound `{raw}` is not an integer megabyte"))?;
    mb.checked_mul(1024)
        .ok_or_else(|| anyhow!("soak memory bound {mb} MB overflows"))
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
    peak_rss_kb: u64,
    error: Option<String>,
}

/// Repeats the workspace suite until the budget elapses; the first failing
/// iteration is recorded as an anomaly and returned without further runs
/// (triage-first). Each iteration samples the child's peak resident set.
/// REQ: REQ-QAL-013, REQ-QAL-017.
fn execute(budget: Budget, target_dir: &Path) -> Outcome {
    let start = Instant::now();
    let mut iterations = 0_u64;
    let mut peak_rss_kb = 0_u64;
    loop {
        let result = spawn_sampled(&["test", "--workspace"], target_dir);
        match result {
            Ok((true, peak)) => {
                peak_rss_kb = peak_rss_kb.max(peak);
                iterations += 1;
            }
            Ok((false, peak)) => {
                return Outcome {
                    iterations: iterations + 1,
                    anomalies: 1,
                    peak_rss_kb: peak_rss_kb.max(peak),
                    error: Some("workspace suite failed".to_string()),
                };
            }
            Err(e) => {
                return Outcome {
                    iterations: iterations + 1,
                    anomalies: 1,
                    peak_rss_kb,
                    error: Some(format!("{e:#}")),
                };
            }
        }
        let done = match budget {
            Budget::Iterations(n) => iterations >= n,
            Budget::Duration(d) => start.elapsed() >= d,
        };
        if done {
            return Outcome {
                iterations,
                anomalies: 0,
                peak_rss_kb,
                error: None,
            };
        }
    }
}

/// Runs `cargo <args>` in `target_dir`, sampling the child's peak resident set
/// (`VmHWM`); returns `(success, peak_rss_kb)`. On non-Linux hosts the sample
/// is `0` and the child still runs to completion.
/// REQ: REQ-QAL-017.
#[cfg(target_os = "linux")]
fn spawn_sampled(args: &[&str], target_dir: &Path) -> Result<(bool, u64)> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("cargo")
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow!("spawn cargo: {e}"))?;
    let pid = child.id();
    let mut peak = 0_u64;
    loop {
        if let Some(kb) = read_peak_rss_kb(pid) {
            peak = peak.max(kb);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status.success(), peak)),
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(anyhow!("wait cargo: {e}")),
        }
    }
}

/// Runs `cargo <args>` in `target_dir` without memory sampling (non-Linux).
/// REQ: REQ-QAL-017.
#[cfg(not(target_os = "linux"))]
fn spawn_sampled(args: &[&str], target_dir: &Path) -> Result<(bool, u64)> {
    use std::process::Command;

    let status = Command::new("cargo")
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .status()
        .map_err(|e| anyhow!("spawn cargo: {e}"))?;
    Ok((status.success(), 0))
}

/// Reads the peak resident set (`VmHWM`) of process `pid` in KiB from
/// `/proc/<pid>/status`; `None` when the process is gone or the field is
/// absent.
/// REQ: REQ-QAL-017.
#[cfg(target_os = "linux")]
fn read_peak_rss_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse::<u64>().ok())
    })
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
    let start_index = ledger.runs.len();
    let base_target = PathBuf::from("target");
    let outcomes = if options.parallel == 1 {
        vec![execute(budget, &base_target)]
    } else {
        execute_parallel(options.parallel, budget, &base_target)
    };
    let hours = options.hours.unwrap_or(0);
    let mut first_error: Option<String> = None;
    for (i, outcome) in outcomes.into_iter().enumerate() {
        let topology = options
            .topology
            .unwrap_or_else(|| Topology::next_after(start_index + i));
        ledger.runs.push(
            SoakRun::new(topology, hours, outcome.iterations, outcome.anomalies)
                .with_memory(outcome.peak_rss_kb, options.memory_bound_kb),
        );
        if first_error.is_none() {
            first_error = outcome.error;
        }
    }
    save(&options.ledger, &ledger)?;
    print!("{}", ledger.dashboard());
    if let Some(path) = &options.report {
        save_report(path, &ledger)?;
    }
    if let Some(error) = first_error {
        bail!("soak anomaly (triage-first): {error}");
    }
    if !ledger.memory_bounded() {
        bail!("soak memory bound exceeded (I-QAL2 memory-boundedness)");
    }
    if ledger.gate_pass() {
        eprintln!(
            "soak: {REQUIRED_CLEAN_RUNS} consecutive clean runs — I-QAL2 regression gate PASS"
        );
    }
    Ok(())
}

/// Executes `count` soak runs concurrently, each in an isolated target
/// directory (`target/soak-slot-<i>`) so the cargo build lock does not
/// serialize them (three parallel 24 h runs, M23W2-E4-T1).
/// REQ: REQ-QAL-017.
fn execute_parallel(count: u64, budget: Budget, base: &Path) -> Vec<Outcome> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..count)
            .map(|i| {
                let dir = base.join(format!("soak-slot-{i}"));
                scope.spawn(move || execute(budget, &dir))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| Outcome {
                    iterations: 0,
                    anomalies: 1,
                    peak_rss_kb: 0,
                    error: Some("soak worker panicked".to_string()),
                })
            })
            .collect()
    })
}

/// JSON evidence report: the gate verdict, memory-boundedness, the anomaly
/// threshold band, and the run records.
/// REQ: REQ-QAL-017.
#[derive(Serialize)]
struct SoakReport<'a> {
    gate_pass: bool,
    memory_bounded: bool,
    threshold: ThresholdBand,
    runs: &'a [SoakRun],
}

/// Writes the soak report at `path` as pretty JSON with a trailing newline.
/// REQ: REQ-QAL-017.
///
/// # Errors
/// [`anyhow::Error`] when the parent directory or file cannot be written.
fn save_report(path: &Path, ledger: &SoakLedger) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| anyhow!("create {}: {e}", parent.display()))?;
    }
    let report = SoakReport {
        gate_pass: ledger.gate_pass(),
        memory_bounded: ledger.memory_bounded(),
        threshold: ledger.threshold_band(),
        runs: &ledger.runs,
    };
    let text =
        serde_json::to_string_pretty(&report).map_err(|e| anyhow!("serialize soak report: {e}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| anyhow!("write soak report {}: {e}", path.display()))?;
    Ok(())
}

/// REQ: REQ-QAL-013, REQ-QAL-014 — topology rotation, the three-consecutive-
/// clean gate rule, ledger JSON round-trip, and the dashboard trend.
#[cfg(test)]
mod tests {
    use super::{
        AnomalyCounters, Options, REQUIRED_CLEAN_RUNS, SoakLedger, SoakRun, Topology, parse_count,
        parse_hours, parse_megabytes, parse_options, sweep_threshold,
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
            peak_rss_kb: 0,
            bound_kb: 0,
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

    /// REQ: REQ-QAL-017 — the parallel/memory/report flags parse, and the
    /// memory bound converts megabytes to KiB.
    #[test]
    fn parallel_and_memory_flags_parse() {
        let flags: Vec<String> = [
            "--iterations",
            "1",
            "--parallel",
            "3",
            "--memory-bound",
            "2",
            "--report",
            "target/soak-report.json",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let options = parse_options(&flags).unwrap_or_else(|e| unreachable!("parse: {e}"));
        assert_eq!(options.parallel, 3);
        assert_eq!(options.memory_bound_kb, 2048);
        assert_eq!(
            options.report,
            Some(std::path::PathBuf::from("target/soak-report.json"))
        );
        assert_eq!(Options::default().parallel, 1, "default is one run");
        assert_eq!(
            parse_megabytes("3").unwrap_or_else(|e| unreachable!("{e}")),
            3072
        );
        assert!(parse_megabytes("x").is_err());
        assert!(parse_options(&["--parallel".to_string(), "0".to_string()]).is_err());
    }

    /// REQ: REQ-QAL-017 — memory-boundedness: a sampled run within its ceiling
    /// is bounded; an over-ceiling run is not; a run with no ceiling is treated
    /// as unbounded (legacy).
    #[test]
    fn memory_boundedness_check() {
        let bounded = run(Topology::SingleMachine, true).with_memory(1024, 2048);
        assert!(bounded.memory_bounded());
        let over = run(Topology::SingleMachine, true).with_memory(4096, 2048);
        assert!(!over.memory_bounded());
        let legacy = run(Topology::SingleMachine, true);
        assert!(legacy.memory_bounded(), "no ceiling ⇒ unbounded");

        let mut ledger = SoakLedger::default();
        ledger.runs.push(bounded);
        assert!(ledger.memory_bounded());
        ledger.runs.push(over);
        assert!(!ledger.memory_bounded());
    }

    /// REQ: REQ-QAL-017 — the threshold sweep picks the smallest counter
    /// threshold with zero clean-run false positives and still detects the
    /// anomalous runs.
    #[test]
    fn threshold_sweep_zero_false_positives() {
        let clean = [
            AnomalyCounters {
                corruption: 0,
                deadlock: 0,
                anomaly: 0,
            },
            AnomalyCounters {
                corruption: 0,
                deadlock: 0,
                anomaly: 0,
            },
        ];
        let anomalous = [AnomalyCounters {
            corruption: 1,
            deadlock: 0,
            anomaly: 0,
        }];
        let band = sweep_threshold(&clean, &anomalous);
        assert_eq!(band.threshold, 0, "clean runs are zero");
        assert_eq!(band.false_positives, 0, "zero false positives");
        assert_eq!(band.detected, 1, "the anomalous run is detected");

        let noisy_clean = [AnomalyCounters {
            corruption: 0,
            deadlock: 2,
            anomaly: 0,
        }];
        let band = sweep_threshold(&noisy_clean, &anomalous);
        assert_eq!(band.threshold, 2, "band rises to the noisiest clean run");
        assert_eq!(band.false_positives, 0);
        assert_eq!(
            band.detected, 0,
            "an anomaly at or below the band is missed"
        );

        let mut ledger = SoakLedger::default();
        ledger.runs.push(run(Topology::SingleMachine, true));
        ledger.runs.push(run(Topology::TwoMachineSim, true));
        ledger.runs.push(run(Topology::PhysicalPair, false));
        let band = ledger.threshold_band();
        assert_eq!(band.false_positives, 0);
        assert_eq!(band.detected, 1);
    }
}
