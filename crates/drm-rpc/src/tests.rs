//! `drm-rpc` suite (`docs/algorithms/ops/rpc-actions.md` §Test Matrix;
//! §Invariants I-RPC1/I-RPC3/I-RPC7/I-RPC9/I-RPC10). M16W1 covers unary
//! request/response, cid dispatch, deadlines, and session identity.
//! REQ: REQ-OPS-010, REQ-OPS-011, REQ-OPS-012, REQ-OPS-013, REQ-OPS-020,
//! REQ-OPS-021, REQ-OPS-022, REQ-OPS-023, REQ-OPS-025.

use drm_core::channel::{Channel, OverflowPolicy};
use drm_types::Status;

use crate::client::{CallConfig, RpcClient};
use crate::codec::{self, Request, RpcEvent};
use crate::server::{ClientLink, RpcServer, UnaryService};
use crate::{CorrelationId, EventKind, Outcome, RpcError};

const SAMPLE_MAX: usize = 128;

/// A `reject` topic with one required sequential consumer (the declared RPC
/// topic shape; `rpc-design-seed.md` §1).
/// REQ: REQ-OPS-012, REQ-OPS-021.
fn topic() -> Channel {
    Channel::activate(
        8,
        9,
        u64::try_from(SAMPLE_MAX).unwrap_or(0),
        8,
        OverflowPolicy::Reject,
        1,
        2,
        None,
    )
    .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

/// Echoes the request payload back (unary success).
/// REQ: REQ-OPS-010.
struct Echo;

impl UnaryService for Echo {
    fn handle(&mut self, request: &[u8]) -> (Status, Vec<u8>) {
        (Status::Ok, request.to_vec())
    }
}

/// Reverses the request payload so distinct calls yield distinct responses.
/// REQ: REQ-OPS-013.
struct Reverse;

impl UnaryService for Reverse {
    fn handle(&mut self, request: &[u8]) -> (Status, Vec<u8>) {
        let mut out = request.to_vec();
        out.reverse();
        (Status::Ok, out)
    }
}

/// Returns a non-success status (terminal `Aborted`).
/// REQ: REQ-OPS-010.
struct Failing;

impl UnaryService for Failing {
    fn handle(&mut self, _request: &[u8]) -> (Status, Vec<u8>) {
        (Status::InvalidArg, Vec::new())
    }
}

/// REQ: REQ-OPS-012, REQ-OPS-020, REQ-OPS-023 — the unary `call`/`serve`
/// round-trip over correlated topic pairs: `Accepted` then `Terminal`; the
/// client observes the server session echo.
#[test]
fn call_round_trip() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut client = RpcClient::new(7, 0xABCD, &request, &cancel, &event, 4)
        .unwrap_or_else(|e| unreachable!("client: {e:?}"));
    let mut server = RpcServer::new(
        0x1234_5678,
        Echo,
        vec![
            ClientLink::new(7, &request, &cancel, &event)
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        ],
        4,
    )
    .unwrap_or_else(|e| unreachable!("server: {e:?}"));

    let cfg = CallConfig {
        timeout_ns: Some(1_000_000),
        now_ns: 0,
        max_iterations: 8,
    };
    let mut pump = || {
        server.serve_step(0)?;
        server.run_workers();
        Ok(())
    };
    let reply = client
        .call(b"hello", cfg, &mut pump)
        .unwrap_or_else(|e| unreachable!("call: {e:?}"));
    assert_eq!(reply.as_slice(), b"hello");
    assert_eq!(client.observed_session(), server.session());
    assert_eq!(server.served(), 1);
    assert_eq!(client.in_flight(), 0);
}

/// REQ: REQ-OPS-010 — a non-success handler surfaces as `Terminal(Aborted)` and
/// [`RpcError::Remote`] with the mapped status.
#[test]
fn call_remote_failure() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut client = RpcClient::new(1, 9, &request, &cancel, &event, 2)
        .unwrap_or_else(|e| unreachable!("client: {e:?}"));
    let mut server = RpcServer::new(
        42,
        Failing,
        vec![
            ClientLink::new(1, &request, &cancel, &event)
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        ],
        2,
    )
    .unwrap_or_else(|e| unreachable!("server: {e:?}"));
    let cfg = CallConfig {
        timeout_ns: None,
        now_ns: 0,
        max_iterations: 8,
    };
    let mut pump = || {
        server.serve_step(0)?;
        server.run_workers();
        Ok(())
    };
    let err = client
        .call(b"x", cfg, &mut pump)
        .err()
        .unwrap_or_else(|| unreachable!("expected remote error"));
    assert_eq!(
        err,
        RpcError::Remote {
            outcome: Outcome::Aborted,
            status: Status::InvalidArg
        }
    );
}

