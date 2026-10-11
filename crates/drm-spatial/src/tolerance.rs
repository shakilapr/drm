//! Reference-suite float-determinism tolerance band (work-plan M21W4-E1-T3;
//! [spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Reference-Suite Float Determinism). A fused multiply-add (`f64::mul_add`,
//! one rounding) and a separate multiply-then-add (two roundings) differ by at
//! most a bounded number of ULPs; a host toolchain and an MCU toolchain may
//! contract or split the same source expression differently. Every reference
//! comparison in the spatial suite therefore uses one combined absolute +
//! relative band, never bit equality.
//!
//! REQ: REQ-SPATIAL-053.

/// A combined absolute + relative equality band for spatial reference values.
///
/// Two reals `actual` and `reference` are equal within the band when their
/// absolute difference is at most `abs + rel·|reference|`. The absolute term
/// dominates near zero (where a relative test is meaningless); the relative
/// term dominates for large magnitudes (where ULPs scale with the value). The
/// band absorbs FMA-vs-separate accumulation differences across host toolchains
/// and MCU targets without accepting a genuine geometry regression.
///
/// REQ: REQ-SPATIAL-053.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToleranceBand {
    abs: f64,
    rel: f64,
}

impl ToleranceBand {
    /// The band used by the spatial reference suite (final-gate evidence rule).
    ///
    /// Chosen from the measured FMA-vs-separate spread over the suite's
    /// operation mix (at most a few ULPs per rounded term, amplified by a
    /// bounded number of composition steps): an absolute floor of `1e-12` m
    /// covers near-zero coordinates and a relative term of `1e-9` covers large
    /// magnitudes, leaving at least three orders of magnitude of margin below
    /// any physically meaningful geometry error.
    ///
    /// REQ: REQ-SPATIAL-053.
    pub const SPATIAL_REFERENCE: Self = Self {
        abs: 1e-12,
        rel: 1e-9,
    };

    /// Builds a band from an absolute floor `abs` and a relative fraction
    /// `rel`. Both terms are clamped to non-negative values.
    ///
    /// REQ: REQ-SPATIAL-053.
    #[must_use]
    pub fn new(abs: f64, rel: f64) -> Self {
        Self {
            abs: abs.max(0.0),
            rel: rel.max(0.0),
        }
    }

    /// The absolute floor of the band.
    ///
    /// REQ: REQ-SPATIAL-053.
    #[must_use]
    pub fn abs(&self) -> f64 {
        self.abs
    }

    /// The relative fraction of the band.
    ///
    /// REQ: REQ-SPATIAL-053.
    #[must_use]
    pub fn rel(&self) -> f64 {
        self.rel
    }

    /// The largest admissible absolute difference for `reference`:
    /// `abs + rel·|reference|`.
    ///
    /// REQ: REQ-SPATIAL-053.
    #[must_use]
    pub fn error_bound(&self, reference: f64) -> f64 {
        self.abs + self.rel * reference.abs()
    }

    /// Whether `actual` equals `reference` within the band. Non-finite values
    /// are equal only when both are the same non-finite value.
    ///
    /// REQ: REQ-SPATIAL-053.
    #[must_use]
    pub fn accepts(&self, reference: f64, actual: f64) -> bool {
        if reference.is_nan() || actual.is_nan() {
            return false;
        }
        if reference.is_infinite() || actual.is_infinite() {
            return reference.is_infinite()
                && actual.is_infinite()
                && reference.is_sign_positive() == actual.is_sign_positive();
        }
        (actual - reference).abs() <= self.error_bound(reference)
    }
}

/// Whether `actual` equals `reference` within the documented spatial reference
/// band ([`ToleranceBand::SPATIAL_REFERENCE`]).
///
/// REQ: REQ-SPATIAL-053.
#[must_use]
pub fn within_reference_band(reference: f64, actual: f64) -> bool {
    ToleranceBand::SPATIAL_REFERENCE.accepts(reference, actual)
}

#[cfg(test)]
mod tests {
    use super::{ToleranceBand, within_reference_band};

    /// REQ: REQ-SPATIAL-053 — the exact value is always inside the band; the
    /// absolute floor admits a near-zero ULP and the relative term scales with
    /// magnitude.
    #[test]
    fn band_admits_roundoff_and_scales() {
        let band = ToleranceBand::SPATIAL_REFERENCE;
        assert!(band.accepts(1.0, 1.0));
        assert!(band.accepts(0.0, 1e-13));
        assert!(!band.accepts(0.0, 1e-6));
        let big = 1.0e9_f64;
        assert!(band.accepts(big, band.error_bound(big).mul_add(0.5, big)));
        assert!(!band.accepts(big, band.error_bound(big).mul_add(2.0, big)));
    }

    /// REQ: REQ-SPATIAL-053 — a genuine geometry error is rejected; NaN can
    /// never satisfy a comparison.
    #[test]
    fn band_rejects_real_error_and_nan() {
        let band = ToleranceBand::SPATIAL_REFERENCE;
        assert!(!band.accepts(3.0, 3.001));
        assert!(!band.accepts(0.0, f64::NAN));
        assert!(!band.accepts(f64::NAN, 0.0));
        assert!(band.accepts(f64::INFINITY, f64::INFINITY));
        assert!(!band.accepts(f64::INFINITY, 1.0));
    }

    /// REQ: REQ-SPATIAL-053 — the convenience predicate uses the documented
    /// suite band and the band constructor clamps negative inputs.
    #[test]
    fn convenience_and_clamp() {
        assert!(within_reference_band(2.0, 2.0 + 1e-13));
        assert!(!within_reference_band(2.0, 2.5));
        let clamped = ToleranceBand::new(-1.0, -1.0);
        assert!(clamped.abs().abs() < f64::EPSILON);
        assert!(clamped.rel().abs() < f64::EPSILON);
    }
}
