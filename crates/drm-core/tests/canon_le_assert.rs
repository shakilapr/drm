//! Explicit little-endian regression guard (canon-codec.md I2, I9;
//! work-plan M01W4-E1-T5). The LE proof is the hand-authored byte oracle —
//! this test only guards against accidental byte-order regressions on the
//! canonical host targets (x86-64, aarch64); it is not a portability proof.
//! REQ: REQ-TOOL-043.

use drm_core::canon;

/// REQ: REQ-TOOL-043 — every write primitive emits LE bytes per the frozen
/// oracle; every read primitive consumes them.
#[test]
fn canon_le_assert() {
    let mut b = [0xAA_u8; 8];

    assert!(canon::write_u16(&mut b, 0, 0x1234).is_ok());
    assert_eq!(&b[..2], &[0x34, 0x12]);
    assert_eq!(canon::read_u16(&b[..2], 0), Ok(0x1234));

    assert!(canon::write_u32(&mut b, 0, 0x1234_5678).is_ok());
    assert_eq!(&b[..4], &[0x78, 0x56, 0x34, 0x12]);
    assert_eq!(canon::read_u32(&b[..4], 0), Ok(0x1234_5678));

    assert!(canon::write_u64(&mut b, 0, 0x0123_4567_89AB_CDEF).is_ok());
    assert_eq!(&b[..], &[0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01]);
    assert_eq!(canon::read_u64(&b, 0), Ok(0x0123_4567_89AB_CDEF));

    assert!(canon::write_i32(&mut b, 0, -2).is_ok());
    assert_eq!(&b[..4], &[0xFE, 0xFF, 0xFF, 0xFF]);

    assert!(canon::write_f32(&mut b, 0, 1.0).is_ok());
    assert_eq!(&b[..4], &[0x00, 0x00, 0x80, 0x3F]);
}
