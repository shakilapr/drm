//! Wire framing: fixed 28-byte little-endian `WireHeader`, record-bound
//! enforcement, and ingress stream ordering, verbatim from
//! `docs/algorithms/router/router-core.md` §Data Structures, §Functions
//! (`ingress` ordering/bound prelude), and §Invariants (I-RT7, I-RT9, I-RT18).
//! REQ: REQ-ROUTE-033 (wire header), REQ-ROUTE-038 (receive bound),
//! REQ-ROUTE-011 (ingress ordering).

use drm_types::Status;

pub use crate::crc32c::{crc32c, crc32c_update};

/// Wire protocol version written by the encoder and required by the decoder.
/// REQ: REQ-ROUTE-033.
pub const WIRE_VERSION: u8 = 1;

/// Fixed header size in bytes (I-RT7).
/// REQ: REQ-ROUTE-033.
pub const HEADER_LEN: usize = 28;

/// Flag bit 0: end-to-end CRC32C trailer present. All other bits must be zero.
/// REQ: REQ-ROUTE-033.
pub const FLAG_E2E_CRC: u8 = 1 << 0;

/// Wire header (I-RT7): little-endian
/// `{version: u8, flags: u8, reserved: u16 = 0, wire_channel_id: u32,
/// stream_epoch: u64, source_seq: u64, payload_len: u32}`. `version` and
/// `reserved` are protocol constants — encoded as [`WIRE_VERSION`] / zero and
/// validated on decode, so they are not struct fields.
/// REQ: REQ-ROUTE-033.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireHeader {
    /// Flag bits (`FLAG_E2E_CRC` set ⇒ trailer follows the payload).
    pub flags: u8,
    /// Transport-scoped channel id from the route binding — never the runtime
    /// topic handle (`wire_id_not_handle`).
    pub wire_channel_id: u32,
    /// Per-source stream epoch; increments on source/import restart; plain u64
    /// ordering is wrap-free.
    pub stream_epoch: u64,
    /// Per-source sequence (distinct from any per-link transport sequence).
    pub source_seq: u64,
    /// Payload byte length.
    pub payload_len: u32,
}

impl WireHeader {
    /// Encodes the header little-endian into `out[0..HEADER_LEN]`.
    /// REQ: REQ-ROUTE-033.
    pub fn encode(&self, out: &mut [u8]) {
        let buf = out
            .get_mut(..HEADER_LEN)
            .unwrap_or_else(|| unreachable!("caller supplies a HEADER_LEN buffer"));
        buf[0] = WIRE_VERSION;
        buf[1] = self.flags;
        buf[2] = 0;
        buf[3] = 0;
        buf[4..8].copy_from_slice(&self.wire_channel_id.to_le_bytes());
        buf[8..16].copy_from_slice(&self.stream_epoch.to_le_bytes());
        buf[16..24].copy_from_slice(&self.source_seq.to_le_bytes());
        buf[24..28].copy_from_slice(&self.payload_len.to_le_bytes());
    }

    /// Encodes into a fresh array.
    /// REQ: REQ-ROUTE-033.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        self.encode(&mut buf);
        buf
    }

    /// Decodes and validates: unknown version, unknown flag bits, or nonzero
    /// reserved bits are rejected before any payload access (I-RT7, I-RT14).
    /// REQ: REQ-ROUTE-033.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when fewer than [`HEADER_LEN`] bytes are
    /// available; [`Status::InvalidData`] on any header validation failure.
    pub fn decode(buf: &[u8]) -> Result<Self, Status> {
        if buf.len() < HEADER_LEN {
            return Err(Status::InvalidArg);
        }
        if buf[0] != WIRE_VERSION {
            return Err(Status::InvalidData);
        }
        if buf[1] & !FLAG_E2E_CRC != 0 {
            return Err(Status::InvalidData);
        }
        if buf[2] != 0 || buf[3] != 0 {
            return Err(Status::InvalidData);
        }
        Ok(Self {
            flags: buf[1],
            wire_channel_id: u32::from_le_bytes(
                buf[4..8]
                    .try_into()
                    .map_err(|_| unreachable!("sliced with bound check"))?,
            ),
            stream_epoch: u64::from_le_bytes(
                buf[8..16]
                    .try_into()
                    .map_err(|_| unreachable!("sliced with bound check"))?,
            ),
            source_seq: u64::from_le_bytes(
                buf[16..24]
                    .try_into()
                    .map_err(|_| unreachable!("sliced with bound check"))?,
            ),
            payload_len: u32::from_le_bytes(
                buf[24..28]
                    .try_into()
                    .map_err(|_| unreachable!("sliced with bound check"))?,
            ),
        })
    }

    /// On-wire frame length: header + payload, plus the 4-byte e2e trailer
    /// when flagged (datagram bound check, I-RT3).
    /// REQ: REQ-ROUTE-032.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN + self.payload_len as usize + usize::from(self.flags & FLAG_E2E_CRC != 0) * 4
    }
}

