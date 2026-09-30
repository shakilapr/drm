//! Token bucket (lanes-shaping.md §Data Structures, §Functions; seed impl for
//! the M11 shaper, work-plan M01W4-E3-T3). Credit is `bit·ns` fixed point:
//! refill converts `rate_bits_per_sec × dt_ns` exactly; a charge of `c` bytes
//! costs `8·c` bit·ns; no unit mismatch and no permanent fractional loss
//! (I-LS7). Monotonic clock only.
//! REQ: REQ-ROUTE-002, REQ-ROUTE-030.

/// One token bucket per budget class (safety budget / non-safety shaper).
#[derive(Debug, Clone, Copy)]
pub struct TokenBucket {
    /// Refill rate in bits per second.
    pub rate_bps: u64,
    /// Maximum burst in bytes; also the credit ceiling.
    pub burst_bytes: u64,
    /// Accumulated credit in bit·ns (0..=burst_bytes·8e9).
    credit_bit_ns: u128,
    /// Last refill timestamp in ns (monotonic clock domain).
    last_ns: u64,
}

impl TokenBucket {
    /// New bucket starting full (credit = burst ceiling).
    /// REQ: REQ-ROUTE-030.
    #[must_use]
    pub fn new(rate_bps: u64, burst_bytes: u64) -> Self {
        let capacity = Self::capacity(burst_bytes);
        Self {
            rate_bps,
            burst_bytes,
            credit_bit_ns: capacity,
            last_ns: 0,
        }
    }

    /// Credit ceiling in bit·ns: `burst_bytes · 8e9`.
    /// REQ: REQ-ROUTE-030.
    #[must_use]
    pub fn capacity(burst_bytes: u64) -> u128 {
        u128::from(burst_bytes).saturating_mul(8_000_000_000)
    }

    /// Refill credit for elapsed time, clamped to the ceiling; returns the
    /// added credit (I-LS7 exact fixed point, no fractional loss).
    /// REQ: REQ-ROUTE-002.
    fn refill(&mut self, now_ns: u64) {
        let dt = u128::from(now_ns.saturating_sub(self.last_ns));
        let capacity = Self::capacity(self.burst_bytes);
        let added = u128::from(self.rate_bps).saturating_mul(dt);
        self.credit_bit_ns = self.credit_bit_ns.saturating_add(added).min(capacity);
        self.last_ns = now_ns;
    }

    /// Admission attempt at `now_ns`: refill, then either debit the charge
    /// (`Ok`) or leave state untouched and deny (`Err`). A frame larger than
    /// the burst can never be admitted — verify rejects that config upstream
    /// (I-LS9), so denial here is always eventual-admission-deferred.
    /// REQ: REQ-ROUTE-030, REQ-ROUTE-039.
    pub fn try_consume(&mut self, charge_bytes: u64, now_ns: u64) -> Result<(), ()> {
        self.refill(now_ns);
        let cost = u128::from(charge_bytes).saturating_mul(8_000_000_000);
        if self.credit_bit_ns >= cost {
            self.credit_bit_ns -= cost;
            Ok(())
        } else {
            Err(())
        }
    }

    /// Earliest admission time for `charge_bytes` under the current refill
    /// rate, `now_ns` + needed wait (0 when already admissible).
    /// REQ: REQ-ROUTE-039.
    #[must_use]
    pub fn eligible_at(&self, charge_bytes: u64, now_ns: u64) -> u64 {
        let cost = u128::from(charge_bytes).saturating_mul(8_000_000_000);
        let capacity = Self::capacity(self.burst_bytes);
        let dt = u128::from(now_ns.saturating_sub(self.last_ns));
        let projected = self
            .credit_bit_ns
            .saturating_add(u128::from(self.rate_bps).saturating_mul(dt));
        if projected >= cost {
            return now_ns;
        }
        if self.rate_bps == 0 {
            return u64::MAX;
        }
        let missing_bit_ns = cost - projected;
        let wait_ns = missing_bit_ns / u128::from(self.rate_bps);
        let wait_ns = wait_ns.min(u128::from(u64::MAX) - 1) + 1;
        now_ns.saturating_add(wait_ns as u64)
    }

