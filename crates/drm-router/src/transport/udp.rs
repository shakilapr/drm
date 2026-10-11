//! UDP datagram adapter (router-core.md §Protocols *UDP transport notes*,
//! I-RT3): one sample per datagram, a non-blocking socket, and the
//! `max_datagram_payload` bound enforced before the send syscall so an oversize
//! sample never reaches the kernel (no IP fragmentation / PMTUD).
//! REQ: REQ-ROUTE-032, REQ-ROUTE-009.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use drm_types::Status;

use crate::transport::{Reliability, TransportAdapter};
use crate::wire::{HEADER_LEN, WireHeader};

/// A non-blocking UDP endpoint carrying at most one sample per datagram.
/// REQ: REQ-ROUTE-032.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    max_payload: usize,
    peer: Option<SocketAddr>,
    scratch: Vec<u8>,
}

impl UdpTransport {
    /// Binds a non-blocking endpoint; `max_payload` is the per-datagram bound
    /// from the route (one sample, no DRM fragmentation). REQ: REQ-ROUTE-032.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] for an in-use or unavailable address;
    /// [`Status::InvalidArg`] for a malformed address.
    pub fn bind(addr: impl ToSocketAddrs, max_payload: u16) -> Result<Self, Status> {
        let socket = UdpSocket::bind(addr).map_err(|e| map_io(&e))?;
        socket.set_nonblocking(true).map_err(|e| map_io(&e))?;
        Ok(Self {
            socket,
            max_payload: usize::from(max_payload),
            peer: None,
            scratch: vec![0_u8; usize::from(max_payload) + HEADER_LEN + 4],
        })
    }

    /// Binds with a configured link peer for the [`TransportAdapter`] surface.
    ///
    /// # Errors
    /// As [`UdpTransport::bind`].
    pub fn bind_peer(
        addr: impl ToSocketAddrs,
        max_payload: u16,
        peer: SocketAddr,
    ) -> Result<Self, Status> {
        let mut t = Self::bind(addr, max_payload)?;
        t.peer = Some(peer);
        Ok(t)
    }

    /// Sets the link peer used by [`TransportAdapter::send_frame`].
    pub fn set_peer(&mut self, peer: SocketAddr) {
        self.peer = Some(peer);
    }

    /// Configured link peer, if any.
    #[must_use]
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// The local bound address.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the address cannot be read.
    pub fn local_addr(&self) -> Result<SocketAddr, Status> {
        self.socket.local_addr().map_err(|e| map_io(&e))
    }

    /// Per-datagram payload bound in bytes.
    /// REQ: REQ-ROUTE-032.
    #[must_use]
    pub fn max_payload(&self) -> usize {
        self.max_payload
    }

    /// Sends one sample; refuses payloads larger than the bound before the
    /// syscall (I-RT3 `NoOversizeSend`). REQ: REQ-ROUTE-032.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `payload.len() > max_payload`; otherwise the
    /// socket IO mapping.
    pub fn send_to(&self, payload: &[u8], dst: SocketAddr) -> Result<(), Status> {
        if payload.len() > self.max_payload {
            return Err(Status::InvalidArg);
        }
        self.socket
            .send_to(payload, dst)
            .map(|_| ())
            .map_err(|e| map_io(&e))
    }

    /// Receives one datagram into `buf`, returning `(len, peer)`.
    /// REQ: REQ-ROUTE-032.
    ///
    /// # Errors
    /// [`Status::Retry`] when no datagram is ready (non-blocking); otherwise
    /// the socket IO mapping.
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), Status> {
        self.socket.recv_from(buf).map_err(|e| map_io(&e))
    }
}

