//! Runtime cache for dynamic spatial edges
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §Protocols
//! `Dynamic edges remain runtime cache entries`;
//! [spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Data Structures `DynamicEdge`/`AtomicTransformSlot`;
//! [`spatial_architecture.md`](../../docs/spatial_architecture.md) §Versioned
//! Local Cache, §Dual Hierarchy). Each dynamic edge owns one versioned slot
//! written by a single spatial authority and read concurrently by any number of
//! nodes.
//!
//! The slot is a seqlock: a monotonic version counter is odd while a write is in
//! flight and even when stable (`I-Q14`). Payload `f64`s are stored as
//! fixed-width [`AtomicU64`] bit patterns, so the protocol needs no `UnsafeCell`
//! and the crate stays `#![forbid(unsafe_code)]`. Each slot stores the transform
//! plus its sample and receive [`TimePoint`]s and a discontinuous flag; the
//! sample clock identity (domain and epoch) rides along, so a device reset bumps
//! the epoch rather than rewriting per-sample bytes (`I-S6`, `I-S7`,
//! `I-Q14`). A generation counter is bumped on delete so outstanding handles
//! invalidate immediately (`I-S12`).
//!
//! REQ: REQ-SPATIAL-050, REQ-SPATIAL-035, REQ-SPATIAL-030, REQ-SPATIAL-011.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::clock::{ClockIdentity, TimePoint};
use crate::types::{SpatialError, Transform3};

/// Sentinel node id meaning "no writer claimed".
const UNCLAIMED: u32 = u32::MAX;

/// Bounded seqlock retry budget before a read gives up (returns `None`).
const MAX_RETRIES: u32 = 64;

/// Bit `0` of an edge slot's `flags` word: the newest sample is discontinuous
/// (a relocalization or teleport). Interpolation never spans a flagged boundary
/// (`I-Q7`).
/// REQ: REQ-SPATIAL-016.
pub const FLAG_DISCONTINUOUS: u32 = 1 << 0;

/// One dynamic-edge slot: versioned transform + sample/receive stamps + flags.
///
/// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
#[derive(Debug)]
pub struct EdgeSlot {
    /// Numeric dynamic-edge id (persistent identity, not a slot index).
    id: u32,
    version: AtomicU64,
    generation: AtomicU32,
    /// `[tx, ty, tz, qx, qy, qz, qw]` as `f64` bit patterns.
    payload: [AtomicU64; 7],
    sample_ns: AtomicU64,
    sample_epoch: AtomicU32,
    sample_domain: AtomicU64,
    receive_ns: AtomicU64,
    flags: AtomicU32,
}

