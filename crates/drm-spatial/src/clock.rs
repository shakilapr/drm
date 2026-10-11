//! Temporal precision and clock domains
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md)
//! §Temporal Precision). Every timestamp carries a full clock identity (domain
//! and epoch); mapping between clocks is affine and resolved per identity. When
//! no valid mapping exists the query returns [`SpatialError::ClockUnresolved`] —
//! timestamps from different domains are never silently compared (`I-S7`).
//!
//! REQ: REQ-SPATIAL-004, REQ-SPATIAL-041, REQ-SPATIAL-051, REQ-SPATIAL-035.

use std::collections::BTreeMap;

use crate::types::SpatialError;

/// The four architecture clock domains — the exact set, no invented variants
/// (`I-S7`).
/// REQ: REQ-SPATIAL-004.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClockDomain {
    /// Internal operating-system monotonic time.
    HostMonotonic,
    /// A device hardware timer (cameras, lidars, microcontrollers), keyed by id.
    Device(u16),
    /// A globally synchronized clock (`PTP` or `GNSS`).
    Global(GlobalSource),
    /// A simulation/replay virtual clock (may pause, slow, or replay).
    SimReplay,
}

/// The globally synchronized clock source.
/// REQ: REQ-SPATIAL-004.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GlobalSource {
    /// IEEE 1588 `PTP`.
    Ptp,
    /// `GNSS`-disciplined time.
    Gnss,
}

/// A clock identity: domain plus epoch. A device reset or re-time bumps the
/// epoch so an old mapping never applies to a new incarnation (`I-S7`).
/// REQ: REQ-SPATIAL-051.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockIdentity {
    /// The clock domain.
    pub domain: ClockDomain,
    /// Incarnation counter; reset/re-time bumps it.
    pub epoch: u32,
}

impl ClockIdentity {
    /// Constructs an identity at epoch `0`.
    /// REQ: REQ-SPATIAL-051.
    #[must_use]
    pub const fn new(domain: ClockDomain) -> Self {
        Self { domain, epoch: 0 }
    }
}

/// A timestamp carrying its clock identity (`I-S6`).
/// REQ: REQ-SPATIAL-035, REQ-SPATIAL-051.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimePoint {
    /// Nanoseconds on the identified clock.
    pub value_ns: u64,
    /// The clock identity (domain + epoch).
    pub clock: ClockIdentity,
}

impl TimePoint {
    /// Constructs a timestamp.
    /// REQ: REQ-SPATIAL-035.
    #[must_use]
    pub const fn new(value_ns: u64, clock: ClockIdentity) -> Self {
        Self { value_ns, clock }
    }
}

/// An affine mapping `t_to = to_ref + r · (t_from − from_ref)` with a reference
/// instant pair, rate ratio, uncertainty at the reference, and bounded
/// uncertainty growth ([spatial-core.md] §Temporal Precision).
/// REQ: REQ-SPATIAL-041.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockMapping {
    /// The target clock identity (domain + epoch) this mapping resolves to.
    pub to: ClockIdentity,
    /// Reference instant on the source clock, nanoseconds.
    pub from_ref_ns: i64,
    /// Reference instant on the target clock, nanoseconds.
    pub to_ref_ns: i64,
    /// Relative rate ratio `r`.
    pub rate_ratio: f64,
    /// Uncertainty at the reference instant, nanoseconds.
    pub uncertainty_at_ref_ns: u64,
    /// Bounded growth as a fraction of elapsed time.
    pub uncertainty_growth: f64,
    /// Validity interval start (source-clock nanoseconds).
    pub valid_from_ns: i64,
    /// Validity interval end (source-clock nanoseconds).
    pub valid_until_ns: i64,
    /// Mapping generation (re-estimation bumps it).
    pub generation: u64,
}

impl ClockMapping {
    /// The mapping's uncertainty at a source-clock instant, bounded by
    /// `uncertainty_growth` per elapsed nanosecond (`I-S7`).
    /// REQ: REQ-SPATIAL-041.
    #[must_use]
    pub fn uncertainty_ns(&self, at_ns: i64) -> u64 {
        let elapsed = (at_ns - self.from_ref_ns).unsigned_abs();
        let grown = self.uncertainty_growth * elapsed as f64;
        let grown = if grown.is_finite() && grown >= 0.0 {
            grown
        } else {
            0.0
        };
        self.uncertainty_at_ref_ns.saturating_add(grown as u64)
    }
}

