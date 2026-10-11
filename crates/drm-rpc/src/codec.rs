//! Bounded wire codec for the correlated RPC topics
//! (`docs/algorithms/ops/rpc-actions.md` §Data Structures). Every frame is
//! little-endian with an explicit tag; decoding rejects truncation, unknown
//! tags, and over-length payloads with [`Status::InvalidData`]. The core
//! channel `sample_max` bounds each frame, so encode performs no dynamic
//! serialization (`I-RPC4`, `I-RPC6`).
//! REQ: REQ-OPS-012, REQ-OPS-020, REQ-OPS-023, REQ-OPS-025.

use drm_types::Status;

use crate::{CorrelationId, EventKind, Outcome};

/// `request/<client_id>` frame tag.
/// REQ: REQ-OPS-012.
pub const REQUEST_TAG: u8 = 1;
/// `cancel/<client_id>` frame tag.
/// REQ: REQ-OPS-012.
pub const CANCEL_TAG: u8 = 2;
/// `event/<client_id>` frame tag.
/// REQ: REQ-OPS-012.
pub const EVENT_TAG: u8 = 3;

/// Request frame header size in bytes (`rpc-actions.md` §Data Structures
/// `Request`).
/// REQ: REQ-OPS-012, REQ-OPS-023.
pub const REQUEST_HEADER: usize = 50;
/// Cancel frame size in bytes (`rpc-actions.md` §Data Structures `Cancel`).
/// REQ: REQ-OPS-012.
pub const CANCEL_FRAME: usize = 21;
/// Event frame header size in bytes (`rpc-actions.md` §Data Structures
/// `RpcEvent`).
/// REQ: REQ-OPS-025.
pub const EVENT_HEADER: usize = 47;

/// Client request (`rpc-actions.md` §Data Structures `Request`).
/// `target_server_session == 0` means unknown / first contact (`I-RPC9`);
/// `remaining_budget_ns` is relative and never an absolute clock reading
/// (`I-RPC3`).
/// REQ: REQ-OPS-012, REQ-OPS-020, REQ-OPS-023.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Correlation identity of the call.
    pub correlation_id: CorrelationId,
    /// Observed server session, or `0` for first contact.
    pub target_server_session: u128,
    /// Relative remaining budget in nanoseconds (`None` = no deadline).
    pub remaining_budget_ns: Option<u64>,
    /// Opaque request payload.
    pub payload: Vec<u8>,
}

/// Client cancellation (`rpc-actions.md` §Data Structures `Cancel`).
/// REQ: REQ-OPS-011, REQ-OPS-012.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancel {
    /// Correlation identity being cancelled.
    pub correlation_id: CorrelationId,
}

/// Server event (`rpc-actions.md` §Data Structures `RpcEvent`). `server_session`
/// is meaningful on [`EventKind::Accepted`]; `status`/`payload` are meaningful
/// on [`EventKind::Terminal`].
/// REQ: REQ-OPS-020, REQ-OPS-022, REQ-OPS-025.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcEvent {
    /// Correlation identity echoed verbatim.
    pub correlation_id: CorrelationId,
    /// Event discriminator.
    pub kind: EventKind,
    /// Server session echo (`Accepted`).
    pub server_session: u128,
    /// Terminal status (`Terminal`).
    pub status: Status,
    /// Event payload (terminal result bytes).
    pub payload: Vec<u8>,
}

impl RpcEvent {
    /// Whether this event is advisory progress (`I-RPC7`).
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub const fn is_progress(&self) -> bool {
        matches!(self.kind, EventKind::Progress)
    }

    /// Whether this event is terminal (`I-RPC7`).
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self.kind, EventKind::Terminal(_))
    }
}

/// Encodes `event_kind` to its stable tag byte.
/// REQ: REQ-OPS-025.
#[must_use]
const fn kind_byte(kind: EventKind) -> u8 {
    match kind {
        EventKind::Accepted => 0,
        EventKind::Started => 1,
        EventKind::Progress => 2,
        EventKind::CancelAccepted => 3,
        EventKind::CancelRejected => 4,
        EventKind::Terminal(_) => 5,
    }
}

