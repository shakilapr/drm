//! DRM shared primitives. Layer `types`: zero dependencies (codebase.md 2.2 R1).
//!
//! The status taxonomy is the single error vocabulary for every DRM crate and is
//! mirrored 1:1 by fixed-width C codes with explicit values (codebase.md 3.1 R4).
//! REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes),
//! REQ-BIND-017 (status C representation is fixed `int32_t` with explicit values).

#![forbid(unsafe_code)]

pub mod blake3;

pub use blake3::{blake3, blake3_128};

/// Rust mirror of the handwritten `DRM_ABI_VERSION` in
/// `bindings/c/include/drm/drm.h` (bump on layout change only; additions do
/// not bump). REQ: REQ-BIND-018, REQ-TOOL-023.
pub const ABI_VERSION: u32 = 0;

/// DRM status taxonomy. Variants map 1:1 to the append-only C code list in
/// `bindings/c/include/drm/drm.h` (never renumbered, never reused; no `Internal`
/// variant — fail-stop, codebase.md 3.2).
/// REQ: REQ-BIND-007, REQ-BIND-017.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Success.
    Ok,
    /// Null pointer, out-of-range length, or malformed input argument.
    InvalidArg,
    /// Referenced object does not exist.
    NotFound,
    /// Exclusive create lost a concurrent create race.
    AlreadyExists,
    /// Authorization or filesystem permission failure.
    Permission,
    /// Bounded queue or channel storage is full.
    QueueFull,
    /// Ordered stream detected a sequence gap (record/replay, RPC events).
    Gap,
    /// Observed object state predates the expected generation (ABA-safe).
    Stale,
    /// Contract fingerprint or ABI version does not match.
    WrongAbi,
    /// Payload pool or hazard pool has no free slot.
    PoolExhausted,
    /// Transient condition; bounded caller retry policy applies.
    Retry,
    /// Non-blocking guard rejected a concurrent operation (one writer, one borrower).
    Busy,
    /// Consumer has no committed sample yet (empty channel, watermark below start).
    NoSample,
    /// Borrow token is stale, foreign, or already released.
    InvalidBorrow,
    /// Reusable-counter generation space is exhausted (hazard slot generation).
    GenerationExhausted,
    /// Payload failed codec validation or size consistency checks.
    InvalidData,
    /// Configuration value is structurally invalid (budgets, limits, manifests).
    InvalidConfig,
    /// Object exists but initialization has not completed yet; startup retry applies.
    NotReady,
    /// OS-backed lock could not be acquired or released.
    LockFailed,
    /// Sequence counter space is exhausted for the channel lifetime.
    SequenceExhausted,
    /// Reclamation epoch space is exhausted; maintenance reclaim required.
    EpochExhausted,
    /// Ring stamp space is exhausted for the ring lifetime.
    RingStampExhausted,
}

impl Status {
    /// Fixed `int32_t` C code with an explicit value (append-only).
    /// REQ: REQ-BIND-007, REQ-BIND-017.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Ok => 0,
            Self::InvalidArg => -1,
            Self::NotFound => -2,
            Self::AlreadyExists => -3,
            Self::Permission => -4,
            Self::QueueFull => -5,
            Self::Gap => -6,
            Self::Stale => -7,
            Self::WrongAbi => -8,
            Self::PoolExhausted => -9,
            Self::Retry => -10,
            Self::Busy => -11,
            Self::NoSample => -12,
            Self::InvalidBorrow => -13,
            Self::GenerationExhausted => -14,
            Self::InvalidData => -15,
            Self::InvalidConfig => -16,
            Self::NotReady => -17,
            Self::LockFailed => -18,
            Self::SequenceExhausted => -19,
            Self::EpochExhausted => -20,
            Self::RingStampExhausted => -21,
        }
    }

    /// Inverse of [`Status::code`]. `None` for unknown codes; bindings surface those
    /// as `UnknownStatus(code)` instead of a `repr(C)` fieldless enum.
    /// REQ: REQ-BIND-017.
    #[must_use]
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Ok),
            -1 => Some(Self::InvalidArg),
            -2 => Some(Self::NotFound),
            -3 => Some(Self::AlreadyExists),
            -4 => Some(Self::Permission),
            -5 => Some(Self::QueueFull),
            -6 => Some(Self::Gap),
            -7 => Some(Self::Stale),
            -8 => Some(Self::WrongAbi),
            -9 => Some(Self::PoolExhausted),
            -10 => Some(Self::Retry),
            -11 => Some(Self::Busy),
            -12 => Some(Self::NoSample),
            -13 => Some(Self::InvalidBorrow),
            -14 => Some(Self::GenerationExhausted),
            -15 => Some(Self::InvalidData),
            -16 => Some(Self::InvalidConfig),
            -17 => Some(Self::NotReady),
            -18 => Some(Self::LockFailed),
            -19 => Some(Self::SequenceExhausted),
            -20 => Some(Self::EpochExhausted),
            -21 => Some(Self::RingStampExhausted),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Status;

    /// REQ: REQ-BIND-007 — every variant has exactly one explicit code value.
    #[test]
    fn status_codes_unique() {
        let variants = [
            Status::Ok,
            Status::InvalidArg,
            Status::NotFound,
            Status::AlreadyExists,
            Status::Permission,
            Status::QueueFull,
            Status::Gap,
            Status::Stale,
            Status::WrongAbi,
            Status::PoolExhausted,
            Status::Retry,
            Status::Busy,
            Status::NoSample,
            Status::InvalidBorrow,
            Status::GenerationExhausted,
            Status::InvalidData,
            Status::InvalidConfig,
            Status::NotReady,
            Status::LockFailed,
            Status::SequenceExhausted,
            Status::EpochExhausted,
            Status::RingStampExhausted,
        ];
        let mut codes = variants.iter().map(|s| s.code()).collect::<Vec<_>>();
        codes.sort_unstable();
        let distinct = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), distinct, "codes must be pairwise distinct");
        assert_eq!(Status::Ok.code(), 0, "DRM_OK is 0");
    }

    /// REQ: REQ-BIND-017 — code mapping is a bijection over the declared range.
    #[test]
    fn status_roundtrip() {
        for code in -21..=0 {
            assert_eq!(
                Status::from_code(code).map(Status::code),
                Some(code),
                "code {code}"
            );
        }
        assert!(Status::from_code(1).is_none());
        assert!(Status::from_code(-22).is_none());
    }
}
