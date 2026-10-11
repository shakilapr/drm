//! Serial UART framing (`docs/algorithms/router/serial-mcu.md`): bounded
//! one-sample frames — one DRM sample per frame, never split (I-SR1) — COBS
//! escaped for a single `0x00` resync delimiter (I-SR4), carrying a mandatory
//! serial CRC32C over the decoded header, payload, and optional end-to-end
//! trailer (I-SR3). Serial is best-effort only (I-RT10). The decoded bound is
//! enforced at encode and refused at decode (I-SR2); steady-state encode and
//! decode allocate nothing (no per-message heap, REQ-CORE-087). Reuses the
//! existing [`crate::wire::WireHeader`] header type and [`crate::crc32c`].
//! REQ: REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026.

use drm_types::Status;

use crate::crc32c::{crc32c, crc32c_update};
use crate::wire::{FLAG_E2E_CRC, HEADER_LEN, WireHeader};

use super::Reliability;

/// Serial CRC32C trailer length in bytes, mandatory on every frame (I-SR3).
/// REQ: REQ-ROUTE-026.
pub const SERIAL_CRC_LEN: usize = 4;

/// End-to-end CRC32C trailer length in bytes when `FLAG_E2E_CRC` is set.
/// REQ: REQ-ROUTE-026.
const E2E_TRAILER_LEN: usize = 4;

/// Single resync byte terminating every COBS-encoded serial frame (I-SR4).
/// REQ: REQ-ROUTE-025.
pub const FRAME_DELIMITER: u8 = 0x00;

/// Per-link serial framing configuration (`serial-mcu.md` §Data Structures).
/// `max_frame_bytes` is the decoded frame bound — [`WireHeader`] plus payload
/// plus the optional end-to-end trailer — excluding the mandatory serial CRC
/// and COBS overhead.
/// REQ: REQ-ROUTE-024, REQ-ROUTE-025.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialLinkConfig {
    /// Line rate in bits/s; drives the I-SR7 blocking estimate.
    pub baud: u32,
    /// Decoded frame bound per link, in bytes (I-SR2).
    pub max_frame_bytes: u16,
}

impl SerialLinkConfig {
    /// Builds a link config; the bound must fit a header-only frame.
    /// REQ: REQ-ROUTE-025.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `max_frame_bytes` cannot hold one
    /// [`WireHeader`] plus the mandatory serial CRC.
    pub fn new(baud: u32, max_frame_bytes: u16) -> Result<Self, Status> {
        if usize::from(max_frame_bytes) < HEADER_LEN + SERIAL_CRC_LEN {
            return Err(Status::InvalidConfig);
        }
        Ok(Self {
            baud,
            max_frame_bytes,
        })
    }

    /// Decoded frame bound in bytes — header + payload + optional e2e trailer.
    /// REQ: REQ-ROUTE-025.
    #[must_use]
    pub fn max_decoded_len(&self) -> usize {
        usize::from(self.max_frame_bytes)
    }

    /// Largest decoded sequence handed to the COBS stage: the frame bound plus
    /// the mandatory serial CRC. Also the static buffer size `frame_decode`
    /// requires (REQ-CORE-087).
    /// REQ: REQ-ROUTE-026.
    #[must_use]
    pub fn max_coded_input_len(&self) -> usize {
        self.max_decoded_len() + SERIAL_CRC_LEN
    }

    /// Worst-case wire size — decoded bound + serial CRC + COBS overhead +
    /// delimiter — i.e. the buffer `frame_encode` may need (I-SR2).
    /// REQ: REQ-ROUTE-025.
    #[must_use]
    pub fn max_wire_bytes(&self) -> usize {
        cobs_max_encoded_len(self.max_coded_input_len()) + 1
    }

    /// Serial links are always best-effort; a `reliable` contract is rejected
    /// at verify (REQ-ROUTE-024).
    #[must_use]
    pub fn reliability(&self) -> Reliability {
        Reliability::BestEffort
    }

    /// Worst-case non-preemptible transmission blocking, in nanoseconds:
    /// `10 * max_wire_bytes / baud` (8N1). `None` when `baud` is zero (I-SR7).
    /// REQ: REQ-ROUTE-040.
    #[must_use]
    pub fn worst_case_blocking_ns(&self) -> Option<u64> {
        if self.baud == 0 {
            return None;
        }
        let bits = u128::try_from(self.max_wire_bytes())
            .ok()?
            .checked_mul(10)?;
        let ns = bits.checked_mul(1_000_000_000)? / u128::from(self.baud);
        u64::try_from(ns).ok()
    }
}

/// One serial frame: a [`WireHeader`], its payload bytes, and the optional
/// end-to-end CRC32C trailer carried when `FLAG_E2E_CRC` is set. On encode the
/// payload is caller-owned; on decode it borrows the caller's output buffer.
/// REQ: REQ-ROUTE-025, REQ-ROUTE-026.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialFrame<'a> {
    /// Fixed little-endian wire header.
    pub header: WireHeader,
    /// Precomputed end-to-end CRC32C trailer; `Some` iff `FLAG_E2E_CRC` is set.
    pub e2e_crc: Option<u32>,
    /// Payload bytes.
    pub payload: &'a [u8],
}

