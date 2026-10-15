//! Phase 0 fixture consumer (tests/integration/primitive).
//!
//! Attaches read-only to a DRM topic's shared-memory segment and reads `count`
//! `u64` samples through the attached data path, checking they arrive in order
//! and with the expected values. Identity inputs are provided by
//! `cargo xtask integration` from the built workspace.
//!
//! REQ: REQ-CORE-103.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use drm_core::channel::attached::AttachedChannel;
use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{consumer_open, LayoutSpec};
use drm_core::channel::ReadOutcome;
use drm_core::shm::{PosixShm, SegmentName};

/// Slot alignment the fixture segment is laid out with.
const SLOT_ALIGN: u64 = 8;

/// Flags parsed from `--key value` pairs.
struct Args {
    segment: String,
    topic: String,
    handle: u32,
    sample_max: u64,
    depth: u64,
    pool: u64,
    schema: [u8; 16],
    layout: [u8; 16],
    count: u64,
    wait_ms: u64,
    deadline_ms: u64,
}

/// Parses one 32-character hex string into 16 bytes.
fn hex16(text: &str) -> Result<[u8; 16], String> {
    if text.len() != 32 {
        return Err(format!("expected 32 hex chars, got {}", text.len()));
    }
    let mut out = [0_u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let pair = text.get(i * 2..i * 2 + 2).ok_or("short hex")?;
        *slot = u8::from_str_radix(pair, 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Parses `--key value` pairs into [`Args`].
fn parse(argv: &[String]) -> Result<Args, String> {
    let get = |key: &str| -> Result<String, String> {
        argv.iter()
            .position(|a| a == key)
            .and_then(|i| argv.get(i + 1))
            .cloned()
            .ok_or_else(|| format!("missing {key}"))
    };
    let num = |key: &str| -> Result<u64, String> {
        get(key)?.parse::<u64>().map_err(|e| format!("{key}: {e}"))
    };
    Ok(Args {
        segment: get("--segment")?,
        topic: get("--topic")?,
        handle: u32::try_from(num("--handle")?).map_err(|_| "--handle out of range")?,
        sample_max: num("--sample-max")?,
        depth: num("--depth")?,
        pool: num("--pool")?,
        schema: hex16(&get("--schema-hash")?)?,
        layout: hex16(&get("--layout-hash")?)?,
        count: num("--count")?,
        wait_ms: argv
            .iter()
            .position(|a| a == "--wait-ms")
            .and_then(|i| argv.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10_000),
        deadline_ms: argv
            .iter()
            .position(|a| a == "--deadline-ms")
            .and_then(|i| argv.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(15_000),
    })
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Ok(args) = parse(&argv) else {
        eprintln!("consumer: bad arguments: {:?}", parse(&argv).err());
        return ExitCode::from(2);
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("consumer: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Attaches, reads `count` samples in order, and reports.
fn run(args: &Args) -> Result<(), String> {
    let segment = SegmentName::new(&args.segment).map_err(|e| format!("segment: {e:?}"))?;
    let expected = Expectation {
        schema_hash: args.schema,
        layout_hash: args.layout,
        topic_handle: args.handle,
        topic_name: args.topic.clone(),
        payload_capacity: args.sample_max,
        history_depth: args.depth,
        pool_size: args.pool,
        overflow_policy: 0,
        consumer_count: 1,
        required_count: 0,
        diagnostic_count: 0,
    };
    let spec = LayoutSpec {
        flags: 0,
        slot_align: SLOT_ALIGN,
    };
    let consumer = consumer_open(
        &PosixShm,
        &segment,
        &expected,
        &spec,
        Some(Duration::from_millis(args.wait_ms)),
    )
    .map_err(|e| format!("consumer_open (waited {}ms): {e:?}", args.wait_ms))?;
    let channel = AttachedChannel::new(consumer.mapping(), &consumer.view, SLOT_ALIGN)
        .map_err(|e| format!("attached view: {e:?}"))?;

    let mut state = channel.reader();
    let mut buf = vec![0_u8; usize::try_from(args.sample_max).map_err(|_| "sample_max")?];
    let mut received: u64 = 0;
    let start = Instant::now();
    while received < args.count {
        if start.elapsed() > Duration::from_millis(args.deadline_ms) {
            return Err(format!(
                "deadline after {received}/{} samples (epoch {})",
                args.count,
                channel.epoch()
            ));
        }
        match channel
            .read_next(&mut state, &mut buf)
            .map_err(|e| format!("read: {e:?}"))?
        {
            ReadOutcome::Sample => {
                let mut head = [0_u8; 8];
                head.copy_from_slice(buf.get(..8).ok_or("short payload")?);
                let value = u64::from_le_bytes(head);
                if value != received {
                    return Err(format!("out of order: got {value}, expected {received}"));
                }
                received += 1;
            }
            ReadOutcome::Gap { missed } => return Err(format!("unexpected gap of {missed}")),
            ReadOutcome::Empty | ReadOutcome::WouldRetry => {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    println!("CONSUMER_OK received={received} last={}", received - 1);
    Ok(())
}
