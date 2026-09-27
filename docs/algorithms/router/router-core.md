# Boundary Router Core Algorithms

Scope: `drm-router` data path — topic filtering from derived forwarding plans, router-owned copy before egress (external-I/O borrow rule), per-transport framing (UDP datagrams, reliable stream records, COBS serial frames), two-way import, multi-hop forwarding, ingress ordering/dedup, session lifecycle, disconnect isolation. Implements architecture.md §The Boundary Router, §Transport Metadata. Owner: E3 (crate `drm-router`). Phase 29. Depends on [route-derivation.md](../toolchain/route-derivation.md), [lanes-shaping.md](lanes-shaping.md), [publisher-lifecycle.md](../runtime/publisher-lifecycle.md), [cursor-delivery.md](../runtime/cursor-delivery.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-router-filter` | transmits only topics with remote consumers on that link | `REQ-ROUTE-022` |
| `REQ-ROUTE-router-owned-copy` | egress copies into router-owned memory and releases the borrow before enqueue | `REQ-ROUTE-023` |
| `REQ-ROUTE-wire-header` | wire header = wire_channel_id, stream_epoch, source_seq, payload_len | `REQ-ROUTE-033` |
| `REQ-ROUTE-udp-single-packet` | one sample per datagram ≤ `max_datagram_payload`; no IP fragmentation/PMTUD | `REQ-ROUTE-032` |
| `REQ-ROUTE-tcp-records` | stream transports: one length-framed record per sample | `REQ-ROUTE-029` |
| `REQ-ROUTE-serial-framing` | serial: COBS + `0x00` delimiter + length + CRC32C | `REQ-ROUTE-027` |
| `REQ-ROUTE-ingress-ordering` | drop same-epoch `seq ≤ last`; epoch increase ⇒ stream reset | `REQ-ROUTE-011` |
| `REQ-ROUTE-two-way-import` | received frames → local SHM native format, headers stripped | `REQ-ROUTE-031` |
| `REQ-ROUTE-disconnect-isolation` | link loss affects only topics on that link | `REQ-ROUTE-007` |
| `REQ-ROUTE-oversize-build-reject` | oversize datagram payload rejected at build time onto stream transports | `REQ-ROUTE-018` |

## Data Structures

```rust
pub struct RouterConfig {
 plans: BTreeMap<TopicPath, TopicRoutePlan>, // route-derivation.md output (forwarding tree)
 transports: BTreeMap<LinkId, TransportAdapter>, // capability-mapped
 datagram_bound: BTreeMap<LinkId, u16>, // max_datagram_payload per datagram link
}

pub struct WireHeader {
 version: u8,
 flags: u8, // bit 0 = end-to-end CRC32C trailer present; remaining bits must be zero
 reserved: u16, // zero
 wire_channel_id: u32, // TRANSPORT id from the route binding — never the runtime topic handle
 stream_epoch: u32, // per-source stream epoch; increments on source/import restart
 source_seq: u64, // per-source sequence (distinct from the per-link transport seq)
 payload_len: u32,
}

pub enum WireFrame {
 Datagram { header: WireHeader, payload: OwnedFrame }, // UDP: one sample, router-owned bytes
 Record { header: WireHeader, payload: OwnedFrame }, // TCP: length-framed record
 Serial { header: WireHeader, payload: OwnedFrame }, // raw UART: COBS framed
}
// OwnedFrame is a slice into a router-owned bounded buffer; it NEVER aliases DRM shared memory.
// No DRM-level fragmentation/reassembly — oversize belongs on stream transports (build-rejected).

pub struct StreamKey { epoch: u32, seq: u64 }
pub struct IngressState { newest: Option<StreamKey> } // per (source, wire_channel_id); reset on session reconnect
```

## Invariants

- **I-RT1 FilterExact**: egresses only topics with ≥ 1 remote consumer on that specific link; local-only topics never egress. Test: `route_filter`.
- **I-RT2 OneFrameOneSample**: a wire frame carries exactly one DRM sample (multiplexed small topics share a datagram bound; never split a sample). Test: `frame_sample_atomicity`.
- **I-RT3 NoOversizeSend**: datagram egress refuses payloads > `max_datagram_payload` (defense-in-depth behind build-time verify). Test: `udp_oversize_reject`.
- **I-RT11 MediaVariants**: transport adapters are media-pluggable — UDP unicast is the MVP datagram medium; UDP multicast is an adapter variant with identical framing and group-address route config; serial uses [serial-mcu.md](serial-mcu.md). Test: adapter registry accepts a multicast adapter stub with identical framing tests.
- **I-RT12 EndToEndIntegrity**: a declared profile appends CRC32C over encoded `{stream_epoch, source_seq, payload_len, payload}`. Transit routers may remap `wire_channel_id` but preserve covered fields and trailer; the final router verifies before import. Test: `e2e_integrity_profile`.
- **I-RT4 ImportNative**: ingress strips wire headers and deposits the sample in native format — consumers cannot distinguish imported from local data. Test: `import_native_equivalence`.
- **I-RT5 SessionScope**: `reliable` = ordered loss recovery while the transport session is valid; reconnection starts clean. Test: `tcp_session_scope`.
- **I-RT6 DisconnectIsolated**: link loss affects only routes over that link; local SHM loops continue. Test: `disconnect_isolation`.
- **I-RT7 WireHeader**: every frame carries fixed 24-byte little-endian `{version, flags, reserved, wire_channel_id, stream_epoch, source_seq, payload_len}`; unknown version/flags or nonzero reserved bits are rejected. Test: `wire_header`, `wire_id_not_handle`.
- **I-RT8 RouterOwnsBytes**: egress performs `borrow → copy into router-owned bounded buffer → release`, then enqueues; no borrowed DRM sample outlives the borrow scope (external-I/O rule). Test: `router_borrow_scope` (instrumented; lane queue holds zero DRM borrows).
- **I-RT9 IngressOrdering**: for a given (link, wire_channel_id), a frame with the same epoch and `source_seq ≤ last_seq` is dropped; a higher epoch triggers a stream reset (imported publisher epoch bump). UDP reorder/duplicate can never make older state the new latest. Test: `udp_reorder`, `udp_duplicate`, `remote_restart_reset`.
- **I-RT10 SerialFraming**: serial frames use a delimiter + COBS encoding + explicit length + CRC32C, so a dropped byte cannot desynchronize the receiver indefinitely. Test: `serial_resync`.

## Functions

```rust
// ---- Egress (per link, per tick) ----
fn egress(&mut self, link: LinkId) -> Result<(), Status> {
 for topic in self.routes_on(link) { // I-RT1: forwarding plan filter
 // I-RT8: momentary borrow, copy to router-owned buffer, release BEFORE any queue/I/O
 let owned = self.momentary_copy_with_key(topic)?; // payload + publisher epoch/sequence
 let header = WireHeader {
 version: 1, flags: self.integrity_flags(topic), reserved: 0,
 wire_channel_id: self.wire_id(topic, link), // I-RT7: transport binding
 stream_epoch: owned.epoch,
 source_seq: owned.seq,
 payload_len: u32::try_from(owned.payload.len()).map_err(|_| OversizeInternal)?,
 };
 let frame = match self.transport_kind(link) {
 Datagram => {
 if encoded_len(&header, &owned.payload)? > self.datagram_bound[link] as usize { return Err(OversizeInternal); }
 WireFrame::Datagram { header, payload: owned.payload }
 }
 Stream => WireFrame::Record { header, payload: owned.payload },
 Serial => WireFrame::Serial { header, payload: owned.payload },
 };
 self.lanes.enqueue(link, priority_class(topic), frame)?; // lanes-shaping.md
 }
 Ok(())
}

// ---- Ingress (per link) ----
fn ingress(&mut self, link: LinkId, frame: WireFrame) -> Result<(), Status> {
 validate_header_and_exact_length(&frame)?; // before lookup, state mutation, or payload access
 let topic = self.topic_for(frame.header.wire_channel_id, link)?;
 let st = self.ingress_state(link, frame.header.wire_channel_id);

 if let Some(old) = st.newest {
 if frame.header.stream_epoch == old.epoch && frame.header.source_seq <= old.seq { return Ok(()); }
 if frame.header.stream_epoch < old.epoch { return Ok(()); }
 if frame.header.stream_epoch > old.epoch { self.import_publisher(topic)?.reset_stream_epoch(); }
 }
 verify_e2e_if_final(topic, &frame)?;
 st.newest = Some(StreamKey { epoch: frame.header.stream_epoch, seq: frame.header.source_seq });

 if self.has_local_consumers(topic) { self.import_publisher(topic)?.publish_copy(&frame.payload)?; }
 self.forward_preserving_origin(link, topic, frame)?; // excludes ingress link; no new source key
 Ok(())
}
```

*Wire encoding:* header integers are little-endian. End-to-end CRC, when flagged, follows the payload. Serial is `COBS{header, payload, optional e2e CRC, serial CRC32C} · 0x00`; the serial CRC covers all decoded preceding bytes.

## Protocols

*Session lifecycle (stream transports):*

1. Connect → per-peer lanes allocated (realtime + bulk).
2. Loss recovery operates while session valid (I-RT5).
3. Disconnect → lanes torn down; routes over that link marked down; local SHM untouched (I-RT6).
4. Reconnect → fresh session, empty ingress ordering state, importer epoch bump; no historical retransmit.

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
| two-host simulation + physical matrix | all classes and transports |
