//! Phase 0 fixture producer (tests/integration/primitive).
//!
//! Publishes `count` `u64` samples to a DRM topic's shared-memory segment through
//! the attached data path. Every identity input (segment name, contract hashes,
//! topic handle, sizes) is *provided* by `cargo xtask integration` from the built
//! workspace, so the application never invents one.
//!
//! REQ: REQ-CORE-102, REQ-CORE-104.

use std::process::ExitCode;
use std::time::Duration;

use drm_core::channel::attached::AttachedChannel;
use drm_core::channel::ccb::Expectation;
use drm_core::channel::lifecycle::{publisher_open, LayoutSpec};
use drm_core::shm::{FlockBackend, LockPaths, PosixShm, SegmentName, ShmBackend};

/// Slot alignment the fixture segment is laid out with.
const SLOT_ALIGN: u64 = 8;

/// Flags parsed from `--key value` pairs.
struct Args {
    segment: String,
    topic: String,
    lock_dir: String,
    handle: u32,
    sample_max: u64,
    depth: u64,
    pool: u64,
    schema: [u8; 16],
    layout: [u8; 16],
    count: u64,
    linger_ms: u64,
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
        lock_dir: get("--lock-dir")?,
        handle: u32::try_from(num("--handle")?).map_err(|_| "--handle out of range")?,
        sample_max: num("--sample-max")?,
        depth: num("--depth")?,
        pool: num("--pool")?,
        schema: hex16(&get("--schema-hash")?)?,
        layout: hex16(&get("--layout-hash")?)?,
        count: num("--count")?,
        linger_ms: argv
            .iter()
            .position(|a| a == "--linger-ms")
            .and_then(|i| argv.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2000),
    })
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Ok(args) = parse(&argv) else {
        eprintln!("producer: bad arguments: {:?}", parse(&argv).err());
        return ExitCode::from(2);
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("producer: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Creates the segment, publishes `count` samples, lingers, then unlinks.
fn run(args: &Args) -> Result<(), String> {
    if args.sample_max < 8 {
        return Err(format!("sample_max {} cannot carry a u64", args.sample_max));
    }
    std::fs::create_dir_all(&args.lock_dir).map_err(|e| format!("lock dir: {e}"))?;
    let segment = SegmentName::new(&args.segment).map_err(|e| format!("segment: {e:?}"))?;
    let _ = PosixShm.unlink(&segment);
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
    let locks = LockPaths::new(&args.lock_dir);
    let publisher = publisher_open(
        &PosixShm,
        &FlockBackend,
        &locks.publisher(args.handle),
        &segment,
        args.topic.as_bytes(),
        &expected,
        &spec,
    )
    .map_err(|e| format!("publisher_open: {e:?}"))?;
    let channel = AttachedChannel::new(publisher.mapping(), &publisher.view, SLOT_ALIGN)
        .map_err(|e| format!("attached view: {e:?}"))?;

    let mut buf = vec![0_u8; usize::try_from(args.sample_max).map_err(|_| "sample_max")?];
    for i in 0..args.count {
        let head = buf.get_mut(..8).ok_or("sample too small")?;
        head.copy_from_slice(&i.to_le_bytes());
        channel
            .publish(&buf)
            .map_err(|e| format!("publish {i}: {e:?}"))?;
    }
    println!(
        "PRODUCER_OK published={} last={} epoch={}",
        args.count,
        args.count.saturating_sub(1),
        channel.epoch()
    );
    // Keep the segment mapped until the consumer has read it.
    std::thread::sleep(Duration::from_millis(args.linger_ms));
    let _ = PosixShm.unlink(&segment);
    Ok(())
}
