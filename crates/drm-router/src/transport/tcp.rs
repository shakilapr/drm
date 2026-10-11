//! TCP stream adapter (router-core.md §Protocols *Session lifecycle (stream
//! transports)*, I-RT5; `WireFrame::Record`): one sample is sent as one
//! length-prefixed record (reusing [`crate::framing::record`]) and received by
//! reading exactly the declared length. A bounded send loops over short writes
//! and `WouldBlock` until the whole record is on the wire, so it never leaves a
//! half-written record. Length-framed decode is bounds-checked (I-RT18): a
//! malformed or oversized declared length is an explicit error, never a panic
//! and never an unbounded allocation.
//!
//! Reconnect and session-scoped reliability (I-RT5) live in
//! [`crate::transport::session`]: a read error or clean close ends the session
//! ([`TcpTransport::is_session_end`]), and a fresh session is re-established by
//! a bounded, deterministic backoff (`session::redial`) — reliable delivery
//! never survives the restart.
//! REQ: REQ-ROUTE-029, REQ-ROUTE-038.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};

use drm_types::Status;

use crate::framing::record::{RECORD_PREFIX, decode_record, encode_record};
use crate::transport::{Reliability, TransportAdapter};
use crate::wire::{HEADER_LEN, WireHeader};

/// A non-blocking TCP endpoint carrying one length-framed record per sample.
/// REQ: REQ-ROUTE-029.
#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
    max_record: usize,
    /// Encoded frame scratch (header + payload), sized at activation.
    frame: Vec<u8>,
    /// Wire scratch (record prefix + frame), sized at activation.
    wire: Vec<u8>,
    /// Receive accumulator (record prefix + max frame), sized at activation.
    read_buf: Vec<u8>,
    /// Bytes currently accumulated in `read_buf`.
    filled: usize,
}

impl TcpTransport {
    /// Connects a non-blocking stream; `max_record` is the generated per-route
    /// receive/charge bound (header + declared payload bound + optional e2e
    /// CRC, I-RT18). REQ: REQ-ROUTE-029, REQ-ROUTE-038.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `max_record` is smaller than the wire
    /// header; otherwise the socket IO mapping.
    pub fn connect(addr: impl ToSocketAddrs, max_record: u32) -> Result<Self, Status> {
        let stream = TcpStream::connect(addr).map_err(|e| map_io(&e))?;
        Self::from_stream(stream, max_record)
    }

    /// Adopts an already-connected stream (e.g. from `accept`) as a
    /// non-blocking transport. REQ: REQ-ROUTE-029.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `max_record` is smaller than the wire
    /// header; otherwise the socket IO mapping.
    pub fn from_stream(stream: TcpStream, max_record: u32) -> Result<Self, Status> {
        let max_record = usize::try_from(max_record).map_err(|_| Status::InvalidConfig)?;
        if max_record < HEADER_LEN {
            return Err(Status::InvalidConfig);
        }
        stream.set_nonblocking(true).map_err(|e| map_io(&e))?;
        Ok(Self {
            stream,
            max_record,
            frame: vec![0_u8; max_record],
            wire: vec![0_u8; RECORD_PREFIX + max_record],
            read_buf: vec![0_u8; RECORD_PREFIX + max_record],
            filled: 0,
        })
    }

    /// Per-route record bound in bytes (I-RT18). REQ: REQ-ROUTE-038.
    #[must_use]
    pub fn max_record(&self) -> usize {
        self.max_record
    }

    /// The local bound address.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the address cannot be read.
    pub fn local_addr(&self) -> Result<SocketAddr, Status> {
        self.stream.local_addr().map_err(|e| map_io(&e))
    }

    /// The connected peer address.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the address cannot be read.
    pub fn peer_addr(&self) -> Result<SocketAddr, Status> {
        self.stream.peer_addr().map_err(|e| map_io(&e))
    }

