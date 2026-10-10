//! Stream-transport session lifecycle (router-core.md §Protocols *Session
//! lifecycle (stream transports)*, I-RT5): `reliable` ordered delivery is valid
//! only while the transport session is live, a clean close or a fatal read
//! error ends the session, and reconnection starts clean with empty ingress
//! ordering state — there is no historical retransmit, so a consumer observes a
//! gap for samples lost across the boundary instead of a silently stitched
//! stream.
//! REQ: REQ-ROUTE-011 (epoch increase ⇒ stream reset; reconnection starts
//! clean), REQ-ROUTE-029 (stream transports: one length-framed record per
//! sample).
//!
//! No registry id names reconnect/session lifecycle directly; I-RT5 is anchored
//! to the nearest real ids above (REQ-ROUTE-011 stream reset, REQ-ROUTE-029
//! stream framing).

use std::time::Duration;

use drm_types::Status;

use crate::transport::tcp::TcpTransport;

/// Deterministic, bounded reconnect backoff pacing (I-RT5 reconnection): a
/// fixed sequence of `attempts` waits starting at `base`, multiplied by
/// `factor` and capped at `cap`. It is a plain iterator over
/// [`Duration`] waits, so a caller — or a test — drives the schedule directly
/// with no wall-clock `sleep`.
/// REQ: REQ-ROUTE-029, REQ-ROUTE-011.
#[derive(Debug, Clone)]
pub struct BackoffSchedule {
    delays: Vec<Duration>,
}

impl BackoffSchedule {
    /// Builds an `attempts`-long schedule: wait `i` is
    /// `min(base × factor^i, cap)`.
    /// REQ: REQ-ROUTE-029.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `attempts` is zero (a schedule that could
    /// never re-dial).
    pub fn new(
        base: Duration,
        factor: u32,
        cap: Duration,
        attempts: usize,
    ) -> Result<Self, Status> {
        if attempts == 0 {
            return Err(Status::InvalidConfig);
        }
        let mut delays = Vec::with_capacity(attempts);
        let mut wait = base.min(cap);
        for _ in 0..attempts {
            delays.push(wait);
            wait = wait.saturating_mul(factor.max(1)).min(cap);
        }
        Ok(Self { delays })
    }

    /// Number of scheduled re-dial attempts — the hard bound on backoff (never
    /// an unbounded loop). REQ: REQ-ROUTE-029.
    #[must_use]
    pub fn attempts(&self) -> usize {
        self.delays.len()
    }

    /// The wait before attempt `attempt` (0-indexed); `None` past the bound.
    /// REQ: REQ-ROUTE-029.
    #[must_use]
    pub fn delay(&self, attempt: usize) -> Option<Duration> {
        self.delays.get(attempt).copied()
    }

    /// The scheduled waits in order — drive this in tests in place of real
    /// time. REQ: REQ-ROUTE-029.
    pub fn delays(&self) -> impl Iterator<Item = Duration> + '_ {
        self.delays.iter().copied()
    }
}

/// Outcome of offering one source sequence to a session-scoped reliable
/// consumer (I-RT5). REQ: REQ-ROUTE-011.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Contiguous with the previous delivery in this session (or the first
    /// accepted sample of a fresh session). REQ: REQ-ROUTE-011.
    InOrder,
    /// Same-or-older sequence already delivered within the epoch ⇒ dropped with
    /// no state change. REQ: REQ-ROUTE-011.
    Duplicate,
    /// A forward jump of `lost` never-delivered samples ⇒ a visible gap that is
    /// never stitched (reliable delivery does not survive a restart).
    /// REQ: REQ-ROUTE-011.
    Gap {
        /// Samples skipped since the last expected sequence.
        lost: u64,
    },
}

/// Session-scoped reliable delivery tracker (I-RT5): ordering/dedup state is
/// valid only while the session is live and is cleared on every reconnect. The
/// logical next-sequence position is retained across a reconnect *only* to
/// detect the boundary gap — the lost samples are never retransmitted, so the
/// reconnect is never a silent stitch.
/// REQ: REQ-ROUTE-011.
#[derive(Debug, Clone)]
pub struct SessionReliability {
    generation: u32,
    live: bool,
    /// Next logical source sequence expected; `None` before the first delivery.
    expected: Option<u64>,
    /// Samples observed lost (never delivered) over this tracker's lifetime.
    lost: u64,
}

