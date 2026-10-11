//! RPC server reactor (`docs/algorithms/ops/rpc-actions.md` §Functions `serve`,
//! `poll_requests`, `run_unary`). The reactor polls cancels, admits requests by
//! reserving an operation slot **before** consuming (`I-RPC7`), and flushes
//! bounded outboxes; user code runs only in [`RpcServer::run_workers`] (the
//! bounded worker pool), never in the reactor. A random `server_session` is
//! regenerated per start and stale-session requests are dropped, never executed
//! (`I-RPC9`).
//! REQ: REQ-OPS-010, REQ-OPS-011, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022,
//! REQ-OPS-025.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use drm_core::channel::{Channel, ReadOutcome, ReaderState};
use drm_types::Status;

use crate::codec::{self, Cancel, Request, RpcEvent};
use crate::{CorrelationId, EventKind, Outcome, RpcError};

/// Bounded read retry budget passed to the core cursor (`cursor-delivery`
/// `I-C10`).
const RETRY_BUDGET: u32 = 4;

/// One admitted operation's execution contract. The M16W1 server runs unary
/// handlers; the action phase machine lands in M16W3.
/// REQ: REQ-OPS-010.
pub trait UnaryService {
    /// Executes one admitted request from owned bytes, returning the terminal
    /// status and payload. Runs on the bounded worker pool, never the reactor.
    /// REQ: REQ-OPS-010.
    fn handle(&mut self, request: &[u8]) -> (Status, Vec<u8>);
}

/// Bounded per-client outbox (`rpc-actions.md` §Functions outbox). Lifecycle and
/// terminal events are guaranteed (capacity reserved at admission); `Progress`
/// is advisory and replaceable (`I-RPC7`).
/// REQ: REQ-OPS-022.
pub struct Outbox {
    pending_progress: Option<RpcEvent>,
    guaranteed: VecDeque<RpcEvent>,
    capacity: usize,
}

impl Outbox {
    /// Builds an outbox with a fixed guaranteed-event capacity.
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            pending_progress: None,
            guaranteed: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Commits an event. `Progress` replaces the pending advisory slot;
    /// lifecycle/terminal events append to the guaranteed queue. Returns `false`
    /// only on a guaranteed-capacity overflow, which admission sizing prevents.
    /// REQ: REQ-OPS-022.
    pub fn commit(&mut self, ev: RpcEvent) -> bool {
        if ev.is_progress() {
            self.pending_progress = Some(ev);
            return true;
        }
        if ev.is_terminal() {
            self.pending_progress = None;
        }
        if self.guaranteed.len() >= self.capacity {
            return false;
        }
        self.guaranteed.push_back(ev);
        true
    }

    /// Publishes pending progress then guaranteed events in order; a full topic
    /// retains the remaining events for the next flush (core never blocks).
    /// REQ: REQ-OPS-022.
    ///
    /// # Errors
    /// [`RpcError::Status`]`(InvalidData)` if an event frame cannot be encoded.
    pub fn flush(&mut self, topic: &Channel, scratch: &mut [u8]) -> Result<usize, RpcError> {
        let mut published = 0;
        if let Some(progress) = self.pending_progress.take() {
            let len = codec::encode_event(&progress, scratch)?;
            if topic.publish(&scratch[..len]).is_ok() {
                published += 1;
            } else {
                self.pending_progress = Some(progress);
            }
        }
        while let Some(ev) = self.guaranteed.front() {
            let len = codec::encode_event(ev, scratch)?;
            if topic.publish(&scratch[..len]).is_err() {
                break;
            }
            self.guaranteed.pop_front();
            published += 1;
        }
        Ok(published)
    }

    /// Queued guaranteed events (advisory progress excluded).
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub fn len(&self) -> usize {
        self.guaranteed.len()
    }

    /// Whether the outbox holds no events.
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guaranteed.is_empty() && self.pending_progress.is_none()
    }
}

