//! Canonical codec (docs/algorithms/canon-codec.md): primitive read/write over raw
//! bytes, bounds-checked, LE on every host; two-region descriptor validation for
//! variable members.
//! REQ: REQ-TOOL-043 (little-endian), REQ-TOOL-044 (standard sizes), REQ-TOOL-041
//! (checked arithmetic), REQ-CORE-083 (no UB), REQ-CORE-082 (no alloc).

use drm_types::Status;

/// REQ: REQ-CORE-081 — bounds check before ANY destination write.
/// # Errors
/// [`Status::InvalidArg`] when `off + size` exceeds `len`.
pub fn check_bounds(len: usize, off: usize, size: usize) -> Result<(), Status> {
    let end = off.checked_add(size).ok_or(Status::InvalidData)?;
    if end > len {
        return Err(Status::InvalidArg);
    }
    Ok(())
}

/// REQ: REQ-TOOL-043, REQ-TOOL-044, REQ-CORE-083 — primitive read/write family.
/// Every accessor bounds-checks first (`I3`) and never panics (`I5`).
mod prim {
    use super::{check_bounds, Status};

    macro_rules! le_pair {
        ($read:ident, $write:ident, $t:ty, $n:expr, $doc:expr) => {
            /// REQ: REQ-TOOL-043 — bounds-checked LE read ($doc).
            #[must_use]
            pub fn $read(buf: &[u8], off: usize) -> Result<$t, Status> {
                let b = <$t>::from_le_bytes(match buf.get(off..off + $n) {
                    Some(s) => s.try_into().ok().ok_or(Status::InvalidData)?,
                    None => return Err(Status::InvalidArg),
                });
                Ok(b)
            }
            /// REQ: REQ-TOOL-043, REQ-CORE-081 — bounds-checked LE write ($doc).
            ///
            /// # Errors
            /// [`Status::InvalidArg`] before any write on short buffer.
            pub fn $write(buf: &mut [u8], off: usize, v: $t) -> Result<(), Status> {
                check_bounds(buf.len(), off, $n)?;
                buf[off..off + $n].copy_from_slice(&v.to_le_bytes());
                Ok(())
            }
        };
    }

    le_pair!(read_u8, write_u8, u8, 1, "1 byte");
    le_pair!(read_i8, write_i8, i8, 1, "1 byte");
    le_pair!(read_u16, write_u16, u16, 2, "2 bytes");
    le_pair!(read_i16, write_i16, i16, 2, "2 bytes");
    le_pair!(read_u32, write_u32, u32, 4, "4 bytes");
    le_pair!(read_i32, write_i32, i32, 4, "4 bytes");
    le_pair!(read_f32, write_f32, f32, 4, "4-byte IEEE");
    le_pair!(read_u64, write_u64, u64, 8, "8 bytes");
    le_pair!(read_i64, write_i64, i64, 8, "8 bytes");
    le_pair!(read_f64, write_f64, f64, 8, "8-byte IEEE");
}

pub use prim::{
    read_f32, read_f64, read_i16, read_i32, read_i64, read_i8, read_u16, read_u32, read_u64,
    read_u8, write_f32, write_f64, write_i16, write_i32, write_i64, write_i8, write_u16, write_u32,
    write_u64, write_u8,
};

/// REQ: REQ-TOOL-044, REQ-CORE-083 — `bool` is byte 0/1; anything else ⇒ `InvalidData`.
/// # Errors
/// [`Status::InvalidData`] for bytes ∉ {0,1}.
pub fn read_bool(buf: &[u8], off: usize) -> Result<bool, Status> {
    Ok(match read_u8(buf, off)? {
        0 => false,
        1 => true,
        _ => return Err(Status::InvalidData),
    })
}

/// REQ: REQ-TOOL-044, REQ-CORE-081 — `bool` write.
///
/// # Errors
/// [`Status::InvalidArg`] before any write on short buffer.
pub fn write_bool(buf: &mut [u8], off: usize, v: bool) -> Result<(), Status> {
    write_u8(buf, off, u8::from(v))
}

