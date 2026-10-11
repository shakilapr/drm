//! Router data path (router-core.md §Functions `egress`/`ingress`): topic
//! filtering from the derived forwarding plan (I-RT1), datagram bound
//! enforcement before any syscall (I-RT3), header-stripped ingress with
//! per-`(link, wire_channel_id)` ordering (I-RT9) and synthetic-publisher
//! deposit in native format (I-RT4), and link-cut isolation (I-RT6: a down link
//! takes only its routes offline; local shared memory and sibling links
//! continue). The router never retains a borrowed DRM sample across the send
//! call (I-RT8 external-I/O rule).
//! REQ: REQ-ROUTE-022, REQ-ROUTE-023, REQ-ROUTE-031, REQ-ROUTE-032,
//! REQ-ROUTE-033, REQ-ROUTE-011, REQ-ROUTE-038, REQ-ROUTE-007.

use std::collections::{BTreeMap, BTreeSet};

use drm_types::Status;

use crate::transport::TransportAdapter;
use crate::wire::{IngressDecision, IngressState, StreamKey, WireHeader};

/// One outbound route binding derived from the forwarding plan: the local topic
/// handle, the egress link, the transport-scoped wire channel id, whether a
/// remote consumer exists on that link (I-RT1), the per-datagram payload bound
/// (I-RT3), and the generated receive bound (I-RT18).
/// REQ: REQ-ROUTE-022, REQ-ROUTE-032, REQ-ROUTE-038.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EgressRoute {
    /// Local topic handle — the runtime key, never a name.
    /// REQ: REQ-ROUTE-022.
    pub topic: u32,
    /// Egress link identity. REQ: REQ-ROUTE-022.
    pub link: u32,
    /// Transport-scoped wire channel id — never the runtime topic handle
    /// (I-RT7). REQ: REQ-ROUTE-033.
    pub wire_channel_id: u32,
    /// `true` iff at least one remote consumer of `topic` sits on `link`; only
    /// such topics egress (I-RT1). REQ: REQ-ROUTE-022.
    pub remote_consumers: bool,
    /// Per-datagram payload bound for this link. REQ: REQ-ROUTE-032.
    pub max_datagram_payload: u16,
    /// Generated receive/record bound enforced before buffering (I-RT18).
    /// REQ: REQ-ROUTE-038.
    pub max_record_bytes: u32,
}

/// Disposition of one received frame. REQ: REQ-ROUTE-011.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressOutcome {
    /// The wire headers were stripped and the native payload deposited to the
    /// importer (I-RT4). REQ: REQ-ROUTE-031.
    Imported,
    /// Same-epoch `source_seq <= last`: reorder/duplicate dropped (I-RT9).
    /// REQ: REQ-ROUTE-011.
    Duplicate,
    /// Lower epoch than the newest accepted (or `epoch 0` after `u64::MAX`):
    /// stale, dropped (I-RT9). REQ: REQ-ROUTE-011.
    StaleEpoch,
    /// Unknown `(link, wire_channel_id)`: dropped without blocking (never a
    /// panic). REQ: REQ-ROUTE-011.
    UnknownChannel,
    /// `link` is down (I-RT6): the frame is dropped without touching ordering
    /// state; local shared memory is untouched by the router.
    /// REQ: REQ-ROUTE-007.
    LinkDown,
}

/// Synthetic-publisher deposit target: receives header-stripped payloads in
/// native format so consumers cannot distinguish imported from local data
/// (I-RT4). The higher-epoch reset mirrors the publisher-lifecycle epoch
/// protocol (I-RT9).
/// REQ: REQ-ROUTE-031, REQ-ROUTE-011.
pub trait ImportSink {
    /// Deposits one header-stripped sample in native format (I-RT4).
    ///
    /// # Errors
    /// The sink's own pressure/authority status; the caller counts it and keeps
    /// running so a local drop never suppresses sibling forwarding (I-RT16).
    /// REQ: REQ-ROUTE-031.
    fn deposit(&mut self, topic: u32, epoch: u64, seq: u64, payload: &[u8]) -> Result<(), Status>;

    /// Bumps the imported publisher's stream epoch when a higher-epoch frame
    /// arrives (I-RT9). Default is a no-op for sinks that carry no epoch.
    ///
    /// # Errors
    /// The sink's authority-contention status. REQ: REQ-ROUTE-011.
    fn reset_epoch(&mut self, _topic: u32) -> Result<(), Status> {
        Ok(())
    }
}

/// The router data path: egress route bindings plus per-`(link,
/// wire_channel_id)` ingress ordering state. REQ: REQ-ROUTE-022, REQ-ROUTE-011.
#[derive(Debug, Clone, Default)]
pub struct Router {
    routes: Vec<EgressRoute>,
    ingress: BTreeMap<(u32, u32), IngressState>,
    down_links: BTreeSet<u32>,
}