/// Decoded sequence length in bytes — header + payload + optional e2e trailer —
/// widened to `u64` so a hostile `payload_len` cannot overflow.
/// REQ: REQ-ROUTE-033.
#[must_use]
fn decoded_frame_len(header: &WireHeader) -> u64 {
    HEADER_LEN as u64
        + u64::from(header.payload_len)
        + if header.flags & FLAG_E2E_CRC != 0 {
            E2E_TRAILER_LEN as u64
        } else {
            0
        }
}

/// Upper bound on the COBS-encoded length of `input_len` bytes, excluding the
/// frame delimiter: `n + ceil(n/254) + 1`. Exact for `n = 0` and `n <= 254`;
/// a safe bound otherwise. REQ: REQ-ROUTE-025.
#[must_use]
pub fn cobs_max_encoded_len(input_len: usize) -> usize {
    input_len + input_len.div_ceil(254) + 1
}

/// COBS-encodes `input` into `out`, returning the encoded length (no
/// delimiter). `out` must be at least [`cobs_max_encoded_len`] bytes; a shorter
/// buffer is rejected before any write (I-SR4). REQ: REQ-ROUTE-025.
///
/// # Errors
/// [`Status::InvalidArg`] when `out` is smaller than the worst-case bound.
pub fn cobs_encode(input: &[u8], out: &mut [u8]) -> Result<usize, Status> {
    if out.len() < cobs_max_encoded_len(input.len()) {
        return Err(Status::InvalidArg);
    }
    let mut writer = CobsWriter::new(out)?;
    for &byte in input {
        writer.push(byte)?;
    }
    writer.finish()
}

/// COBS-decodes `input` (no delimiter) into `out`, returning the decoded
/// length. Malformed input — a zero byte or a code that overruns the buffer —
/// is rejected, never panicked (I-SR4). REQ: REQ-ROUTE-025.
///
/// # Errors
/// [`Status::InvalidData`] on a structural error; [`Status::InvalidArg`] when
/// `out` cannot hold the decoded bytes.
pub fn cobs_decode(input: &[u8], out: &mut [u8]) -> Result<usize, Status> {
    let mut read = 0usize;
    let mut write = 0usize;
    while read < input.len() {
        let code = usize::from(input[read]);
        if code == 0 {
            return Err(Status::InvalidData);
        }
        read += 1;
        let data_end = read.checked_add(code - 1).ok_or(Status::InvalidData)?;
        if data_end > input.len() {
            return Err(Status::InvalidData);
        }
        out.get_mut(write..write + code - 1)
            .ok_or(Status::InvalidArg)?
            .copy_from_slice(&input[read..data_end]);
        write += code - 1;
        read = data_end;
        if code != 0xFF && read < input.len() {
            *out.get_mut(write).ok_or(Status::InvalidArg)? = 0;
            write += 1;
        }
    }
    Ok(write)
}

/// Fixed-state COBS writer: appends bytes into caller-owned storage and
/// backfills each block's code byte in place — no allocation, O(1) state.
/// REQ: REQ-ROUTE-025.
struct CobsWriter<'o> {
    out: &'o mut [u8],
    len: usize,
    code_idx: usize,
    code: u8,
}

impl<'o> CobsWriter<'o> {
    /// Opens an empty first block; `out` must be nonempty. REQ: REQ-ROUTE-025.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `out` is empty.
    fn new(out: &'o mut [u8]) -> Result<Self, Status> {
        *out.get_mut(0).ok_or(Status::InvalidArg)? = 1;
        Ok(Self {
            out,
            len: 1,
            code_idx: 0,
            code: 1,
        })
    }

    /// Pushes one input byte, closing a block on a zero or a full 254-byte run.
    /// REQ: REQ-ROUTE-025.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `out` runs out of room (never with a buffer
    /// sized by [`cobs_max_encoded_len`]).
    fn push(&mut self, byte: u8) -> Result<(), Status> {
        if byte == 0 {
            self.out[self.code_idx] = self.code;
            self.code_idx = self.len;
            *self.out.get_mut(self.len).ok_or(Status::InvalidArg)? = 1;
            self.len += 1;
            self.code = 1;
        } else {
            *self.out.get_mut(self.len).ok_or(Status::InvalidArg)? = byte;
            self.len += 1;
            self.code += 1;
            if self.code == 0xFF {
                self.out[self.code_idx] = self.code;
                self.code_idx = self.len;
                *self.out.get_mut(self.len).ok_or(Status::InvalidArg)? = 1;
                self.len += 1;
                self.code = 1;
            }
        }
        Ok(())
    }

    /// Backfills the open block's code byte and returns the encoded length.
    /// REQ: REQ-ROUTE-025.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the reserved code byte is unreachable.
    fn finish(self) -> Result<usize, Status> {
        let Self {
            out,
            len,
            code_idx,
            code,
        } = self;
        *out.get_mut(code_idx).ok_or(Status::InvalidArg)? = code;
        Ok(len)
    }
}