/// Decodes an event tag byte plus terminal outcome.
/// REQ: REQ-OPS-025.
fn kind_from(tag: u8, outcome: u8) -> Result<EventKind, Status> {
    match tag {
        0 => Ok(EventKind::Accepted),
        1 => Ok(EventKind::Started),
        2 => Ok(EventKind::Progress),
        3 => Ok(EventKind::CancelAccepted),
        4 => Ok(EventKind::CancelRejected),
        5 => Ok(EventKind::Terminal(match outcome {
            0 => Outcome::Succeeded,
            1 => Outcome::Aborted,
            2 => Outcome::Cancelled,
            _ => return Err(Status::InvalidData),
        })),
        _ => Err(Status::InvalidData),
    }
}

/// Terminal outcome tag byte.
/// REQ: REQ-OPS-025.
#[must_use]
const fn outcome_byte(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Succeeded => 0,
        Outcome::Aborted => 1,
        Outcome::Cancelled => 2,
    }
}

/// Bounds-checked little-endian writer over a fixed frame buffer.
struct Sink<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Sink<'_> {
    fn new(buf: &mut [u8]) -> Sink<'_> {
        Sink { buf, pos: 0 }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Status> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .ok_or(Status::InvalidArg)?;
        let dst = self.buf.get_mut(self.pos..end).ok_or(Status::InvalidArg)?;
        dst.copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    fn u8(&mut self, v: u8) -> Result<(), Status> {
        self.put(&v.to_le_bytes())
    }

    fn u32(&mut self, v: u32) -> Result<(), Status> {
        self.put(&v.to_le_bytes())
    }

    fn u64(&mut self, v: u64) -> Result<(), Status> {
        self.put(&v.to_le_bytes())
    }

    fn u128(&mut self, v: u128) -> Result<(), Status> {
        self.put(&v.to_le_bytes())
    }

    fn len(&self) -> usize {
        self.pos
    }
}

