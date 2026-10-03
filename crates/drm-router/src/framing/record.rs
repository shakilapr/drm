//! Record framing (work-plan M02W2-E3-T1): length-prefixed records over
//! stream transports — one sample = one record. Bounds-checked decode;
//! truncation and malformed lengths are errors, never panics (I-RT18 class).
//! REQ: REQ-ROUTE-038, REQ-ROUTE-033.

use drm_types::Status;

/// Record length prefix size (`u32` LE byte count of the payload).
/// REQ: REQ-ROUTE-038.
pub const RECORD_PREFIX: usize = 4;

/// Total on-wire length of one record (prefix + payload).
/// REQ: REQ-ROUTE-038.
#[must_use]
pub fn record_len(payload_len: usize) -> usize {
    RECORD_PREFIX + payload_len
}

/// Encodes one record: `u32` LE payload length + payload bytes. Bounds are
/// checked before any destination write (I-RT18 class: no partial frame).
/// REQ: REQ-ROUTE-038.
///
/// # Errors
/// [`Status::InvalidArg`] when the payload exceeds `u32::MAX` or `dst` is
/// shorter than the total record length.
pub fn encode_record(dst: &mut [u8], payload: &[u8]) -> Result<usize, Status> {
    let plen = u32::try_from(payload.len()).map_err(|_| Status::InvalidArg)?;
    let total = record_len(payload.len());
    if dst.len() < total {
        return Err(Status::InvalidArg);
    }
    dst[..RECORD_PREFIX].copy_from_slice(&plen.to_le_bytes());
    let body = dst
        .get_mut(RECORD_PREFIX..total)
        .ok_or(Status::InvalidArg)?;
    body.copy_from_slice(payload);
    Ok(total)
}

/// Decodes one record starting at `pos`: returns the payload slice and the
/// next record offset. Malformed or truncated input is an error, never a
/// panic; a declared length that cannot fit is rejected before any use.
/// REQ: REQ-ROUTE-038.
///
/// # Errors
/// [`Status::InvalidArg`] when fewer than [`RECORD_PREFIX`] bytes remain
/// (truncated prefix); [`Status::InvalidData`] when the declared body
/// overruns the buffer (truncated record) or exceeds `u32::MAX`.
pub fn decode_record(src: &[u8], pos: usize) -> Result<(&[u8], usize), Status> {
    if src.len() < pos || pos.checked_add(RECORD_PREFIX).is_none_or(|e| e > src.len()) {
        return Err(Status::InvalidArg);
    }
    let plen = u32::from_le_bytes(
        src[pos..pos + RECORD_PREFIX]
            .try_into()
            .unwrap_or_else(|_| unreachable!("sliced with bound check")),
    );
    let body_len = usize::try_from(plen).map_err(|_| Status::InvalidData)?;
    let end = pos
        .checked_add(RECORD_PREFIX)
        .and_then(|p| p.checked_add(body_len))
        .ok_or(Status::InvalidData)?;
    if end > src.len() {
        return Err(Status::InvalidData);
    }
    let payload = src
        .get(pos + RECORD_PREFIX..end)
        .ok_or(Status::InvalidData)?;
    Ok((payload, end))
}