/// Encodes one bounded serial frame — `COBS{header, payload, optional e2e
/// CRC32C, serial CRC32C} 0x00` — returning the wire length. The serial CRC32C
/// covers the decoded header, payload, and optional e2e trailer (I-SR3); the
/// decoded frame bound is enforced before any write (I-SR2); nothing is
/// allocated. REQ: REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026.
///
/// # Errors
/// [`Status::InvalidArg`] on an inconsistent frame (unknown flag bits, e2e
/// trailer present/absent mismatch, `payload_len` mismatch), an oversize
/// decoded frame, or an `out` shorter than the actual worst-case encoding.
pub fn frame_encode(
    cfg: &SerialLinkConfig,
    frame: &SerialFrame<'_>,
    out: &mut [u8],
) -> Result<usize, Status> {
    let header = &frame.header;
    let e2e_bytes = frame.e2e_crc.map(u32::to_le_bytes);
    if header.flags & !FLAG_E2E_CRC != 0
        || e2e_bytes.is_some() != (header.flags & FLAG_E2E_CRC != 0)
        || header.payload_len as usize != frame.payload.len()
    {
        return Err(Status::InvalidArg);
    }
    if decoded_frame_len(header) > u64::from(cfg.max_frame_bytes) {
        return Err(Status::InvalidArg); // I-SR2
    }
    let coded_len = usize::try_from(decoded_frame_len(header))
        .ok()
        .and_then(|n| n.checked_add(SERIAL_CRC_LEN))
        .ok_or(Status::InvalidArg)?;
    let need = cobs_max_encoded_len(coded_len)
        .checked_add(1)
        .ok_or(Status::InvalidArg)?;
    if out.len() < need {
        return Err(Status::InvalidArg); // before any write
    }
    // I-SR3 canonical coverage: header || payload || optional e2e trailer.
    let header_bytes = header.to_bytes();
    let mut acc = crc32c_update(0xFFFF_FFFF, &header_bytes);
    acc = crc32c_update(acc, frame.payload);
    if let Some(bytes) = &e2e_bytes {
        acc = crc32c_update(acc, bytes);
    }
    let crc_bytes = (acc ^ 0xFFFF_FFFF).to_le_bytes();
    let mut writer = CobsWriter::new(out)?;
    for &byte in &header_bytes {
        writer.push(byte)?;
    }
    for &byte in frame.payload {
        writer.push(byte)?;
    }
    if let Some(bytes) = &e2e_bytes {
        for &byte in bytes {
            writer.push(byte)?;
        }
    }
    for &byte in &crc_bytes {
        writer.push(byte)?;
    }
    let body_len = writer.finish()?;
    *out.get_mut(body_len).ok_or(Status::InvalidArg)? = FRAME_DELIMITER;
    Ok(body_len + 1)
}

