//! Patterned-payload integrity scheme for billion-cycle stress runs
//! (channel-publish.md §Stress Integrity Harness). A writer stamps a
//! self-describing pattern into the sample; a reader classifies a bad read as
//! `Torn`, `MixedGeneration`, or `Invalid` without a software CRC.
//! REQ: REQ-CORE-082, REQ-QAL-006.

/// Sequence field offset (little-endian `u64`).
pub const SEQ_OFF: usize = 0;
/// Slot-echo field offset (little-endian `u32`).
pub const SLOT_OFF: usize = 8;
/// Padding between the slot echo and the generation field.
pub const SLOT_PAD: usize = 4;
/// Generation-echo field offset (little-endian `u64`).
pub const GEN_OFF: usize = 16;
/// First probe block offset (each block = `seq: u64`, `gen: u64`).
pub const PROBE_BASE: usize = 24;
/// Probe block stride in bytes.
pub const PROBE_STRIDE: usize = 16;
/// Number of repeated probe blocks (spanning early and late sample bytes).
pub const PROBE_COUNT: usize = 4;
/// First derived fill byte.
pub const FILL_START: usize = PROBE_BASE + PROBE_COUNT * PROBE_STRIDE;
/// Minimum sample length carrying the full pattern.
pub const MIN_LEN: usize = FILL_START;

/// Why a patterned sample failed validation.
/// REQ: REQ-QAL-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anomaly {
    /// Probe blocks disagree on `sequence`: a partial overwrite across two
    /// committed samples.
    Torn,
    /// Probes agree on `sequence` but disagree on `generation`: stale slot
    /// reuse.
    MixedGeneration,
    /// Probes are coherent but a byte does not match its derived value:
    /// single-byte corruption.
    Invalid,
}

/// Little-endian `u64` at `off`.
fn read_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0_u8; 8];
    if let Some(s) = b.get(off..off + 8) {
        a.copy_from_slice(s);
    }
    u64::from_le_bytes(a)
}

/// Deterministic fill byte derived from `(sequence, generation, index)`.
/// REQ: REQ-CORE-082.
#[must_use]
pub fn fill_byte(sequence: u64, generation: u64, index: usize) -> u8 {
    let mut x = sequence
        .wrapping_add(generation.rotate_left(17))
        .wrapping_add(
            u64::try_from(index)
                .unwrap_or(0)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15),
        );
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    u8::try_from(x & 0xFF).unwrap_or(0)
}

/// Writes the full pattern for `(sequence, slot, generation)` into `buf`.
/// The caller publishes exactly `buf` as the sample body.
/// REQ: REQ-CORE-082.
pub fn write_pattern(buf: &mut [u8], sequence: u64, slot: u32, generation: u64) {
    if buf.len() < MIN_LEN {
        return;
    }
    buf[SEQ_OFF..SEQ_OFF + 8].copy_from_slice(&sequence.to_le_bytes());
    buf[SLOT_OFF..SLOT_OFF + 4].copy_from_slice(&slot.to_le_bytes());
    buf[GEN_OFF..GEN_OFF + 8].copy_from_slice(&generation.to_le_bytes());
    for k in 0..PROBE_COUNT {
        let off = PROBE_BASE + k * PROBE_STRIDE;
        buf[off..off + 8].copy_from_slice(&sequence.to_le_bytes());
        buf[off + 8..off + 16].copy_from_slice(&generation.to_le_bytes());
    }
    for (i, b) in buf[FILL_START..].iter_mut().enumerate() {
        *b = fill_byte(sequence, generation, FILL_START + i);
    }
}

/// Validates a patterned sample against itself, classifying any anomaly.
/// REQ: REQ-QAL-006.
///
/// # Errors
/// [`Anomaly::Torn`] on disagreeing probe sequences;
/// [`Anomaly::MixedGeneration`] on agreeing sequences with differing
/// generations; [`Anomaly::Invalid`] on a mismatched derived fill byte or a
/// too-short buffer.
pub fn validate_pattern(buf: &[u8]) -> Result<(), Anomaly> {
    if buf.len() < MIN_LEN {
        return Err(Anomaly::Invalid);
    }
    let seq = read_u64(buf, SEQ_OFF);
    let generation = read_u64(buf, GEN_OFF);
    let mut seq0: Option<u64> = None;
    let mut gen0: Option<u64> = None;
    let mut torn = false;
    let mut mixed = false;
    for k in 0..PROBE_COUNT {
        let off = PROBE_BASE + k * PROBE_STRIDE;
        let ps = read_u64(buf, off);
        let pg = read_u64(buf, off + 8);
        match seq0 {
            None => seq0 = Some(ps),
            Some(v) if v != ps => torn = true,
            _ => {}
        }
        match gen0 {
            None => gen0 = Some(pg),
            Some(v) if v != pg => mixed = true,
            _ => {}
        }
    }
    if torn {
        return Err(Anomaly::Torn);
    }
    if mixed {
        return Err(Anomaly::MixedGeneration);
    }
    if seq0 != Some(seq) {
        return Err(Anomaly::Torn);
    }
    if gen0 != Some(generation) {
        return Err(Anomaly::MixedGeneration);
    }
    for (i, b) in buf[FILL_START..].iter().enumerate() {
        if *b != fill_byte(seq, generation, FILL_START + i) {
            return Err(Anomaly::Invalid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Anomaly, FILL_START, MIN_LEN, validate_pattern, write_pattern};

    fn patterned(sequence: u64, generation: u64) -> Vec<u8> {
        let mut buf = vec![0_u8; 128];
        write_pattern(&mut buf, sequence, 3, generation);
        buf
    }

    /// REQ: REQ-CORE-082 — a freshly written pattern validates.
    #[test]
    fn pattern_clean() {
        assert_eq!(validate_pattern(&patterned(1, 1)), Ok(()));
        assert_eq!(
            validate_pattern(&[0_u8; MIN_LEN - 1]),
            Err(Anomaly::Invalid)
        );
    }

    /// REQ: REQ-QAL-006 — a single flipped fill byte is `Invalid`.
    #[test]
    fn pattern_byte_flip() {
        let mut buf = patterned(7, 4);
        buf[FILL_START + 5] ^= 0xFF;
        assert_eq!(validate_pattern(&buf), Err(Anomaly::Invalid));
    }

    /// REQ: REQ-QAL-006 — mixed halves from two sequences are `Torn`.
    #[test]
    fn pattern_torn() {
        let a = patterned(1, 9);
        let b = patterned(2, 9);
        let mut mixed = a.clone();
        mixed[64..].copy_from_slice(&b[64..]);
        assert_eq!(validate_pattern(&mixed), Err(Anomaly::Torn));
    }

    /// REQ: REQ-QAL-006 — same sequence, two generations is `MixedGeneration`.
    #[test]
    fn pattern_mixed_generation() {
        let a = patterned(5, 1);
        let b = patterned(5, 2);
        let mut mixed = a.clone();
        mixed[64..].copy_from_slice(&b[64..]);
        assert_eq!(validate_pattern(&mixed), Err(Anomaly::MixedGeneration));
    }
}