/// Receive-bound probe (I-RT18): `payload_len + header_len` widened to u64 is
/// compared against the generated per-route bound BEFORE any payload
/// buffering; equality is allowed.
/// REQ: REQ-ROUTE-038.
#[must_use]
pub fn record_bound_violated(payload_len: u32, max_record_bytes: u32) -> bool {
    u64::from(payload_len) + HEADER_LEN as u64 > u64::from(max_record_bytes)
}

/// Per-source stream position carried by every frame.
/// REQ: REQ-ROUTE-011.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamKey {
    /// Stream epoch.
    pub epoch: u64,
    /// Source sequence within the epoch.
    pub seq: u64,
}

/// Ingress ordering decision (I-RT9): `Duplicate`/`StaleEpoch` drop the frame
/// before any state mutation; `EpochReset` accepts AND bumps the imported
/// publisher epoch; `Accept` advances state.
/// REQ: REQ-ROUTE-011.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressDecision {
    /// Same epoch, `seq <= last`: reorder/duplicate ⇒ drop.
    Duplicate,
    /// Lower epoch than the newest seen ⇒ drop.
    StaleEpoch,
    /// Higher epoch: source/import restart ⇒ stream reset, then accept.
    EpochReset,
    /// In-order new sample.
    Accept,
}

/// Per `(link, wire_channel_id)` ordering state; reset on reconnect.
/// REQ: REQ-ROUTE-011.
#[derive(Debug, Clone, Default)]
pub struct IngressState {
    newest: Option<StreamKey>,
}

