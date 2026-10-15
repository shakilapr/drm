//! `xtask integration` — the Phase 0 integration fixture runner.
//!
//! Proves, against one small fixture, that the toolchain is trustworthy:
//! generation runs, a second plan is fully cached, the strict `gen-check` accepts
//! the fixture (topics + nodes + artifacts present), two *independently built*
//! binaries meet on one shared-memory segment, and the reader sees every sample
//! the writer published in order.
//!
//! REQ: REQ-TOOL-024, REQ-TOOL-027, REQ-CORE-102, REQ-CORE-103.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use drm_workspace::codegen::{FingerprintDb, execute, plan};
use drm_workspace::model::layout_hash;

/// Fixture workspace used when no `--workspace` is given.
const DEFAULT_WORKSPACE: &str = "tests/integration/primitive";
/// Canonical topic the fixture exchanges.
const TOPIC: &str = "counter/value";
/// Topic source, relative to the fixture root (its attributes provision depth/pool).
const TOPIC_SOURCE: &str = "packages/counter/topics/value.drm";
/// Samples the two applications exchange.
const SAMPLES: u64 = 64;
/// Topic handle both sides pass (one topic per segment in this fixture).
const TOPIC_HANDLE: u32 = 1;
/// Lock directory the fixture apps share.
const LOCK_DIR: &str = "target/integration-locks";