impl Router {
    /// Creates a router over the derived route bindings.
    /// REQ: REQ-ROUTE-022.
    #[must_use]
    pub fn new(routes: Vec<EgressRoute>) -> Self {
        Self {
            routes,
            ingress: BTreeMap::new(),
            down_links: BTreeSet::new(),
        }
    }

    /// Marks `link` down (I-RT6): routes over it stop egressing and its ingress
    /// frames are dropped, while local shared memory and every other link are
    /// untouched. Idempotent.
    /// REQ: REQ-ROUTE-007.
    pub fn link_down(&mut self, link: u32) {
        self.down_links.insert(link);
    }

    /// Marks `link` up after a reconnect (I-RT6): egress resumes and the
    /// per-link ingress ordering state is dropped — a fresh session with empty
    /// ordering state and no historical retransmit (I-RT9). Idempotent.
    /// REQ: REQ-ROUTE-007.
    pub fn link_up(&mut self, link: u32) {
        self.down_links.remove(&link);
        self.ingress.retain(|(l, _), _| *l != link);
    }

    /// Whether `link` is currently down (I-RT6).
    /// REQ: REQ-ROUTE-007.
    #[must_use]
    pub fn link_is_down(&self, link: u32) -> bool {
        self.down_links.contains(&link)
    }

    /// The registered route bindings, in input order.
    /// REQ: REQ-ROUTE-022.
    #[must_use]
    pub fn routes(&self) -> &[EgressRoute] {
        &self.routes
    }

    /// Route binding for `(link, topic)`, if any.
    /// REQ: REQ-ROUTE-022.
    #[must_use]
    pub fn route(&self, link: u32, topic: u32) -> Option<EgressRoute> {
        self.routes
            .iter()
            .find(|r| r.link == link && r.topic == topic)
            .copied()
    }

    /// Route binding for `(link, wire_channel_id)`, if any.
    /// REQ: REQ-ROUTE-022.
    #[must_use]
    pub fn route_by_wire(&self, link: u32, wire_channel_id: u32) -> Option<EgressRoute> {
        self.routes
            .iter()
            .find(|r| r.link == link && r.wire_channel_id == wire_channel_id)
            .copied()
    }