impl IngressState {
    /// Classifies an incoming key against the newest accepted one. Plain u64
    /// comparisons are wrap-free: `epoch 0` after `u64::MAX` is stale, never
    /// "newer" (`remote_restart_reset`, `epoch_max → 0`).
    /// REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn classify(&self, k: StreamKey) -> IngressDecision {
        match self.newest {
            None => IngressDecision::Accept,
            Some(old) if k.epoch == old.epoch => {
                if k.seq <= old.seq {
                    IngressDecision::Duplicate
                } else {
                    IngressDecision::Accept
                }
            }
            Some(old) if k.epoch > old.epoch => IngressDecision::EpochReset,
            Some(_) => IngressDecision::StaleEpoch,
        }
    }

    /// Advances ordering state with a validated key (once per frame, I-RT16).
    /// REQ: REQ-ROUTE-011.
    pub fn commit(&mut self, k: StreamKey) {
        self.newest = Some(k);
    }

    /// Newest accepted key, if any.
    /// REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn newest(&self) -> Option<StreamKey> {
        self.newest
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HEADER_LEN, IngressDecision, IngressState, StreamKey, WireHeader, record_bound_violated,
    };
    use drm_types::Status;

    fn hdr() -> WireHeader {
        WireHeader {
            flags: 0,
            wire_channel_id: 0xDEAD_BEEF,
            stream_epoch: 7,
            source_seq: 42,
            payload_len: 11,
        }
    }

    /// REQ: REQ-ROUTE-033 — 28-byte LE layout, byte-exact field placement.
    #[test]
    fn wire_header_layout() {
        let bytes = hdr().to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN);
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[1], 0);
        assert_eq!(bytes[2], 0);
        assert_eq!(bytes[3], 0);
        assert_eq!(&bytes[4..8], &0xDEAD_BEEF_u32.to_le_bytes());
        assert_eq!(&bytes[8..16], &7_u64.to_le_bytes());
        assert_eq!(&bytes[16..24], &42_u64.to_le_bytes());
        assert_eq!(&bytes[24..28], &11_u32.to_le_bytes());
        assert_eq!(WireHeader::decode(&bytes), Ok(hdr()));
    }

    /// REQ: REQ-ROUTE-033 — I-RT7 rejections: unknown version, unknown flag
    /// bits, nonzero reserved, short buffer.
    #[test]
    fn wire_header_rejects() {
        let mut b = hdr().to_bytes();
        b[0] = 2;
        assert_eq!(WireHeader::decode(&b), Err(Status::InvalidData));
        let mut b = hdr().to_bytes();
        b[1] = 0x02;
        assert_eq!(WireHeader::decode(&b), Err(Status::InvalidData));
        let mut b = hdr().to_bytes();
        b[3] = 1;
        assert_eq!(WireHeader::decode(&b), Err(Status::InvalidData));
        assert_eq!(WireHeader::decode(&b[..27]), Err(Status::InvalidArg));
        // e2e-CRC flag alone is legal and reflected in encoded_len.
        let mut b = hdr().to_bytes();
        b[1] = 1;
        let h = WireHeader::decode(&b).unwrap_or_else(|e| unreachable!("crc flag: {e:?}"));
        assert_eq!(h.encoded_len(), HEADER_LEN + 11 + 4);
        assert_eq!(hdr().encoded_len(), HEADER_LEN + 11);
    }

    /// REQ: REQ-ROUTE-033 — transport id is opaque on the wire: arbitrary u32
    /// values survive the round trip (`wire_id_not_handle`).
    #[test]
    fn wire_id_not_handle() {
        for id in [0u32, 1, 0xFFFF_FFFF, 0x1234_5678] {
            let mut h = hdr();
            h.wire_channel_id = id;
            assert_eq!(WireHeader::decode(&h.to_bytes()), Ok(h));
        }
    }

    /// REQ: REQ-ROUTE-038 — I-RT18 bound: equality allowed, above rejected,
    /// u64-widened arithmetic (no overflow at u32 extremes).
    #[test]
    fn record_bound() {
        assert!(!record_bound_violated(100, 128));
        assert!(record_bound_violated(101, 128));
        let hdr32 = u32::try_from(HEADER_LEN).unwrap_or(0);
        assert!(record_bound_violated(u32::MAX, u32::MAX));
        assert!(!record_bound_violated(u32::MAX - hdr32, u32::MAX));
    }

    /// REQ: REQ-ROUTE-011 — I-RT9 reorder and duplicate drop within an epoch.
    #[test]
    fn udp_reorder_duplicate() {
        let mut st = IngressState::default();
        assert_eq!(
            st.classify(StreamKey { epoch: 3, seq: 5 }),
            IngressDecision::Accept
        );
        st.commit(StreamKey { epoch: 3, seq: 5 });
        assert_eq!(
            st.classify(StreamKey { epoch: 3, seq: 3 }),
            IngressDecision::Duplicate
        );
        assert_eq!(
            st.classify(StreamKey { epoch: 3, seq: 5 }),
            IngressDecision::Duplicate
        );
        assert_eq!(
            st.classify(StreamKey { epoch: 3, seq: 6 }),
            IngressDecision::Accept
        );
    }

    /// REQ: REQ-ROUTE-011 — restart: higher epoch resets; lower epoch stale.
    #[test]
    fn remote_restart_reset() {
        let mut st = IngressState::default();
        st.commit(StreamKey { epoch: 9, seq: 100 });
        assert_eq!(
            st.classify(StreamKey { epoch: 10, seq: 0 }),
            IngressDecision::EpochReset
        );
        st.commit(StreamKey { epoch: 10, seq: 0 });
        assert_eq!(
            st.classify(StreamKey {
                epoch: 9,
                seq: u64::MAX
            }),
            IngressDecision::StaleEpoch
        );
    }

    /// REQ: REQ-ROUTE-011 — wrap-free ordering: epoch 0 after `u64::MAX` is
    /// stale, never mistaken for a restart (`epoch_max → 0`).
    #[test]
    fn epoch_max_then_zero_is_stale() {
        let mut st = IngressState::default();
        st.commit(StreamKey {
            epoch: u64::MAX,
            seq: 1,
        });
        assert_eq!(
            st.classify(StreamKey { epoch: 0, seq: 0 }),
            IngressDecision::StaleEpoch
        );
    }
}
