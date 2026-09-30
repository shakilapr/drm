//! In-process mutation campaign over the fuzz seed corpus (work-plan M01W3-E3-T5
//! substitute: libFuzzer/cargo-fuzz cannot build on the Windows dev host). Every
//! mutated corpus input goes through `parse_bytes` and must produce a model or a
//! diagnostic — never a panic. Ignored by default; run explicitly:
//!
//! `cargo test -p drm-workspace --test fuzz_campaign -- --ignored --nocapture`
//!
//! Budget: `DRM_FUZZ_SECONDS` (default 60). The campaign log is archived to
//! `docs/metrics/fuzz-campaign.log` next to the repo root.
//! REQ: REQ-QAL-007, REQ-CORE-004.
#![allow(clippy::cast_possible_truncation)] // mutation arithmetic stays in u8/u32 ranges

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// xorshift64* PRNG — deterministic, dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n.max(1) as u64)) as usize
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fuzz/corpus/schema")
}

fn load_seeds() -> Vec<(String, Vec<u8>)> {
    let dir = corpus_dir();
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| unreachable!("read corpus dir: {e:?}"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| unreachable!("corpus entry: {e:?}"));
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut seeds = Vec::new();
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let bytes =
            std::fs::read(e.path()).unwrap_or_else(|e| unreachable!("read seed {name}: {e:?}"));
        seeds.push((name, bytes));
    }
    assert!(seeds.len() >= 42, "seed corpus too small: {}", seeds.len());
    seeds
}

/// Applies one random mutation to `buf` in place.
fn mutate(rng: &mut Rng, buf: &mut Vec<u8>) {
    if buf.is_empty() {
        buf.push(rng.next() as u8);
        return;
    }
    match rng.below(7) {
        0 => {
            let i = rng.below(buf.len());
            buf[i] = rng.next() as u8;
        }
        1 => {
            let i = rng.below(buf.len());
            buf[i] = buf[i].wrapping_add(1);
        }
        2 => {
            if buf.len() < 1 << 20 {
                let i = rng.below(buf.len());
                buf.insert(i, rng.next() as u8);
            }
        }
        3 => {
            let i = rng.below(buf.len());
            buf.remove(i);
        }
        4 => {
            let keep = rng.below(buf.len());
            buf.truncate(keep);
        }
        5 => {
            let n = rng.below(64) + 1;
            if buf.len() + n < 1 << 20 {
                for _ in 0..n {
                    let i = rng.below(buf.len());
                    buf.insert(i, b'd');
                }
            }
        }
        _ => {
            if buf.len() > 8 {
                let src = rng.below(buf.len());
                let dst = rng.below(buf.len());
                let len = rng.below(buf.len() - src.max(dst)).min(4096);
                for k in 0..len {
                    buf[dst + k] = buf[src + k];
                }
            }
        }
    }
}

fn budget() -> Duration {
    match std::env::var("DRM_FUZZ_SECONDS") {
        Ok(s) => Duration::from_secs(
            s.parse::<u64>()
                .unwrap_or_else(|e| unreachable!("DRM_FUZZ_SECONDS `{s}`: {e:?}")),
        ),
        Err(_) => Duration::from_secs(60),
    }
}

/// REQ: REQ-QAL-007 — hour-class mutation campaign, zero panics; log archived.
#[test]
#[ignore = "ops campaign; run explicitly with -- --ignored"]
fn hour_campaign() {
    let seeds = load_seeds();
    let mut rng = Rng(0x5EED_0000_01D2_0003);
    let deadline = Instant::now() + budget();
    let mut cases: u64 = 0;
    let mut accepted: u64 = 0;
    let mut rejected: u64 = 0;
    let mut log = String::from("case,seed,mutations,size_in,outcome\n");

    let mut buf: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
        let (seed_name, seed_bytes) = &seeds[rng.below(seeds.len())];
        buf.clear();
        buf.extend_from_slice(seed_bytes);
        let mutations = rng.below(16);
        for _ in 0..mutations {
            mutate(&mut rng, &mut buf);
        }
        let size_in = buf.len();
        match drm_workspace::parser::parse_bytes(&buf) {
            Ok(_) => accepted += 1,
            Err(_) => rejected += 1,
        }
        cases += 1;
        if cases.is_multiple_of(200_000) {
            let line = format!("{cases},{seed_name},{mutations},{size_in},ok\n");
            log.push_str(&line);
        }
    }

    let _ = write!(
        log,
        "TOTAL cases={cases} accepted={accepted} rejected={rejected} \
         seeds={} seconds={}\n",
        seeds.len(),
        budget().as_secs()
    );
    print!("{log}");
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/metrics");
    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| unreachable!("metrics dir: {e:?}"));
    std::fs::write(out_dir.join("fuzz-campaign.log"), &log)
        .unwrap_or_else(|e| unreachable!("write log: {e:?}"));
    assert!(accepted > 0, "campaign never accepted an input");
}
