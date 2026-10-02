//! Datagram multiplex framing (work-plan M02W2-E3-T2): packs multiple small
//! records into one datagram (architecture §Decoupled Framing). Size guard:
//! the packed datagram never exceeds `max_datagram_payload`; oversize ⇒
//! error at encode time (the topic belongs on a stream transport).
//! REQ: REQ-ROUTE-032, REQ-ROUTE-038.

use drm_types::Status;

use crate::framing::record::{RECORD_PREFIX, decode_record, record_len};

/// Encodes a multiplex datagram: concatenated length-prefixed records.
/// Returns the encoded length; the datagram bound is enforced against
/// `max_payload` before any write (never exceeds; oversize ⇒ `InvalidArg`).
/// REQ: REQ-ROUTE-032.
///
/// # Errors
/// [`Status::InvalidArg`] when the packed size exceeds `max_payload`.
pub fn encode_multiplex(
    dst: &mut [u8],
    payloads: &[&[u8]],
    max_payload: usize,
) -> Result<usize, Status> {
    let total: usize = payloads.iter().map(|p| record_len(p.len())).sum();
    if total > max_payload {
        return Err(Status::InvalidArg);
    }
    let mut cursor = 0;
    for p in payloads {
        let n = crate::framing::record::encode_record(&mut dst[cursor..], p)?;
        cursor += n;
    }
    Ok(total)
}

/// One demultiplexed record.
/// REQ: REQ-ROUTE-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxRecord<'a> {
    /// Record payload.
    pub payload: &'a [u8],
}

/// Decodes every record in a multiplex datagram. Malformed or truncated
/// input is an error, never a panic; trailing bytes shorter than a record
/// prefix are rejected.
/// REQ: REQ-ROUTE-032, REQ-ROUTE-038.
///
/// # Errors
/// [`Status::InvalidArg`] on a truncated prefix; [`Status::InvalidData`] on
/// a truncated body.
pub fn decode_multiplex(src: &[u8]) -> Result<Vec<MuxRecord<'_>>, Status> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < src.len() {
        let (payload, next) = decode_record(src, pos)?;
        out.push(MuxRecord { payload });
        pos = next;
    }
    Ok(out)
}

/// Max records per datagram for reserve sizing (prefix per record).
/// REQ: REQ-ROUTE-032.
#[must_use]
pub fn max_records(max_payload: usize, max_record: usize) -> usize {
    if max_record <= RECORD_PREFIX {
        return 0;
    }
    max_payload / max_record
}

/// Packed-size check used at encode: `prefix + payload` per record summed.
/// REQ: REQ-ROUTE-032.
#[must_use]
pub fn packed_len(payloads: &[&[u8]]) -> usize {
    payloads.iter().map(|p| record_len(p.len())).sum()
}