/// REQ: REQ-OPS-013 — I-RPC1: with C ∈ 1..=4 concurrent calls, every response is
/// delivered to exactly its requesting client; responses never cross-correlate.
#[test]
fn dispatch_concurrent_no_cross_correlation() {
    let clients_n = 4_u32;
    let mut requests: Vec<Channel> = Vec::new();
    let mut cancels: Vec<Channel> = Vec::new();
    let mut events: Vec<Channel> = Vec::new();
    for _ in 0..clients_n {
        requests.push(topic());
        cancels.push(topic());
        events.push(topic());
    }
    let mut clients: Vec<RpcClient<'_>> = Vec::new();
    let mut links: Vec<ClientLink<'_>> = Vec::new();
    for i in 0..clients_n {
        let idx = usize::try_from(i).unwrap_or(0);
        clients.push(
            RpcClient::new(i, 1, &requests[idx], &cancels[idx], &events[idx], 4)
                .unwrap_or_else(|e| unreachable!("client: {e:?}")),
        );
        links.push(
            ClientLink::new(i, &requests[idx], &cancels[idx], &events[idx])
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        );
    }
    let mut server =
        RpcServer::new(0xBEEF, Reverse, links, 4).unwrap_or_else(|e| unreachable!("server: {e:?}"));

    let mut cids: Vec<CorrelationId> = Vec::new();
    for (i, client) in clients.iter_mut().enumerate() {
        let byte = u8::try_from(i + 1).unwrap_or(1);
        let cid = client
            .submit(&[byte, byte, byte], Some(1_000_000), 0)
            .unwrap_or_else(|e| unreachable!("submit: {e:?}"));
        cids.push(cid);
    }
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("serve: {e:?}"));
    assert_eq!(server.ready(), usize::try_from(clients_n).unwrap_or(0));
    server.run_workers();
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("flush: {e:?}"));

    for (i, client) in clients.iter_mut().enumerate() {
        client
            .pump(0)
            .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
        let reply = client
            .take(cids[i])
            .unwrap_or_else(|| unreachable!("reply missing for client {i}"))
            .unwrap_or_else(|e| unreachable!("reply err: {e:?}"));
        let byte = u8::try_from(i + 1).unwrap_or(1);
        assert_eq!(reply, vec![byte, byte, byte], "client {i} own response");
        assert_eq!(client.stale_event_count(), 0);
    }
    assert_eq!(server.served(), u64::from(clients_n));
}

/// REQ: REQ-OPS-023 — I-RPC3: a client deadline elapses ⇒ [`RpcError::Timeout`]
/// plus a best-effort `Cancel`; the late terminal is dropped as stale by cid and
/// the server counts the unknown-cid cancel.
#[test]
fn dispatch_timeout_expires_and_late_terminal_stale() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut client = RpcClient::new(3, 1, &request, &cancel, &event, 4)
        .unwrap_or_else(|e| unreachable!("client: {e:?}"));
    let mut server = RpcServer::new(
        0x77,
        Echo,
        vec![
            ClientLink::new(3, &request, &cancel, &event)
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        ],
        4,
    )
    .unwrap_or_else(|e| unreachable!("server: {e:?}"));

    let cid = client
        .submit(b"slow", Some(100), 0)
        .unwrap_or_else(|e| unreachable!("submit: {e:?}"));
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("admit: {e:?}"));
    client
        .pump(0)
        .unwrap_or_else(|e| unreachable!("pump: {e:?}"));

    // Deadline elapses: the waiter completes with Timeout and a Cancel is sent.
    client
        .pump(200)
        .unwrap_or_else(|e| unreachable!("expire: {e:?}"));
    let err = client
        .take(cid)
        .unwrap_or_else(|| unreachable!("timeout result missing"))
        .err()
        .unwrap_or_else(|| unreachable!("expected timeout"));
    assert_eq!(err, RpcError::Timeout);

    // The handler still runs and its terminal arrives late.
    server.run_workers();
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("late flush: {e:?}"));
    client
        .pump(0)
        .unwrap_or_else(|e| unreachable!("late pump: {e:?}"));
    assert_eq!(client.stale_event_count(), 1, "late terminal dropped stale");
    assert_eq!(
        server.unknown_cancel_count(),
        1,
        "cancel for a terminal cid"
    );
}

