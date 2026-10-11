//! Runtime cache groundwork for dynamic spatial edges
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §Protocols
//! `Dynamic edges remain runtime cache entries`;
//! [`spatial_architecture.md`](../../docs/spatial_architecture.md) §Dual
//! Hierarchy). Each dynamic edge owns one versioned slot written by a
//! single spatial authority and read concurrently by any number of nodes.
//!
//! The slot is a seqlock: a monotonic sequence counter is odd while a write is
//! in flight and even when stable. Payload `f64`s are stored as fixed-width
//! [`AtomicU64`] bit patterns, so the protocol needs no `UnsafeCell` and the
//! crate stays `#![forbid(unsafe_code)]`. A generation counter is bumped on
//! delete so outstanding handles invalidate immediately (`I-S12`).
//!
//! REQ: REQ-SPATIAL-050, REQ-SPATIAL-030, REQ-SPATIAL-011.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::types::{SpatialError, Transform3};

/// Sentinel node id meaning "no writer claimed".
const UNCLAIMED: u32 = u32::MAX;

/// Bounded seqlock retry budget before a read gives up (returns `None`).
const MAX_RETRIES: u32 = 64;

/// One dynamic-edge slot: versioned transform + timestamps + generation.
///
/// REQ: REQ-SPATIAL-050.
#[derive(Debug)]
pub struct EdgeSlot {
    sequence: AtomicU64,
    generation: AtomicU32,
    /// `[tx, ty, tz, qx, qy, qz, qw]` as `f64` bit patterns.
    payload: [AtomicU64; 7],
    sample_ns: AtomicU64,
    receive_ns: AtomicU64,
}

impl EdgeSlot {
    /// Creates a slot at `generation`.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn new(generation: u32) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            generation: AtomicU32::new(generation),
            payload: [const { AtomicU64::new(0) }; 7],
            sample_ns: AtomicU64::new(0),
            receive_ns: AtomicU64::new(0),
        }
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

    /// Publishes a transform sample (single-writer; caller holds the claim).
    /// REQ: REQ-SPATIAL-050, REQ-SPATIAL-035.
    pub fn publish(&self, transform: &Transform3, sample_ns: u64, receive_ns: u64) {
        // Enter: sequence becomes odd (write in flight).
        self.sequence.fetch_add(1, Ordering::AcqRel);
        let bits = [
            transform.translation[0],
            transform.translation[1],
            transform.translation[2],
            transform.rotation[0],
            transform.rotation[1],
            transform.rotation[2],
            transform.rotation[3],
        ];
        for (slot, v) in self.payload.iter().zip(bits) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.sample_ns.store(sample_ns, Ordering::Relaxed);
        self.receive_ns.store(receive_ns, Ordering::Relaxed);
        // Exit: sequence becomes even (stable) with release ordering.
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Reads the newest stable sample, or `None` if the retry budget is
    /// exhausted by concurrent writers (never a torn read).
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn read(&self) -> Option<CachedSample> {
        for _ in 0..MAX_RETRIES {
            let s1 = self.sequence.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                continue;
            }
            let mut bits = [0_u64; 7];
            for (i, slot) in self.payload.iter().enumerate() {
                bits[i] = slot.load(Ordering::Relaxed);
            }
            let sample_ns = self.sample_ns.load(Ordering::Relaxed);
            let receive_ns = self.receive_ns.load(Ordering::Relaxed);
            let s2 = self.sequence.load(Ordering::Acquire);
            if s1 == s2 {
                return Some(CachedSample {
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
                    sample_ns,
                    receive_ns,
                    generation: self.generation(),
                });
            }
        }
        None
    }
}

/// A stable snapshot read from an [`EdgeSlot`].
/// REQ: REQ-SPATIAL-050.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedSample {
    /// The transform.
    pub transform: Transform3,
    /// Sample time, nanoseconds.
    pub sample_ns: u64,
    /// Receive time, nanoseconds.
    pub receive_ns: u64,
    /// Slot generation at read time.
    pub generation: u32,
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
    /// Allocates `edges` slots at generation `0`.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn new(edges: usize) -> Self {
        Self {
            slots: (0..edges).map(|_| EdgeSlot::new(0)).collect(),
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

    /// Reads the newest stable sample for `edge`, if any.
    /// REQ: REQ-SPATIAL-050.
    #[must_use]
    pub fn read(&self, edge: usize) -> Option<CachedSample> {
        self.slots.get(edge).and_then(EdgeSlot::read)
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
    use crate::types::{SpatialError, Transform3};

    /// REQ: REQ-SPATIAL-050 — a published sample round-trips bit-exactly through
    /// the lockless slot.
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
        assert_eq!(s.sample_ns, 111);
        assert_eq!(s.receive_ns, 222);
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
