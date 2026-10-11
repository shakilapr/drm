//! Transport adapters (codebase.md 3.1 R8 — the only non-`shm` OS-code home).
//! v1 media surface (work-plan M03W2-E3-T1): a `TransportAdapter` owns one
//! link's framing and reports its capability class. Media-pluggable per I-RT11;
//! capability mapping is authoritative (REQ-TOOL-061).
//! REQ: REQ-ROUTE-009, REQ-TOOL-061.

use drm_types::Status;

use crate::wire::WireHeader;

pub mod registry;
pub mod serial;
pub mod session;
pub mod tcp;
pub mod udp;

/// Capability class of a transport binding (router-core.md `Reliability`).
/// REQ: REQ-TOOL-061.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    /// No ordering or loss recovery (e.g. UDP datagrams).
    BestEffort,
    /// Ordered loss recovery while the transport session is valid.
    Reliable,
}

/// One link's media adapter: encodes/decodes frames and reports capability.
/// Frames are handed as a [`WireHeader`] plus router-owned payload bytes;
/// the adapter never aliases DRM shared memory. REQ: REQ-ROUTE-009.
pub trait TransportAdapter {
    /// Sends one frame on this link.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] on an oversize frame (before any syscall);
    /// [`Status::Retry`] when the link cannot currently accept it; otherwise
    /// the media-specific error.
    fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status>;

    /// Receives one frame into `buf`, returning the header and payload slice.
    ///
    /// # Errors
    /// [`Status::Retry`] when no frame is ready; [`Status::InvalidData`] on a
    /// malformed frame; otherwise the media-specific error.
    fn recv_frame<'a>(
        &mut self,
        buf: &'a mut [u8],
    ) -> Result<Option<(WireHeader, &'a [u8])>, Status>;

    /// Capability class reported by this adapter.
    fn capabilities(&self) -> Reliability;
}

/// Zero-cost frame header length re-export for adapter implementations.
/// REQ: REQ-ROUTE-009.
pub use crate::wire::HEADER_LEN;

use std::collections::VecDeque;

/// Bounded receive queue: overflow drops the newest arrival and counts it,
/// so a receive thread is never blocked by downstream pressure (I-LS5
/// best-effort path). REQ: REQ-ROUTE-032.
#[derive(Debug, Clone)]
pub struct RecvQueue<T> {
    items: VecDeque<T>,
    cap: usize,
    dropped: u64,
}

impl<T> RecvQueue<T> {
    /// Creates a queue with a static capacity (activation-time sizing).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            items: VecDeque::new(),
            cap,
            dropped: 0,
        }
    }

    /// Pushes at the tail; a full queue drops this newest item and returns
    /// `false` (the drop is counted). REQ: REQ-ROUTE-032.
    pub fn push(&mut self, item: T) -> bool {
        if self.items.len() >= self.cap {
            self.dropped += 1;
            return false;
        }
        self.items.push_back(item);
        true
    }

    /// Pops the oldest queued item.
    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    /// Items dropped because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Queued items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{Reliability, TransportAdapter};
    use crate::wire::WireHeader;
    use drm_types::Status;

    /// Stub adapter proving the capability enum maps exactly (I-RT11 table).
    struct Stub(Reliability, Vec<u8>);

    impl TransportAdapter for Stub {
        fn send_frame(&mut self, header: &WireHeader, payload: &[u8]) -> Result<(), Status> {
            self.1.clear();
            self.1.extend_from_slice(&header.to_bytes());
            self.1.extend_from_slice(payload);
            Ok(())
        }

        fn recv_frame<'a>(
            &mut self,
            buf: &'a mut [u8],
        ) -> Result<Option<(WireHeader, &'a [u8])>, Status> {
            let n = self.1.len();
            if buf.len() < n {
                return Err(Status::InvalidArg);
            }
            buf[..n].copy_from_slice(&self.1);
            let header = WireHeader::decode(buf)?;
            let split = crate::wire::HEADER_LEN;
            Ok(Some((header, &buf[split..n])))
        }

        fn capabilities(&self) -> Reliability {
            self.0
        }
    }

    fn header() -> WireHeader {
        WireHeader {
            flags: 0,
            wire_channel_id: 3,
            stream_epoch: 1,
            source_seq: 9,
            payload_len: 4,
        }
    }

    /// REQ: REQ-TOOL-061, REQ-ROUTE-009 — each adapter reports its declared
    /// capability, and a frame round-trips through the trait unchanged.
    #[test]
    fn adapter_capability() {
        let mut best = Stub(Reliability::BestEffort, Vec::new());
        let reliable = Stub(Reliability::Reliable, Vec::new());
        assert_eq!(best.capabilities(), Reliability::BestEffort);
        assert_eq!(reliable.capabilities(), Reliability::Reliable);

        best.send_frame(&header(), &[1, 2, 3, 4])
            .unwrap_or_else(|e| unreachable!("send: {e:?}"));
        let mut buf = [0_u8; 64];
        let got = best.recv_frame(&mut buf);
        match got {
            Ok(Some((h, p))) => {
                assert_eq!(h, header());
                assert_eq!(p, &[1, 2, 3, 4]);
            }
            other => unreachable!("recv: {other:?}"),
        }
    }

    /// REQ: REQ-ROUTE-032 — a full receive queue drops the newest arrival and
    /// counts it; queued items keep FIFO order.
    #[test]
    fn recv_queue_policy() {
        let mut q = super::RecvQueue::new(2);
        assert!(q.push(1));
        assert!(q.push(2));
        assert!(!q.push(3), "newest dropped");
        assert_eq!(q.dropped(), 1);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert!(q.is_empty());
    }
}
