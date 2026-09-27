# Boundary Router Core Algorithms

Scope: `drm-router` data path — topic filtering from derived forwarding plans, router-owned copy before egress (external-I/O borrow rule), per-transport framing (UDP datagrams, reliable stream records, COBS/HDLC serial frames), two-way import with wire-header stripping, ingress ordering/dedup and remote-restart reset, session lifecycle, disconnect isolation. Implements architecture.md §The Boundary Router, §Transport Metadata. Owner: E3 (crate `drm-router`). Phase 29. Depends on [route-derivation.md](../toolchain/route-derivation.md), [lanes-shaping.md](lanes-shaping.md), [publisher-lifecycle.md](../runtime/publisher-lifecycle.md), [cursor-delivery.md](../runtime/cursor-delivery.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-router-filter` | transmits only topics with remote consumers on that link | — |
| `REQ-ROUTE-router-owned-copy` | egress copies into router-owned memory and releases the borrow before enqueue | — |
| `REQ-ROUTE-wire-header` | wire header = wire_channel_id, stream_epoch, source_seq, payload_len | — |
| `REQ-ROUTE-udp-single-packet` | one sample per datagram ≤ `max_datagram_payload`; no IP fragmentation/PMTUD | — |
| `REQ-ROUTE-tcp-records` | stream transports: one length-framed record per sample | — |
| `REQ-ROUTE-serial-framing` | serial: COBS/HDLC delimiter + length + CRC32C | — |
| `REQ-ROUTE-ingress-ordering` | drop same-epoch `seq ≤ last`; epoch increase ⇒ stream reset | — |
| `REQ-ROUTE-two-way-import` | received frames → local SHM native format, headers stripped | — |
| `REQ-ROUTE-disconnect-isolation` | link loss affects only topics on that link | — |
| `REQ-ROUTE-oversize-build-reject` | oversize datagram payload rejected at build time onto stream transports | — |

## Data Structures

```rust
pub struct RouterConfig {
 plans: BTreeMap<TopicPath, TopicRoutePlan>, // route-derivation.md output (forwarding tree)
 transports: BTreeMap<LinkId, TransportAdapter>, // capability-mapped
 datagram_bound: BTreeMap<LinkId, u16>, // max_datagram_payload per datagram link
}

pub struct WireHeader {
 wire_channel_id: u32, // TRANSPORT id from the route binding — never the runtime topic handle
 stream_epoch: u32, // per-source stream epoch; increments on source/import restart
 source_seq: u64, // per-source sequence (distinct from the per-link transport seq)
 payload_len: u32,
}

pub enum WireFrame {
 Datagram { header: WireHeader, payload: OwnedFrame }, // UDP: one sample, router-owned bytes
 Record { header: WireHeader, payload: OwnedFrame }, // TCP: length-framed record
 Serial { header: WireHeader, payload: OwnedFrame }, // raw UART: COBS/HDLC framed
}
// OwnedFrame is a slice into a router-owned bounded buffer; it NEVER aliases DRM shared memory.
// No DRM-level fragmentation/reassembly — oversize belongs on stream transports (build-rejected).

pub struct IngressState { epoch: u32, last_seq: u64 } // per (link, wire_channel_id)
```

## Invariants

- **I-RT1 FilterExact**: egresses only topics with ≥ 1 remote consumer on that specific link; local-only topics never egress. Test: `route_filter`.
- **I-RT2 OneFrameOneSample**: a wire frame carries exactly one DRM sample (multiplexed small topics share a datagram bound; never split a sample). Test: `frame_sample_atomicity`.
- **I-RT3 NoOversizeSend**: datagram egress refuses payloads > `max_datagram_payload` (defense-in-depth behind build-time verify). Test: `udp_oversize_reject`.
- **I-RT4 ImportNative**: ingress strips wire headers and deposits the sample in native format — consumers cannot distinguish imported from local data. Test: `import_native_equivalence`.
- **I-RT5 SessionScope**: `reliable` = ordered loss recovery while the transport session is valid; reconnection starts clean. Test: `tcp_session_scope`.
- **I-RT6 DisconnectIsolated**: link loss affects only routes over that link; local SHM loops continue. Test: `disconnect_isolation`.
- **I-RT7 WireHeader**: every frame carries `{wire_channel_id, stream_epoch, source_seq, payload_len}`; `wire_channel_id` is the route's transport identifier, never the runtime topic handle. Test: `wire_header`, `wire_id_not_handle`.
- **I-RT8 RouterOwnsBytes**: egress performs `borrow → copy into router-owned bounded buffer → release`, then enqueues; no borrowed DRM sample outlives the borrow scope (external-I/O rule). Test: `router_borrow_scope` (instrumented; lane queue holds zero DRM borrows).
- **I-RT9 IngressOrdering**: for a given (link, wire_channel_id), a frame with the same epoch and `source_seq ≤ last_seq` is dropped; a higher epoch triggers a stream reset (imported publisher epoch bump). UDP reorder/duplicate can never make older state the new latest. Test: `udp_reorder`, `udp_duplicate`, `remote_restart_reset`.
- **I-RT10 SerialFraming**: serial frames use a delimiter + COBS encoding + explicit length + CRC32C, so a dropped byte cannot desynchronize the receiver indefinitely. Test: `serial_resync`.

## Functions

```rust
// ---- Egress (per link, per tick) ----
fn egress(&mut self, link: LinkId) -> Result<(), Status> {
 for topic in self.routes_on(link) { // I-RT1: forwarding plan filter
 // I-RT8: momentary borrow, copy to router-owned buffer, release BEFORE any queue/I/O
 let owned = self.momentary_copy(topic)?; // borrow_latest → copy → release hazard
 let header = WireHeader {
 wire_channel_id: self.wire_id(topic, link), // I-RT7: transport binding
 stream_epoch: self.stream_epoch(topic),
 source_seq: self.source_seq(topic).fetch_add(1, Relaxed),
 payload_len: owned.len() as u32,
 };
 let frame = match self.transport_kind(link) {
 Datagram => {
 if owned.len() > self.datagram_bound[link] as usize { return Err(OversizeInternal); } // I-RT3
 WireFrame::Datagram { header, payload: owned }
 }
 Stream => WireFrame::Record { header, payload: owned },
 Serial => WireFrame::Serial { header, payload: owned }, // I-RT10: COBS/HDLC framed below
 };
 self.lanes.enqueue(link, priority_class(topic), frame)?; // lanes-shaping.md
 }
 Ok(())
}

// ---- Ingress (per link) ----
fn ingress(&mut self, link: LinkId, frame: WireFrame) -> Result<(), Status> {
 let topic = self.topic_for(frame.header.wire_channel_id, link)?;
 let st = self.ingress_state(link, frame.header.wire_channel_id);

 if frame.header.stream_epoch == st.epoch {
 if frame.header.source_seq <= st.last_seq { return Ok(()); } // I-RT9: stale/duplicate
 } else if frame.header.stream_epoch > st.epoch {
 st.epoch = frame.header.stream_epoch;
 self.import_publisher(topic)?.reset_stream_epoch(); // remote restart ⇒ local reset
 } else {
 return Ok(()); // old epoch
 }
 st.last_seq = frame.header.source_seq;

 let sample = frame.payload; // already router-owned (I-RT8)
 self.import_publisher(topic)?.publish_copy(sample)?; // I-RT4: native payload only
 Ok(())
}
```

*Serial framing (I-RT10):* `delimiter · COBS{version, channel_id, epoch, sequence, payload_length, payload, CRC32C} · delimiter`. No capability flag: serial always uses this frame; a transport that already provides reliable packet framing is a distinct adapter.

## Protocols

*Session lifecycle (stream transports):*

1. Connect → per-peer lanes allocated (realtime + bulk).
2. Loss recovery operates while session valid (I-RT5).
3. Disconnect → lanes torn down; routes over that link marked down; local SHM untouched (I-RT6).
4. Reconnect → fresh session; importer increments epoch (consumers reset); no historical retransmit.

*Disconnect isolation (I-RT6):*

1. Link-down event → mark affected routes down only.
2. Local publishers/consumers continue (shared memory untouched by the router).
3. Link-up → routes resume; importer epoch bump (I-RT4/I-RT9 path).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Unknown wire_channel_id on ingress | drop + observability-plane counter (never block) |
| UDP datagrams out of order | stale `seq ≤ last` dropped for the epoch (I-RT9) |
| UDP duplicate | dropped (I-RT9) |
| Remote publisher restart (higher stream_epoch) | stream reset; local importer epoch bump (I-RT9) |
| Serial byte loss/corruption | COBS delimiter resync; CRC rejects the bad frame (I-RT10) |
| Oversize record on stream transport | accepted (stream framing allows large samples) |
| Importer publisher lock contention on restart | publisher-lifecycle authority protocol applies |
| Loop of routers (A↔B↔C) | forwarding plan excludes ingress-link re-export (route-derivation I-R7) |

## Test Matrix

| Case | Expected |
|---|---|
| `route_filter` | I-RT1 |
| `frame_sample_atomicity` (property) | I-RT2 |
| `udp_oversize_reject` (send + verify) | I-RT3 + V-RT |
| `import_native_equivalence` (bytes + timing) | I-RT4 |
| `wire_header` / `wire_id_not_handle` | I-RT7 |
| `router_borrow_scope` (no DRM borrow held in lane queues) | I-RT8 |
| `udp_reorder` / `udp_duplicate` / `remote_restart_reset` | I-RT9 |
| `serial_resync` (dropped byte, corruption) | I-RT10 |
| `tcp_session_scope` (kill mid-stream) | I-RT5 |
| `disconnect_isolation` (link cut) | I-RT6 |
| two-host sim + physical matrix () | all classes, both transports |