/// Resolves source clock identity → target domain to an affine mapping, keyed by
/// full identity (domain + epoch) so an epoch bump invalidates old mappings.
/// REQ: REQ-SPATIAL-051, REQ-SPATIAL-041.
#[derive(Debug, Clone, Default)]
pub struct ClockTable {
    entries: BTreeMap<(ClockIdentity, ClockDomain), ClockMapping>,
}

impl ClockTable {
    /// An empty table (every lookup unresolved).
    /// REQ: REQ-SPATIAL-004.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces a mapping keyed by `(from identity, target domain)`.
    /// REQ: REQ-SPATIAL-041.
    pub fn insert(&mut self, from: ClockIdentity, mapping: ClockMapping) {
        self.entries.insert((from, mapping.to.domain), mapping);
    }

    /// Looks up the mapping for a full source identity and target domain.
    /// REQ: REQ-SPATIAL-051.
    #[must_use]
    pub fn resolve(&self, from: ClockIdentity, to: ClockDomain) -> Option<&ClockMapping> {
        self.entries.get(&(from, to))
    }
}

/// Maps a timestamp into target domain `to`, or returns
/// [`SpatialError::ClockUnresolved`] when no valid mapping exists (`I-S7`).
///
/// # Errors
/// [`SpatialError::ClockUnresolved`] for a missing mapping, a mapping outside
/// its validity interval, a non-finite rate, or a non-representable result.
/// REQ: REQ-SPATIAL-004, REQ-SPATIAL-041, REQ-SPATIAL-051.
pub fn map_timestamp(
    t: TimePoint,
    to: ClockDomain,
    table: &ClockTable,
) -> Result<TimePoint, SpatialError> {
    map_timestamp_with_uncertainty(t, to, table).map(|(mapped, _)| mapped)
}

/// Maps a timestamp into target domain `to` and reports the mapping's propagated
/// uncertainty in nanoseconds (`I-S7`). The affine relation
/// `t_to = to_ref + r·(t_from − from_ref)` applies the drift rate; uncertainty
/// at the source instant grows per the mapping's bounded
/// `uncertainty_growth`.
///
/// # Errors
/// [`SpatialError::ClockUnresolved`] for a missing mapping, a mapping outside
/// its validity interval, a non-finite rate, or a non-representable result.
/// REQ: REQ-SPATIAL-004, REQ-SPATIAL-041, REQ-SPATIAL-051.
pub fn map_timestamp_with_uncertainty(
    t: TimePoint,
    to: ClockDomain,
    table: &ClockTable,
) -> Result<(TimePoint, u64), SpatialError> {
    let Some(m) = table.resolve(t.clock, to) else {
        return Err(SpatialError::ClockUnresolved);
    };
    if !m.rate_ratio.is_finite() {
        return Err(SpatialError::ClockUnresolved);
    }
    let from_ns = i64::try_from(t.value_ns).map_err(|_| SpatialError::ClockUnresolved)?;
    if from_ns < m.valid_from_ns || from_ns > m.valid_until_ns {
        return Err(SpatialError::ClockUnresolved);
    }
    let mapped = m.to_ref_ns as f64 + m.rate_ratio * (from_ns - m.from_ref_ns) as f64;
    if !mapped.is_finite() || mapped < 0.0 || mapped > u64::MAX as f64 {
        return Err(SpatialError::ClockUnresolved);
    }
    Ok((
        TimePoint {
            value_ns: mapped as u64,
            clock: m.to,
        },
        m.uncertainty_ns(from_ns),
    ))
}

#[cfg(test)]
mod tests {
    use crate::clock::{
        ClockDomain, ClockIdentity, ClockMapping, ClockTable, TimePoint, map_timestamp,
    };
    use crate::types::SpatialError;

    fn table() -> ClockTable {
        let mut t = ClockTable::new();
        t.insert(
            ClockIdentity::new(ClockDomain::Device(7)),
            ClockMapping {
                to: ClockIdentity::new(ClockDomain::HostMonotonic),
                from_ref_ns: 1_000,
                to_ref_ns: 5_000,
                rate_ratio: 1.0,
                uncertainty_at_ref_ns: 10,
                uncertainty_growth: 0.0,
                valid_from_ns: 0,
                valid_until_ns: 10_000,
                generation: 1,
            },
        );
        t
    }