/// One client's correlated topic endpoints plus reader cursors and outbox.
/// The server is the required sequential consumer of `request`/`cancel` and the
/// single publisher of `event` (`I-RPC4`, `I-RPC5`).
/// REQ: REQ-OPS-012, REQ-OPS-021, REQ-OPS-022.
pub struct ClientLink<'a> {
    client_id: u32,
    request: &'a Channel,
    cancel: &'a Channel,
    event: &'a Channel,
    request_state: ReaderState,
    cancel_state: ReaderState,
    request_scratch: Vec<u8>,
    cancel_scratch: Vec<u8>,
    event_scratch: Vec<u8>,
    outbox: Outbox,
}

impl<'a> ClientLink<'a> {
    /// Builds a link over one client's request/cancel/event channels.
    /// REQ: REQ-OPS-012, REQ-OPS-021.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when a channel payload bound is unaddressable.
    pub fn new(
        client_id: u32,
        request: &'a Channel,
        cancel: &'a Channel,
        event: &'a Channel,
    ) -> Result<Self, Status> {
        let request_scratch =
            vec![0_u8; usize::try_from(request.sample_max()).map_err(|_| Status::InvalidConfig)?];
        let cancel_scratch =
            vec![0_u8; usize::try_from(cancel.sample_max()).map_err(|_| Status::InvalidConfig)?];
        let event_scratch =
            vec![0_u8; usize::try_from(event.sample_max()).map_err(|_| Status::InvalidConfig)?];
        Ok(Self {
            client_id,
            request,
            cancel,
            event,
            request_state: request.reader(),
            cancel_state: cancel.reader(),
            request_scratch,
            cancel_scratch,
            event_scratch,
            outbox: Outbox::new(0),
        })
    }

    /// Reads one request, or `None` when the topic is empty/gapped.
    /// REQ: REQ-OPS-012.
    ///
    /// # Errors
    /// Channel read errors and malformed request frames.
    fn read_request(&mut self) -> Result<Option<Request>, RpcError> {
        match self.request.try_next(
            &mut self.request_state,
            0,
            &mut self.request_scratch,
            RETRY_BUDGET,
        ) {
            Ok(ReadOutcome::Sample) => Ok(Some(codec::decode_request(&self.request_scratch)?)),
            Ok(ReadOutcome::Gap { .. } | ReadOutcome::Empty | ReadOutcome::WouldRetry) => Ok(None),
            Err(e) => Err(RpcError::from(e)),
        }
    }

    /// Reads one cancel, or `None` when the topic is empty/gapped.
    /// REQ: REQ-OPS-011.
    ///
    /// # Errors
    /// Channel read errors and malformed cancel frames.
    fn read_cancel(&mut self) -> Result<Option<Cancel>, RpcError> {
        match self.cancel.try_next(
            &mut self.cancel_state,
            0,
            &mut self.cancel_scratch,
            RETRY_BUDGET,
        ) {
            Ok(ReadOutcome::Sample) => Ok(Some(codec::decode_cancel(&self.cancel_scratch)?)),
            Ok(ReadOutcome::Gap { .. } | ReadOutcome::Empty | ReadOutcome::WouldRetry) => Ok(None),
            Err(e) => Err(RpcError::from(e)),
        }
    }
}

/// One admitted operation.
/// REQ: REQ-OPS-021.
struct Operation {
    cid: CorrelationId,
    client_id: u32,
    request: Vec<u8>,
    server_deadline_ns: Option<u64>,
    cancel_requested: bool,
}

/// Fixed-capacity operation table; a reservation includes the operation's
/// guaranteed terminal slot (`I-RPC6`, `I-RPC7`).
/// REQ: REQ-OPS-021.
struct OperationTable {
    slots: Vec<Option<Operation>>,
}