impl SessionReliability {
    /// Starts an initial, live session at generation 0.
    /// REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: 0,
            live: true,
            expected: None,
            lost: 0,
        }
    }

    /// Active session generation (bumped by every successful reconnect).
    /// REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Whether the session is live (`false` between a disconnect and the next
    /// reconnect). REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Total samples observed lost (never delivered).
    /// REQ: REQ-ROUTE-011.
    #[must_use]
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Offers source sequence `seq` to the live session.
    /// REQ: REQ-ROUTE-011.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the session is not live — reliable delivery
    /// never spans a session restart (I-RT5).
    pub fn deliver(&mut self, seq: u64) -> Result<Delivery, Status> {
        if !self.live {
            return Err(Status::InvalidArg);
        }
        Ok(match self.expected {
            None => {
                self.expected = Some(seq.saturating_add(1));
                Delivery::InOrder
            }
            Some(exp) if seq == exp => {
                self.expected = Some(exp.saturating_add(1));
                Delivery::InOrder
            }
            Some(exp) if seq > exp => {
                let lost = seq - exp;
                self.lost = self.lost.saturating_add(lost);
                self.expected = Some(seq.saturating_add(1));
                Delivery::Gap { lost }
            }
            Some(_) => Delivery::Duplicate,
        })
    }

    /// Ends the live session on disconnect: the session is no longer valid, so
    /// no further delivery is accepted until a reconnect, and per-session
    /// ordering state is gone. The logical position is kept only for gap
    /// detection across the boundary. REQ: REQ-ROUTE-011.
    pub fn end_session(&mut self) {
        self.live = false;
    }

    /// Begins a fresh session after reconnect: a new generation and empty
    /// per-session state (I-RT5). The next delivery whose sequence is beyond
    /// the retained logical position observes a gap for the samples lost across
    /// the boundary — never a stitch.
    /// REQ: REQ-ROUTE-011.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the session is already live (a reconnect
    /// must follow a disconnect).
    pub fn begin_session(&mut self) -> Result<(), Status> {
        if self.live {
            return Err(Status::InvalidArg);
        }
        self.generation = self.generation.saturating_add(1);
        self.live = true;
        Ok(())
    }
}

impl Default for SessionReliability {
    fn default() -> Self {
        Self::new()
    }
}

/// A completed reconnect: the fresh transport plus the re-dial attempts taken
/// and the total scheduled wait consumed.
/// REQ: REQ-ROUTE-029.
#[derive(Debug)]
pub struct Redialed {
    /// The freshly connected transport — session state is empty (I-RT5).
    pub transport: TcpTransport,
    /// Attempts made before success.
    pub attempts: usize,
    /// Total scheduled wait consumed before success.
    pub waited: Duration,
}

