//! CRC32C (Castagnoli, reflected poly `0x82F63B78`, init/final `0xFFFFFFFF`).
//! Used by the end-to-end integrity trailer (I-RT12) and serial framing
//! (serial-mcu.md I-SR3). Bitwise implementation — no tables, no allocation.
//! REQ: REQ-ROUTE-033, REQ-ROUTE-027.

/// Streams `data` into the raw CRC state (pre-inverse, post-inverse caller).
/// REQ: REQ-ROUTE-033.
#[must_use]
pub fn crc32c_update(raw: u32, data: &[u8]) -> u32 {
    let mut crc = raw;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    crc
}

/// One-shot CRC32C over `data`.
/// REQ: REQ-ROUTE-033.
#[must_use]
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c_update(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::{crc32c, crc32c_update};

    /// REQ: REQ-ROUTE-033 — canonical check value for `123456789`.
    #[test]
    fn canonical_vector() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0x0000_0000);
        assert_eq!(crc32c(b"a"), 0xC1D0_4330);
    }

    /// REQ: REQ-ROUTE-033 — streaming equals one-shot (serial resync use).
    #[test]
    fn streaming_matches_oneshot() {
        let data = b"stream epoch seq payload bytes...";
        let one = crc32c(data);
        let mid = data.len() / 2;
        let streamed =
            crc32c_update(crc32c_update(0xFFFF_FFFF, &data[..mid]), &data[mid..]) ^ 0xFFFF_FFFF;
        assert_eq!(streamed, one);
    }
}