/// REQ: REQ-OPS-021 — I-RPC7: admission reserves an operation slot before
/// consuming; with a full table nothing more is consumed (`publish ≠ accepted`).
#[test]
fn dispatch_admission_busy() {
    let r0 = topic();
    let c0 = topic();
    let e0 = topic();
    let r1 = topic();
    let c1 = topic();
    let e1 = topic();
    let mut clients = [
        RpcClient::new(0, 1, &r0, &c0, &e0, 2).unwrap_or_else(|e| unreachable!("c0: {e:?}")),
        RpcClient::new(1, 1, &r1, &c1, &e1, 2).unwrap_or_else(|e| unreachable!("c1: {e:?}")),
    ];
    let links = vec![
        ClientLink::new(0, &r0, &c0, &e0).unwrap_or_else(|e| unreachable!("l0: {e:?}")),
        ClientLink::new(1, &r1, &c1, &e1).unwrap_or_else(|e| unreachable!("l1: {e:?}")),
    ];
    let mut server =
        RpcServer::new(5, Echo, links, 1).unwrap_or_else(|e| unreachable!("server: {e:?}"));

    for (i, client) in clients.iter_mut().enumerate() {
        let byte = u8::try_from(i).unwrap_or(0);
        let _ = client
            .submit(&[byte], None, 0)
            .unwrap_or_else(|e| unreachable!("submit: {e:?}"));
    }
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("serve: {e:?}"));
    assert_eq!(server.operations_in_use(), 1, "one slot reserved");
    assert_eq!(server.ready(), 1, "one operation scheduled");
    assert_eq!(
        r1.write_seq(),
        1,
        "second request stays published, never consumed without a slot"
    );
}

/// REQ: REQ-OPS-020 — I-RPC9: a request carrying a stale non-zero session is
/// dropped and counted, never executed; a session-0 request is admitted.
#[test]
fn server_session_stale_dropped() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut server = RpcServer::new(
        0xDEAD_BEEF,
        Echo,
        vec![
            ClientLink::new(0, &request, &cancel, &event)
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        ],
        4,
    )
    .unwrap_or_else(|e| unreachable!("server: {e:?}"));
    let cid = CorrelationId {
        client_id: 0,
        session_nonce: 1,
        counter: 0,
    };
    let stale = Request {
        correlation_id: cid,
        target_server_session: 0x1234,
        remaining_budget_ns: None,
        payload: b"stale".to_vec(),
    };
    let mut buf = [0_u8; SAMPLE_MAX];
    let n =
        codec::encode_request(&stale, &mut buf).unwrap_or_else(|e| unreachable!("encode: {e:?}"));
    assert!(request.publish(&buf[..n]).is_ok());
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("serve: {e:?}"));
    assert_eq!(server.stale_request_count(), 1);
    assert_eq!(server.served(), 0, "stale request never executed");
    assert_eq!(server.operations_in_use(), 0);
}

/// REQ: REQ-OPS-025 — I-RPC10: `Terminal` before `Accepted` is rejected by the
/// per-cid state machine and counted.
#[test]
fn event_order_invalid_terminal_first() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut client = RpcClient::new(0, 1, &request, &cancel, &event, 2)
        .unwrap_or_else(|e| unreachable!("client: {e:?}"));
    let cid = client
        .submit(b"q", None, 0)
        .unwrap_or_else(|e| unreachable!("submit: {e:?}"));
    let bad = RpcEvent {
        correlation_id: cid,
        kind: EventKind::Terminal(Outcome::Succeeded),
        server_session: 0,
        status: Status::Ok,
        payload: b"too soon".to_vec(),
    };
    let mut buf = [0_u8; SAMPLE_MAX];
    let n = codec::encode_event(&bad, &mut buf).unwrap_or_else(|e| unreachable!("encode: {e:?}"));
    assert!(event.publish(&buf[..n]).is_ok());
    client
        .pump(0)
        .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
    assert_eq!(client.invalid_order_count(), 1);
    assert_eq!(client.stale_event_count(), 0);
}

