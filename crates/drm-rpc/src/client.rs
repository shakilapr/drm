//! RPC client (`docs/algorithms/ops/rpc-actions.md` §Functions `call`,
//! `pump_events`). The client owns `request/<client_id>` and
//! `cancel/<client_id>` (single publisher each) and consumes the server's
//! ordered `event/<client_id>` stream, dispatching by the full correlation
//! identity so concurrent responses never cross-correlate (`I-RPC1`). A client
//! timeout emits [`RpcError::Timeout`] plus a best-effort `Cancel`, and the late
//! terminal is dropped as stale by cid (`I-RPC3`).
//! REQ: REQ-OPS-012, REQ-OPS-013, REQ-OPS-023, REQ-OPS-025.

use drm_core::channel::{Channel, ReadOutcome, ReaderState};
use drm_types::Status;

use crate::codec::{self, Cancel, Request, RpcEvent};
use crate::{CorrelationId, EventKind, Outcome, RpcError};

/// Bounded read retry budget passed to the core cursor (`cursor-delivery`
/// `I-C10`).
const RETRY_BUDGET: u32 = 4;

/// Parameters for [`RpcClient::call`].
/// REQ: REQ-OPS-023.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallConfig {
    /// Relative deadline budget in nanoseconds (`None` = no deadline).
    pub timeout_ns: Option<u64>,
    /// Current time in the caller's clock domain, nanoseconds.
    pub now_ns: u64,
    /// Maximum `server_pump` iterations before the call is abandoned as
    /// [`RpcError::Timeout`] (the bounded `park` in `call`).
    pub max_iterations: u32,
}

/// Per-cid waiter state machine (`I-RPC10`: `Accepted` before `Terminal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaiterState {
    AwaitingAccepted,
    AwaitingTerminal,
    Done,
}

/// One in-flight call's correlation bookkeeping.
struct Waiter {
    cid: CorrelationId,
    deadline_ns: Option<u64>,
    state: WaiterState,
    result: Option<Result<Vec<u8>, RpcError>>,
}

/// Dispatch decision for one received event, computed before mutating a waiter
/// so the borrow of the waiter table stays short.
enum Decision {
    Accept,
    Terminal(Outcome),
    Stale,
    Invalid,
}

/// Classifies `kind` against the waiter's per-cid state (`I-RPC10`).
/// REQ: REQ-OPS-025.
fn classify(state: WaiterState, kind: EventKind) -> Decision {
    match (state, kind) {
        (WaiterState::AwaitingTerminal, EventKind::Terminal(outcome)) => {
            Decision::Terminal(outcome)
        }
        // `Accepted` opens the waiter; any other lifecycle event
        // (Started/Progress/CancelAccepted/CancelRejected) is valid only before
        // `Terminal` and leaves the waiter awaiting it.
        (WaiterState::AwaitingAccepted, EventKind::Accepted)
        | (WaiterState::AwaitingTerminal, _) => Decision::Accept,
        (WaiterState::Done, _) => Decision::Stale,
        _ => Decision::Invalid,
    }
}

/// Builds the terminal result: `Ok(payload)` only for `Succeeded` + `Ok`.
/// REQ: REQ-OPS-010, REQ-OPS-025.
fn terminal_result(
    outcome: Outcome,
    status: Status,
    payload: Vec<u8>,
) -> Result<Vec<u8>, RpcError> {
    if outcome == Outcome::Succeeded && status == Status::Ok {
        Ok(payload)
    } else {
        Err(RpcError::Remote { outcome, status })
    }
}

/// Client endpoint over one correlated topic pair set. The client is the single
/// publisher of `request`/`cancel` and the required sequential consumer of
/// `event` (`I-RPC5`, `I-RPC4`).
/// REQ: REQ-OPS-012, REQ-OPS-013, REQ-OPS-023, REQ-OPS-025.
pub struct RpcClient<'a> {
    client_id: u32,
    session_nonce: u64,
    next_counter: u64,
    observed_session: u128,
    request: &'a Channel,
    cancel: &'a Channel,
    event: &'a Channel,
    event_state: ReaderState,
    event_epoch: u32,
    waiters: Vec<Option<Waiter>>,
    stale_event_count: u64,
    invalid_order_count: u64,
    write_buf: Vec<u8>,
    read_buf: Vec<u8>,
    cancel_buf: [u8; codec::CANCEL_FRAME],
}