/// Decodes exactly one delimited serial frame from `wire` into caller-owned
/// `out`, returning the header, optional e2e trailer, and a borrowed payload.
/// The serial CRC32C is verified over the decoded header + payload + optional
/// e2e trailer; malformed or CRC-failing input is rejected (I-SR3) and an
/// oversize decoded frame is refused (I-SR2). Nothing is allocated.
/// REQ: REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026.
///
/// `out` must hold [`SerialLinkConfig::max_coded_input_len`] bytes — the fixed
/// static buffer the streaming MCU decoder mirrors (REQ-CORE-087).
///
/// # Errors
/// [`Status::InvalidArg`] when `wire` is empty or `out` is too small;
/// [`Status::InvalidData`] when the frame is not delimiter-terminated, embeds a
/// delimiter, is malformed, exceeds the decoded bound, or fails the CRC.
pub fn frame_decode<'a>(
    cfg: &SerialLinkConfig,
    wire: &[u8],
    out: &'a mut [u8],
) -> Result<SerialFrame<'a>, Status> {
    if out.len() < cfg.max_coded_input_len() {
        return Err(Status::InvalidArg);
    }
    let (&delimiter, body) = wire.split_last().ok_or(Status::InvalidArg)?;
    if delimiter != FRAME_DELIMITER || body.contains(&FRAME_DELIMITER) {
        return Err(Status::InvalidData);
    }
    let decoded = cobs_decode(body, out)?;
    if decoded < HEADER_LEN + SERIAL_CRC_LEN {
        return Err(Status::InvalidData);
    }
    let header = WireHeader::decode(&out[..HEADER_LEN])?;
    if decoded_frame_len(&header) > u64::from(cfg.max_frame_bytes) {
        return Err(Status::InvalidData); // I-SR2: foreign oversize
    }
    let coded_len = usize::try_from(decoded_frame_len(&header))
        .ok()
        .and_then(|n| n.checked_add(SERIAL_CRC_LEN))
        .ok_or(Status::InvalidData)?;
    if decoded != coded_len {
        return Err(Status::InvalidData); // exact decoded length
    }
    let covered = &out[..coded_len - SERIAL_CRC_LEN];
    let stored = u32::from_le_bytes(
        out[coded_len - SERIAL_CRC_LEN..coded_len]
            .try_into()
            .map_err(|_| Status::InvalidData)?,
    );
    if crc32c(covered) != stored {
        return Err(Status::InvalidData); // I-SR3
    }
    let payload_len = header.payload_len as usize;
    let payload = out
        .get(HEADER_LEN..HEADER_LEN + payload_len)
        .ok_or(Status::InvalidData)?;
    let e2e_crc = if header.flags & FLAG_E2E_CRC != 0 {
        let start = HEADER_LEN + payload_len;
        Some(u32::from_le_bytes(
            out.get(start..start + E2E_TRAILER_LEN)
                .ok_or(Status::InvalidData)?
                .try_into()
                .map_err(|_| Status::InvalidData)?,
        ))
    } else {
        None
    };
    Ok(SerialFrame {
        header,
        e2e_crc,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        FLAG_E2E_CRC, FRAME_DELIMITER, SERIAL_CRC_LEN, SerialFrame, SerialLinkConfig, cobs_decode,
        cobs_encode, cobs_max_encoded_len, crc32c, frame_decode, frame_encode,
    };
    use crate::transport::Reliability;
    use crate::transport::registry::{TransportKind, TransportRegistry};
    use crate::wire::{HEADER_LEN, WireHeader};
    use drm_types::Status;

    /// Deterministic xorshift64 so the corruption/escape campaigns reproduce.
    struct Xorshift(u64);

    impl Xorshift {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn next_u32(&mut self) -> u32 {
            u32::try_from(self.next_u64() >> 32).unwrap_or(0)
        }

        fn next_u8(&mut self) -> u8 {
            u8::try_from(self.next_u64() & 0xFF).unwrap_or(0)
        }

        /// Uniform value in `0..bound` (`bound > 0`).
        fn below(&mut self, bound: usize) -> usize {
            usize::try_from(self.next_u64() % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
        }
    }

    fn link_config(baud: u32, max_frame_bytes: u16) -> SerialLinkConfig {
        SerialLinkConfig::new(baud, max_frame_bytes)
            .unwrap_or_else(|e| unreachable!("config: {e:?}"))
    }

    fn hdr(payload_len: u32) -> WireHeader {
        WireHeader {
            flags: 0,
            wire_channel_id: 0x1234,
            stream_epoch: 9,
            source_seq: 77,
            payload_len,
        }
    }

    fn out_buf(cfg: &SerialLinkConfig) -> Vec<u8> {
        vec![0u8; cfg.max_coded_input_len()]
    }

    fn cobs_vec(input: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; cobs_max_encoded_len(input.len())];
        let n = cobs_encode(input, &mut out).unwrap_or_else(|e| unreachable!("cobs: {e:?}"));
        out.truncate(n);
        out
    }

    fn assert_cobs_roundtrip(input: &[u8]) {
        let mut enc = vec![0u8; cobs_max_encoded_len(input.len())];
        let n = cobs_encode(input, &mut enc).unwrap_or_else(|e| unreachable!("cobs enc: {e:?}"));
        assert!(
            enc[..n].iter().all(|&b| b != FRAME_DELIMITER),
            "COBS output must be zero-free"
        );
        let mut dec = vec![0u8; input.len().max(1)];
        let m =
            cobs_decode(&enc[..n], &mut dec).unwrap_or_else(|e| unreachable!("cobs dec: {e:?}"));
        assert_eq!(m, input.len());
        assert_eq!(&dec[..m], input);
    }

    /// COBS-encodes `decoded` and appends the resync delimiter: one wire frame
    /// body for a decoder under test.
    fn wire_from_decoded(decoded: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; cobs_max_encoded_len(decoded.len())];
        let n = cobs_encode(decoded, &mut body).unwrap_or_else(|e| unreachable!("cobs: {e:?}"));
        body.truncate(n);
        body.push(FRAME_DELIMITER);
        body
    }

    /// Decodes `wire` under `cfg`, discarding the frame so only the status
    /// remains (the borrowed payload is tied to a local buffer).
    fn decode_status(cfg: &SerialLinkConfig, wire: &[u8]) -> Result<(), Status> {
        let mut out = out_buf(cfg);
        frame_decode(cfg, wire, &mut out).map(|_| ())
    }

    /// REQ: REQ-ROUTE-025, REQ-ROUTE-026 — one sample per bounded frame: a
    /// clean frame round-trips (header, payload, no trailer) and a
    /// `payload_len` mismatch is refused at encode.
    #[test]
    fn serial_frame_atomicity() {
        let cfg = link_config(115_200, 64);
        let header = hdr(5);
        let payload = [1u8, 2, 3, 4, 5];
        let frame = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        let n =
            frame_encode(&cfg, &frame, &mut wire).unwrap_or_else(|e| unreachable!("encode: {e:?}"));
        let mut out = out_buf(&cfg);
        let got = frame_decode(&cfg, &wire[..n], &mut out)
            .unwrap_or_else(|e| unreachable!("decode: {e:?}"));
        assert_eq!(got.header, header);
        assert_eq!(got.payload, &payload);
        assert_eq!(got.e2e_crc, None);

        let bad = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload[..3],
        };
        assert_eq!(frame_encode(&cfg, &bad, &mut wire), Err(Status::InvalidArg));
    }

    /// REQ: REQ-ROUTE-025 — I-SR2: a decoded frame over `max_frame_bytes` is
    /// refused at encode; a too-small `out` is refused before any write; a
    /// foreign oversize frame is refused at decode.
    #[test]
    fn serial_oversize() {
        let cfg = link_config(115_200, 32); // decoded bound: header(28) + crc(4) => payload <= 4
        let ok_payload = [9u8; 4];
        let good = SerialFrame {
            header: hdr(4),
            e2e_crc: None,
            payload: &ok_payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        assert!(frame_encode(&cfg, &good, &mut wire).is_ok());

        let over_payload = [9u8; 5];
        let over = SerialFrame {
            header: hdr(5),
            e2e_crc: None,
            payload: &over_payload,
        };
        assert_eq!(
            frame_encode(&cfg, &over, &mut wire),
            Err(Status::InvalidArg)
        );

        let mut tiny = [0xAAu8; 8];
        assert_eq!(
            frame_encode(&cfg, &good, &mut tiny),
            Err(Status::InvalidArg)
        );
        assert!(tiny.iter().all(|&b| b == 0xAA), "no partial write");

        // Encode a large frame, then decode it under a smaller bound: the
        // foreign oversize frame is refused.
        let big_cfg = link_config(115_200, 256);
        let big_payload = [0x5Au8; 100];
        let big = SerialFrame {
            header: hdr(100),
            e2e_crc: None,
            payload: &big_payload,
        };
        let mut big_wire = vec![0u8; big_cfg.max_wire_bytes()];
        let bn = frame_encode(&big_cfg, &big, &mut big_wire)
            .unwrap_or_else(|e| unreachable!("encode: {e:?}"));
        let mut big_out = out_buf(&big_cfg);
        assert_eq!(
            frame_decode(&cfg, &big_wire[..bn], &mut big_out),
            Err(Status::InvalidData)
        );
    }

    /// REQ: REQ-ROUTE-026 — I-SR3: every single-bit wire flip and every decoded
    /// burst of <=32 bits is rejected (CRC-guaranteed), including an e2e
    /// trailer bit flip; a randomized multi-bit wire campaign never panics.
    #[test]
    fn serial_corruption() {
        let cfg = link_config(115_200, 256);
        for e2e in [false, true] {
            check_corruption(&cfg, e2e);
        }
    }

    fn check_corruption(cfg: &SerialLinkConfig, e2e: bool) {
        let header = WireHeader {
            flags: if e2e { FLAG_E2E_CRC } else { 0 },
            wire_channel_id: 0x0102_0304,
            stream_epoch: 0x11,
            source_seq: 0x22,
            payload_len: 5,
        };
        let payload = [0x00u8, 0x7F, 0x00, 0xFF, 0x55];
        let frame = SerialFrame {
            header,
            e2e_crc: e2e.then_some(0xDEAD_BEEF),
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        let n = frame_encode(cfg, &frame, &mut wire).unwrap_or_else(|e| unreachable!("{e:?}"));
        let base = wire[..n].to_vec();

        for i in 0..base.len() {
            for bit in 0..8u8 {
                let mut w = base.clone();
                if let Some(b) = w.get_mut(i) {
                    *b ^= 1 << bit;
                }
                assert!(
                    frame_decode(cfg, &w, &mut out_buf(cfg)).is_err(),
                    "single-bit {i}/{bit} accepted (e2e={e2e})"
                );
            }
        }

        let decoded = canonical_decoded(&header, &payload, frame.e2e_crc);
        let total_bits = decoded.len() * 8;
        for len_bits in [1usize, 2, 3, 4, 8, 16, 32] {
            for start in 0..total_bits {
                assert!(
                    decoded_burst_rejects(cfg, &decoded, start, len_bits),
                    "burst start={start} len={len_bits} accepted (e2e={e2e})"
                );
            }
        }

        let mut rng = Xorshift(0x1234_5678_9ABC_DEF0);
        for _ in 0..2_000 {
            let mut w = base.clone();
            let flips = 1 + rng.below(40);
            for _ in 0..flips {
                let idx = rng.below(w.len());
                let shift = u32::try_from(rng.next_u64() % 8).unwrap_or(0);
                if let Some(b) = w.get_mut(idx) {
                    *b ^= 1u8 << shift;
                }
            }
            assert!(
                frame_decode(cfg, &w, &mut out_buf(cfg)).is_err(),
                "random wire campaign accepted corruption (e2e={e2e})"
            );
        }
    }

    /// Canonical decoded sequence `header || payload || [e2e] || serial CRC`.
    fn canonical_decoded(header: &WireHeader, payload: &[u8], e2e_crc: Option<u32>) -> Vec<u8> {
        let mut decoded = Vec::new();
        decoded.extend_from_slice(&header.to_bytes());
        decoded.extend_from_slice(payload);
        if let Some(e2e) = e2e_crc {
            decoded.extend_from_slice(&e2e.to_le_bytes());
        }
        let crc = crc32c(&decoded);
        decoded.extend_from_slice(&crc.to_le_bytes());
        decoded
    }

    /// Flips `len_bits` consecutive bits of the decoded sequence at `start`,
    /// re-encodes with the stale CRC, and asserts the decode rejects it.
    fn decoded_burst_rejects(
        cfg: &SerialLinkConfig,
        decoded: &[u8],
        start: usize,
        len_bits: usize,
    ) -> bool {
        let mut d = decoded.to_vec();
        for k in 0..len_bits {
            let bit = start + k;
            if let Some(b) = d.get_mut(bit / 8) {
                *b ^= 1u8 << u32::try_from(bit % 8).unwrap_or(0);
            }
        }
        let mut body = vec![0u8; cobs_max_encoded_len(d.len())];
        let blen = cobs_encode(&d, &mut body).unwrap_or_else(|e| unreachable!("cobs: {e:?}"));
        let mut wire = body[..blen].to_vec();
        wire.push(FRAME_DELIMITER);
        frame_decode(cfg, &wire, &mut out_buf(cfg)).is_err()
    }

    /// REQ: REQ-ROUTE-025, REQ-ROUTE-026 — I-SR4/I-SR6: COBS golden vectors,
    /// boundary lengths around the 254-byte block size, and full-frame
    /// round-trips straddling those boundaries.
    #[test]
    fn serial_cobs_boundary() {
        assert_eq!(cobs_vec(&[]), vec![0x01]);
        assert_eq!(cobs_vec(&[0x00]), vec![0x01, 0x01]);
        assert_eq!(cobs_vec(&[1, 2, 3]), vec![0x04, 1, 2, 3]);
        assert_eq!(cobs_vec(&[0x11, 0x00, 0x22]), vec![0x02, 0x11, 0x02, 0x22]);

        for &len in &[0usize, 1, 253, 254, 255, 507, 508, 509, 510] {
            let nonzero: Vec<u8> = (0..len)
                .map(|i| u8::try_from(i % 255 + 1).unwrap_or(1))
                .collect();
            let zeros = vec![0u8; len];
            let mixed: Vec<u8> = (0..len)
                .map(|i| {
                    if i % 3 == 0 {
                        0
                    } else {
                        u8::try_from((i * 7 + 1) % 256).unwrap_or(1)
                    }
                })
                .collect();
            assert_cobs_roundtrip(&nonzero);
            assert_cobs_roundtrip(&zeros);
            assert_cobs_roundtrip(&mixed);
            // Exact overhead for a zero-free run: one code byte per 254 bytes.
            assert_eq!(
                cobs_vec(&nonzero).len(),
                len + 1 + len / 254,
                "block boundary len {len}"
            );
        }

        let cfg = link_config(115_200, 1_400);
        for &plen in &[0usize, 1, 221, 222, 223, 475, 476, 477, 600] {
            let payload: Vec<u8> = (0..plen)
                .map(|i| u8::try_from((i * 5 + 1) % 256).unwrap_or(0))
                .collect();
            let header = WireHeader {
                flags: 0,
                wire_channel_id: 1,
                stream_epoch: 1,
                source_seq: u64::try_from(plen).unwrap_or(0),
                payload_len: u32::try_from(plen).unwrap_or(0),
            };
            let frame = SerialFrame {
                header,
                e2e_crc: None,
                payload: &payload,
            };
            let mut wire = vec![0u8; cfg.max_wire_bytes()];
            let n = frame_encode(&cfg, &frame, &mut wire)
                .unwrap_or_else(|e| unreachable!("encode: {e:?}"));
            let mut out = out_buf(&cfg);
            let got = frame_decode(&cfg, &wire[..n], &mut out)
                .unwrap_or_else(|e| unreachable!("decode: {e:?}"));
            assert_eq!(got.payload, payload.as_slice(), "boundary payload {plen}");
        }
    }

    /// REQ: REQ-ROUTE-025 — I-SR4: 10k deterministic iterations of arbitrary
    /// bytes (including zeros) survive COBS encode->decode and full-frame
    /// loopback byte-exact.
    #[test]
    fn serial_escape() {
        let cfg = link_config(115_200, 256);
        let mut rng = Xorshift(0x0BAD_F00D_CAFE_1234);
        for _ in 0..10_000 {
            let len = rng.below(601);
            let bytes: Vec<u8> = (0..len).map(|_| rng.next_u8()).collect();
            assert_cobs_roundtrip(&bytes);

            let plen = rng.below(129);
            let payload: Vec<u8> = (0..plen).map(|_| rng.next_u8()).collect();
            let use_e2e = rng.next_u64().is_multiple_of(2);
            let header = WireHeader {
                flags: if use_e2e { FLAG_E2E_CRC } else { 0 },
                wire_channel_id: rng.next_u32(),
                stream_epoch: rng.next_u64(),
                source_seq: rng.next_u64(),
                payload_len: u32::try_from(payload.len()).unwrap_or(0),
            };
            let e2e = use_e2e.then(|| rng.next_u32());
            let frame = SerialFrame {
                header,
                e2e_crc: e2e,
                payload: &payload,
            };
            let mut wire = vec![0u8; cfg.max_wire_bytes()];
            let n = frame_encode(&cfg, &frame, &mut wire)
                .unwrap_or_else(|e| unreachable!("encode: {e:?}"));
            let mut out = out_buf(&cfg);
            let got = frame_decode(&cfg, &wire[..n], &mut out)
                .unwrap_or_else(|e| unreachable!("decode: {e:?}"));
            assert_eq!(got.header, header);
            assert_eq!(got.payload, payload.as_slice());
            assert_eq!(got.e2e_crc, e2e);
        }
    }

    /// REQ: REQ-OPS-004, REQ-CORE-086 — I-SR6: the encoder's wire bytes equal a
    /// canonically assembled frame byte-for-byte (host/MCU interop), encoding is
    /// deterministic, and an independently configured decoder accepts it.
    #[test]
    fn mcu_host_interop() {
        let cfg = link_config(115_200, 256);
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 0xABCD_1234,
            stream_epoch: 5,
            source_seq: 6,
            payload_len: 3,
        };
        let payload = [0xDEu8, 0x00, 0xAD];
        let frame = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        let n =
            frame_encode(&cfg, &frame, &mut wire).unwrap_or_else(|e| unreachable!("encode: {e:?}"));

        let decoded = canonical_decoded(&header, &payload, None);
        let mut expected = vec![0u8; cobs_max_encoded_len(decoded.len()) + 1];
        let blen =
            cobs_encode(&decoded, &mut expected).unwrap_or_else(|e| unreachable!("cobs: {e:?}"));
        expected.truncate(blen);
        expected.push(FRAME_DELIMITER);
        assert_eq!(&wire[..n], expected.as_slice());

        let mut wire2 = vec![0u8; cfg.max_wire_bytes()];
        let n2 = frame_encode(&cfg, &frame, &mut wire2)
            .unwrap_or_else(|e| unreachable!("encode: {e:?}"));
        assert_eq!(&wire[..n], &wire2[..n2], "encode must be deterministic");

        let peer = link_config(9_600, 256);
        let mut out = out_buf(&peer);
        let got = frame_decode(&peer, &wire[..n], &mut out)
            .unwrap_or_else(|e| unreachable!("decode: {e:?}"));
        assert_eq!(got.header, header);
        assert_eq!(got.payload, &payload);
    }

    /// REQ: REQ-ROUTE-040 — I-SR7: worst-case blocking is `10 * wire / baud`
    /// nanoseconds, scales inversely with baud, and is undefined at baud 0.
    #[test]
    fn serial_blocking_estimate() {
        let fast = link_config(115_200, 64);
        let wire = fast.max_wire_bytes();
        let expected =
            u64::try_from(u128::try_from(wire).unwrap_or(0) * 10 * 1_000_000_000 / 115_200)
                .unwrap_or(0);
        assert_eq!(fast.worst_case_blocking_ns(), Some(expected));
        assert!(
            link_config(9_600, 64).worst_case_blocking_ns().unwrap_or(0) > expected,
            "lower baud blocks longer"
        );
        assert_eq!(link_config(0, 64).worst_case_blocking_ns(), None);
        assert_eq!(SERIAL_CRC_LEN, 4);
        assert_eq!(HEADER_LEN, 28);
    }

    /// REQ: REQ-ROUTE-025 — I-SR2: a truncated frame (no trailing delimiter)
    /// and a decoded sequence shorter than the header plus serial CRC are
    /// rejected with an explicit error, never a panic.
    #[test]
    fn serial_partial_frame_rejected() {
        let cfg = link_config(115_200, 64);
        let header = hdr(4);
        let payload = [1u8, 2, 3, 4];
        let frame = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        let n =
            frame_encode(&cfg, &frame, &mut wire).unwrap_or_else(|e| unreachable!("encode: {e:?}"));
        assert!(n > 0, "frame encoded");

        // Missing trailing delimiter: the last body byte is not `0x00`.
        assert_eq!(
            decode_status(&cfg, &wire[..n - 1]),
            Err(Status::InvalidData)
        );
        // Empty wire carries no delimiter at all.
        assert_eq!(decode_status(&cfg, &[]), Err(Status::InvalidArg));

        // Short declared length: one byte under header + serial CRC.
        let short = [0xAAu8; HEADER_LEN + SERIAL_CRC_LEN - 1];
        assert_eq!(
            decode_status(&cfg, &wire_from_decoded(&short)),
            Err(Status::InvalidData)
        );
    }

    /// REQ: REQ-ROUTE-026 — I-SR3: an otherwise well-formed frame whose serial
    /// CRC does not match the covered bytes is rejected with the explicit CRC
    /// error status — both a corrupted stored trailer and a corrupted covered
    /// byte against a stale trailer.
    #[test]
    fn serial_crc_mismatch_rejected() {
        let cfg = link_config(115_200, 64);
        let header = hdr(4);
        let payload = [0x11u8, 0x22, 0x33, 0x44];
        let good = canonical_decoded(&header, &payload, None);
        let mut out = out_buf(&cfg);
        assert!(
            frame_decode(&cfg, &wire_from_decoded(&good), &mut out).is_ok(),
            "baseline frame decodes clean"
        );

        // Stored CRC byte corrupted: covered bytes unchanged, trailer wrong.
        let mut bad_crc = good.clone();
        let last = bad_crc.len() - 1;
        bad_crc[last] ^= 0x01;
        assert_eq!(
            decode_status(&cfg, &wire_from_decoded(&bad_crc)),
            Err(Status::InvalidData)
        );

        // Covered payload byte corrupted with a stale CRC: length still exact.
        let mut bad_payload = good.clone();
        bad_payload[HEADER_LEN] ^= 0x01;
        assert_eq!(
            decode_status(&cfg, &wire_from_decoded(&bad_payload)),
            Err(Status::InvalidData)
        );
    }

    /// REQ: REQ-ROUTE-025 — I-SR4: malformed COBS — a zero code byte outside
    /// the delimiter and a block claiming more bytes than are present — is
    /// rejected by `cobs_decode` and by the frame decoder.
    #[test]
    fn serial_malformed_cobs_rejected() {
        // A zero code byte is structurally impossible (delimiter-only).
        assert_eq!(
            cobs_decode(&[0x00], &mut [0u8; 8]),
            Err(Status::InvalidData)
        );
        // Block code `0x05` promises four data bytes but only two follow.
        assert_eq!(
            cobs_decode(&[0x05, 0x01, 0x02], &mut [0u8; 8]),
            Err(Status::InvalidData)
        );

        let cfg = link_config(115_200, 64);
        let header = hdr(4);
        let payload = [1u8, 2, 3, 4];
        let frame = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        let n =
            frame_encode(&cfg, &frame, &mut wire).unwrap_or_else(|e| unreachable!("encode: {e:?}"));

        // Embedded `0x00` inside the body, before the trailing delimiter.
        let mut embedded = wire[..n].to_vec();
        embedded.insert(2, 0x00);
        assert_eq!(decode_status(&cfg, &embedded), Err(Status::InvalidData));

        // Over-long block at the frame level: the code overruns the body.
        let over_long = [0x05u8, 0x01, 0x02, FRAME_DELIMITER];
        assert_eq!(decode_status(&cfg, &over_long), Err(Status::InvalidData));
    }

    /// REQ: REQ-ROUTE-025 — I-SR2: a declared decoded length beyond
    /// `max_frame_bytes` is refused at encode (before any write) and at decode
    /// (before the payload is addressed), so no bound-sized allocation occurs.
    #[test]
    fn serial_oversize_declared_rejected() {
        let cfg = link_config(115_200, 64); // decoded bound 64 ⇒ payload ≤ 36
        let payload = [0x5Au8; 37]; // one byte over the payload bound
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 7,
            stream_epoch: 1,
            source_seq: 2,
            payload_len: 37,
        };
        let frame = SerialFrame {
            header,
            e2e_crc: None,
            payload: &payload,
        };
        let mut wire = vec![0u8; cfg.max_wire_bytes()];
        assert_eq!(
            frame_encode(&cfg, &frame, &mut wire),
            Err(Status::InvalidArg)
        );

        // Decode side: a header declaring a length far beyond the bound is
        // refused with the fixed `max_coded_input_len` output buffer.
        let foreign = hdr(65_535); // header (28) + 65535 payload >> 64
        let mut decoded = foreign.to_bytes().to_vec();
        decoded.extend_from_slice(&[0u8; SERIAL_CRC_LEN]);
        let mut out = out_buf(&cfg);
        assert_eq!(
            frame_decode(&cfg, &wire_from_decoded(&decoded), &mut out),
            Err(Status::InvalidData)
        );
    }

    /// REQ: REQ-ROUTE-024, REQ-ROUTE-034 — serial links are best-effort: the
    /// per-link config reports `Reliability::BestEffort` regardless of baud or
    /// decoded bound, and the authoritative registry row for
    /// `TransportKind::Serial` maps to the same best-effort class.
    #[test]
    fn serial_reliability_best_effort() {
        assert_eq!(
            link_config(115_200, 256).reliability(),
            Reliability::BestEffort
        );
        assert_eq!(
            link_config(9_600, 64).reliability(),
            Reliability::BestEffort
        );

        let registry = TransportRegistry::from_defaults();
        let row = registry
            .lookup(TransportKind::Serial)
            .unwrap_or_else(|e| unreachable!("registry: {e:?}"));
        assert_eq!(row.reliability(), Reliability::BestEffort);
        let selection = registry
            .select(TransportKind::Serial)
            .unwrap_or_else(|e| unreachable!("select: {e:?}"));
        assert_eq!(selection.reliability(), Reliability::BestEffort);
    }

    /// REQ: REQ-ROUTE-025, REQ-ROUTE-026, REQ-OPS-004 — I-SR1/I-SR4/I-SR6: a
    /// virtual pair (Unix socketpair) carries 10 000 encoded frames; the reader
    /// reassembles each frame by its `0x00` delimiter and decodes it byte-exact,
    /// proving the wire format survives a real streaming link.
    #[cfg(unix)]
    #[test]
    fn serial_loopback_x10k() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let (mut tx, mut rx) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(e) => unreachable!("socketpair: {e:?}"),
        };
        if let Err(e) = rx.set_read_timeout(Some(Duration::from_secs(30))) {
            unreachable!("set read timeout: {e:?}");
        }
        let cfg = link_config(115_200, 256);
        let cfg_tx = cfg.clone();

        let writer = std::thread::spawn(move || {
            for i in 0..10_000_u32 {
                let plen = usize::try_from(i % 33).unwrap_or(0);
                let base = usize::try_from(i).unwrap_or(0);
                let payload: Vec<u8> = (0..plen)
                    .map(|k| u8::try_from((base + k) % 256).unwrap_or(0))
                    .collect();
                let frame = SerialFrame {
                    header: WireHeader {
                        flags: 0,
                        wire_channel_id: i,
                        stream_epoch: 3,
                        source_seq: u64::from(i),
                        payload_len: u32::try_from(payload.len()).unwrap_or(0),
                    },
                    e2e_crc: None,
                    payload: &payload,
                };
                let mut wire = vec![0u8; cfg_tx.max_wire_bytes()];
                let n = match frame_encode(&cfg_tx, &frame, &mut wire) {
                    Ok(n) => n,
                    Err(e) => unreachable!("encode {i}: {e:?}"),
                };
                if let Err(e) = tx.write_all(&wire[..n]) {
                    unreachable!("write {i}: {e:?}");
                }
            }
        });

        let mut out = out_buf(&cfg);
        let mut acc: Vec<u8> = Vec::new();
        let mut byte = [0_u8; 1];
        for i in 0..10_000_u32 {
            acc.clear();
            loop {
                if let Err(e) = rx.read_exact(&mut byte) {
                    unreachable!("read {i}: {e:?}");
                }
                acc.push(byte[0]);
                if byte[0] == FRAME_DELIMITER {
                    break;
                }
            }
            let got = match frame_decode(&cfg, &acc, &mut out) {
                Ok(f) => f,
                Err(e) => unreachable!("decode {i}: {e:?}"),
            };
            assert_eq!(got.header.source_seq, u64::from(i), "frame {i} order");
            assert_eq!(got.header.payload_len, i % 33, "frame {i} length");
        }
        writer
            .join()
            .unwrap_or_else(|_| unreachable!("writer join"));
    }
}
