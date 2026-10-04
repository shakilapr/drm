//! UDP datagram adapter (router-core.md §Protocols *UDP transport notes*,
//! I-RT3): one sample per datagram, a non-blocking socket, and the
//! `max_datagram_payload` bound enforced before the send syscall so an oversize
//! sample never reaches the kernel (no IP fragmentation / PMTUD).
//! REQ: REQ-ROUTE-032, REQ-ROUTE-009.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use drm_types::Status;

/// A non-blocking UDP endpoint carrying at most one sample per datagram.
/// REQ: REQ-ROUTE-032.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    max_payload: usize,
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
        })
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
}