/// Two-region variable-member descriptor (canon-codec §Data Structures).
/// REQ: REQ-TOOL-042, REQ-TOOL-001.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Declared element/byte count (canonical packing derives offsets from counts).
    pub count: u64,
    /// Declared structural max (`u32::MAX`-capped at schema level).
    pub max: u64,
    /// Absolute offset from sample start (element-aligned, ≥ `header_end`).
    pub offset: u64,
    /// End of the fixed header (bodies must start at or after it).
    pub header_end: u64,
    /// Element size in the tail (`string`/`bytes` ⇒ 1).
    pub elem_size: u64,
    /// Element alignment (`string`/`bytes` ⇒ 1).
    pub elem_align: u64,
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040, REQ-CORE-083 — validate one variable body against
/// the sample: bound, alignment, canonical packing position, containment.
/// # Errors
/// [`Status::InvalidData`] on any descriptor violation (I5: value or error, never UB).
pub fn body_range(buf_len: u64, d: &Descriptor, sample_len: u64) -> Result<(u64, u64), Status> {
    if d.count > d.max {
        return Err(Status::InvalidData);
    }
    if d.offset < d.header_end || d.elem_align == 0 || d.offset % d.elem_align != 0 {
        return Err(Status::InvalidData);
    }
    let bytes = d.count.checked_mul(d.elem_size).ok_or(Status::InvalidData)?;
    let end = d.offset.checked_add(bytes).ok_or(Status::InvalidData)?;
    if end > sample_len || sample_len > buf_len {
        return Err(Status::InvalidData);
    }
    Ok((d.offset, end))
}

/// Runtime two-region layout: fixed-header end, per-variable descriptors, computed
/// sample capacity. Produced by the toolchain layout computation and mirrored here
/// for mapped-sample validation (canon-codec I6).
/// REQ: REQ-TOOL-042, REQ-TOOL-051.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonLayout {
    /// End of fixed header + descriptor block.
    pub header_end: u64,
    /// Maximum canonical sample size (fixed capacity bound).
    pub sample_max: u64,
    /// Slot/struct alignment.
    pub struct_align: u32,
    /// Variable members in declaration order.
    pub descriptors: Vec<Descriptor>,
}

