//! System performance matrix (`cargo xtask perf-matrix`): measures the runtime
//! data plane over the declared payload classes of `tests/perf/workspace`
//! (64 B fixed, 1 MiB fixed, 32 B backpressure, 4 KiB-bounded variable, 128 B
//! deep history), checks the results against the declared budgets and the
//! published baseline bands, and records the cross-process segment
//! open/adopt lifecycle (one segment, two processes).
//!
//! The in-process data plane is the production [`drm_core::channel::Channel`].
//! Per-topic payload capacities are read from the codegen output
//! (`SAMPLE_MAX`) and the topic attributes from the `.drm` sources, so the
//! matrix is driven by the fixtures and never by duplicated constants.
//!
//! Implements [bench-methodology.md](../../docs/spec/bench-methodology.md)
//! §System perf matrix; the baseline comparison targets
//! [bench-baseline.md](../../docs/spec/bench-baseline.md).
//!
//! REQ: REQ-QAL-011, REQ-QAL-012, REQ-TOOL-004, REQ-CORE-054, REQ-CORE-071.
//!
//! Known limit (recorded, not hidden): the cross-process section exercises
//! segment create/adopt and authority, because no runtime API publishes
//! *through* a mapped segment yet — the node-facing C ABI is a single
//! in-process channel (`crates/drm-ffi/src/lib.rs` §Context). Sample movement
//! across processes is therefore NOT measured here.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{LayoutSpec, consumer_open, publisher_open, segment_layout};
use drm_core::channel::{Channel, OverflowPolicy};
use drm_core::shm::{FlockBackend, LockPaths, PosixShm, SegmentName, ShmBackend};

/// Fixture workspace measured by default.
/// REQ: REQ-QAL-011.
const DEFAULT_WORKSPACE: &str = "tests/perf/workspace";
/// Default JSON output path.
/// REQ: REQ-QAL-011.
const DEFAULT_JSON: &str = "target/perf-matrix.json";
/// Steady-state core p99 ceiling for the publish and read paths
/// (bench-methodology.md §In-bench invariants).
/// REQ: REQ-QAL-012.
const CORE_P99_BUDGET_NS: u64 = 50_000;
/// Published `publish+latest` p50 baseline (bench-baseline.md, M23W4).
/// REQ: REQ-QAL-011.
const BASELINE_PUBLISH_P50_NS: u64 = 190;
/// Published `publish+latest` p99 baseline (bench-baseline.md, M23W4).
/// REQ: REQ-QAL-011.
const BASELINE_PUBLISH_P99_NS: u64 = 200;
/// p50 run-to-run noise band (fraction).
/// REQ: REQ-QAL-011.
const NOISE_P50: f64 = 0.20;
/// p99 run-to-run noise band (fraction).
/// REQ: REQ-QAL-011.
const NOISE_P99: f64 = 0.30;
/// Payload slot alignment used for every measured channel.
/// REQ: REQ-CORE-015.
const SLOT_ALIGN: u64 = 8;
/// Topic handle used for the synthetic expectation (one topic per segment).
/// REQ: REQ-CORE-016.
const PERF_TOPIC_HANDLE: u32 = 42;
/// Lock directory for the cross-process section (under `target/`).
/// REQ: REQ-CORE-069.
const LOCK_DIR: &str = "target/perf-locks";

/// Payload ceiling for the baseline-band comparison: `bench-baseline.md`
/// records a 64-byte sample, so larger payload classes are not comparable.
/// REQ: REQ-QAL-011.
const BASELINE_PAYLOAD_MAX: u64 = 4096;

/// One topic declared by the fixture workspace.
/// REQ: REQ-QAL-011.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopicSpec {
    /// Canonical topic path (`package/topic`).
    name: String,
    /// Declared payload capacity in bytes (codegen `SAMPLE_MAX`).
    sample_max: u64,
    /// History depth R.
    depth: u64,
    /// Payload slot count.
    pool: u64,
    /// `true` for `@overflow: reject`.
    reject: bool,
}