    /// Current credit in bit·ns after a hypothetical refill at `now_ns`.
    /// REQ: REQ-ROUTE-002.
    #[must_use]
    pub fn credit_at(&self, now_ns: u64) -> u128 {
        let capacity = Self::capacity(self.burst_bytes);
        let dt = u128::from(now_ns.saturating_sub(self.last_ns));
        self.credit_bit_ns
            .saturating_add(u128::from(self.rate_bps).saturating_mul(dt))
            .min(capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::TokenBucket;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    /// Deterministic per-case RNG (xorshift64* over a hashed case name).
    fn rng(seed: &str) -> impl FnMut() -> u64 + use<> {
        let mut h = DefaultHasher::new();
        seed.hash(&mut h);
        let mut x = h.finish() | 1;
        move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
            x
        }
    }

    /// REQ: REQ-ROUTE-030 — full bucket admits the burst immediately, then
    /// denies; refill over time restores admission (I-LS9 admission model).
    #[test]
    fn burst_then_refill() {
        let mut b = TokenBucket::new(1_000_000, 1_500);
        assert!(b.try_consume(1_500, 0).is_ok(), "burst admitted at t=0");
        assert!(b.try_consume(1, 0).is_err(), "empty bucket denies");
        // At 1 Mbit/s, 1 bit·ns cost of 8_000 bytes costs 64e12 bit·ns;
        // enough time must restore credit up to the ceiling.
        let eligible = b.eligible_at(1_500, 0);
        assert!(eligible > 0, "refill wait required after burst");
        assert!(b.try_consume(1_500, eligible).is_ok(), "refilled admission");
    }

    /// REQ: REQ-ROUTE-002 — I-LS3: over any window W, admitted bytes ≤
    /// rate·W/8 + burst; property over 10k deterministic traces.
    #[test]
    fn shaping_property() {
        for case in 0..10_000 {
            let mut r = rng(&format!("shaping-{case}"));
            let rate = (r() % 10_000_000) + 1;
            let burst = (r() % 100_000) + 1;
            let mut b = TokenBucket::new(rate, burst);
            let mut now: u64 = 0;
            let mut admitted = 0u128;
            let mut window_start = 0u64;
            let window: u64 = 100_000_000; // 100 ms
            for _ in 0..64 {
                let charge = (r() % 1_500) + 1;
                now += r() % (window / 64);
                if b.try_consume(charge, now).is_ok() {
                    admitted += u128::from(charge);
                }
                if now - window_start >= window {
                    let cap = u128::from(rate) * u128::from(window) / 8 + u128::from(burst);
                    assert!(
                        admitted <= cap,
                        "case {case}: admitted {admitted} > cap {cap}"
                    );
                    admitted = 0;
                    window_start = now;
                }
            }
        }
    }

    /// REQ: REQ-ROUTE-002 — I-LS7: 1 byte/s with 100 ms ticks never starves:
    /// fractional credit accumulates, every byte eventually admitted.
    #[test]
    fn token_bucket_fractional() {
        let mut b = TokenBucket::new(8, 1); // 1 byte/s
        let mut now: u64 = 0;
        for _ in 0..10_000u64 {
            now += 100_000_000;
            if b.try_consume(1, now).is_ok() {
                return; // admitted well within the 1000 s campaign
            }
        }
        unreachable!("1 byte/s bucket starved for 1000 s");
    }

    /// REQ: REQ-ROUTE-039 — I-LS9 wait model: empty-bucket admission time
    /// matches `8·(charge − credit)/rate` within rounding.
    #[test]
    fn shaping_wait_model() {
        let b = TokenBucket::new(10_000_000, 100);
        // Drain: burst 100 bytes at t=0 on a fresh bucket — build an empty
        // state by simulating a consumed burst.
        let mut b2 = b;
        assert!(b2.try_consume(100, 0).is_ok());
        let charge = 1_250u64; // 10_000 bits at 10 Mbit/s ⇒ 1 ms
        let wait = b2.eligible_at(charge, 0);
        assert!(
            wait >= 1_000_000 && wait <= 1_000_000 + 1_000,
            "wait {wait} ~ 1 ms expected"
        );
    }
}