/// Adapter surface (I-RT11): UDP is best-effort; a frame is one datagram.
/// REQ: REQ-TOOL-061, REQ-ROUTE-032.
impl TransportAdapter for UdpTransport {
    fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status> {
        if header.payload_len as usize != payload.len() {
            return Err(Status::InvalidArg);
        }
        let total = header.encoded_len();
        if total > self.max_payload || self.scratch.len() < total {
            return Err(Status::InvalidArg); // I-RT3: before any syscall
        }
        let Some(peer) = self.peer else {
            return Err(Status::InvalidConfig);
        };
        header.encode(&mut self.scratch[..HEADER_LEN]);
        self.scratch[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
        self.socket
            .send_to(&self.scratch[..total], peer)
            .map(|_| ())
            .map_err(|e| map_io(&e))
    }

    fn recv_frame<'a>(
        &mut self,
        buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
        let n = match self.socket.recv_from(buf) {
            Ok((n, _)) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(map_io(&e)),
        };
        if n < HEADER_LEN {
            return Err(Status::InvalidData);
        }
        let header = WireHeader::decode(&buf[..n])?;
        if header.payload_len as usize != n - HEADER_LEN {
            return Err(Status::InvalidData);
        }
        Ok(Some((header, &buf[HEADER_LEN..n])))
    }

    fn capabilities(&self) -> Reliability {
        Reliability::BestEffort
    }
}