impl OperationTable {
    fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity).map(|_| None).collect(),
        }
    }

    fn try_reserve(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    fn place(&mut self, slot: usize, op: Operation) {
        if let Some(cell) = self.slots.get_mut(slot) {
            *cell = Some(op);
        }
    }

    fn take(&mut self, slot: usize) -> Option<Operation> {
        self.slots.get_mut(slot).and_then(Option::take)
    }

    fn release(&mut self, slot: usize) {
        if let Some(cell) = self.slots.get_mut(slot) {
            *cell = None;
        }
    }

    fn contains_cid(&self, cid: CorrelationId) -> bool {
        self.slots.iter().flatten().any(|op| op.cid == cid)
    }

    fn find_slot(&self, cid: CorrelationId) -> Option<usize> {
        self.slots
            .iter()
            .position(|op| op.as_ref().is_some_and(|op| op.cid == cid))
    }

    fn get_mut(&mut self, slot: usize) -> Option<&mut Operation> {
        self.slots.get_mut(slot).and_then(Option::as_mut)
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Operation> {
        self.slots.iter_mut().flatten()
    }

    fn in_use(&self) -> usize {
        self.slots.iter().filter(|op| op.is_some()).count()
    }
}

/// Server reactor over a set of client links. `S` is the unary service run on
/// the bounded worker pool.
/// REQ: REQ-OPS-010, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022.
pub struct RpcServer<'a, S: UnaryService> {
    session: u128,
    service: S,
    clients: Vec<ClientLink<'a>>,
    ops: OperationTable,
    ready: VecDeque<usize>,
    rr: usize,
    stale_request_count: u64,
    unknown_cancel_count: u64,
    duplicate_cid_count: u64,
    outbox_overflow_count: u64,
    served: u64,
}

impl<'a, S: UnaryService> RpcServer<'a, S> {
    /// Builds a server with an explicit `session` (tests) and a fixed operation
    /// table. Outbox capacity is sized from `max_operations` (`I-RPC6`).
    /// REQ: REQ-OPS-020, REQ-OPS-021, REQ-OPS-022.
    ///
    /// # Errors
    /// [`RpcError::Status`]`(InvalidConfig)` when `max_operations` is zero or
    /// `clients` is empty.
    pub fn new(
        session: u128,
        service: S,
        mut clients: Vec<ClientLink<'a>>,
        max_operations: usize,
    ) -> Result<Self, RpcError> {
        if max_operations == 0 || clients.is_empty() {
            return Err(RpcError::Status(Status::InvalidConfig));
        }
        let outbox_capacity = max_operations.saturating_mul(2);
        for link in &mut clients {
            link.outbox = Outbox::new(outbox_capacity);
        }
        Ok(Self {
            session,
            service,
            clients,
            ops: OperationTable::new(max_operations),
            ready: VecDeque::with_capacity(max_operations),
            rr: 0,
            stale_request_count: 0,
            unknown_cancel_count: 0,
            duplicate_cid_count: 0,
            outbox_overflow_count: 0,
            served: 0,
        })
    }

    /// Builds a server with a fresh random session (`I-RPC9`).
    /// REQ: REQ-OPS-020, REQ-OPS-021.
    ///
    /// # Errors
    /// As [`RpcServer::new`].
    pub fn with_random_session(
        service: S,
        clients: Vec<ClientLink<'a>>,
        max_operations: usize,
    ) -> Result<Self, RpcError> {
        Self::new(random_session(), service, clients, max_operations)
    }

    /// One reactor iteration: poll/apply cancels, admit requests
    /// (reserve-then-consume), inject deadline expiry, and flush outboxes. Never
    /// executes user code (`I-RPC7`).
    /// REQ: REQ-OPS-010, REQ-OPS-011, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022,
    /// REQ-OPS-023.
    ///
    /// # Errors
    /// Channel read errors and malformed frames.
    pub fn serve_step(&mut self, now_ns: u64) -> Result<(), RpcError> {
        self.poll_cancels()?;
        self.poll_requests(now_ns)?;
        self.expire_deadlines(now_ns);
        self.flush_outboxes()?;
        Ok(())
    }