/// One measured in-process row.
/// REQ: REQ-QAL-011.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopicRow {
    /// Canonical topic path.
    topic: String,
    /// Declared payload capacity in bytes.
    sample_max: u64,
    /// History depth R.
    depth: u64,
    /// Payload slot count.
    pool: u64,
    /// Overflow policy label.
    policy: String,
    /// Publish p50 (ns).
    publish_p50_ns: u64,
    /// Publish p99 (ns).
    publish_p99_ns: u64,
    /// `latest` read p50 (ns).
    read_p50_ns: u64,
    /// `latest` read p99 (ns).
    read_p99_ns: u64,
    /// Write-loan borrow round-trip p50 (ns).
    borrow_p50_ns: u64,
    /// Write-loan borrow round-trip p99 (ns).
    borrow_p99_ns: u64,
    /// Achieved publish rate (ops/s).
    ops_per_sec: u64,
    /// Achieved publish bandwidth (MiB/s).
    mib_per_sec: f64,
    /// Deployed segment footprint in bytes (`segment_layout`).
    segment_bytes: u64,
}

/// Cross-process segment evidence.
/// REQ: REQ-CORE-054, REQ-CORE-071.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CrossProcessRow {
    /// Segment size in bytes.
    segment_bytes: u64,
    /// Publisher create/adopt latency (ns).
    publisher_open_ns: u64,
    /// Publisher epoch observed after acquisition.
    epoch: u32,
    /// Consumer (separate process) open latency (ns), `None` when it failed.
    consumer_open_ns: Option<u64>,
    /// Whether the second process attached successfully.
    consumer_attached: bool,
    /// Whether a second publisher was refused while the first held authority.
    second_publisher_refused: bool,
}

/// One budget verdict.
/// REQ: REQ-QAL-012.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BudgetRow {
    /// Topic the verdict applies to.
    topic: String,
    /// Metric name.
    metric: String,
    /// Measured value (ns).
    measured_ns: u64,
    /// Declared ceiling (ns).
    budget_ns: u64,
    /// `PASS` or `FAIL`.
    verdict: String,
}

/// The complete matrix.
/// REQ: REQ-QAL-011.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Matrix {
    /// Host and kernel description.
    host: String,
    /// Fixture workspace path.
    workspace: String,
    /// Per-topic rows.
    topics: Vec<TopicRow>,
    /// Cross-process evidence.
    cross_process: CrossProcessRow,
    /// Budget verdicts.
    budgets: Vec<BudgetRow>,
    /// True when every budget verdict passed.
    all_pass: bool,
}

/// Entry point: `cargo xtask perf-matrix [--workspace <dir>] [--json <path>]
/// [--quick] [--child consumer-open <args...>]`.
/// REQ: REQ-QAL-011.
///
/// # Errors
/// Propagation from the fixture scan, the runtime calls, or the JSON write.
pub fn run(flags: &[String]) -> Result<()> {
    let mut workspace = PathBuf::from(DEFAULT_WORKSPACE);
    let mut json = PathBuf::from(DEFAULT_JSON);
    let mut quick = false;
    let mut child: Option<Vec<String>> = None;
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--workspace" => {
                workspace = PathBuf::from(need(flags, index, "--workspace")?);
                index += 2;
            }
            "--json" => {
                json = PathBuf::from(need(flags, index, "--json")?);
                index += 2;
            }
            "--quick" => {
                quick = true;
                index += 1;
            }
            "--child" => {
                child = Some(flags[index + 1..].to_vec());
                index = flags.len();
            }
            other => bail!("unknown perf-matrix flag `{other}`; see xtask/README.md"),
        }
    }
    if let Some(rest) = child {
        return run_child(&rest);
    }
    let matrix = measure(&workspace, quick)?;
    let json_text = serde_json::to_string_pretty(&matrix).context("serialize matrix")?;
    std::fs::write(&json, &json_text).with_context(|| format!("write {}", json.display()))?;
    print_matrix(&matrix, &json);
    if matrix.all_pass {
        Ok(())
    } else {
        bail!("perf matrix: one or more budget ceilings failed (see table above)")
    }
}