    /// Egress route bindings for `link`, in registration order — the egress scan
    /// iterates only the routes of the link being serviced, so a topic bound to
    /// another link is never examined here (I-RT1). Local-only topics are
    /// included (they carry `remote_consumers == false`) so the scan can skip
    /// them without a separate lookup.
    /// REQ: REQ-ROUTE-022.
    pub fn routes_on(&self, link: u32) -> impl Iterator<Item = EgressRoute> + '_ {
        self.routes.iter().filter(move |r| r.link == link).copied()
    }

    /// Egresses one sample for `topic` on `link`: filtered to routes with a
    /// remote consumer (I-RT1), the encoded frame bound checked before the send
    /// syscall (I-RT3), then handed to the transport adapter. Returns `Ok(false)`
    /// when the topic is filtered (local-only or unregistered) — no frame, no
    /// work, no reservation.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when the encoded frame exceeds the route's
    /// `max_datagram_payload` or `payload` is longer than `u32`; otherwise the
    /// adapter's media error. REQ: REQ-ROUTE-022, REQ-ROUTE-032.
    pub fn egress_udp<A: TransportAdapter>(
        &mut self,
        link: u32,
        topic: u32,
        epoch: u64,
        seq: u64,
        payload: &[u8],
        adapter: &mut A,
    ) -> Result<bool, Status> {
        let Some((route, header)) = self.prepare_egress(link, topic, epoch, seq, payload)? else {
            return Ok(false);
        };
        if header.encoded_len() > usize::from(route.max_datagram_payload) {
            return Err(Status::InvalidArg); // I-RT3: before any syscall
        }
        adapter.send_frame(&header, payload)?;
        Ok(true)
    }

    /// Egresses one sample for `topic` on `link` over a stream/serial transport:
    /// the same filtering (I-RT1) and header construction as
    /// [`Router::egress_udp`], but without the per-datagram bound — the frame
    /// bound is the transport's own ([`crate::transport::serial`] I-SR2, TCP
    /// I-RT18). Serial is best-effort, so the adapter's [`Status::Retry`] is
    /// surfaced to the caller, which counts and continues (I-RT15/I-SR).
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `payload` is longer than `u32`; otherwise the
    /// adapter's media error. REQ: REQ-ROUTE-022, REQ-ROUTE-024, REQ-ROUTE-034.
    pub fn egress_frame<A: TransportAdapter>(
        &mut self,
        link: u32,
        topic: u32,
        epoch: u64,
        seq: u64,
        payload: &[u8],
        adapter: &mut A,
    ) -> Result<bool, Status> {
        let Some((_route, header)) = self.prepare_egress(link, topic, epoch, seq, payload)? else {
            return Ok(false);
        };
        adapter.send_frame(&header, payload)?;
        Ok(true)
    }

    /// Builds the egress header for `(link, topic)` after the forwarding-plan
    /// filter (I-RT1); `Ok(None)` means the topic is filtered — no frame, no
    /// work, no reservation. A down link (I-RT6) also yields `Ok(None)`: routes
    /// over a cut link are offline, so no frame is emitted. Shared by
    /// [`Router::egress_udp`] and [`Router::egress_frame`].
    /// REQ: REQ-ROUTE-022, REQ-ROUTE-033, REQ-ROUTE-007.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `payload` is longer than `u32`.
    fn prepare_egress(
        &self,
        link: u32,
        topic: u32,
        epoch: u64,
        seq: u64,
        payload: &[u8],
    ) -> Result<Option<(EgressRoute, WireHeader)>, Status> {
        if self.down_links.contains(&link) {
            return Ok(None); // I-RT6: link cut takes only its routes offline
        }
        let Some(route) = self.route(link, topic) else {
            return Ok(None);
        };
        if !route.remote_consumers {
            return Ok(None); // I-RT1: local-only topics never egress
        }
        let Ok(payload_len) = u32::try_from(payload.len()) else {
            return Err(Status::InvalidArg);
        };
        let header = WireHeader {
            flags: 0,
            wire_channel_id: route.wire_channel_id,
            stream_epoch: epoch,
            source_seq: seq,
            payload_len,
        };
        Ok(Some((route, header)))
    }

    /// Ingests one decoded frame for `link`: a down link (I-RT6) drops the frame
    /// with no state change; otherwise validates exact length (I-RT14) and the
    /// generated receive bound before any state mutation (I-RT18), classifies
    /// per-`(link, wire_channel_id)` ordering (I-RT9), strips the wire headers,
    /// and deposits the native payload to `sink` (I-RT4).
    ///
    /// # Errors
    /// [`Status::InvalidData`] on a length mismatch or receive-bound violation
    /// (no state changes, I-RT14); the sink's status after state advances.
    /// REQ: REQ-ROUTE-011, REQ-ROUTE-031, REQ-ROUTE-038, REQ-ROUTE-007.
    pub fn ingress_frame<S: ImportSink>(
        &mut self,
        link: u32,
        header: &WireHeader,
        payload: &[u8],
        sink: &mut S,
    ) -> Result<IngressOutcome, Status> {
        if self.down_links.contains(&link) {
            return Ok(IngressOutcome::LinkDown); // I-RT6: no ordering state touched
        }
        if header.payload_len as usize != payload.len() {
            return Err(Status::InvalidData); // I-RT14: before state mutation
        }
        let Some(route) = self.route_by_wire(link, header.wire_channel_id) else {
            return Ok(IngressOutcome::UnknownChannel);
        };
        if crate::wire::record_bound_violated(header.payload_len, route.max_record_bytes) {
            return Err(Status::InvalidData); // I-RT18: before any buffering
        }
        let key = StreamKey {
            epoch: header.stream_epoch,
            seq: header.source_seq,
        };
        let decision = self
            .ingress
            .entry((link, header.wire_channel_id))
            .or_default()
            .classify(key);
        match decision {
            IngressDecision::Duplicate => return Ok(IngressOutcome::Duplicate),
            IngressDecision::StaleEpoch => return Ok(IngressOutcome::StaleEpoch),
            IngressDecision::EpochReset => sink.reset_epoch(route.topic)?,
            IngressDecision::Accept => {}
        }
        if let Some(state) = self.ingress.get_mut(&(link, header.wire_channel_id)) {
            state.commit(key);
        }
        sink.deposit(route.topic, key.epoch, key.seq, payload)?;
        Ok(IngressOutcome::Imported)
    }

    /// Receives one frame from `adapter` into `buf` and ingests it. Returns
    /// `Ok(None)` when no frame is ready (non-blocking link).
    ///
    /// # Errors
    /// As [`Router::ingress_frame`] plus the adapter's receive error.
    /// REQ: REQ-ROUTE-032.
    pub fn ingress_udp<A: TransportAdapter, S: ImportSink>(
        &mut self,
        link: u32,
        adapter: &mut A,
        buf: &mut [u8],
        sink: &mut S,
    ) -> Result<Option<IngressOutcome>, Status> {
        let Some((header, payload)) = adapter.recv_frame(buf)? else {
            return Ok(None);
        };
        let outcome = self.ingress_frame(link, &header, payload, sink)?;
        Ok(Some(outcome))
    }
}
