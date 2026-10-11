//! Loopback UDP pair (work-plan M03W1-E3-T2): two endpoints exchange `N`
//! samples, one datagram each, asserting peer identity, length, and byte
//! integrity on every leg; prints the clean count and the measured throughput
//! (work-plan M08W1-E3-T1 perf baseline).
//! REQ: REQ-ROUTE-032, REQ-ROUTE-009.

use std::net::SocketAddr;
use std::time::Instant;

use drm_router::transport::udp::UdpTransport;
use drm_types::Status;

const EXCHANGES: u32 = 10_000;

/// Bounded non-blocking receive: loopback delivers promptly, so a large spin
/// budget always succeeds; exhaustion is reported rather than hanging.
fn recv_wait(t: &UdpTransport, buf: &mut [u8]) -> Result<(usize, SocketAddr), Status> {
    for _ in 0..100_000_000 {
        match t.recv_from(buf) {
            Ok(v) => return Ok(v),
            Err(Status::Retry) => std::hint::spin_loop(),
            Err(e) => return Err(e),
        }
    }
    Err(Status::Retry)
}

fn main() -> Result<(), Status> {
    let a = UdpTransport::bind("127.0.0.1:0", 1400)?;
    let b = UdpTransport::bind("127.0.0.1:0", 1400)?;
    let addr_a = a.local_addr()?;
    let peer_b = b.local_addr()?;
    let mut buf = [0_u8; 1400];

    let start = Instant::now();
    for i in 0..EXCHANGES {
        let payload = i.to_le_bytes();

        a.send_to(&payload, peer_b)?;
        let (n, from) = recv_wait(&b, &mut buf)?;
        assert_eq!(from, addr_a, "b: wrong peer at {i}");
        assert_eq!(n, payload.len(), "b: length at {i}");
        assert_eq!(buf[..n], payload, "b: corrupt at {i}");

        b.send_to(&payload, addr_a)?;
        let (n, from) = recv_wait(&a, &mut buf)?;
        assert_eq!(from, peer_b, "a: wrong peer at {i}");
        assert_eq!(n, payload.len(), "a: length at {i}");
        assert_eq!(buf[..n], payload, "a: corrupt at {i}");
    }
    let elapsed = start.elapsed();
    let per_exchange = elapsed / EXCHANGES;
    println!("udp_pair: {EXCHANGES} exchanges clean in {elapsed:?} ({per_exchange:?}/exchange)");
    Ok(())
}