/// Positional flag value (the token after `flag`).
/// REQ: REQ-QAL-011.
fn need<'a>(flags: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    match flags.get(index + 1) {
        Some(v) => Ok(v.as_str()),
        None => bail!("`{flag}` requires a value"),
    }
}

/// Measures the whole matrix.
/// REQ: REQ-QAL-011.
fn measure(workspace: &Path, quick: bool) -> Result<Matrix> {
    let specs = scan_specs(workspace)?;
    if specs.is_empty() {
        bail!(
            "no topic specs found under {} (run `drm build` on the fixture workspace first)",
            workspace.display()
        );
    }
    let iters = if quick { 2_000 } else { 20_000 };
    let budget = if quick {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(400)
    };
    let mut rows = Vec::with_capacity(specs.len());
    for spec in &specs {
        rows.push(measure_topic(spec, iters, budget)?);
    }
    let cross_process = cross_process(&specs[0], quick)?;
    let budgets = check_budgets(&rows);
    let all_pass = budgets.iter().all(|b| b.verdict == "PASS");
    Ok(Matrix {
        host: host_description(),
        workspace: workspace.display().to_string(),
        topics: rows,
        cross_process,
        budgets,
        all_pass,
    })
}

/// Scans `workspace/packages/*/topics/*.drm` for declared attributes and reads
/// each topic's capacity from the generated `SAMPLE_MAX` constant.
/// REQ: REQ-QAL-011.
fn scan_specs(workspace: &Path) -> Result<Vec<TopicSpec>> {
    let packages = workspace.join("packages");
    let mut specs = Vec::new();
    let entries = match std::fs::read_dir(&packages) {
        Ok(entries) => entries,
        Err(e) => bail!("read {}: {e}", packages.display()),
    };
    for entry in entries {
        let package = entry.context("read package entry")?.path();
        let topics = package.join("topics");
        let Ok(files) = std::fs::read_dir(&topics) else {
            continue;
        };
        for file in files {
            let path = file.context("read topic entry")?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("drm") {
                continue;
            }
            let package_name = match package.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let topic_name = match path.file_stem().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let canonical = format!("{package_name}/{topic_name}");
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let sample_max = generated_capacity(workspace, &canonical)?;
            specs.push(TopicSpec {
                name: canonical,
                sample_max,
                depth: attribute(&text, "@history")?,
                pool: attribute(&text, "@pool_size")?,
                reject: text.contains("@overflow: reject"),
            });
        }
    }
    specs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(specs)
}

/// Reads `SAMPLE_MAX` from the generated Rust topic module (authoritative
/// capacity produced by the real codegen).
/// REQ: REQ-QAL-011.
fn generated_capacity(workspace: &Path, canonical: &str) -> Result<u64> {
    let path = workspace
        .join(".drm/gen/rust")
        .join(canonical)
        .join("target.rs");
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "read {} — run `drm build` on the fixture workspace first",
            path.display()
        )
    })?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("pub const SAMPLE_MAX: usize = ") {
            let digits = rest.trim_end_matches(';').trim();
            return digits
                .parse::<u64>()
                .with_context(|| format!("parse SAMPLE_MAX `{digits}`"));
        }
    }
    bail!("no SAMPLE_MAX in {}", path.display())
}

/// Parses a numeric topic attribute (`@history: 8`, `@pool_size: 12`).
/// REQ: REQ-QAL-011.
fn attribute(text: &str, key: &str) -> Result<u64> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let value = rest.trim_start_matches(':').trim();
            if value == "latest" {
                return Ok(1);
            }
            return value
                .parse::<u64>()
                .with_context(|| format!("parse {key} `{value}`"));
        }
    }
    bail!("topic is missing `{key}` (the perf fixtures always declare it)")
}