/// REQ: REQ-TOOL-024 — `xtask integration [--workspace <path>] [--check]`.
///
/// # Errors
/// A missing or vacuous fixture, a generation/verification failure, a build
/// failure, or an exchange that does not deliver every sample.
pub fn run(flags: &[String]) -> Result<()> {
    let (root, check, ts_only) = parse(flags)?;
    if ts_only {
        // TS-only mode: the repo-root `studio-protocol` surface, which declares no
        // DRM packages and would fail the DRM half of a full check.
        return crate::gencheck::run(&[
            "--workspace".to_string(),
            root.display().to_string(),
            "--ts-only".to_string(),
        ]);
    }
    if check && !root.is_dir() {
        bail!(
            "integration: fixture `{}` does not exist — a missing mandatory fixture is a failure",
            root.display()
        );
    }

    // 1. Generate, then prove an unchanged re-plan is fully cached.
    let model = crate::gencheck::model_at(&root)?;
    let mut db = FingerprintDb::load(&root).map_err(|e| anyhow!("integration: load db: {e:?}"))?;
    let first = plan(&model, &db, &root);
    let rendered = first.dirty_units.len();
    execute(&first, &mut db, &root).map_err(|e| anyhow!("integration: execute: {e:?}"))?;
    let second = plan(&model, &db, &root);
    if !second.dirty_units.is_empty() {
        bail!(
            "integration: an unchanged build must be cached, but {} unit(s) re-planned",
            second.dirty_units.len()
        );
    }
    println!(
        "integration: generated {rendered} unit(s); unchanged re-plan cached (0 dirty, {} topics, {} nodes)",
        model.topics.len(),
        model.nodes.len()
    );

    // 2. The strict gate must accept this fixture as-is.
    crate::gencheck::check(&root).map_err(|e| anyhow!("integration: gen-check: {e}"))?;

    // 3. Provision the two applications from the built model.
    let entry = model
        .topics
        .get(TOPIC)
        .ok_or_else(|| anyhow!("integration: fixture declares no topic `{TOPIC}`"))?;
    let text = std::fs::read_to_string(root.join(TOPIC_SOURCE))
        .with_context(|| format!("integration: read `{TOPIC_SOURCE}`"))?;
    let depth = attribute(&text, "@history")?;
    let pool = attribute(&text, "@pool_size")?;
    // A reader may legitimately start at the oldest *retained* sample, so asserting
    // "every sample arrives" only holds while the ring can still hold the whole
    // exchange. Make that requirement explicit rather than flaky.
    if SAMPLES > depth {
        bail!(
            "integration: fixture exchanges {SAMPLES} samples but `@history` retains {depth}; \
             the sequential assertion would depend on the writer's pace"
        );
    }
    let sample_max = u64::try_from(entry.layout.sample_max).context("sample_max")?;
    let schema = hex(&entry.schema_hash);
    let layout = hex(&layout_hash(&entry.layout));
    let segment = "/drm.integration.primitive";
    let lock_dir = root.join(LOCK_DIR);
    std::fs::create_dir_all(&lock_dir).context("integration: create lock dir")?;

    let args: Vec<String> = [
        "--segment",
        segment,
        "--topic",
        TOPIC,
        "--lock-dir",
        &lock_dir.display().to_string(),
        "--handle",
        &TOPIC_HANDLE.to_string(),
        "--sample-max",
        &sample_max.to_string(),
        "--depth",
        &depth.to_string(),
        "--pool",
        &pool.to_string(),
        "--schema-hash",
        &schema,
        "--layout-hash",
        &layout,
        "--count",
        &SAMPLES.to_string(),
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    // 4. Build both applications (independent projects → independent builds).
    for app in ["producer", "consumer"] {
        let manifest = root.join("apps").join(app).join("Cargo.toml");
        if !manifest.is_file() {
            bail!(
                "integration: missing fixture application `{}`",
                manifest.display()
            );
        }
        let out = Command::new(cargo())
            .args(["build", "--quiet", "--manifest-path"])
            .arg(&manifest)
            .output()
            .context("integration: spawn cargo build")?;
        if !out.status.success() {
            bail!(
                "integration: building `{app}` failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    // 5. Start the reader (it waits for the segment to appear), then the writer.
    let consumer = root.join("apps/consumer/target/debug/consumer");
    let producer = root.join("apps/producer/target/debug/producer");
    let reader = Command::new(&consumer)
        .args(&args)
        .args(["--wait-ms", "10000", "--deadline-ms", "20000"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("integration: spawn consumer")?;
    std::thread::sleep(Duration::from_millis(300));
    let writer = Command::new(&producer)
        .args(&args)
        .args(["--linger-ms", "3000"])
        .output()
        .context("integration: run producer")?;
    let reader_out = reader
        .wait_with_output()
        .context("integration: wait for consumer")?;

    let producer_stdout = String::from_utf8_lossy(&writer.stdout).trim().to_string();
    let consumer_stdout = String::from_utf8_lossy(&reader_out.stdout)
        .trim()
        .to_string();
    if !writer.status.success() {
        bail!(
            "integration: producer exited {}:\n{producer_stdout}\n{}",
            writer.status,
            String::from_utf8_lossy(&writer.stderr)
        );
    }
    if !reader_out.status.success() {
        bail!(
            "integration: consumer exited {}:\n{consumer_stdout}\n{}",
            reader_out.status,
            String::from_utf8_lossy(&reader_out.stderr)
        );
    }
    if !producer_stdout.contains(&format!("published={SAMPLES}")) {
        bail!("integration: producer did not report {SAMPLES} samples:\n{producer_stdout}");
    }
    if !consumer_stdout.contains(&format!("received={SAMPLES}")) {
        bail!("integration: consumer did not receive {SAMPLES} samples:\n{consumer_stdout}");
    }
    println!("integration: exchange verified — {producer_stdout} | {consumer_stdout}");
    Ok(())
}

/// Parses `--workspace <path>` and `--check`.
///
/// # Errors
/// Unknown flags or a missing path argument.
fn parse(flags: &[String]) -> Result<(PathBuf, bool, bool)> {
    let mut root = PathBuf::from(DEFAULT_WORKSPACE);
    let mut check = false;
    let mut ts_only = false;
    let mut i = 0;
    while i < flags.len() {
        match flags.get(i).map(String::as_str) {
            Some("--check") => check = true,
            Some("--ts-only") => ts_only = true,
            Some("--workspace") => {
                let path = flags
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("integration: --workspace needs a path"))?;
                root = PathBuf::from(path);
                i += 1;
            }
            other => bail!("integration: unknown flag {other:?}"),
        }
        i += 1;
    }
    Ok((root, check, ts_only))
}

/// Parses a numeric topic attribute (`@history: 4`, `@pool_size: 8`).
///
/// # Errors
/// A missing or unparsable attribute.
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
                .with_context(|| format!("integration: parse `{key} {value}`"));
        }
    }
    bail!("integration: topic is missing `{key}`")
}

/// Lowercase hex rendering of a digest (what the fixture apps parse).
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `cargo` for the active toolchain.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}
