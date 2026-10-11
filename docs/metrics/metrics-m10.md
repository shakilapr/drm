# Metrics — Month 10 (M10)

Normative source: [work-plan.md](../../work-plan.md) W4 Boundary Router &
Transports. One section per weekly tranche.

## M10W1 — Router UDP Path Completion, TCP Streams

| Variable | Target | Measured |
|---|---|---|
| `UDP_NO_FRAG` | one sample = one datagram; oversize never sent (I-RT2/I-RT3) | `cargo test -p drm-router route_filter`: 1/1 green (local-only topic emits zero frames; oversize refused before any send); `cargo run -p drm-router --example udp_pair`: 10 000 exchanges clean, 5.03 µs/exchange |
| `TCP_RECORDS` | length-framed records exact; reconnect session-scoped (I-RT5) | `cargo test -p drm-router transport::tcp`: 4/4 green — `tcp_partial_write_matrix`, `tcp_loopback_roundtrip`, `tcp_malformed_length_rejected`, `tcp_oversize_send_rejected`; `transport::session` reconnect lifecycle green (`tcp_reconnect_session_scope`) |
| `ROUTE_FILTER` | local-only topic never egresses (I-RT1) | `cargo test -p drm-router route_filter`: `route_filter` green; `Router::routes_on(link)` scan observed directly |
| `ROUTE_FIT` | V-RT rejects oversize datagram topic with an actionable stream-transport hint (REQ-ROUTE-018) | `cargo test -p drm-workspace verify`: `verify_engine` 12/12, `verify_negatives_v2` 8/8 (incl. `verify_negatives_v2_route_fit_actionable`) |
| `IMPORT_HOOKS` | import/export e2e through standard SHM publish path (I-RT4) | `cargo test -p drm-router shm_hooks`: `import_export_shm_hooks` green (header stripped; imported == local byte-for-byte) |
| `UDP_FUZZ` | malformed datagrams ⇒ error, no panic (PR smoke) | `tests/fuzz/fuzz_targets/datagram.rs` registered; CI `fuzz-pr` runs a 120 s datagram smoke |
| `GATE_STATUS(M10W1)` | PASS = all weekly boxes | PASS — TCP streams green, UDP no-frag enforced, lanes doc frozen, variables measured |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Route-table consumption `crates/drm-router/src/data_path.rs`: `Router::routes_on(link)`
  — the egress scan iterates only the link's route bindings (I-RT1), so a
  local-only topic (`remote_consumers == false`) or a topic bound to another
  link is never examined on that link.
- Route-filter suite `crates/drm-router/tests/route_filter.rs`: a recording
  adapter proves `routes_on(link)` isolation and that the local-only topic
  produces zero datagrams on the wire.
- SHM import hook `crates/drm-router/src/import.rs`: `ChannelSink` implements
  `ImportSink` over the ordinary `drm_core::channel::Channel::publish` path (no
  router-only privilege); a higher-epoch frame uses the publisher-lifecycle
  `restart_run` epoch bump. The ingress borrow is copied into the channel pool
  and released before return (transient-borrow rule).
- SHM hook e2e `crates/drm-router/tests/shm_hooks.rs`: export a live local
  channel sample over UDP, then import it into a second channel through the
  hook; the consumer of the imported channel reads the exact native bytes.
- Route-fit e2e `crates/drm-workspace/src/verify/engine.rs`: the `V-RT` datagram
  fit failure now names the canon-encoded payload size and directs the author to
  a stream transport (e.g. TCP), citing `REQ-ROUTE-018`; negative fixture
  `verify_negatives_v2_route_fit_actionable`.
- UDP datagram fuzz target `tests/fuzz/fuzz_targets/datagram.rs` (registered in
  `tests/fuzz/Cargo.toml`): wire-header decode, receive-bound probe, and
  multiplex demux are payload-or-error, never a panic.
- CI: `.github/workflows/ci.yml` `fuzz-pr` adds the datagram smoke;
  `.github/workflows/fuzz-nightly.yml` adds a `udp-pair` loopback matrix job.