/// Maps a socket IO error to a canonical [`Status`].
/// REQ: REQ-ROUTE-032.
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
    use std::net::SocketAddr;

    use super::UdpTransport;
    use drm_types::Status;

    /// Bounded receive: non-blocking endpoint + loopback deliver promptly.
    fn recv_wait(t: &UdpTransport, buf: &mut [u8]) -> (usize, SocketAddr) {
        for _ in 0..10_000_000 {
            match t.recv_from(buf) {
                Ok(v) => return v,
                Err(Status::Retry) => std::hint::spin_loop(),
                Err(e) => unreachable!("recv: {e:?}"),
            }
        }
        unreachable!("udp test: no datagram within budget");
    }

    /// REQ: REQ-ROUTE-032 — an oversize payload is refused before the syscall
    /// (I-RT3); a payload exactly at the bound is accepted.
    #[test]
    fn udp_oversize_reject() {
        let rx = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let dst = rx.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
        let tx = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(tx.max_payload(), 1400);
        let big = [0_u8; 1401];
        assert_eq!(tx.send_to(&big, dst), Err(Status::InvalidArg));
        let at = [0_u8; 1400];
        assert_eq!(tx.send_to(&at, dst), Ok(()));
    }

    /// REQ: REQ-ROUTE-032 — a non-blocking endpoint reports `Retry` when no
    /// datagram is ready.
    #[test]
    fn udp_empty_is_retry() {
        let t = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut buf = [0_u8; 64];
        assert_eq!(t.recv_from(&mut buf), Err(Status::Retry));
    }

    /// REQ: REQ-ROUTE-032 — loopback round-trip preserves bytes both ways.
    #[test]
    fn udp_loopback_roundtrip() {
        let a = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let b = UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let addr_a = a.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
        let peer_b = b.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut buf = [0_u8; 1400];

        let request = [1_u8, 2, 3, 4];
        assert_eq!(a.send_to(&request, peer_b), Ok(()));
        let (n, from) = recv_wait(&b, &mut buf);
        assert_eq!(&buf[..n], &request);
        assert_eq!(from, addr_a);

        let reply = [9_u8, 8, 7];
        assert_eq!(b.send_to(&reply, addr_a), Ok(()));
        let (n, from) = recv_wait(&a, &mut buf);
        assert_eq!(&buf[..n], &reply);
        assert_eq!(from, peer_b);
    }

    /// REQ: REQ-TOOL-061, REQ-ROUTE-032 — the adapter surface reports
    /// best-effort and round-trips a framed sample over loopback.
    #[test]
    fn udp_adapter_roundtrip() {
        use crate::transport::{Reliability, TransportAdapter};
        use crate::wire::WireHeader;

        let mut rx =
            UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let dst = rx.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut tx = UdpTransport::bind_peer("127.0.0.1:0", 1400, dst)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(tx.capabilities(), Reliability::BestEffort);
        assert_eq!(tx.peer(), Some(dst));

        let header = WireHeader {
            flags: 0,
            wire_channel_id: 5,
            stream_epoch: 2,
            source_seq: 7,
            payload_len: 4,
        };
        assert_eq!(tx.send_frame(&header, &[4, 3, 2, 1]), Ok(()));

        let mut buf = [0_u8; 1400];
        let mut got = None;
        for _ in 0..10_000_000 {
            match rx.recv_frame(&mut buf) {
                Ok(Some((h, p))) => {
                    got = Some((h, p.to_vec()));
                    break;
                }
                Ok(None) | Err(Status::Retry) => std::hint::spin_loop(),
                Err(e) => unreachable!("recv: {e:?}"),
            }
        }
        let (h, p) = got.unwrap_or_else(|| unreachable!("no frame"));
        assert_eq!(h, header);
        assert_eq!(p, vec![4, 3, 2, 1]);

        // Oversize is refused before the syscall (I-RT3).
        let mut over = header;
        over.payload_len = 1401;
        assert_eq!(tx.send_frame(&over, &[0_u8; 1401]), Err(Status::InvalidArg));
    }

    /// REQ: REQ-ROUTE-032 — socket-error mapping is platform-neutral: the
    /// canonical `Status` classes are identical on Windows and Linux (E3
    /// Windows transport smoke).
    #[test]
    fn udp_error_mapping_parity() {
        use std::io::{Error, ErrorKind};
        let cases = [
            (ErrorKind::WouldBlock, Status::Retry),
            (ErrorKind::TimedOut, Status::Retry),
            (ErrorKind::PermissionDenied, Status::Permission),
            (ErrorKind::AddrInUse, Status::InvalidConfig),
            (ErrorKind::AddrNotAvailable, Status::InvalidConfig),
            (ErrorKind::ConnectionRefused, Status::InvalidArg),
        ];
        for (kind, want) in cases {
            assert_eq!(super::map_io(&Error::new(kind, "t")), want, "{kind:?}");
        }
    }

    /// REQ: REQ-ROUTE-032 — a bounded receive queue drops the newest arrivals
    /// under burst (counted), and the receive path never blocks.
    #[test]
    fn udp_backpressure() {
        use crate::transport::{RecvQueue, TransportAdapter};
        use crate::wire::WireHeader;

        let mut rx =
            UdpTransport::bind("127.0.0.1:0", 1400).unwrap_or_else(|e| unreachable!("{e:?}"));
        let dst = rx.local_addr().unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut tx = UdpTransport::bind_peer("127.0.0.1:0", 1400, dst)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let header = WireHeader {
            flags: 0,
            wire_channel_id: 1,
            stream_epoch: 1,
            source_seq: 0,
            payload_len: 1,
        };
        for i in 0..5_u8 {
            let mut h = header;
            h.source_seq = u64::from(i);
            assert_eq!(tx.send_frame(&h, &[i]), Ok(()));
        }

        let mut q = RecvQueue::new(3);
        let mut buf = [0_u8; 64];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while q.len() + usize::try_from(q.dropped()).unwrap_or(0) < 5
            && std::time::Instant::now() < deadline
        {
            match rx.recv_frame(&mut buf) {
                Ok(Some((h, p))) => {
                    q.push((h.source_seq, p[0]));
                }
                Ok(None) | Err(Status::Retry) => std::hint::spin_loop(),
                Err(e) => unreachable!("recv: {e:?}"),
            }
        }
        assert_eq!(q.dropped(), 2, "newest two dropped");
        assert_eq!(q.len(), 3);
        assert_eq!(q.pop(), Some((0, 0)));
        assert_eq!(q.pop(), Some((1, 1)));
        assert_eq!(q.pop(), Some((2, 2)));
    }
}
