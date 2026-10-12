//! Per-topic payload pool (docs/algorithms/runtime/payload-pool.md).
//! REQ: REQ-CORE-043 (dynamic count, {base, stride, count}), REQ-CORE-044 (static
//! offsets), REQ-CORE-046 (isolation), REQ-CORE-048 (reject before write),
//! REQ-CORE-041 (latest default pool 3).

use drm_types::Status;

/// Slot layout constants; offsets never relocate for the mapping lifetime.
/// REQ: REQ-CORE-043, REQ-CORE-044.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolLayout {
    /// Offset of slot 0 from the segment base (static constant).
    pub base: u64,
    /// `offset(i) = base + i * stride`; `stride = align_up(slot_size, slot_align)`.
    pub stride: u64,
    /// Slot count (deployment configuration, not compile time).
    pub count: u64,
    /// Canonical header size (canon-codec two-region `header_end`).
    pub header_size: u64,
    /// Canonical `sample_max` bound (one full sample).
    pub sample_max: u64,
    /// Slot alignment (element alignment of the tail included).
    pub slot_align: u64,
}

/// Offset of slot `i`.
/// REQ: REQ-CORE-044.
///
/// # Errors
/// [`Status::InvalidArg`] when `i >= count`; [`Status::InvalidConfig`] on checked-math
/// overflow or `usize` narrowing.
pub fn slot_offset(layout: &PoolLayout, i: u64) -> Result<usize, Status> {
    if i >= layout.count {
        return Err(Status::InvalidArg);
    }
    let offset = layout
        .stride
        .checked_mul(i)
        .and_then(|n| layout.base.checked_add(n))
        .ok_or(Status::InvalidConfig)?;
    usize::try_from(offset).map_err(|_| Status::InvalidConfig)
}

/// REQ: REQ-CORE-048 — structural rejection BEFORE any destination byte changes (I-P3).
/// # Errors
/// [`Status::InvalidArg`] when `canonical` exceeds the sample bound or the slot bounds.
pub fn write_checked(
    buf: &mut [u8],
    layout: &PoolLayout,
    slot: u64,
    canonical: &[u8],
) -> Result<u64, Status> {
    if canonical.len() as u64 > layout.sample_max {
        return Err(Status::InvalidArg);
    }
    let off = slot_offset(layout, slot)?;
    let end = off
        .checked_add(canonical.len())
        .ok_or(Status::InvalidConfig)?;
    buf.get_mut(off..end)
        .ok_or(Status::InvalidConfig)?
        .copy_from_slice(canonical);
    Ok(canonical.len() as u64)
}

/// REQ: REQ-CORE-041 — default pool for `@history: latest`.
#[must_use]
pub const fn default_pool_size(history_is_latest: bool) -> u64 {
    if history_is_latest { 3 } else { 8 }
}

#[cfg(test)]
mod tests {
    use super::{PoolLayout, default_pool_size, slot_offset, write_checked};
    use drm_types::Status;

    fn layout(count: u64) -> PoolLayout {
        PoolLayout {
            base: 64,
            stride: 32,
            count,
            header_size: 0,
            sample_max: 16,
            slot_align: 8,
        }
    }

    /// REQ: REQ-CORE-044 — slot `i` sits at `base + i*stride`, stable across
    /// repeated derivation (I-P1); out-of-range is denied.
    #[test]
    fn pool_offsets() {
        let l = layout(4);
        for i in 0..4_u64 {
            let expect = usize::try_from(64 + i * 32).unwrap_or(0);
            assert_eq!(slot_offset(&l, i), Ok(expect), "slot {i}");
            assert_eq!(slot_offset(&l, i), Ok(expect), "re-derived slot {i}");
        }
        assert_eq!(slot_offset(&l, 4), Err(Status::InvalidArg));
    }

    /// REQ: REQ-CORE-046 — per-topic pools are independent partitions: a write
    /// into one pool's slot never mutates another pool's bytes (I-P2).
    #[test]
    fn pool_isolation() {
        let a = layout(2);
        let b = layout(2);
        let mut buf_a = [0xAA_u8; 128];
        let mut buf_b = [0xBB_u8; 128];
        assert_eq!(write_checked(&mut buf_a, &a, 0, &[1_u8; 16]), Ok(16));
        assert_eq!(&buf_b[64..80], &[0xBB_u8; 16], "pool b untouched");
        assert_eq!(&buf_a[64..80], &[1_u8; 16]);
        assert_eq!(write_checked(&mut buf_b, &b, 0, &[2_u8; 16]), Ok(16));
        assert_eq!(&buf_a[64..80], &[1_u8; 16], "pool a untouched");
    }

    /// REQ: REQ-CORE-048 — oversize payload rejected before any write: the
    /// destination keeps its sentinel fill (I-P3); `max` and empty pass.
    #[test]
    fn pool_size_reject() {
        let l = layout(2);
        let mut buf = [0x5A_u8; 128];
        assert_eq!(write_checked(&mut buf, &l, 0, &[9_u8; 16]), Ok(16));
        let before = buf;
        assert_eq!(
            write_checked(&mut buf, &l, 0, &[7_u8; 17]),
            Err(Status::InvalidArg)
        );
        assert_eq!(buf, before, "no destination byte changed");
        assert_eq!(write_checked(&mut buf, &l, 1, &[]), Ok(0));
    }

    /// REQ: REQ-CORE-044 — checked offset arithmetic refuses an overflowing
    /// layout and a slot beyond the destination region as `InvalidConfig`
    /// (I-P1/I-P3): the destination is never partially written.
    #[test]
    fn pool_offset_overflow_and_bounds() {
        // base + stride*i overflows u64 ⇒ InvalidConfig, not a wrapped offset.
        let overflow = PoolLayout {
            base: u64::MAX,
            stride: 2,
            count: 4,
            header_size: 0,
            sample_max: 16,
            slot_align: 8,
        };
        assert_eq!(slot_offset(&overflow, 1), Err(Status::InvalidConfig));
        // A valid offset past the destination slice is InvalidConfig.
        let l = layout(4);
        let mut buf = [0_u8; 96];
        assert_eq!(
            write_checked(&mut buf, &l, 3, &[3_u8; 16]),
            Err(Status::InvalidConfig),
            "slot 3 ends at 160 > buf 96"
        );
    }

    /// REQ: REQ-CORE-041 — the declared default pool is 3 for `@history:
    /// latest` and 8 otherwise.
    #[test]
    fn pool_default_sizes() {
        assert_eq!(default_pool_size(true), 3);
        assert_eq!(default_pool_size(false), 8);
    }
}