impl<'a> RpcClient<'a> {
    /// Builds a client over an existing correlated topic pair set.
    /// `max_in_flight` fixes the correlation-table capacity (`I-RPC6`).
    /// REQ: REQ-OPS-012, REQ-OPS-021.
    ///
    /// # Errors
    /// [`RpcError::Status`]`(InvalidConfig)` when `max_in_flight` is zero or a
    /// channel payload bound exceeds the addressable range.
    pub fn new(
        client_id: u32,
        session_nonce: u64,
        request: &'a Channel,
        cancel: &'a Channel,
        event: &'a Channel,
        max_in_flight: usize,
    ) -> Result<Self, RpcError> {
        if max_in_flight == 0 {
            return Err(RpcError::Status(Status::InvalidConfig));
        }
        let write_buf = vec![
            0_u8;
            usize::try_from(request.sample_max())
                .map_err(|_| RpcError::Status(Status::InvalidConfig))?
        ];
        let read_buf = vec![
            0_u8;
            usize::try_from(event.sample_max())
                .map_err(|_| RpcError::Status(Status::InvalidConfig))?
        ];
        Ok(Self {
            client_id,
            session_nonce,
            next_counter: 0,
            observed_session: 0,
            request,
            cancel,
            event,
            event_state: event.reader(),
            event_epoch: event.epoch(),
            waiters: (0..max_in_flight).map(|_| None).collect(),
            stale_event_count: 0,
            invalid_order_count: 0,
            write_buf,
            read_buf,
            cancel_buf: [0_u8; codec::CANCEL_FRAME],
        })
    }

    /// Assigns the next correlation identity; the counter never wraps.
    /// REQ: REQ-OPS-012.
    ///
    /// # Errors
    /// [`RpcError::SequenceExhausted`] when the counter space is exhausted.
    pub fn next_cid(&mut self) -> Result<CorrelationId, RpcError> {
        let counter = self.next_counter;
        self.next_counter = counter.checked_add(1).ok_or(RpcError::SequenceExhausted)?;
        Ok(CorrelationId {
            client_id: self.client_id,
            session_nonce: self.session_nonce,
            counter,
        })
    }

    /// Reserves a correlation waiter, publishes a `Request` stamped with the
    /// observed server session and the relative budget, and returns the cid.
    /// REQ: REQ-OPS-012, REQ-OPS-013, REQ-OPS-021, REQ-OPS-023.
    ///
    /// # Errors
    /// [`RpcError::Busy`] when the correlation table is full or the request
    /// topic is gated (`QueueFull`); [`RpcError::SequenceExhausted`] on counter
    /// exhaustion; channel/publish errors otherwise.
    pub fn submit(
        &mut self,
        payload: &[u8],
        timeout_ns: Option<u64>,
        now_ns: u64,
    ) -> Result<CorrelationId, RpcError> {
        let slot = self
            .waiters
            .iter()
            .position(Option::is_none)
            .ok_or(RpcError::Busy)?;
        let cid = match self.next_cid() {
            Ok(cid) => cid,
            Err(e) => {
                if let Some(w) = self.waiters.get_mut(slot) {
                    *w = None;
                }
                return Err(e);
            }
        };
        let request = Request {
            correlation_id: cid,
            target_server_session: self.observed_session,
            remaining_budget_ns: timeout_ns,
            payload: payload.to_vec(),
        };
        let len = codec::encode_request(&request, &mut self.write_buf)?;
        if let Err(e) = self.request.publish(&self.write_buf[..len]) {
            if let Some(w) = self.waiters.get_mut(slot) {
                *w = None;
            }
            return Err(RpcError::from(e));
        }
        if let Some(w) = self.waiters.get_mut(slot) {
            *w = Some(Waiter {
                cid,
                deadline_ns: timeout_ns.map(|t| now_ns.saturating_add(t)),
                state: WaiterState::AwaitingAccepted,
                result: None,
            });
        }
        Ok(cid)
    }