impl EdgeSlot {
    /// Creates a slot for edge `id` at `generation`.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn new(id: u32, generation: u32) -> Self {
        Self {
            id,
            version: AtomicU64::new(0),
            generation: AtomicU32::new(generation),
            payload: [const { AtomicU64::new(0) }; 7],
            sample_ns: AtomicU64::new(0),
            sample_epoch: AtomicU32::new(0),
            sample_domain: AtomicU64::new(0),
            receive_ns: AtomicU64::new(0),
            flags: AtomicU32::new(0),
        }
    }

    /// The numeric dynamic-edge id.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// The current seqlock version; odd ⇒ a write is in flight (`I-Q14`).
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// The slot's current generation (`I-S12`).
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// Bumps the generation so outstanding handles invalidate (`I-S12`).
    /// REQ: REQ-SPATIAL-011.
    pub fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Encodes a clock domain into a `u64` for lockless storage.
    /// REQ: REQ-SPATIAL-050.
    fn encode_domain(clock: ClockIdentity) -> u64 {
        use crate::clock::{ClockDomain, GlobalSource};
        let tag = match clock.domain {
            ClockDomain::HostMonotonic => 0_u64,
            ClockDomain::Device(id) => 1_u64 | (u64::from(id) << 8),
            ClockDomain::Global(GlobalSource::Ptp) => 2_u64,
            ClockDomain::Global(GlobalSource::Gnss) => 3_u64,
            ClockDomain::SimReplay => 4_u64,
        };
        tag | (u64::from(clock.epoch) << 32)
    }

    /// Decodes a clock identity from its stored `u64`.
    /// REQ: REQ-SPATIAL-050.
    fn decode_domain(encoded: u64) -> ClockIdentity {
        use crate::clock::{ClockDomain, GlobalSource};
        let tag = encoded & 0xFF;
        let epoch = (encoded >> 32) as u32;
        let domain = match tag & 0x0F {
            1 => ClockDomain::Device(((encoded >> 8) & 0xFFFF) as u16),
            2 => ClockDomain::Global(GlobalSource::Ptp),
            3 => ClockDomain::Global(GlobalSource::Gnss),
            4 => ClockDomain::SimReplay,
            _ => ClockDomain::HostMonotonic,
        };
        ClockIdentity { domain, epoch }
    }

    /// Publishes a full sample (single-writer; caller holds the claim). The
    /// version counter goes odd, fields are written, then it goes even with
    /// release ordering so readers accept only a stable snapshot (`I-Q14`).
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
    pub fn publish_sample(&self, sample: &EdgeSample) {
        self.version.fetch_add(1, Ordering::AcqRel);
        let bits = [
            sample.transform.translation[0],
            sample.transform.translation[1],
            sample.transform.translation[2],
            sample.transform.rotation[0],
            sample.transform.rotation[1],
            sample.transform.rotation[2],
            sample.transform.rotation[3],
        ];
        for (slot, v) in self.payload.iter().zip(bits) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.sample_ns
            .store(sample.sample.value_ns, Ordering::Relaxed);
        self.sample_epoch
            .store(sample.sample.clock.epoch, Ordering::Relaxed);
        self.sample_domain
            .store(Self::encode_domain(sample.sample.clock), Ordering::Relaxed);
        self.receive_ns
            .store(sample.receive.value_ns, Ordering::Relaxed);
        self.flags.store(sample.flags, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Publishes a transform with host-monotonic sample/receive nanoseconds (the
    /// M19W1 signature); equivalent to [`Self::publish_sample`] with host
    /// identities and no flags.
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
    pub fn publish(&self, transform: &Transform3, sample_ns: u64, receive_ns: u64) {
        let host = ClockIdentity::new(crate::clock::ClockDomain::HostMonotonic);
        self.publish_sample(&EdgeSample {
            transform: *transform,
            sample: TimePoint::new(sample_ns, host),
            receive: TimePoint::new(receive_ns, host),
            flags: 0,
        });
    }

    /// Reads the newest stable sample, or `None` if the retry budget is
    /// exhausted by concurrent writers (never a torn read). A stable snapshot
    /// with an odd read-back version is impossible by construction (`I-Q14`).
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
    #[must_use]
    pub fn read(&self) -> Option<CachedSample> {
        for _ in 0..MAX_RETRIES {
            let v1 = self.version.load(Ordering::Acquire);
            if v1 & 1 == 1 {
                continue;
            }
            let mut bits = [0_u64; 7];
            for (i, slot) in self.payload.iter().enumerate() {
                bits[i] = slot.load(Ordering::Relaxed);
            }
            let sample_ns = self.sample_ns.load(Ordering::Relaxed);
            let sample_epoch = self.sample_epoch.load(Ordering::Relaxed);
            let sample_domain = self.sample_domain.load(Ordering::Relaxed);
            let receive_ns = self.receive_ns.load(Ordering::Relaxed);
            let flags = self.flags.load(Ordering::Relaxed);
            let v2 = self.version.load(Ordering::Acquire);
            if v1 == v2 {
                let mut clock = Self::decode_domain(sample_domain);
                clock.epoch = sample_epoch;
                return Some(CachedSample {
                    id: self.id,
                    transform: Transform3 {
                        translation: [
                            f64::from_bits(bits[0]),
                            f64::from_bits(bits[1]),
                            f64::from_bits(bits[2]),
                        ],
                        rotation: [
                            f64::from_bits(bits[3]),
                            f64::from_bits(bits[4]),
                            f64::from_bits(bits[5]),
                            f64::from_bits(bits[6]),
                        ],
                    },
                    sample: TimePoint::new(sample_ns, clock),
                    receive: TimePoint::new(receive_ns, clock),
                    flags,
                    version: v1,
                    generation: self.generation(),
                });
            }
        }
        None
    }
}

/// A full dynamic-edge sample: geometry plus sample/receive [`TimePoint`]s and
/// per-sample flags.
/// REQ: REQ-SPATIAL-035, REQ-SPATIAL-016.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeSample {
    /// The transform.
    pub transform: Transform3,
    /// The instant the pose was true, on its source clock (`I-S6`).
    pub sample: TimePoint,
    /// The instant DRM received the sample (`I-S6`).
    pub receive: TimePoint,
    /// Per-sample flag bits ([`FLAG_DISCONTINUOUS`]).
    pub flags: u32,
}

impl EdgeSample {
    /// Constructs a sample with no flags.
    /// REQ: REQ-SPATIAL-035.
    #[must_use]
    pub const fn new(transform: Transform3, sample: TimePoint, receive: TimePoint) -> Self {
        Self {
            transform,
            sample,
            receive,
            flags: 0,
        }
    }

    /// Whether this sample is flagged discontinuous (`I-Q7`).
    /// REQ: REQ-SPATIAL-016.
    #[must_use]
    pub const fn is_discontinuous(&self) -> bool {
        self.flags & FLAG_DISCONTINUOUS != 0
    }

    /// Returns `self` with the discontinuous flag set (`I-Q7`).
    /// REQ: REQ-SPATIAL-016.
    #[must_use]
    pub const fn with_discontinuous(mut self) -> Self {
        self.flags |= FLAG_DISCONTINUOUS;
        self
    }
}

/// A stable snapshot read from an [`EdgeSlot`].
/// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedSample {
    /// Numeric dynamic-edge id.
    pub id: u32,
    /// The transform.
    pub transform: Transform3,
    /// Sample time with its clock identity (`I-S6`).
    pub sample: TimePoint,
    /// Receive time with its clock identity (`I-S6`).
    pub receive: TimePoint,
    /// Per-sample flags ([`FLAG_DISCONTINUOUS`]).
    pub flags: u32,
    /// The seqlock version observed at read time.
    pub version: u64,
    /// Slot generation at read time.
    pub generation: u32,
}