/// Measures one topic's data plane.
/// REQ: REQ-QAL-011, REQ-QAL-012.
fn measure_topic(spec: &TopicSpec, iters: usize, budget: Duration) -> Result<TopicRow> {
    let capacity = usize::try_from(spec.sample_max).context("capacity out of range")?;
    let expected = expectation(spec);
    let layout = segment_layout(&expected, &layout_spec())
        .map_err(|e| anyhow!("segment layout {}: {e:?}", spec.name))?;
    let policy = if spec.reject {
        OverflowPolicy::Reject
    } else {
        OverflowPolicy::Overwrite
    };
    let channel = Channel::activate(
        spec.depth,
        spec.pool,
        spec.sample_max,
        SLOT_ALIGN,
        policy,
        0,
        2,
        None,
    )
    .map_err(|e| anyhow!("activate {}: {e:?}", spec.name))?;
    let src = vec![0x5A_u8; capacity];
    let mut buf = vec![0_u8; capacity];
    let mut state = channel.reader();

    let publish = latencies(iters, || {
        let _ = channel.publish(&src);
    });
    let _ = channel.publish(&src);
    let read = latencies(iters, || {
        let _ = channel.latest(&mut state, 0, &mut buf);
    });
    let borrow = latencies(iters, || {
        if let Ok(loan) = channel.write_acquire(spec.sample_max) {
            if let Ok(dst) = channel.loan_bytes(&loan) {
                dst.copy_from_slice(&src);
            }
            let _ = channel.write_commit(loan, spec.sample_max);
        }
    });
    let (ops_per_sec, mib_per_sec) = throughput(&channel, &src, budget, spec.sample_max);
    Ok(TopicRow {
        topic: spec.name.clone(),
        sample_max: spec.sample_max,
        depth: spec.depth,
        pool: spec.pool,
        policy: if spec.reject { "reject" } else { "overwrite" }.to_string(),
        publish_p50_ns: percentile(&publish, 50),
        publish_p99_ns: percentile(&publish, 99),
        read_p50_ns: percentile(&read, 50),
        read_p99_ns: percentile(&read, 99),
        borrow_p50_ns: percentile(&borrow, 50),
        borrow_p99_ns: percentile(&borrow, 99),
        ops_per_sec,
        mib_per_sec,
        segment_bytes: layout.segment_size,
    })
}

/// Builds the validation expectation for one fixture topic (shared by every
/// process that attaches to the segment).
/// REQ: REQ-CORE-016.
fn expectation(spec: &TopicSpec) -> Expectation {
    Expectation {
        schema_hash: [7; 16],
        layout_hash: [9; 16],
        topic_handle: PERF_TOPIC_HANDLE,
        topic_name: spec.name.clone(),
        payload_capacity: spec.sample_max,
        history_depth: spec.depth,
        pool_size: spec.pool,
        overflow_policy: u8::from(spec.reject),
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    }
}

/// Descriptor flags and slot alignment for the measured channels.
/// REQ: REQ-CORE-015.
fn layout_spec() -> LayoutSpec {
    LayoutSpec {
        flags: 0,
        slot_align: SLOT_ALIGN,
    }
}

/// Per-iteration latencies in nanoseconds, sorted ascending.
/// REQ: REQ-QAL-012.
fn latencies(iters: usize, mut block: impl FnMut()) -> Vec<u64> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        block();
        samples.push(u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    samples.sort_unstable();
    samples
}

/// Percentile of sorted nanosecond samples: floor index `(n-1)*pct/100`,
/// matching `crates/drm-core/benches/common/mod.rs` so the numbers stay
/// comparable with the recorded baselines.
/// REQ: REQ-QAL-012.
#[must_use]
fn percentile(samples: &[u64], pct: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let last = samples.len() - 1;
    let index = (last.saturating_mul(pct) / 100).min(last);
    samples[index]
}

/// Publishes for `budget`, returning (ops/s, MiB/s).
/// REQ: REQ-QAL-011.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn throughput(channel: &Channel, src: &[u8], budget: Duration, sample_max: u64) -> (u64, f64) {
    const BATCH: u64 = 64;
    let mut state = channel.reader();
    let mut buf = vec![0_u8; src.len()];
    let start = Instant::now();
    let mut ops: u64 = 0;
    while start.elapsed() < budget {
        for _ in 0..BATCH {
            if channel.publish(src).is_err() {
                break;
            }
            ops += 1;
        }
        // A live consumer retires slots; without it a `reject` channel exhausts
        // its pool after `pool_size` publishes and the rate reads as zero.
        let _ = channel.latest(&mut state, 0, &mut buf);
    }
    let elapsed = start.elapsed().as_secs_f64().max(f64::MIN_POSITIVE);
    // Rate and byte totals stay far below 2^53; the cast truncation is
    // irrelevant to a reported rate.
    let rate = ops as f64 / elapsed;
    let bytes = ops as f64 * sample_max as f64;
    (rate as u64, bytes / elapsed / (1024.0 * 1024.0))
}