    /// Drains the event topic, dispatches by cid (`I-RPC1`/`I-RPC10`), and marks
    /// expired waiters with [`RpcError::Timeout`] plus a best-effort `Cancel`
    /// (`I-RPC3`). An event-topic epoch change fails open calls with
    /// [`RpcError::PeerRestarted`] (`I-RPC9`).
    /// REQ: REQ-OPS-013, REQ-OPS-020, REQ-OPS-023, REQ-OPS-025.
    ///
    /// # Errors
    /// Channel read errors and malformed event frames.
    pub fn pump(&mut self, now_ns: u64) -> Result<usize, RpcError> {
        if self.event.epoch() != self.event_epoch {
            self.event_epoch = self.event.epoch();
            self.observed_session = 0;
            self.event_state = self.event.reader();
            self.fail_open(&RpcError::PeerRestarted);
        }
        loop {
            match self
                .event
                .try_next(&mut self.event_state, 0, &mut self.read_buf, RETRY_BUDGET)
            {
                Ok(ReadOutcome::Sample) => {
                    let ev = codec::decode_event(&self.read_buf)?;
                    self.dispatch(ev);
                }
                Ok(ReadOutcome::Gap { .. } | ReadOutcome::Empty | ReadOutcome::WouldRetry) => break,
                Err(e) => return Err(RpcError::from(e)),
            }
        }
        let expired: Vec<CorrelationId> = self
            .waiters
            .iter()
            .flatten()
            .filter(|w| {
                w.state != WaiterState::Done
                    && w.deadline_ns.is_some_and(|deadline| now_ns >= deadline)
            })
            .map(|w| w.cid)
            .collect();
        for cid in expired {
            self.publish_cancel(cid);
            if let Some(w) = self.find_waiter_mut(cid) {
                w.result = Some(Err(RpcError::Timeout));
                w.state = WaiterState::Done;
            }
        }
        Ok(self
            .waiters
            .iter()
            .flatten()
            .filter(|w| w.state == WaiterState::Done)
            .count())
    }

    /// Removes and returns the terminal result for `cid`, if the waiter has
    /// completed.
    /// REQ: REQ-OPS-013, REQ-OPS-023.
    #[must_use]
    pub fn take(&mut self, cid: CorrelationId) -> Option<Result<Vec<u8>, RpcError>> {
        let idx = self.waiters.iter().position(|w| {
            w.as_ref()
                .is_some_and(|w| w.cid == cid && w.state == WaiterState::Done)
        })?;
        let waiter = self.waiters.get_mut(idx)?.take()?;
        waiter.result
    }

    /// Synchronous `call`: submit → pump the event stream until the cid reaches
    /// `Terminal`, advancing the server through `server_pump` (`rpc-actions.md`
    /// §Functions `call`). Returns [`RpcError::Timeout`] after `max_iterations`
    /// or once the deadline elapses.
    /// REQ: REQ-OPS-012, REQ-OPS-023.
    ///
    /// # Errors
    /// As [`RpcClient::submit`]/[`RpcClient::pump`], or [`RpcError::Timeout`].
    pub fn call(
        &mut self,
        payload: &[u8],
        cfg: CallConfig,
        server_pump: &mut dyn FnMut() -> Result<(), RpcError>,
    ) -> Result<Vec<u8>, RpcError> {
        let cid = self.submit(payload, cfg.timeout_ns, cfg.now_ns)?;
        for _ in 0..cfg.max_iterations {
            server_pump()?;
            self.pump(cfg.now_ns)?;
            if let Some(result) = self.take(cid) {
                return result;
            }
        }
        self.publish_cancel(cid);
        if let Some(w) = self.find_waiter_mut(cid) {
            w.result = Some(Err(RpcError::Timeout));
            w.state = WaiterState::Done;
        }
        self.take(cid).unwrap_or(Err(RpcError::Timeout))
    }