/// Drives a bounded re-dial (I-RT5): for each scheduled wait, call `on_wait`
/// (the pacing hook — production paces, tests record) then attempt `connect`.
/// Returns the first success; if every scheduled attempt fails, it stops with
/// [`Status::Retry`] — never an unbounded loop, and never a real `sleep` in the
/// tested path.
/// REQ: REQ-ROUTE-029, REQ-ROUTE-011.
///
/// # Errors
/// [`Status::Retry`] when every scheduled attempt fails; otherwise the first
/// non-retryable `connect` error.
pub fn redial<C, W>(
    schedule: &BackoffSchedule,
    mut connect: C,
    mut on_wait: W,
) -> Result<Redialed, Status>
where
    C: FnMut() -> Result<TcpTransport, Status>,
    W: FnMut(Duration),
{
    let mut waited = Duration::ZERO;
    for (i, wait) in schedule.delays().enumerate() {
        on_wait(wait);
        waited = waited.saturating_add(wait);
        match connect() {
            Ok(transport) => {
                return Ok(Redialed {
                    transport,
                    attempts: i + 1,
                    waited,
                });
            }
            Err(Status::Retry) => {}
            Err(other) => return Err(other),
        }
    }
    Err(Status::Retry)
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, TcpListener};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{BackoffSchedule, Delivery, SessionReliability, redial};
    use crate::transport::TransportAdapter;
    use crate::transport::tcp::TcpTransport;
    use crate::wire::WireHeader;
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

    /// One framed sample header for sequence `seq`. REQ: REQ-ROUTE-029.
    fn hdr(seq: u64) -> WireHeader {
        WireHeader {
            flags: 0,
            wire_channel_id: 9,
            stream_epoch: 1,
            source_seq: seq,
            payload_len: 8,
        }
    }

    /// REQ: REQ-ROUTE-029 — the bounded schedule is exact and finite: capped,
    /// deterministic and iterable, with a hard attempt bound.
    #[test]
    fn tcp_reconnect_backoff_bounded() {
        let s = BackoffSchedule::new(Duration::from_millis(10), 2, Duration::from_millis(40), 5)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(s.attempts(), 5);
        assert_eq!(s.delay(5), None, "bounded: no attempt past the schedule");
        let got: Vec<Duration> = s.delays().collect();
        assert_eq!(
            got,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(40),
                Duration::from_millis(40),
            ]
        );
        // A zero-attempt schedule can never re-dial.
        assert_eq!(
            BackoffSchedule::new(Duration::ZERO, 1, Duration::ZERO, 0).map(|s| s.attempts()),
            Err(Status::InvalidConfig)
        );
    }

    /// REQ: REQ-ROUTE-029 — an all-failing dial is bounded by the schedule:
    /// every scheduled wait is consumed, no unbounded loop, no real time.
    #[test]
    fn tcp_reconnect_backoff_exhausted_is_retry() {
        let s = BackoffSchedule::new(Duration::from_secs(1), 3, Duration::from_secs(8), 4)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut waits = Vec::new();
        let out = redial(&s, || Err(Status::Retry), |d| waits.push(d));
        assert_eq!(out.err(), Some(Status::Retry));
        assert_eq!(waits, s.delays().collect::<Vec<_>>());
    }

    /// Binds a loopback listener on an ephemeral port, returning it with its
    /// address (never a fixed port). REQ: REQ-ROUTE-029.
    fn bind() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e:?}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        (listener, addr)
    }

    /// Accepts one loopback peer on `listener` as a non-blocking transport.
    /// REQ: REQ-ROUTE-029.
    fn adopt(listener: &TcpListener) -> TcpTransport {
        let (stream, _) = listener
            .accept()
            .unwrap_or_else(|e| unreachable!("accept: {e:?}"));
        TcpTransport::from_stream(stream, 1024).unwrap_or_else(|e| unreachable!("{e:?}"))
    }

    /// REQ: REQ-ROUTE-011, REQ-ROUTE-029 — full reconnect lifecycle: kill the
    /// remote mid-stream, detect the session end, re-dial under the bounded
    /// schedule, and assert `reliable` semantics are session-scoped
    /// (lost-across-restart ⇒ a visible gap, never stitched).
    #[test]
    fn tcp_reconnect_session_scope() {
        // ---- Session 1: a live loopback pair ----
        let (listener, addr) = bind();
        let (drop_tx, drop_rx) = mpsc::channel::<()>();
        let server = thread::spawn(move || {
            let mut t = adopt(&listener);
            t.send_frame(&hdr(0), &[0x11; 8])
                .unwrap_or_else(|e| unreachable!("send: {e:?}"));
            drop_rx
                .recv()
                .unwrap_or_else(|_| unreachable!("drop signal"));
            drop(t); // closes the connection ⇒ the client observes EOF
        });

        let mut client =
            TcpTransport::connect(addr, 1024).unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut rel = SessionReliability::new();
        assert_eq!(rel.generation(), 0);
        assert!(rel.is_live());

        let mut buf = [0_u8; 2048];
        let (h, p) = recv_blocking(&mut client, &mut buf);
        assert_eq!((h, p), (hdr(0), vec![0x11; 8]));
        assert_eq!(rel.deliver(0), Ok(Delivery::InOrder));

        // ---- Kill the remote; detect the session end ----
        drop_tx
            .send(())
            .unwrap_or_else(|_| unreachable!("signal drop"));
        let end = loop {
            match client.recv_frame(&mut buf) {
                Ok(None) | Err(Status::Retry) => thread::yield_now(),
                Ok(Some(_)) => unreachable!("no further frames this session"),
                Err(e) => break e,
            }
        };
        assert!(
            TcpTransport::is_session_end(end),
            "clean close detected: {end:?}"
        );
        rel.end_session();
        assert!(!rel.is_live());
        // Delivery never spans a restart: offering a sequence on a dead session fails.
        assert_eq!(rel.deliver(1), Err(Status::InvalidArg));

        // ---- Reconnect under the bounded schedule to a fresh listener ----
        let (listener2, addr2) = bind();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server2 = thread::spawn(move || {
            let mut t = adopt(&listener2);
            // Samples 1..=4 never crossed the boundary; the fresh source resumes at 5.
            t.send_frame(&hdr(5), &[0x22; 8])
                .unwrap_or_else(|e| unreachable!("send: {e:?}"));
            done_rx
                .recv()
                .unwrap_or_else(|_| unreachable!("done signal"));
        });

        let schedule =
            BackoffSchedule::new(Duration::from_millis(1), 2, Duration::from_millis(8), 4)
                .unwrap_or_else(|e| unreachable!("{e:?}"));
        let mut waits = Vec::new();
        let dialed = redial(
            &schedule,
            || TcpTransport::connect(addr2, 1024),
            |d| waits.push(d),
        )
        .unwrap_or_else(|e| unreachable!("redial: {e:?}"));
        // The first attempt succeeds ⇒ exactly one scheduled wait consumed.
        assert_eq!(dialed.attempts, 1);
        assert_eq!(waits, vec![schedule.delay(0).unwrap_or(Duration::ZERO)]);

        // A fresh session: generation bumped, per-session state reset.
        rel.begin_session()
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(rel.generation(), 1);
        // A reconnect must follow a disconnect.
        assert_eq!(rel.begin_session(), Err(Status::InvalidArg));

        let mut client2 = dialed.transport;
        let (h2, p2) = recv_blocking(&mut client2, &mut buf);
        assert_eq!((h2, p2), (hdr(5), vec![0x22; 8]));

        // Lost 1..=4 across the boundary ⇒ a visible gap, never a stitch.
        assert_eq!(rel.deliver(5), Ok(Delivery::Gap { lost: 4 }));
        assert_eq!(rel.lost(), 4);
        // Contiguity resumes after the observed gap.
        assert_eq!(rel.deliver(6), Ok(Delivery::InOrder));

        done_tx.send(()).unwrap_or_else(|_| unreachable!("done"));
        server
            .join()
            .unwrap_or_else(|_| unreachable!("server 1 panicked"));
        server2
            .join()
            .unwrap_or_else(|_| unreachable!("server 2 panicked"));
    }
}