/// Cross-process evidence: one process creates and holds publisher authority
/// while a second process attaches as a consumer (open-only) to the same
/// segment.
/// REQ: REQ-CORE-054, REQ-CORE-071.
fn cross_process(spec: &TopicSpec, _quick: bool) -> Result<CrossProcessRow> {
    let expected = expectation(spec);
    let layout = segment_layout(&expected, &layout_spec())
        .map_err(|e| anyhow!("segment layout {}: {e:?}", spec.name))?;
    std::fs::create_dir_all(LOCK_DIR).with_context(|| format!("create {LOCK_DIR}"))?;
    let locks = LockPaths::new(LOCK_DIR);
    let name = SegmentName::new(&unique_segment_name()).context("segment name")?;
    let lock_path = locks.publisher(PERF_TOPIC_HANDLE);

    let start = Instant::now();
    let publisher = publisher_open(
        &PosixShm,
        &FlockBackend,
        &lock_path,
        &name,
        expected.topic_name.as_bytes(),
        &expected,
        &layout_spec(),
    )
    .map_err(|e| anyhow!("publisher open: {e:?}"))?;
    let publisher_open_ns = nanos(start.elapsed());
    let epoch = publisher.epoch;

    let child = spawn_consumer(&name, spec)?;
    let consumer_open_ns = child.0;
    let consumer_attached = child.1;

    // A second publisher must be refused while the first holds authority (I-L1).
    let second_publisher_refused = publisher_open(
        &PosixShm,
        &FlockBackend,
        &locks.publisher(PERF_TOPIC_HANDLE),
        &name,
        expected.topic_name.as_bytes(),
        &expected,
        &layout_spec(),
    )
    .is_err();

    drop(publisher);
    let _ = PosixShm.unlink(&name);
    Ok(CrossProcessRow {
        segment_bytes: layout.segment_size,
        publisher_open_ns,
        epoch,
        consumer_open_ns,
        consumer_attached,
        second_publisher_refused,
    })
}