    /// REQ: REQ-SPATIAL-041 — the affine mapping applies the reference pair and
    /// rate ratio exactly.
    #[test]
    fn affine_mapping() {
        let out = map_timestamp(
            TimePoint::new(1_500, ClockIdentity::new(ClockDomain::Device(7))),
            ClockDomain::HostMonotonic,
            &table(),
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(out.value_ns, 5_500);
        assert_eq!(out.clock.domain, ClockDomain::HostMonotonic);
    }

    /// REQ: REQ-SPATIAL-004, REQ-SPATIAL-051 — an unknown domain/epoch is
    /// unresolved, never silently compared (`I-S7`).
    #[test]
    fn unresolved_rejected() {
        let missing = map_timestamp(
            TimePoint::new(1_500, ClockIdentity::new(ClockDomain::SimReplay)),
            ClockDomain::HostMonotonic,
            &table(),
        );
        assert_eq!(missing, Err(SpatialError::ClockUnresolved));

        // Epoch bump ⇒ the old mapping no longer applies.
        let bumped = ClockIdentity {
            domain: ClockDomain::Device(7),
            epoch: 2,
        };
        assert_eq!(
            map_timestamp(
                TimePoint::new(1_500, bumped),
                ClockDomain::HostMonotonic,
                &table()
            ),
            Err(SpatialError::ClockUnresolved)
        );
    }

    /// REQ: REQ-SPATIAL-041 — uncertainty grows with elapsed time and is bounded
    /// by the reference value otherwise.
    #[test]
    fn uncertainty_growth() {
        let mut t = table();
        t.insert(
            ClockIdentity::new(ClockDomain::Device(7)),
            ClockMapping {
                to: ClockIdentity::new(ClockDomain::HostMonotonic),
                from_ref_ns: 0,
                to_ref_ns: 0,
                rate_ratio: 1.0,
                uncertainty_at_ref_ns: 10,
                uncertainty_growth: 0.001,
                valid_from_ns: 0,
                valid_until_ns: 1_000_000,
                generation: 1,
            },
        );
        let m = t
            .resolve(
                ClockIdentity::new(ClockDomain::Device(7)),
                ClockDomain::HostMonotonic,
            )
            .unwrap_or_else(|| unreachable!("mapping present"));
        assert_eq!(m.uncertainty_ns(0), 10);
        assert_eq!(m.uncertainty_ns(10_000), 20);
    }

    /// REQ: REQ-SPATIAL-041 — the drift rate scales the elapsed source interval
    /// and the propagated uncertainty is the mapping's value at the source
    /// instant.
    #[test]
    fn drift_and_uncertainty_propagated() {
        let mut t = ClockTable::new();
        t.insert(
            ClockIdentity::new(ClockDomain::Global(crate::clock::GlobalSource::Ptp)),
            ClockMapping {
                to: ClockIdentity::new(ClockDomain::HostMonotonic),
                from_ref_ns: 1_000_000,
                to_ref_ns: 2_000_000,
                rate_ratio: 1.000_002,
                uncertainty_at_ref_ns: 50,
                uncertainty_growth: 0.000_01,
                valid_from_ns: 0,
                valid_until_ns: 10_000_000,
                generation: 1,
            },
        );
        // Elapsed = 100_000 ns from ref ⇒ drift adds 0.2 ns.
        let (out, unc) = super::map_timestamp_with_uncertainty(
            TimePoint::new(
                1_100_000,
                ClockIdentity::new(ClockDomain::Global(crate::clock::GlobalSource::Ptp)),
            ),
            ClockDomain::HostMonotonic,
            &t,
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(out.value_ns, 2_100_000);
        // uncertainty = 50 + 0.00001 * 100_000 = 51.
        assert_eq!(unc, 51);
    }

    /// REQ: REQ-SPATIAL-004 — a mapping outside its validity interval is
    /// unresolved, never silently extrapolated (`I-S7`).
    #[test]
    fn out_of_validity_rejected() {
        let t = table();
        let out = map_timestamp(
            TimePoint::new(50_000, ClockIdentity::new(ClockDomain::Device(7))),
            ClockDomain::HostMonotonic,
            &t,
        );
        assert_eq!(out, Err(SpatialError::ClockUnresolved));
    }
}