impl CachedSample {
    /// Whether the sample is flagged discontinuous (`I-Q7`).
    /// REQ: REQ-SPATIAL-016.
    #[must_use]
    pub const fn is_discontinuous(&self) -> bool {
        self.flags & FLAG_DISCONTINUOUS != 0
    }
}

/// Single-writer authority over one dynamic edge (`I-S11`).
/// REQ: REQ-SPATIAL-030.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterClaim {
    edge: usize,
    node: u32,
}

impl WriterClaim {
    /// The claimed edge index.
    /// REQ: REQ-SPATIAL-030.
    #[must_use]
    pub fn edge(self) -> usize {
        self.edge
    }
}

/// A fixed-capacity cache of dynamic-edge slots, one writer per edge.
/// REQ: REQ-SPATIAL-050, REQ-SPATIAL-030.
#[derive(Debug)]
pub struct SpatialEdgeCache {
    slots: Vec<EdgeSlot>,
    writers: Vec<AtomicU32>,
}

impl SpatialEdgeCache {
    /// Allocates `edges` slots at generation `0`; edge `i` gets id `i`.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn new(edges: usize) -> Self {
        Self {
            slots: (0..edges).map(|i| EdgeSlot::new(i as u32, 0)).collect(),
            writers: (0..edges).map(|_| AtomicU32::new(UNCLAIMED)).collect(),
        }
    }

    /// Number of dynamic edges.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the cache holds no edges.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Claims sole write authority for `edge` for `node` (`I-S11`).
    ///
    /// # Errors
    /// [`SpatialError::WriterConflict`] when another node already owns the edge,
    /// or [`SpatialError::InvalidHandle`] for an out-of-range edge.
    /// REQ: REQ-SPATIAL-030.
    pub fn claim_writer(&self, edge: usize, node: u32) -> Result<WriterClaim, SpatialError> {
        let slot = self.writers.get(edge).ok_or(SpatialError::InvalidHandle)?;
        slot.compare_exchange(UNCLAIMED, node, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SpatialError::WriterConflict)?;
        Ok(WriterClaim { edge, node })
    }

    /// Publishes a sample under a held claim, verifying the claim still owns the
    /// edge.
    ///
    /// # Errors
    /// [`SpatialError::WriterConflict`] when the claim no longer owns the edge.
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-030.
    pub fn publish(
        &self,
        claim: &WriterClaim,
        transform: &Transform3,
        sample_ns: u64,
        receive_ns: u64,
    ) -> Result<(), SpatialError> {
        let owner = self
            .writers
            .get(claim.edge)
            .ok_or(SpatialError::InvalidHandle)?;
        if owner.load(Ordering::Acquire) != claim.node {
            return Err(SpatialError::WriterConflict);
        }
        self.slots
            .get(claim.edge)
            .ok_or(SpatialError::InvalidHandle)?
            .publish(transform, sample_ns, receive_ns);
        Ok(())
    }

    /// Publishes a full [`EdgeSample`] under a held claim, verifying the claim
    /// still owns the edge.
    ///
    /// # Errors
    /// [`SpatialError::WriterConflict`] when the claim no longer owns the edge,
    /// or [`SpatialError::InvalidHandle`] for an out-of-range edge.
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-030, REQ-SPATIAL-035.
    pub fn publish_sample(
        &self,
        claim: &WriterClaim,
        sample: &EdgeSample,
    ) -> Result<(), SpatialError> {
        let owner = self
            .writers
            .get(claim.edge)
            .ok_or(SpatialError::InvalidHandle)?;
        if owner.load(Ordering::Acquire) != claim.node {
            return Err(SpatialError::WriterConflict);
        }
        self.slots
            .get(claim.edge)
            .ok_or(SpatialError::InvalidHandle)?
            .publish_sample(sample);
        Ok(())
    }

    /// Reads the newest stable sample for `edge`, if any.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn read(&self, edge: usize) -> Option<CachedSample> {
        self.slots.get(edge).and_then(EdgeSlot::read)
    }

    /// Borrows the underlying slot for `edge`, exposing its version counter so a
    /// query can record the exact observed-version vector (`I-Q2`).
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-002.
    #[must_use]
    pub fn slot(&self, edge: usize) -> Option<&EdgeSlot> {
        self.slots.get(edge)
    }

    /// Deletes an edge: bumps its generation and releases its writer claim.
    ///
    /// # Errors
    /// [`SpatialError::InvalidHandle`] for an out-of-range edge.
    /// REQ: REQ-SPATIAL-011.
    pub fn delete(&self, edge: usize) -> Result<(), SpatialError> {
        let slot = self.slots.get(edge).ok_or(SpatialError::InvalidHandle)?;
        slot.bump_generation();
        let owner = self.writers.get(edge).ok_or(SpatialError::InvalidHandle)?;
        owner.store(UNCLAIMED, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cache::SpatialEdgeCache;
    use crate::cache::{EdgeSample, FLAG_DISCONTINUOUS};
    use crate::clock::{ClockDomain, ClockIdentity, TimePoint};
    use crate::types::{SpatialError, Transform3};

    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035 — a published sample round-trips
    /// bit-exactly, including its sample/receive stamps and id.
    #[test]
    #[allow(clippy::float_cmp)] // exact `f64` bit patterns round-trip unchanged
    fn publish_read_roundtrip() {
        let cache = SpatialEdgeCache::new(1);
        let claim = cache
            .claim_writer(0, 3)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let t = Transform3::new([1.5, -2.5, 3.5], [0.1, 0.2, 0.3, 0.927_361_8])
            .unwrap_or_else(|e| unreachable!("{e}"));
        cache
            .publish(&claim, &t, 111, 222)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let s = cache.read(0).unwrap_or_else(|| unreachable!("sample"));
        assert_eq!(s.transform.translation, [1.5, -2.5, 3.5]);
        assert_eq!(s.sample.value_ns, 111);
        assert_eq!(s.receive.value_ns, 222);
        assert_eq!(s.id, 0);
        assert!(!s.is_discontinuous());
    }

    /// REQ: REQ-SPATIAL-035, REQ-SPATIAL-051 — a full sample preserves its
    /// device clock identity and epoch, and the discontinuous flag (`I-Q7`).
    #[test]
    fn full_sample_preserves_clock_and_flags() {
        let cache = SpatialEdgeCache::new(1);
        let claim = cache
            .claim_writer(0, 3)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let dev = ClockIdentity {
            domain: ClockDomain::Device(7),
            epoch: 2,
        };
        let sample = EdgeSample::new(
            Transform3::IDENTITY,
            TimePoint::new(500, dev),
            TimePoint::new(900, ClockIdentity::new(ClockDomain::HostMonotonic)),
        )
        .with_discontinuous();
        cache
            .publish_sample(&claim, &sample)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let s = cache.read(0).unwrap_or_else(|| unreachable!("sample"));
        assert_eq!(s.sample, TimePoint::new(500, dev));
        assert!(s.is_discontinuous());
        assert_eq!(s.flags, FLAG_DISCONTINUOUS);
    }

    /// REQ: REQ-SPATIAL-050 — the version counter increments by two per publish
    /// (odd in flight, even stable) and is read back even (`I-Q14`).
    #[test]
    fn version_tracks_publishes() {
        let cache = SpatialEdgeCache::new(1);
        let claim = cache
            .claim_writer(0, 1)
            .unwrap_or_else(|e| unreachable!("{e}"));
        cache
            .publish(&claim, &Transform3::IDENTITY, 1, 1)
            .unwrap_or_else(|e| unreachable!("{e}"));
        cache
            .publish(&claim, &Transform3::IDENTITY, 2, 2)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let s = cache.read(0).unwrap_or_else(|| unreachable!("sample"));
        assert_eq!(s.version % 2, 0);
        assert_eq!(s.version, 4);
    }

    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-030 — publishing a full sample under a
    /// lost claim is refused.
    #[test]
    fn publish_sample_checks_claim() {
        let cache = SpatialEdgeCache::new(1);
        let claim = cache
            .claim_writer(0, 1)
            .unwrap_or_else(|e| unreachable!("{e}"));
        cache.delete(0).unwrap_or_else(|e| unreachable!("{e}"));
        let sample = EdgeSample::new(
            Transform3::IDENTITY,
            TimePoint::new(0, ClockIdentity::new(ClockDomain::HostMonotonic)),
            TimePoint::new(0, ClockIdentity::new(ClockDomain::HostMonotonic)),
        );
        assert_eq!(
            cache.publish_sample(&claim, &sample),
            Err(SpatialError::WriterConflict)
        );
    }

    /// REQ: REQ-SPATIAL-030 — a second node cannot claim an owned edge (`I-S11`).
    #[test]
    fn single_writer_enforced() {
        let cache = SpatialEdgeCache::new(1);
        let _ = cache
            .claim_writer(0, 1)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(cache.claim_writer(0, 2), Err(SpatialError::WriterConflict));
    }

    /// REQ: REQ-SPATIAL-011 — delete bumps the generation and releases the claim.
    #[test]
    fn delete_bumps_generation() {
        let cache = SpatialEdgeCache::new(1);
        assert_eq!(cache.read(0).map(|s| s.generation), Some(0));
        cache.delete(0).unwrap_or_else(|e| unreachable!("{e}"));
        cache
            .claim_writer(0, 9)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(cache.read(0).map(|s| s.generation), Some(1));
    }
}