/// Spawns this binary as a consumer child; returns (open ns, attached).
/// REQ: REQ-CORE-071.
fn spawn_consumer(name: &SegmentName, spec: &TopicSpec) -> Result<(Option<u64>, bool)> {
    let exe = std::env::current_exe().context("current exe")?;
    let output = Command::new(exe)
        .arg("perf-matrix")
        .arg("--child")
        .arg("consumer-open")
        .arg(name.as_str())
        .arg(spec.name.clone())
        .arg(spec.sample_max.to_string())
        .arg(spec.depth.to_string())
        .arg(spec.pool.to_string())
        .arg(u8::from(spec.reject).to_string())
        .env("DRM_PERF_CHILD", "1")
        .output()
        .context("spawn consumer child")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(rest) = stdout.split("CONSUMER_OK ").nth(1) {
        let mut open_ns = None;
        for field in rest.split_whitespace() {
            if let Some(value) = field.strip_prefix("open_ns=") {
                open_ns = value.parse::<u64>().ok();
            }
        }
        return Ok((open_ns, true));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("perf-matrix: consumer child failed: {stderr}");
    Ok((None, false))
}

/// Child entry: attaches as a consumer to an existing segment and reports.
/// REQ: REQ-CORE-071.
fn run_child(args: &[String]) -> Result<()> {
    let Some(mode) = args.first() else {
        bail!("--child requires a mode");
    };
    if mode != "consumer-open" {
        bail!("unknown --child mode `{mode}`");
    }
    let name = args.get(1).context("consumer-open: segment name")?;
    let spec = TopicSpec {
        name: args.get(2).context("consumer-open: topic")?.clone(),
        sample_max: parse_arg(args, 3, "sample_max")?,
        depth: parse_arg(args, 4, "depth")?,
        pool: parse_arg(args, 5, "pool")?,
        reject: args.get(6).is_some_and(|v| v == "1"),
    };
    let segment = SegmentName::new(name).context("segment name")?;
    let expected = expectation(&spec);
    let start = Instant::now();
    match consumer_open(
        &PosixShm,
        &segment,
        &expected,
        &layout_spec(),
        Some(Duration::from_secs(5)),
    ) {
        Ok(consumer) => {
            let open_ns = nanos(start.elapsed());
            let capacity = consumer.view.descriptor.payload_capacity;
            println!("CONSUMER_OK open_ns={open_ns} capacity={capacity}");
            Ok(())
        }
        Err(e) => bail!("consumer open: {e:?}"),
    }
}

/// Parses one numeric child argument.
/// REQ: REQ-QAL-011.
fn parse_arg(args: &[String], index: usize, what: &str) -> Result<u64> {
    let raw = args
        .get(index)
        .with_context(|| format!("consumer-open: {what}"))?;
    raw.parse::<u64>()
        .with_context(|| format!("parse {what} `{raw}`"))
}

/// Budget verdicts against the declared ceilings.
/// REQ: REQ-QAL-012.
fn check_budgets(rows: &[TopicRow]) -> Vec<BudgetRow> {
    let mut budgets = Vec::with_capacity(rows.len() * 2);
    for row in rows {
        for (metric, measured) in [
            ("publish_p99", row.publish_p99_ns),
            ("read_p99", row.read_p99_ns),
        ] {
            budgets.push(BudgetRow {
                topic: row.topic.clone(),
                metric: metric.to_string(),
                measured_ns: measured,
                budget_ns: CORE_P99_BUDGET_NS,
                verdict: if measured <= CORE_P99_BUDGET_NS {
                    "PASS"
                } else {
                    "FAIL"
                }
                .to_string(),
            });
        }
    }
    budgets
}

/// Nanosecond conversion that saturates instead of wrapping.
/// REQ: REQ-QAL-011.
fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Fresh per-process segment name (POSIX-shm legal).
/// REQ: REQ-CORE-088.
fn unique_segment_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("/drm.perf.{}.{nanos:x}", std::process::id())
}

/// Host + kernel description for the report header.
/// REQ: REQ-QAL-011.
fn host_description() -> String {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".to_string(), |s| s.trim().to_string());
    format!(
        "{} {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH,
        kernel
    )
}

/// Renders the human-readable matrix.
/// REQ: REQ-QAL-011.
fn print_matrix(matrix: &Matrix, json: &Path) {
    println!("perf-matrix | host {}", matrix.host);
    println!("workspace {}", matrix.workspace);
    println!(
        "{:<22} {:>9} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9} {:>9}",
        "topic",
        "bytes",
        "R",
        "pool",
        "pub p50",
        "pub p99",
        "rd p50",
        "rd p99",
        "ops/s",
        "MiB/s",
        "seg B"
    );
    for row in &matrix.topics {
        println!(
            "{:<22} {:>9} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9.1} {:>9}",
            row.topic,
            row.sample_max,
            row.depth,
            row.pool,
            row.publish_p50_ns,
            row.publish_p99_ns,
            row.read_p50_ns,
            row.read_p99_ns,
            row.ops_per_sec,
            row.mib_per_sec,
            row.segment_bytes
        );
    }
    let cross = &matrix.cross_process;
    println!(
        "cross-process | segment {} B | publisher_open {} ns | epoch {} | consumer_open {} | attached {} | 2nd publisher refused {}",
        cross.segment_bytes,
        cross.publisher_open_ns,
        cross.epoch,
        cross
            .consumer_open_ns
            .map_or_else(|| "-".to_string(), |v| v.to_string()),
        cross.consumer_attached,
        cross.second_publisher_refused
    );
    for row in &matrix.budgets {
        println!(
            "budget | {:<22} {:<12} {:>9} ns <= {:>9} ns  {}",
            row.topic, row.metric, row.measured_ns, row.budget_ns, row.verdict
        );
    }
    println!(
        "baseline band | publish p50 {BASELINE_PUBLISH_P50_NS} ns +/-20%, p99 {BASELINE_PUBLISH_P99_NS} ns +/-30% (bench-baseline.md M23W4)"
    );
    println!(
        "noise bands | p50 +/-{}%, p99 +/-{}%",
        NOISE_P50 * 100.0,
        NOISE_P99 * 100.0
    );
    print!("{}", band_lines(matrix));
    println!("wrote {}", json.display());
}

