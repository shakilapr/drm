# Phase 29 Gate — Boundary Router & Transports

Chaired at M11W4 (E3 owns the router; evidence by E2/E4; E3 presents). Evidence
is executable and stays green under the permanent gates. Verdict: **PASS**
(non-physical lines evidenced; the physical two-host line is scheduled at M11W3
on real hosts, `PHYS_PREP` carried from [work-plan-m10-w3.md](../work-plan/work-plan-m10-w3.md)).

Implementation: `crates/drm-router` (framing, transports, data path, import),
`crates/drm-core::channel` (synthetic-publisher publish path), `crates/drm-workspace`
(verify `V-RT`). Algorithm: [router-core.md](../algorithms/router/router-core.md).
Owner: E3. Gate chair: E3.

## Criteria ⇄ evidence

| Criterion (work-plan #29) | Invariant | Artifact | Test | Result |
|---|---|---|---|---|
| Two physical hosts exchange every supported topic class | I-RT1/I-RT4 | two-host sim (`tests/conformance/two_host.rs`); physical run M11W3 | `cargo xtask test --two-host` | sim PASS; physical scheduled M11W3 |
| UDP never uses DRM fragmentation; oversize UDP sample rejected by verification | I-RT2/I-RT3 | `udp.rs` datagram bound; verify `V-RT` | `route_filter`, `route_e2e_oversize_agrees`, `verify_negatives_v2` | PASS |
| TCP reconstructs exact samples | I-RT5 | `transport/tcp.rs` length-framed records | `transport::tcp::tcp_loopback_roundtrip`, `framing_record` | PASS |
| Received external data indistinguishable from local topic data | I-RT4 | `import.rs` `ChannelSink`; `import` suite | `import_native_equivalence` (bytes + timing) | PASS |
| Disconnect affects remote routes only | I-RT6 | `data_path.rs` link down/up | `disconnect_isolation`, `two_host_disconnect_isolation` | PASS |

## Supporting evidence

| Case | Invariant | Test | Result |
|---|---|---|---|
| Wire header exact 28-byte LE layout; transport id not a handle | I-RT7 | `wire::wire_header_layout`, `wire::wire_id_not_handle` | PASS |
| Ingress ordering: reorder/duplicate drop; restart reset (epoch max → 0) | I-RT9 | `wire::udp_reorder_duplicate`, `wire::remote_restart_reset`, `wire::epoch_max_then_zero_is_stale` | PASS |
| Import epoch reconnect-reset (synthetic `publisher_open`) | I-RT4/I-RT9 | `import::import_epoch_reconnect_reset` | PASS |
| Receive bound enforced before buffering | I-RT18 | `router_core::ingress_rejects_before_state` | PASS |
| Serial COBS/CRC32C route integration | I-SR1..I-SR6 | `serial_route` | PASS |
| Freshness transitions (`Active`/`Stale`/`Offline`, epoch → `NoCurrentSample`) | I-L9 | `channel::freshness` | PASS |
| Route e2e: derived egress set == router filter set | I-RT1 | `route_e2e::route_e2e_plan_matches_router_filter` | PASS |

## How to reproduce

```text
cargo test -p drm-router import
cargo test -p drm-router --test disconnect_isolation
cargo test -p drm-router route_filter
cargo test -p drm-router transport::tcp
cargo test -p drm-router serial_route
cargo test -p drm-core --lib freshness
cargo test -p drm-workspace --test verify_negatives_v2
cargo xtask test --two-host
```

## Notes

- The two-host harness is an in-process loopback simulation (two router
  instances, separate deployment namespaces and loopback addresses); loopback
  sim ≠ real network (no NAT/MTU variance), so the physical two-host line stays
  scheduled for M11W3 on real hosts ([router-core.md](../algorithms/router/router-core.md)
  §Protocols).
- `tests/conformance` is a standalone package excluded from the root workspace
  (it links runtime and toolchain layers); it is driven by
  `cargo xtask test --two-host`, not by `cargo xtask test`.
