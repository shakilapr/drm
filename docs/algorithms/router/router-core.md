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
| `REQ-ROUTE-ingress-ordering` | drop same-epoch `seq ≤ last`; epoch increase ⇒ stream reset (u64 epochs, wrap-free) | `REQ-ROUTE-011` |
| `REQ-ROUTE-two-way-import` | received frames → local SHM native format, headers stripped | `REQ-ROUTE-031` |
| `REQ-ROUTE-disconnect-isolation` | link loss affects only topics on that link | `REQ-ROUTE-007` |
| `REQ-ROUTE-oversize-build-reject` | oversize datagram payload rejected at build time onto stream transports | `REQ-ROUTE-018` |
| `REQ-ROUTE-link-scheduler` | per-link scheduler shapes all transports; only reliable streams use ordered lanes | `REQ-ROUTE-034` |
| `REQ-ROUTE-egress-scoped-failure` | one route's egress failure never blocks other routes on the link | `REQ-ROUTE-035` |
| `REQ-ROUTE-branch-independent` | transit failure never suppresses local import or sibling branches | `REQ-ROUTE-036` |
| `REQ-ROUTE-branch-session-fail` | exhausted reliable branch ⇒ branch session fails; others continue | `REQ-ROUTE-037` |
| `REQ-ROUTE-stream-receive-bound` | generated max record length enforced before buffering | `REQ-ROUTE-038` |

## Data Structures

```rust
pub struct RouterConfig {
 plans: BTreeMap<TopicPath, TopicRoutePlan>, // route-derivation.md output (forwarding tree)
 transports: BTreeMap<LinkId, TransportAdapter>, // capability-mapped
 datagram_bound: BTreeMap<LinkId, u16>, // max_datagram_payload per datagram link
 max_record_bytes: BTreeMap<(LinkId, WireChannelId), u32>, // generated: header + declared topic payload bound + optional e2e CRC (I-RT18)
}

pub struct WireHeader {
 version: u8,
 flags: u8, // bit 0 = end-to-end CRC32C trailer present; remaining bits must be zero
 reserved: u16, // zero
 wire_channel_id: u32, // TRANSPORT id from the route binding — never the runtime topic handle
 stream_epoch: u64, // per-source stream epoch; increments on source/import restart; plain u64 ordering is wrap-free
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

pub struct StreamKey { epoch: u64, seq: u64 }
pub struct IngressState { newest: Option<StreamKey> } // per (link, wire_channel_id); reset on reconnect
pub struct EgressCursor { next: Option<StreamKey> } // router-private per (topic, link) sequential position;
 // distinct from a consumer ReaderState (cursor-delivery.md) — the router tracks its own egress position
pub struct EgressState {
 cursor: EgressCursor,             // per (topic, link), sequential when no export limiter
 last_exported: Option<StreamKey>, // latest-sampling routes never resend an unchanged sample
}
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
- **I-RT7 WireHeader**: every frame carries fixed 28-byte little-endian `{version, flags, reserved, wire_channel_id, stream_epoch: u64, source_seq, payload_len}`; unknown version/flags or nonzero reserved bits are rejected. Test: `wire_header`, `wire_id_not_handle`.
- **I-RT8 RouterOwnsBytes**: egress performs `borrow → copy into router-owned bounded buffer → release`, then enqueues; no borrowed DRM sample outlives the borrow scope (external-I/O rule). Test: `router_borrow_scope` (instrumented; lane queue holds zero DRM borrows).
- **I-RT9 IngressOrdering**: for a given (link, wire_channel_id), a frame with the same epoch and `source_seq ≤ last_seq` is dropped; a higher epoch triggers a stream reset (imported publisher epoch bump). Epochs are `u64` — plain integer ordering is wrap-free, so a restart is always recognized (test includes the maximal-epoch restart case). UDP reorder/duplicate can never make older state the new latest. Test: `udp_reorder`, `udp_duplicate`, `remote_restart_reset` (incl. `epoch_max → 0`).
- **I-RT10 SerialFraming**: serial frames use a delimiter + COBS encoding + explicit length + CRC32C, so a dropped byte cannot desynchronize the receiver indefinitely. Test: `serial_resync`.
- **I-RT13 ForwardOnce**: each route forwards a source key at most once; an unrestricted route advances a per-route sequential cursor, while an export-limited route samples a newer latest key only after its period opens. Test: `egress_cursor`, `export_no_duplicate`.
- **I-RT14 ValidateBeforeState**: malformed or integrity-failing frames change no ingress ordering/importer state. Test: `ingress_invalid_no_state`.
- **I-RT15 EgressScopedFailure**: any single route's failure (queue full, oversize, encode error) is counted and scoped to that route; the link scan continues, so a full lower-priority queue never prevents a safety route from being examined. No work ⇒ no reservation is taken. Test: `egress_scoped_failure`.
- **I-RT16 BranchIndependence**: a validated frame advances ingress ordering state once, then attempts local import and each egress branch as independent outcomes; one branch's failure (counted, never `?`-propagated across the tree) never suppresses local delivery or sibling branches. Test: `fanout_branch_isolation`.
- **I-RT17 BranchSessionFail**: a reliable branch whose lane stays exhausted past the stall window fails its transport session: pending branch state is discarded, the branch marks down (I-RT5/I-RT6 semantics), and bounded memory holds — never unbounded growth, never source-wide backpressure. Test: `reliable_branch_stall`.
- **I-RT18 ReceiveBound**: stream/serial receivers enforce the generated per-route `max_record_bytes` before buffering payload; a declared length above it is a protocol error (drop + resync; reliable stream: session reset) — never an oversized allocation. Test: `stream_record_bound`.

## Functions

```rust
// ---- Egress (per link, per tick) ----
fn egress(&mut self, link: LinkId) -> Status {
 for topic in self.routes_on(link) { // I-RT1: forwarding plan filter
  let Some(key) = self.peek_next_or_limited_latest(topic, link) else { continue; }; // metadata-only peek (no payload borrow, I-RT8); no work: no reservation taken
  match self.admit_and_send(link, topic, key) {
   Err(e) => { self.count_route_error(topic, link, e); continue; } // I-RT15: scoped to this route, scan continues
   Ok(()) => {}
  }
 }
 Status::Ok
}