/// Formats a band verdict for one measurement. Only a *slower* move is a
/// regression; a faster-than-baseline move is reported as such, never as drift.
/// REQ: REQ-QAL-011.
#[must_use]
#[allow(clippy::cast_precision_loss)] // per-op latencies stay far below 2^53
fn band(measured: u64, baseline: u64, tolerance: f64) -> &'static str {
    let limit = tolerance * (baseline as f64);
    if measured >= baseline {
        if (measured - baseline) as f64 <= limit {
            "within-band"
        } else {
            "SLOWER"
        }
    } else if (baseline - measured) as f64 <= limit {
        "within-band"
    } else {
        "faster"
    }
}

/// Writes the band comparison into the report text (kept separate from the
/// table so `--json` stays machine-readable).
/// REQ: REQ-QAL-011.
fn band_lines(matrix: &Matrix) -> String {
    let mut out = String::new();
    for row in &matrix.topics {
        if row.sample_max > BASELINE_PAYLOAD_MAX {
            let _ = writeln!(
                out,
                "band | {:<29} n/a (payload {} B exceeds the {} B baseline class)",
                row.topic, row.sample_max, BASELINE_PAYLOAD_MAX
            );
            continue;
        }
        let _ = writeln!(
            out,
            "band | {:<22} pub p50 {} | pub p99 {}",
            row.topic,
            band(row.publish_p50_ns, BASELINE_PUBLISH_P50_NS, NOISE_P50),
            band(row.publish_p99_ns, BASELINE_PUBLISH_P99_NS, NOISE_P99)
        );
    }
    out
}

/// Unit tests for the pure helpers.
/// REQ: REQ-QAL-011.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_floor_indexed() {
        let samples = vec![10, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 50), 30);
        assert_eq!(percentile(&samples, 99), 40);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn attribute_reads_numeric_and_latest() {
        assert_eq!(
            attribute("@history: 8\nax: f32", "@history").unwrap_or(0),
            8
        );
        assert_eq!(attribute("@history: latest\n", "@history").unwrap_or(9), 1);
    }

    #[test]
    fn band_reports_slower_outside_tolerance() {
        assert_eq!(band(200, 190, 0.20), "within-band");
        assert_eq!(band(400, 190, 0.20), "SLOWER");
        assert_eq!(band(60, 190, 0.20), "faster");
    }

    #[test]
    fn band_lines_render_every_topic() {
        let matrix = Matrix {
            host: "test".into(),
            workspace: "ws".into(),
            topics: vec![row("a", 190, 200)],
            cross_process: CrossProcessRow {
                segment_bytes: 1,
                publisher_open_ns: 1,
                epoch: 1,
                consumer_open_ns: Some(1),
                consumer_attached: true,
                second_publisher_refused: true,
            },
            budgets: Vec::new(),
            all_pass: true,
        };
        let text = band_lines(&matrix);
        assert!(text.contains('a'));
        assert!(text.contains("within-band"));
    }

    fn row(topic: &str, p50: u64, p99: u64) -> TopicRow {
        TopicRow {
            topic: topic.to_string(),
            sample_max: 64,
            depth: 8,
            pool: 12,
            policy: "overwrite".into(),
            publish_p50_ns: p50,
            publish_p99_ns: p99,
            read_p50_ns: 1,
            read_p99_ns: 2,
            borrow_p50_ns: 1,
            borrow_p99_ns: 2,
            ops_per_sec: 1,
            mib_per_sec: 1.0,
            segment_bytes: 1,
        }
    }
}