/// REQ: REQ-OPS-011 — I-RPC2 subset: a cancel for a non-terminal operation sets
/// the token, commits `CancelAccepted`, and the worker terminal is
/// `Terminal(Cancelled)`.
#[test]
fn cancel_accepted_terminal_cancelled() {
    let request = topic();
    let cancel = topic();
    let event = topic();
    let mut client = RpcClient::new(2, 1, &request, &cancel, &event, 2)
        .unwrap_or_else(|e| unreachable!("client: {e:?}"));
    let mut server = RpcServer::new(
        0x55,
        Echo,
        vec![
            ClientLink::new(2, &request, &cancel, &event)
                .unwrap_or_else(|e| unreachable!("link: {e:?}")),
        ],
        2,
    )
    .unwrap_or_else(|e| unreachable!("server: {e:?}"));
    let cid = client
        .submit(b"work", None, 0)
        .unwrap_or_else(|e| unreachable!("submit: {e:?}"));
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("admit: {e:?}"));

    let frame = codec::Cancel {
        correlation_id: cid,
    };
    let mut buf = [0_u8; SAMPLE_MAX];
    let n =
        codec::encode_cancel(&frame, &mut buf).unwrap_or_else(|e| unreachable!("encode: {e:?}"));
    assert!(cancel.publish(&buf[..n]).is_ok());
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("cancel poll: {e:?}"));
    assert_eq!(server.unknown_cancel_count(), 0);
    server.run_workers();
    server
        .serve_step(0)
        .unwrap_or_else(|e| unreachable!("flush: {e:?}"));

    client
        .pump(0)
        .unwrap_or_else(|e| unreachable!("pump: {e:?}"));
    let err = client
        .take(cid)
        .unwrap_or_else(|| unreachable!("terminal missing"))
        .err()
        .unwrap_or_else(|| unreachable!("expected cancelled"));
    assert_eq!(
        err,
        RpcError::Remote {
            outcome: Outcome::Cancelled,
            status: Status::Ok
        }
    );
    assert_eq!(
        client.invalid_order_count(),
        0,
        "lifecycle events are ordered"
    );
}

/// REQ: REQ-OPS-012, REQ-OPS-020, REQ-OPS-023, REQ-OPS-025 — codec round-trip
/// for every frame kind; truncation is rejected.
#[test]
fn codec_round_trip() {
    let cid = CorrelationId {
        client_id: 5,
        session_nonce: 0x1122_3344,
        counter: 9,
    };
    let req = Request {
        correlation_id: cid,
        target_server_session: 0xAA,
        remaining_budget_ns: Some(777),
        payload: b"payload".to_vec(),
    };
    let mut buf = [0_u8; SAMPLE_MAX];
    let n = codec::encode_request(&req, &mut buf).unwrap_or_else(|e| unreachable!("req: {e:?}"));
    assert_eq!(
        codec::decode_request(&buf[..n]).unwrap_or_else(|e| unreachable!("req dec: {e:?}")),
        req
    );
    assert_eq!(
        codec::decode_request(&buf[..n - 1]).err(),
        Some(Status::InvalidData)
    );

    let cancel = codec::Cancel {
        correlation_id: cid,
    };
    let n =
        codec::encode_cancel(&cancel, &mut buf).unwrap_or_else(|e| unreachable!("cancel: {e:?}"));
    assert_eq!(
        codec::decode_cancel(&buf[..n]).unwrap_or_else(|e| unreachable!("cancel dec: {e:?}")),
        cancel
    );

    let ev = RpcEvent {
        correlation_id: cid,
        kind: EventKind::Terminal(Outcome::Succeeded),
        server_session: 0x99,
        status: Status::Ok,
        payload: b"done".to_vec(),
    };
    let n = codec::encode_event(&ev, &mut buf).unwrap_or_else(|e| unreachable!("event: {e:?}"));
    assert_eq!(
        codec::decode_event(&buf[..n]).unwrap_or_else(|e| unreachable!("event dec: {e:?}")),
        ev
    );
    assert_eq!(
        codec::decode_event(&buf[..n - 1]).err(),
        Some(Status::InvalidData)
    );
}