    /// Whether `status` reports the end of the transport session (I-RT5): a
    /// clean close — [`TransportAdapter::recv_frame`] maps peer EOF to
    /// [`Status::InvalidData`] — or a fatal link error. Anything other than
    /// success or "no data yet" ([`Status::Retry`]) ends the session; the
    /// caller then reconnects via [`crate::transport::session::redial`], which
    /// starts a clean session (no historical retransmit).
    /// REQ: REQ-ROUTE-029.
    #[must_use]
    pub fn is_session_end(status: Status) -> bool {
        !matches!(status, Status::Ok | Status::Retry)
    }
}

/// Adapter surface (I-RT11): TCP is a reliable ordered stream; a frame is one
/// length-framed record (REQ-ROUTE-034: only reliable ordered streams use
/// ordered lanes). REQ: REQ-ROUTE-029, REQ-ROUTE-034.
impl TransportAdapter for TcpTransport {
    fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status> {
        if header.payload_len as usize != payload.len() {
            return Err(Status::InvalidArg);
        }
        let frame_len = header.encoded_len();
        if frame_len > self.max_record {
            return Err(Status::InvalidArg); // REQ-ROUTE-018: before any syscall
        }
        header.encode(&mut self.frame[..HEADER_LEN]);
        self.frame[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
        // Prefix the whole frame (header + payload) as one record.
        let total = encode_record(&mut self.wire, &self.frame[..frame_len])?;
        write_all_record(&mut self.stream, &self.wire[..total])
    }

    fn recv_frame<'a>(
        &mut self,
        buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
        loop {
            if self.filled >= RECORD_PREFIX {
                let plen = u32::from_le_bytes(
                    self.read_buf[..RECORD_PREFIX]
                        .try_into()
                        .unwrap_or_else(|_| unreachable!("prefix bound checked")),
                );
                let body_len = usize::try_from(plen).map_err(|_| Status::InvalidData)?;
                if body_len > self.max_record {
                    self.filled = 0; // I-RT18: over-length ⇒ protocol error, no buffering
                    return Err(Status::InvalidData);
                }
                let total = RECORD_PREFIX
                    .checked_add(body_len)
                    .ok_or(Status::InvalidData)?;
                if self.filled >= total {
                    let (body, _next) = decode_record(&self.read_buf[..self.filled], 0)?;
                    let header = WireHeader::decode(body)?;
                    if header.encoded_len() != body.len() {
                        self.filled = 0;
                        return Err(Status::InvalidData);
                    }
                    if buf.len() < body.len() {
                        return Err(Status::InvalidArg);
                    }
                    buf[..body.len()].copy_from_slice(body);
                    self.read_buf.copy_within(total..self.filled, 0);
                    self.filled -= total;
                    let plen = header.payload_len as usize;
                    return Ok(Some((header, &buf[HEADER_LEN..HEADER_LEN + plen])));
                }
            }
            if self.filled == self.read_buf.len() {
                // Bounded buffer full with no complete record: a declared length
                // within the bound should always fit, so this is a protocol error.
                self.filled = 0;
                return Err(Status::InvalidData);
            }
            match self.stream.read(&mut self.read_buf[self.filled..]) {
                Ok(0) => return Err(Status::InvalidData), // peer closed mid-record
                Ok(n) => self.filled += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(map_io(&e)),
            }
        }
    }

    fn capabilities(&self) -> Reliability {
        Reliability::Reliable
    }
}

/// Writes a whole record, looping over short writes and `WouldBlock` so a
/// bounded send never leaves a half-written record on the stream.
/// REQ: REQ-ROUTE-029.
///
/// # Errors
/// [`Status::InvalidData`] when the sink accepts no bytes (no progress);
/// otherwise the mapped IO error.
fn write_all_record<W: io::Write>(sink: &mut W, wire: &[u8]) -> Result<(), Status> {
    let mut off = 0;
    while off < wire.len() {
        match sink.write(&wire[off..]) {
            Ok(0) => return Err(Status::InvalidData),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(map_io(&e)),
        }
    }
    Ok(())
}

