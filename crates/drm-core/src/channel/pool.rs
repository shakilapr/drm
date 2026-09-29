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