Notes:

- `lanes-shaping.md` was frozen at M09W2 ([lanes-shaping.md](../algorithms/router/lanes-shaping.md)
  design freeze); the M10W1 plan's "new this week" note is superseded — no
  divergence was recorded, so no doc change landed.
- `ChannelSink` binds one topic to one channel; a router importing several
  topics holds one sink per topic. This mirrors the "synthetic publisher per
  imported topic" design in [router-core.md](../algorithms/router/router-core.md)
  §Protocols.
- The fuzz target is excluded from the workspace (as are the other targets), so
  the datagram smoke runs in CI under nightly, not in `cargo xtask test`.

## M10W2 — Router Import Path, Two-Host Integration Start

| Variable | Target | Measured |
|---|---|---|
| `IMPORT_NATIVE` | imported samples indistinguishable from local (I-RT4) | `cargo test -p drm-router import`: 4/4 green — `import_native_equivalence` (byte-identical + timing probe), `import_strips_wire_headers`, `import_epoch_reconnect_reset`, `import_sink_binds_channel` |
| `DISCONNECT_ISOLATION` | link cut ⇒ only remote topics affected (I-RT6) | `cargo test -p drm-router --test disconnect_isolation`: 2/2 green; `two_host_disconnect_isolation` green in the two-host sim |
| `TWO_HOST_SIM` | 2-host loopback sim exchanging all topic classes | `cargo xtask test --two-host`: 6/6 green (structured/udp, bulk/tcp, event/udp both directions; namespaces isolated; disconnect isolation; route e2e ×2) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Import completion `crates/drm-router/src/import.rs` + `crates/drm-router/src/data_path.rs`:
  `ChannelSink` republishes the header-stripped payload through the ordinary
  publish path (I-RT4); the router now bumps the synthetic importer epoch once
  per `(link, wire_channel_id)` after a reconnect (`Router::link_up`), mirroring
  `publisher_open` (I-RT9, publisher-lifecycle.md I-L4).
- Import suite `crates/drm-router/tests/import.rs`: native equivalence with a
  bytes + timing probe, wire-header stripping, and the epoch reconnect-reset
  path (a stale consumer resets and reads the post-reconnect sample).
- Publisher-lifecycle freshness `crates/drm-core/src/channel/freshness.rs`:
  `FreshnessTracker` / `StreamState` with monotonic-arrival `Active`/`Stale`/
  `Offline` transitions and epoch-change `NoCurrentSample` (I-L9).
- Two-host harness `tests/conformance/two_host.rs` (whole-system package
  `tests/conformance`, excluded from the root workspace like `tests/fuzz`):
  two router instances as distinct hosts (separate deployment namespaces +
  loopback addresses) exchange every topic class; parameterized by
  `TopicClass`.
- Route e2e `tests/conformance/route_e2e.rs`: derived forwarding-plan egress
  set equals the router filter set (I-RT1), a local-only topic never egresses,
  and the `V-RT` datagram bound agrees with the router's I-RT3 send-time refusal.
- Runner `xtask/src/two_host.rs` + `cargo xtask test --two-host`; nightly
  `.github/workflows/fuzz-nightly.yml` `two-host` job (matrix ×3).

Notes:

- `E2-T2 feasibility stubs`: the verify report's labeled `estimates` section is
  already plumbed and covered by `verify_engine::verify_boundary_labels`
  (I-V4, REQ-TOOL-059); the estimation math and the
  [feasibility-estimation.md](../algorithms/toolchain/feasibility-estimation.md)
  algorithm doc land M10W3.
- `tests/conformance` is a standalone package (not a workspace member) because
  it links the runtime and toolchain layers, which per-crate layering forbids
  (codebase.md §2.1); the whole-system suite is exactly where that crossing is
  allowed, matching the `tests/fuzz` precedent.
- Loopback sim ≠ real network (no NAT/MTU variance); the physical two-host gate
  stays M11W3 on real hosts (router-core.md §Protocols).