    /// Runs the bounded worker pool once: executes each ready operation's
    /// handler (or `Cancelled` when the token was observed) and commits the
    /// terminal to its outbox (`rpc-actions.md` §Functions `run_unary`).
    /// REQ: REQ-OPS-010, REQ-OPS-022.
    pub fn run_workers(&mut self) -> usize {
        let mut ran = 0;
        while let Some(slot) = self.ready.pop_front() {
            let Some(op) = self.ops.take(slot) else {
                continue;
            };
            let cid = op.cid;
            let client_id = op.client_id;
            let terminal = if op.cancel_requested {
                self.event(
                    cid,
                    EventKind::Terminal(Outcome::Cancelled),
                    Status::Ok,
                    Vec::new(),
                )
            } else {
                let (status, payload) = self.service.handle(&op.request);
                let outcome = if status == Status::Ok {
                    Outcome::Succeeded
                } else {
                    Outcome::Aborted
                };
                self.event(cid, EventKind::Terminal(outcome), status, payload)
            };
            if let Some(link) = self.clients.iter_mut().find(|c| c.client_id == client_id)
                && !link.outbox.commit(terminal)
            {
                self.outbox_overflow_count = self.outbox_overflow_count.saturating_add(1);
            }
            self.served = self.served.saturating_add(1);
            ran += 1;
        }
        ran
    }

    /// Builds an event stamped with this server's session.
    /// REQ: REQ-OPS-020, REQ-OPS-025.
    fn event(
        &self,
        cid: CorrelationId,
        kind: EventKind,
        status: Status,
        payload: Vec<u8>,
    ) -> RpcEvent {
        RpcEvent {
            correlation_id: cid,
            kind,
            server_session: self.session,
            status,
            payload,
        }
    }

    /// Polls every cancel topic and applies observed tokens (`I-RPC2`).
    /// REQ: REQ-OPS-011.
    ///
    /// # Errors
    /// Channel read errors and malformed cancel frames.
    fn poll_cancels(&mut self) -> Result<(), RpcError> {
        for i in 0..self.clients.len() {
            loop {
                let cancel = match self.clients[i].read_cancel() {
                    Ok(Some(cancel)) => cancel,
                    Ok(None) => break,
                    Err(e) => return Err(e),
                };
                self.apply_cancel(cancel);
            }
        }
        Ok(())
    }

    /// Applies one cancel: non-terminal op ⇒ set token + `CancelAccepted`;
    /// terminal/unknown cid ⇒ drop + counter (`I-RPC10`).
    /// REQ: REQ-OPS-011, REQ-OPS-025.
    fn apply_cancel(&mut self, cancel: Cancel) {
        let cid = cancel.correlation_id;
        let Some(slot) = self.ops.find_slot(cid) else {
            self.unknown_cancel_count = self.unknown_cancel_count.saturating_add(1);
            return;
        };
        let mut first = false;
        if let Some(op) = self.ops.get_mut(slot)
            && !op.cancel_requested
        {
            op.cancel_requested = true;
            first = true;
        }
        if first {
            let ev = self.event(cid, EventKind::CancelAccepted, Status::Ok, Vec::new());
            if let Some(link) = self
                .clients
                .iter_mut()
                .find(|c| c.client_id == cid.client_id)
                && !link.outbox.commit(ev)
            {
                self.outbox_overflow_count = self.outbox_overflow_count.saturating_add(1);
            }
        }
    }

    /// Deterministic round-robin admission: reserve an operation slot, then
    /// consume one request per client; a stale session or duplicate cid is
    /// dropped + counted (`I-RPC7`, `I-RPC9`).
    /// REQ: REQ-OPS-020, REQ-OPS-021, REQ-OPS-023.
    ///
    /// # Errors
    /// Channel read errors and malformed request frames.
    fn poll_requests(&mut self, now_ns: u64) -> Result<(), RpcError> {
        let n = self.clients.len();
        for k in 0..n {
            let i = (self.rr + k) % n;
            let Some(slot) = self.ops.try_reserve() else {
                break;
            };
            let client_id = self.clients[i].client_id;
            let request = match self.clients[i].read_request() {
                Ok(Some(request)) => request,
                Ok(None) => {
                    self.ops.release(slot);
                    continue;
                }
                Err(e) => {
                    self.ops.release(slot);
                    return Err(e);
                }
            };
            if request.target_server_session != 0 && request.target_server_session != self.session {
                self.ops.release(slot);
                self.stale_request_count = self.stale_request_count.saturating_add(1);
                continue;
            }
            if self.ops.contains_cid(request.correlation_id) {
                self.ops.release(slot);
                self.duplicate_cid_count = self.duplicate_cid_count.saturating_add(1);
                continue;
            }
            let cid = request.correlation_id;
            let deadline = request
                .remaining_budget_ns
                .map(|budget| now_ns.saturating_add(budget));
            self.ops.place(
                slot,
                Operation {
                    cid,
                    client_id,
                    request: request.payload,
                    server_deadline_ns: deadline,
                    cancel_requested: false,
                },
            );
            let accepted = self.event(cid, EventKind::Accepted, Status::Ok, Vec::new());
            if !self.clients[i].outbox.commit(accepted) {
                self.outbox_overflow_count = self.outbox_overflow_count.saturating_add(1);
            }
            self.ready.push_back(slot);
        }
        if n != 0 {
            self.rr = (self.rr + 1) % n;
        }
        Ok(())
    }