fn admit_and_send(&mut self, link: LinkId, topic: TopicPath, key: StreamKey) -> Result<(), Status> {
 // Reserve a fixed scheduler slot first, then copy directly into it. Cursor commits only after reserve.
 let Some(mut reserved) = self.scheduler.try_reserve(link, priority_class(topic))? else {
  return Err(QueueFull); // route-scoped (I-RT15) — scheduler spans all transports (REQ-ROUTE-034)
 };
 let owned = self.copy_into_reserved(topic, link, key, &mut reserved)?; // borrow → copy → release; reserved bytes never alias SHM (I-RT8/I-RT13)
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
 reserved.commit(frame); // cannot fail after reservation; egress cursor/key commits here
 Ok(())
}

// ---- Ingress (per link) ----
fn ingress(&mut self, link: LinkId, frame: WireFrame) -> Result<(), Status> {
 validate_header_and_exact_length(&frame)?; // before lookup, state mutation, or payload access
 if frame.header.payload_len as u64 + header_len() > self.max_record_bytes(link, frame.header.wire_channel_id) as u64 {
  return Err(ProtocolError); // I-RT18: generated bound checked before any payload buffering
 }
 let topic = self.topic_for(frame.header.wire_channel_id, link)?;
 verify_e2e_if_final(topic, &frame)?; // I-RT14: integrity before reset/dedup state mutation
 let st = self.ingress_state(link, frame.header.wire_channel_id);

 if let Some(old) = st.newest {
  if frame.header.stream_epoch == old.epoch && frame.header.source_seq <= old.seq { return Ok(()); }
  if frame.header.stream_epoch < old.epoch { return Ok(()); } // stale lower epoch
  if frame.header.stream_epoch > old.epoch { self.import_publisher(topic)?.reset_stream_epoch(); }
  // u64 epoch: plain < / > is wrap-free (I-RT9)
 }
 st.newest = Some(StreamKey { epoch: frame.header.stream_epoch, seq: frame.header.source_seq }); // validated frame advances ordering state once (I-RT16)

 // Independent outcomes from here — no `?` spans the forwarding tree (I-RT16):
 if self.has_local_consumers(topic) {
  match self.import_publisher(topic) {
   Ok(publisher) => if let Err(e) = publisher.publish_copy(&frame.payload) {
    self.count_import_drop(topic, e); // local pressure never affects transit (I-RT4)
   },
   Err(e) => self.count_import_drop(topic, e), // authority contention counted; branches unaffected
  }
 }
 for branch in self.egress_branches(link, topic) { // excludes ingress link (route-derivation I-R7)
  if let Err(e) = self.forward_branch(branch, topic, &frame) {
   self.count_branch_error(branch, e); // I-RT16: scoped to this branch
   if self.reliable_branch_stalled(branch) { self.fail_branch_session(branch); } // I-RT17: bounded failure, others continue
  }
 }
 Ok(())
}
```

*Wire encoding:* header integers are little-endian; the header is 28 bytes (I-RT7). End-to-end CRC, when flagged, follows the payload. Serial is `COBS{header, payload, optional e2e CRC, serial CRC32C} · 0x00`; the serial CRC covers all decoded preceding bytes (header, payload, and e2e trailer — [serial-mcu.md](serial-mcu.md) I-SR3).

## Protocols

*Session lifecycle (stream transports):*

1. Connect → per-peer lanes allocated (realtime + bulk).
2. Loss recovery operates while session valid (I-RT5).
3. Reliable branch lane exhausted past the stall window ⇒ branch session fails: pending branch state discarded, branch marked down; other branches and local delivery continue (I-RT17).
4. Disconnect → lanes torn down; routes over that link marked down; local SHM untouched (I-RT6).
5. Reconnect → fresh session, empty ingress ordering state, importer epoch bump; no historical retransmit.

*Disconnect isolation (I-RT6):*

1. Link-down event → mark affected routes down only.
2. Local publishers/consumers continue (shared memory untouched by the router).
3. Link-up → routes resume; importer epoch bump (I-RT4/I-RT9 path).

*UDP transport notes (M02W1-E3-T3 seed; REQ: REQ-ROUTE-009):*

1. **Batching**: receive with `recvmmsg(2)` — up to 64 datagrams per syscall; per-datagram framing decode (WireHeader + payload), stale/duplicate classification per I-RT9 before any queueing.
2. **SO_RCVBUF sizing**: set to `max_record_bytes × in-flight budget` (≥ 256 datagrams); kernel drops beyond that are counted per link (observability-plane), never silently ignored.
3. **Two-host sim plan**: two processes over loopback UDP, one router process per "host"; inject reorder (send-side shuffle window ≤ 32) and duplicates (2× retransmit of the same frame) to exercise I-RT9; remote-restart test raises `stream_epoch` on reconnect (epoch-max → 0 case included).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Unknown wire_channel_id on ingress | drop + observability-plane counter (never block) |
| UDP datagrams out of order | stale `seq ≤ last` dropped for the epoch (I-RT9) |
| UDP duplicate | dropped (I-RT9) |
| Remote publisher restart (higher stream_epoch) | stream reset; local importer epoch bump (I-RT9) |
| Serial byte loss/corruption | COBS delimiter resync; CRC rejects the bad frame (I-RT10) |
| Record longer than generated `max_record_bytes` | protocol error before buffering: drop + resync; reliable stream: session reset (I-RT18) |
| One route's queue full during egress scan | route error counted; scan continues — safety routes always examined (I-RT15) |
| Transit branch failure on a valid frame | local import and sibling branches unaffected (I-RT16) |
| Reliable branch receiver stalled, lane exhausted | branch session fails, pending discarded; other branches continue (I-RT17) |
| Importer publisher lock contention on restart | publisher-lifecycle authority protocol applies |
| Loop of routers (A↔B↔C) | forwarding plan excludes ingress-link re-export (route-derivation I-R7) |
| Egress tick with no new source key | no frame emitted (I-RT13) |
| Bad higher-epoch CRC | reject; importer epoch and dedup state unchanged (I-RT14) |
| Local import pool exhausted at transit router | local drop counted; valid frame still follows its forwarding tree |

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
| `egress_cursor` / `export_no_duplicate` | I-RT13 |
| `ingress_invalid_no_state` | I-RT14 |
| `egress_scoped_failure` (full low-priority route, ready safety route; reversed iteration order) | I-RT15: safety admitted |
| `fanout_branch_isolation` (branch queue full + local consumer) | I-RT16: local + sibling branches delivered |
| `reliable_branch_stall` (bounded branch queue, stalled receiver) | I-RT17: branch session fails, others continue, memory bounded |
| `stream_record_bound` (declared length > generated max) | I-RT18: protocol error, no allocation |
| `remote_restart_reset` (epoch max → 0) | I-RT9: wrap-free restart recognized |
| `e2e_integrity_profile` (CRC32C over `stream_epoch`/`source_seq`/`payload_len`/payload; transit remap preserves covered fields + trailer) | I-RT12 |