/// Maps a socket IO error to a canonical [`Status`].
/// REQ: REQ-ROUTE-029.
fn map_io(e: &io::Error) -> Status {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Status::Retry,
        io::ErrorKind::PermissionDenied => Status::Permission,
        io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable => Status::InvalidConfig,
        _ => Status::InvalidArg,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{TcpTransport, write_all_record};
    use crate::framing::record::{RECORD_PREFIX, encode_record};
    use crate::transport::{Reliability, TransportAdapter};
    use crate::wire::{HEADER_LEN, WireHeader};
    use drm_types::Status;

    /// Deterministic receiver: spins (yielding) until a frame decodes, never
    /// sleeping. REQ: REQ-ROUTE-029.
    fn recv_blocking(t: &mut TcpTransport, buf: &mut [u8]) -> (WireHeader, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match t.recv_frame(buf) {
                Ok(Some((h, p))) => return (h, p.to_vec()),
                Ok(None) | Err(Status::Retry) => {
                    assert!(Instant::now() < deadline, "no frame within deadline");
                    thread::yield_now();
                }
                Err(e) => unreachable!("recv: {e:?}"),
            }
        }
    }

    /// A sink that accepts at most `accept` bytes per write and yields
    /// `WouldBlock` for the first `block_remaining` writes.
    struct MockSink {
        accept: usize,
        block_remaining: usize,
        out: Vec<u8>,
    }

    impl io::Write for MockSink {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if self.block_remaining > 0 {
                self.block_remaining -= 1;
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "would block"));
            }
            let n = self.accept.min(data.len());
            self.out.extend_from_slice(&data[..n]);
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// REQ: REQ-ROUTE-029 — a 64-byte sample driven through every 1..=64 split
    /// (with intermittent `WouldBlock`) yields bytes identical to the
    /// unwrapped record write. REQ: REQ-ROUTE-032 (framing context).
    #[test]
    fn tcp_partial_write_matrix() {
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 11,
            stream_epoch: 1,
            source_seq: 3,
            payload_len: 64,
        };
        let payload = [0xAB_u8; 64];

        // Reference: the whole record encoded in one shot.
        let mut frame = [0_u8; HEADER_LEN + 64];
        header.encode(&mut frame[..HEADER_LEN]);
        frame[HEADER_LEN..].copy_from_slice(&payload);
        let mut reference = vec![0_u8; RECORD_PREFIX + frame.len()];
        let total = encode_record(&mut reference, &frame).unwrap_or_else(|e| unreachable!("{e:?}"));
        reference.truncate(total);

        for accept in 1..=64 {
            for block in [0, accept % 3] {
                let mut sink = MockSink {
                    accept,
                    block_remaining: block,
                    out: Vec::new(),
                };
                write_all_record(&mut sink, &reference)
                    .unwrap_or_else(|e| unreachable!("write: {e:?}"));
                assert_eq!(sink.out, reference, "accept={accept} block={block}");
            }
        }
    }

    /// REQ: REQ-ROUTE-029, REQ-ROUTE-034 — a framed sample round-trips over a
    /// loopback listener/stream pair; both endpoints report `Reliable`.
    #[test]
    fn tcp_loopback_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e:?}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 5,
            stream_epoch: 2,
            source_seq: 7,
            payload_len: 64,
        };
        let request = [0x5A_u8; 64];
        let request_server = request;

        let server = thread::spawn(move || {
            let (stream, _) = listener
                .accept()
                .unwrap_or_else(|e| unreachable!("accept: {e:?}"));
            let mut t =
                TcpTransport::from_stream(stream, 1024).unwrap_or_else(|e| unreachable!("{e:?}"));
            assert_eq!(t.capabilities(), Reliability::Reliable);
            assert_eq!(t.max_record(), 1024);
            let mut buf = [0_u8; 2048];
            let (h, p) = recv_blocking(&mut t, &mut buf);
            assert_eq!(h, header);
            assert_eq!(p, request_server);
            let reply = WireHeader {
                flags: 0,
                wire_channel_id: 5,
                stream_epoch: 2,
                source_seq: 8,
                payload_len: 3,
            };
            t.send_frame(&reply, &[9, 8, 7])
                .unwrap_or_else(|e| unreachable!("{e:?}"));
        });

        let mut client =
            TcpTransport::connect(addr, 1024).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(client.capabilities(), Reliability::Reliable);
        client
            .send_frame(&header, &request)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut buf = [0_u8; 2048];
        let (rh, rp) = recv_blocking(&mut client, &mut buf);
        assert_eq!(rh.source_seq, 8);
        assert_eq!(rh.payload_len, 3);
        assert_eq!(rp, vec![9, 8, 7]);
        server
            .join()
            .unwrap_or_else(|_| unreachable!("server panicked"));
    }

    /// Establishes a connected loopback pair without a listener thread:
    /// `connect` completes against the kernel backlog before `accept`.
    fn loopback_pair(max_record: u32) -> (TcpStream, TcpTransport) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e:?}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let client = TcpStream::connect(addr).unwrap_or_else(|e| unreachable!("{e:?}"));
        let (stream, _) = listener
            .accept()
            .unwrap_or_else(|e| unreachable!("accept: {e:?}"));
        let t =
            TcpTransport::from_stream(stream, max_record).unwrap_or_else(|e| unreachable!("{e:?}"));
        (client, t)
    }

    /// REQ: REQ-ROUTE-038 — a declared length far above the generated budget is
    /// an explicit `InvalidData` error, not a panic or an allocation.
    #[test]
    fn tcp_malformed_length_rejected() {
        let (mut client, mut t) = loopback_pair(64);
        let mut buf = [0_u8; 128];
        let deadline = Instant::now() + Duration::from_secs(5);

        for declared in [u32::MAX, 1_000] {
            client
                .write_all(&declared.to_le_bytes())
                .unwrap_or_else(|e| unreachable!("{e:?}"));
            loop {
                match t.recv_frame(&mut buf) {
                    Err(Status::InvalidData) => break,
                    Ok(None) | Err(Status::Retry) => {
                        assert!(Instant::now() < deadline, "no error for len {declared}");
                        thread::yield_now();
                    }
                    other => unreachable!("expected InvalidData, got {other:?}"),
                }
            }
        }
    }

    /// REQ: REQ-ROUTE-018 — an over-budget frame is refused before any syscall.
    #[test]
    fn tcp_oversize_send_rejected() {
        let (_client, mut t) = loopback_pair(64);
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 1,
            stream_epoch: 1,
            source_seq: 0,
            payload_len: 100,
        };
        assert_eq!(t.send_frame(&header, &[0_u8; 100]), Err(Status::InvalidArg));
    }

    /// REQ: REQ-ROUTE-029 — socket-error mapping is platform-neutral: the
    /// canonical `Status` classes are identical on Windows and Linux (E3
    /// Windows transport smoke).
    #[test]
    fn tcp_error_mapping_parity() {
        use std::io::ErrorKind;
        let cases = [
            (ErrorKind::WouldBlock, Status::Retry),
            (ErrorKind::TimedOut, Status::Retry),
            (ErrorKind::PermissionDenied, Status::Permission),
            (ErrorKind::AddrInUse, Status::InvalidConfig),
            (ErrorKind::AddrNotAvailable, Status::InvalidConfig),
            (ErrorKind::BrokenPipe, Status::InvalidArg),
        ];
        for (kind, want) in cases {
            let e = io::Error::new(kind, "t");
            assert_eq!(super::map_io(&e), want, "{kind:?}");
        }
    }
}
