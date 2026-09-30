//! Read-fuzz gate (canon-codec I5; work-plan M01W4-E1-T1): deterministic
//! pseudo-random byte buffers through every canon primitive read at every
//! offset — each call yields a value or a `Status`, never a panic or UB.
//! Ignored by default; run explicitly:
//! `cargo test -p drm-core --test canon_read_fuzz -- --ignored`
//! Budget: `DRM_READFUZZ_SECONDS` (default 5).
//! REQ: REQ-CORE-004, REQ-TOOL-043.
#![allow(clippy::cast_possible_truncation)]

use std::time::{Duration, Instant};

/// xorshift64* PRNG.
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
}

fn budget() -> Duration {
    match std::env::var("DRM_READFUZZ_SECONDS") {
        Ok(s) => Duration::from_secs(
            s.parse::<u64>()
                .unwrap_or_else(|e| unreachable!("DRM_READFUZZ_SECONDS `{s}`: {e:?}")),
        ),
        Err(_) => Duration::from_secs(5),
    }
}

/// REQ: REQ-CORE-004 — I5 no-UB: arbitrary bytes decode to a value or
/// `InvalidData`/`InvalidArg`, never a panic, for every primitive at every
/// offset.
#[test]
#[ignore = "read-fuzz budget; run explicitly with -- --ignored"]
fn read_fuzz_no_panic() {
    let mut rng = Rng(0xCAFE_0000_D00D_0001);
    let deadline = Instant::now() + budget();
    let mut cases: u64 = 0;
    let mut ok: u64 = 0;
    let mut invalid: u64 = 0;

    let mut buf = [0u8; 32];
    while Instant::now() < deadline {
        for b in &mut buf {
            *b = rng.next() as u8;
        }
        for off in 0..buf.len() {
            match drm_core::canon::read_bool(&buf, off) {
                Ok(_) | Err(drm_types::Status::InvalidData) => {}
                Err(other) => unreachable!("bool read status {other:?}"),
            }
            let r: Result<u128, drm_types::Status> = match rng.next() % 10 {
                0 => drm_core::canon::read_u8(&buf, off).map(u128::from),
                1 => drm_core::canon::read_i8(&buf, off).map(|v| u128::from(v.cast_unsigned())),
                2 => drm_core::canon::read_u16(&buf, off).map(u128::from),
                3 => drm_core::canon::read_i16(&buf, off).map(|v| u128::from(v.cast_unsigned())),
                4 => drm_core::canon::read_u32(&buf, off).map(u128::from),
                5 => drm_core::canon::read_i32(&buf, off).map(|v| u128::from(v.cast_unsigned())),
                6 => drm_core::canon::read_f32(&buf, off).map(|v| u128::from(v.to_bits())),
                7 => drm_core::canon::read_u64(&buf, off).map(u128::from),
                8 => drm_core::canon::read_i64(&buf, off).map(|v| u128::from(v.cast_unsigned())),
                _ => drm_core::canon::read_f64(&buf, off).map(|v| u128::from(v.to_bits())),
            };
            match r {
                Ok(_) => ok += 1,
                Err(drm_types::Status::InvalidArg) => invalid += 1,
                Err(other) => unreachable!("unexpected read status {other:?}"),
            }
            cases += 1;
        }
    }
    println!("read_fuzz: cases={cases} ok={ok} invalid_arg={invalid}");
    assert!(cases > 1_000_000, "read-fuzz too small: {cases}");
}

/// REQ: REQ-CORE-004 — default-size smoke always runs in CI (no budget).
#[test]
fn read_fuzz_smoke() {
    let mut rng = Rng(0xAAAA_0000_0000_0001);
    let mut buf = [0u8; 24];
    for _ in 0..10_000 {
        for b in &mut buf {
            *b = rng.next() as u8;
        }
        for off in 0..buf.len() {
            let _ = drm_core::canon::read_bool(&buf, off);
            let _ = drm_core::canon::read_f64(&buf, off);
            let _ = drm_core::canon::read_i16(&buf, off);
            let _ = drm_core::canon::read_u64(&buf, off);
        }
    }
}