    /// Marks operations whose relative budget elapsed (`I-RPC3`: the server
    /// derives a deadline in its own clock domain).
    /// REQ: REQ-OPS-023.
    fn expire_deadlines(&mut self, now_ns: u64) {
        for op in self.ops.iter_mut() {
            if !op.cancel_requested
                && op
                    .server_deadline_ns
                    .is_some_and(|deadline| now_ns >= deadline)
            {
                op.cancel_requested = true;
            }
        }
    }

    /// Flushes every client outbox (guaranteed order preserved).
    /// REQ: REQ-OPS-022.
    ///
    /// # Errors
    /// Event encode errors.
    fn flush_outboxes(&mut self) -> Result<(), RpcError> {
        for link in &mut self.clients {
            link.outbox.flush(link.event, &mut link.event_scratch)?;
        }
        Ok(())
    }

    /// The server session identity (`I-RPC9`).
    /// REQ: REQ-OPS-020.
    #[must_use]
    pub fn session(&self) -> u128 {
        self.session
    }

    /// Requests dropped for a stale session (`I-RPC9`).
    /// REQ: REQ-OPS-020.
    #[must_use]
    pub fn stale_request_count(&self) -> u64 {
        self.stale_request_count
    }

    /// Cancels for terminal/unknown cids (`I-RPC10`).
    /// REQ: REQ-OPS-011, REQ-OPS-025.
    #[must_use]
    pub fn unknown_cancel_count(&self) -> u64 {
        self.unknown_cancel_count
    }

    /// Duplicate-cid requests dropped (bug guard; `I-RPC1`).
    /// REQ: REQ-OPS-013.
    #[must_use]
    pub fn duplicate_cid_count(&self) -> u64 {
        self.duplicate_cid_count
    }

    /// Guaranteed-outbox overflows (should stay zero; `I-RPC7`).
    /// REQ: REQ-OPS-022.
    #[must_use]
    pub fn outbox_overflow_count(&self) -> u64 {
        self.outbox_overflow_count
    }

    /// Operations executed by the worker pool.
    /// REQ: REQ-OPS-010.
    #[must_use]
    pub fn served(&self) -> u64 {
        self.served
    }

    /// Operation-table occupancy.
    /// REQ: REQ-OPS-021.
    #[must_use]
    pub fn operations_in_use(&self) -> usize {
        self.ops.in_use()
    }

    /// Number of ready (scheduled, not yet executed) operations.
    /// REQ: REQ-OPS-021.
    #[must_use]
    pub fn ready(&self) -> usize {
        self.ready.len()
    }
}

/// Generates a random-ish `u128` server session without external entropy: wall
/// clock nanoseconds mixed with a process-local counter via FNV-1a. Never zero.
/// REQ: REQ-OPS-020.
fn random_session() -> u128 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut bytes = [0_u8; 24];
    bytes[..16].copy_from_slice(&nanos.to_le_bytes());
    bytes[16..].copy_from_slice(&seq.to_le_bytes());
    let mut lo = 0xcbf2_9ce4_8422_2325_u64;
    let mut hi = 0x9e37_79b9_7f4a_7c15_u64;
    for &b in &bytes {
        lo ^= u64::from(b);
        lo = lo.wrapping_mul(0x0000_0100_0000_01B3);
        hi ^= u64::from(b);
        hi = hi.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let raw = (u128::from(hi) << 64) | u128::from(lo);
    if raw == 0 { 1 } else { raw }
}