/// Bounds-checked little-endian reader over a received frame.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Cursor<'a> {
        Cursor { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Status> {
        let end = self.pos.checked_add(n).ok_or(Status::InvalidData)?;
        let slice = self.buf.get(self.pos..end).ok_or(Status::InvalidData)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, Status> {
        let b = self.take(1)?;
        Ok(b[0])
    }

    fn u32(&mut self) -> Result<u32, Status> {
        let b: [u8; 4] = self.take(4)?.try_into().map_err(|_| Status::InvalidData)?;
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64, Status> {
        let b: [u8; 8] = self.take(8)?.try_into().map_err(|_| Status::InvalidData)?;
        Ok(u64::from_le_bytes(b))
    }

    fn u128(&mut self) -> Result<u128, Status> {
        let b: [u8; 16] = self.take(16)?.try_into().map_err(|_| Status::InvalidData)?;
        Ok(u128::from_le_bytes(b))
    }
}

/// Encodes a [`Request`] into `out`, returning the frame length.
/// REQ: REQ-OPS-012, REQ-OPS-023.
///
/// # Errors
/// [`Status::InvalidArg`] when `out` cannot hold the frame.
pub fn encode_request(req: &Request, out: &mut [u8]) -> Result<usize, Status> {
    let payload_len = u32::try_from(req.payload.len()).map_err(|_| Status::InvalidArg)?;
    let mut sink = Sink::new(out);
    sink.u8(REQUEST_TAG)?;
    sink.u32(req.correlation_id.client_id)?;
    sink.u64(req.correlation_id.session_nonce)?;
    sink.u64(req.correlation_id.counter)?;
    sink.u128(req.target_server_session)?;
    if let Some(budget) = req.remaining_budget_ns {
        sink.u8(1)?;
        sink.u64(budget)?;
    } else {
        sink.u8(0)?;
        sink.u64(0)?;
    }
    sink.u32(payload_len)?;
    sink.put(&req.payload)?;
    Ok(sink.len())
}

/// Decodes a [`Request`] frame.
/// REQ: REQ-OPS-012, REQ-OPS-023.
///
/// # Errors
/// [`Status::InvalidData`] for a truncated, mistagged, or trailing-byte frame.
pub fn decode_request(buf: &[u8]) -> Result<Request, Status> {
    let mut cur = Cursor::new(buf);
    if cur.u8()? != REQUEST_TAG {
        return Err(Status::InvalidData);
    }
    let client_id = cur.u32()?;
    let session_nonce = cur.u64()?;
    let counter = cur.u64()?;
    let target_server_session = cur.u128()?;
    let has_budget = cur.u8()?;
    let budget = cur.u64()?;
    let payload_len = usize::try_from(cur.u32()?).map_err(|_| Status::InvalidData)?;
    let payload = cur.take(payload_len)?.to_vec();
    Ok(Request {
        correlation_id: CorrelationId {
            client_id,
            session_nonce,
            counter,
        },
        target_server_session,
        remaining_budget_ns: if has_budget == 0 { None } else { Some(budget) },
        payload,
    })
}

/// Encodes a [`Cancel`] into `out`, returning the frame length.
/// REQ: REQ-OPS-011, REQ-OPS-012.
///
/// # Errors
/// [`Status::InvalidArg`] when `out` is too small.
pub fn encode_cancel(cancel: &Cancel, out: &mut [u8]) -> Result<usize, Status> {
    let mut sink = Sink::new(out);
    sink.u8(CANCEL_TAG)?;
    sink.u32(cancel.correlation_id.client_id)?;
    sink.u64(cancel.correlation_id.session_nonce)?;
    sink.u64(cancel.correlation_id.counter)?;
    Ok(sink.len())
}

/// Decodes a [`Cancel`] frame.
/// REQ: REQ-OPS-011, REQ-OPS-012.
///
/// # Errors
/// [`Status::InvalidData`] for a truncated, mistagged, or trailing-byte frame.
pub fn decode_cancel(buf: &[u8]) -> Result<Cancel, Status> {
    let mut cur = Cursor::new(buf);
    if cur.u8()? != CANCEL_TAG {
        return Err(Status::InvalidData);
    }
    let client_id = cur.u32()?;
    let session_nonce = cur.u64()?;
    let counter = cur.u64()?;
    Ok(Cancel {
        correlation_id: CorrelationId {
            client_id,
            session_nonce,
            counter,
        },
    })
}

/// Encodes an [`RpcEvent`] into `out`, returning the frame length.
/// REQ: REQ-OPS-020, REQ-OPS-022, REQ-OPS-025.
///
/// # Errors
/// [`Status::InvalidArg`] when `out` cannot hold the frame.
pub fn encode_event(ev: &RpcEvent, out: &mut [u8]) -> Result<usize, Status> {
    let payload_len = u32::try_from(ev.payload.len()).map_err(|_| Status::InvalidArg)?;
    let mut sink = Sink::new(out);
    sink.u8(EVENT_TAG)?;
    sink.u32(ev.correlation_id.client_id)?;
    sink.u64(ev.correlation_id.session_nonce)?;
    sink.u64(ev.correlation_id.counter)?;
    sink.u8(kind_byte(ev.kind))?;
    sink.u128(ev.server_session)?;
    sink.u32(u32::from_le_bytes(ev.status.code().to_le_bytes()))?;
    sink.u8(match ev.kind {
        EventKind::Terminal(o) => outcome_byte(o),
        _ => 0,
    })?;
    sink.u32(payload_len)?;
    sink.put(&ev.payload)?;
    Ok(sink.len())
}

/// Decodes an [`RpcEvent`] frame.
/// REQ: REQ-OPS-020, REQ-OPS-022, REQ-OPS-025.
///
/// # Errors
/// [`Status::InvalidData`] for a truncated, mistagged, or trailing-byte frame.
pub fn decode_event(buf: &[u8]) -> Result<RpcEvent, Status> {
    let mut cur = Cursor::new(buf);
    if cur.u8()? != EVENT_TAG {
        return Err(Status::InvalidData);
    }
    let client_id = cur.u32()?;
    let session_nonce = cur.u64()?;
    let counter = cur.u64()?;
    let kind_tag = cur.u8()?;
    let server_session = cur.u128()?;
    let status = Status::from_code(i32::from_le_bytes(cur.u32()?.to_le_bytes()))
        .ok_or(Status::InvalidData)?;
    let outcome = cur.u8()?;
    let kind = kind_from(kind_tag, outcome)?;
    let payload_len = usize::try_from(cur.u32()?).map_err(|_| Status::InvalidData)?;
    let payload = cur.take(payload_len)?.to_vec();
    Ok(RpcEvent {
        correlation_id: CorrelationId {
            client_id,
            session_nonce,
            counter,
        },
        kind,
        server_session,
        status,
        payload,
    })
}