/// REQ: REQ-TOOL-042, REQ-CORE-083 — whole-sample decoder validation: every descriptor
/// must agree with the canonical packing derived from the counts; `sample_len` is
/// derived, never stored.
/// # Errors
/// [`Status::InvalidData`] on any mismatch (I5).
pub fn validate_sample(buf: &[u8], layout: &CanonLayout, sample_len: u64) -> Result<(), Status> {
    let buf_len = u64::try_from(buf.len()).map_err(|_| Status::InvalidData)?;
    if sample_len < layout.header_end || sample_len > layout.sample_max || sample_len > buf_len {
        return Err(Status::InvalidData);
    }
    for d in &layout.descriptors {
        body_range(buf_len, d, sample_len)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ: REQ-TOOL-043 — primitive boundary round-trip over a scratch buffer.
    #[test]
    fn prim_roundtrip_boundary() {
        let mut b = [0u8; 32];
        assert!(write_u8(&mut b, 0, u8::MAX).is_ok());
        assert_eq!(read_u8(&b, 0), Ok(u8::MAX));
        assert!(write_i8(&mut b, 1, i8::MIN).is_ok());
        assert_eq!(read_i8(&b, 1), Ok(i8::MIN));
        assert!(write_u16(&mut b, 2, u16::MAX).is_ok());
        assert_eq!(read_u16(&b, 2), Ok(u16::MAX));
        assert!(write_i16(&mut b, 4, i16::MIN).is_ok());
        assert_eq!(read_i16(&b, 4), Ok(i16::MIN));
        assert!(write_u32(&mut b, 6, u32::MAX).is_ok());
        assert_eq!(read_u32(&b, 6), Ok(u32::MAX));
        assert!(write_i32(&mut b, 10, i32::MIN).is_ok());
        assert_eq!(read_i32(&b, 10), Ok(i32::MIN));
        assert!(write_u64(&mut b, 14, u64::MAX).is_ok());
        assert_eq!(read_u64(&b, 14), Ok(u64::MAX));
        assert!(write_i64(&mut b, 22, i64::MIN).is_ok());
        assert_eq!(read_i64(&b, 22), Ok(i64::MIN));
        assert!(write_f32(&mut b, 28, 1.0).is_ok());
        assert_eq!(read_f32(&b, 28), Ok(1.0_f32));
        assert!(write_bool(&mut b, 0, true).is_ok());
        assert_eq!(read_bool(&b, 0), Ok(true));
    }

    /// REQ: REQ-TOOL-043 — LE byte oracle, hand-authored (I2/I9): 0x12345678 ⇒ 78 56 34 12.
    #[test]
    fn le_byte_oracle() {
        let mut b = [0xAA; 4];
        assert!(write_u32(&mut b, 0, 0x1234_5678).is_ok());
        assert_eq!(b, [0x78, 0x56, 0x34, 0x12]);
    }

    /// REQ: REQ-CORE-083 — bool decode rejects every byte outside {0,1}.
    #[test]
    fn bool_strict() {
        assert_eq!(read_bool(&[0], 0), Ok(false));
        assert_eq!(read_bool(&[1], 0), Ok(true));
        assert_eq!(read_bool(&[2], 0), Err(Status::InvalidData));
        assert_eq!(read_bool(&[255], 0), Err(Status::InvalidData));
    }

    /// REQ: REQ-CORE-083, REQ-CORE-081 — short buffers error before any write (I3).
    #[test]
    fn bounds_reject_before_write() {
        let mut b = [0xEE; 8];
        assert_eq!(write_u64(&mut b, 1, 0), Err(Status::InvalidArg));
        assert_eq!(b, [0xEE; 8]);
        assert_eq!(read_u64(&[0u8; 3], 0), Err(Status::InvalidArg));
        assert_eq!(read_u64(&[0u8; 3], usize::MAX / 2), Err(Status::InvalidArg));
    }

    /// REQ: REQ-TOOL-042, REQ-TOOL-040 — two-region body validation: bound, alignment,
    /// packing position, containment (I5).
    #[test]
    fn body_range_validate() {
        let d_ok = Descriptor {
            count: 4,
            max: 16,
            offset: 24,
            header_end: 20,
            elem_size: 8,
            elem_align: 8,
        };
        assert_eq!(body_range(128, &d_ok, 56), Ok((24, 56)));
        let d_over_max = Descriptor { count: 17, ..d_ok };
        assert_eq!(body_range(128, &d_over_max, 160), Err(Status::InvalidData));
        let d_misaligned = Descriptor { offset: 25, ..d_ok };
        assert_eq!(body_range(128, &d_misaligned, 60), Err(Status::InvalidData));
        let d_before_header = Descriptor { offset: 16, ..d_ok };
        assert_eq!(body_range(128, &d_before_header, 60), Err(Status::InvalidData));
        let d_overflow = Descriptor {
            count: u64::MAX,
            max: u64::MAX,
            offset: 24,
            header_end: 20,
            elem_size: 8,
            elem_align: 8,
        };
        assert_eq!(body_range(128, &d_overflow, u64::MAX), Err(Status::InvalidData));
        let d_past_sample = Descriptor { offset: 24, ..d_ok };
        assert_eq!(body_range(128, &d_past_sample, 30), Err(Status::InvalidData));
    }

    /// REQ: REQ-TOOL-042, REQ-CORE-083 — validate_sample rejects bad lengths/headers.
    #[test]
    fn sample_validate() {
        let layout = CanonLayout {
            header_end: 20,
            sample_max: 64,
            struct_align: 8,
            descriptors: vec![Descriptor {
                count: 2,
                max: 4,
                offset: 24,
                header_end: 20,
                elem_size: 1,
                elem_align: 1,
            }],
        };
        let buf = [0u8; 64];
        assert!(validate_sample(&buf, &layout, 26).is_ok());
        assert_eq!(validate_sample(&buf, &layout, 18), Err(Status::InvalidData));
        assert_eq!(validate_sample(&buf, &layout, 80), Err(Status::InvalidData));
        assert_eq!(validate_sample(&buf, &layout, 100), Err(Status::InvalidData));
    }
}