    /// Number of in-flight waiters.
    /// REQ: REQ-OPS-021.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.waiters.iter().filter(|w| w.is_some()).count()
    }

    /// Server session most recently echoed by `Accepted` (`I-RPC9`).
    /// REQ: REQ-OPS-020.
    #[must_use]
    pub fn observed_session(&self) -> u128 {
        self.observed_session
    }

    /// Events dropped because no waiter matched (late terminal; `I-RPC3`).
    /// REQ: REQ-OPS-013, REQ-OPS-023.
    #[must_use]
    pub fn stale_event_count(&self) -> u64 {
        self.stale_event_count
    }

    /// Events rejected by the per-cid state machine (`I-RPC10`).
    /// REQ: REQ-OPS-025.
    #[must_use]
    pub fn invalid_order_count(&self) -> u64 {
        self.invalid_order_count
    }

    /// Publishes a best-effort `Cancel` for `cid` (failure is silent, matching
    /// `rpc-actions.md` §Edge Cases "Cancel topic full").
    /// REQ: REQ-OPS-011, REQ-OPS-023.
    fn publish_cancel(&mut self, cid: CorrelationId) {
        let cancel = Cancel {
            correlation_id: cid,
        };
        if let Ok(len) = codec::encode_cancel(&cancel, &mut self.cancel_buf) {
            let _ = self.cancel.publish(&self.cancel_buf[..len]);
        }
    }

    /// Applies one event to its waiter (`I-RPC1`/`I-RPC10`).
    /// REQ: REQ-OPS-013, REQ-OPS-025.
    fn dispatch(&mut self, ev: RpcEvent) {
        if matches!(ev.kind, EventKind::Accepted) {
            self.observed_session = ev.server_session;
        }
        let decision = match self
            .waiters
            .iter()
            .position(|w| w.as_ref().is_some_and(|w| w.cid == ev.correlation_id))
        {
            Some(idx) => match self.waiters.get(idx).and_then(Option::as_ref) {
                Some(w) => classify(w.state, ev.kind),
                None => Decision::Stale,
            },
            None => Decision::Stale,
        };
        match decision {
            Decision::Accept => {
                if let Some(Some(w)) = self
                    .waiters
                    .iter_mut()
                    .find(|w| w.as_ref().is_some_and(|w| w.cid == ev.correlation_id))
                {
                    w.state = WaiterState::AwaitingTerminal;
                }
            }
            Decision::Terminal(outcome) => {
                if let Some(Some(w)) = self
                    .waiters
                    .iter_mut()
                    .find(|w| w.as_ref().is_some_and(|w| w.cid == ev.correlation_id))
                {
                    w.result = Some(terminal_result(outcome, ev.status, ev.payload));
                    w.state = WaiterState::Done;
                }
            }
            Decision::Stale => self.stale_event_count = self.stale_event_count.saturating_add(1),
            Decision::Invalid => {
                self.invalid_order_count = self.invalid_order_count.saturating_add(1);
            }
        }
    }

    /// Mutable waiter lookup by full correlation identity.
    /// REQ: REQ-OPS-013.
    fn find_waiter_mut(&mut self, cid: CorrelationId) -> Option<&mut Waiter> {
        self.waiters.iter_mut().flatten().find(|w| w.cid == cid)
    }

    /// Marks every open waiter with `err` (`I-RPC9` epoch change).
    /// REQ: REQ-OPS-020.
    fn fail_open(&mut self, err: &RpcError) {
        for w in self.waiters.iter_mut().flatten() {
            if w.state != WaiterState::Done {
                w.result = Some(Err(err.clone()));
                w.state = WaiterState::Done;
            }
        }
    }
}
