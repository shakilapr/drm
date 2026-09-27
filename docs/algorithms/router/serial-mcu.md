# Serial & MCU Runtime Algorithms

Scope: serial-link transport framing and the constrained-device (MCU) runtime path — bounded frames, CRC integrity, escaping, freestanding C subset, zero-forbidden-heap data path, host↔MCU interop. Implements architecture.md §External Device Interoperability, §Decoupled Framing and Packing (bounded serial frames), codebase §3.3 (MCU subset). Owner: E3 (crate `drm-router::serial`, `bindings/c` MCU subset). Phase 32. Depends on [router-core.md](router-core.md) (frame = one sample), [lanes-shaping.md](lanes-shaping.md) (shaping), [c-abi-surface.md](../bindings/c-abi-surface.md) (surface).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-serial-bounded-frames` | one DRM sample = one bounded serial frame | — |
| `REQ-ROUTE-serial-best-effort` | serial = best-effort; `reliable` over serial rejected at verify | — |
| `REQ-ROUTE-serial-crc` | CRC when link lacks hardware checksums | — |
| `REQ-CORE-mcu-zero-heap` | MCU data path: no heap, no recursion, bounded loops | — |
| `REQ-CORE-mcu-opaque-payload` | third-party device packets via `bytes[<=N]` opaque mapping | — |
| `REQ-OPS-mcu-interop` | serial framing interoperates byte-for-byte with host | — |

## Data Structures

```rust
pub struct SerialFrame {
 channel: u8, // multiplexed channel id (topic handle low byte on serial links)
 len: u16, // payload length ≤ max_frame_bytes (link config)
 payload_crc: u32, // CRC32C over payload
 payload: Bytes,
}
// Wire encoding: [DLE] [STX] channel len_hi len_lo [payload…crc32c…] [DLE] [ETX]
// Payload escaping: 0x10 (DLE) and 0x03 (ETX) inside payload → DLE-stuffed (choice recorded options table).
pub struct SerialLinkConfig { baud: u32, max_frame_bytes: u16 } // declared per link in drm.toml
```

## Invariants

- **I-SR1 OneSampleOneFrame**: a serial frame carries exactly one DRM sample; never split (router-core I-RT2 serial row). Test: `serial_frame_atomicity`.
- **I-SR2 BoundEnforced**: frames larger than `max_frame_bytes` rejected at build (verify V-RT) and refused at encode (defense-in-depth). Test: `serial_oversize`.
- **I-SR3 CrcDetects**: single-bit and burst corruption in payload or header ⇒ CRC mismatch ⇒ frame dropped + counter (best-effort semantics). Test: `serial_corruption` (fault injection).
- **I-SR4 EscapeReversible**: arbitrary payload bytes (incl. DLE/ETX collisions) survive encode→decode byte-exact. Test: `serial_escape` property (random bytes ×10k).
- **I-SR5 McuNoHeap**: MCU steady-state publish/read path allocates nothing — enforced by linker-section audit, not review ([serial-mcu] build rule). Test: `mcu_zero_heap_audit` (no malloc-family symbols reachable from data path).
- **I-SR6 ByteForByteInterop**: host encoder output decodes identically by MCU decoder and vice versa. Test: `mcu_host_interop` (cross-decode matrix).

## Functions

```rust
// Host/router side (drm-router::serial)
fn frame_encode(cfg: &SerialLinkConfig, ch: u8, payload: &[u8]) -> Result<Vec<u8>, Status> {
 if payload.len() > cfg.max_frame_bytes { return Err(InvalidArg); } // I-SR2
 let crc = crc32c(payload); // lanes crc32c
 let mut out = vec_with_capacity(6 + payload.len() + 2 * escapes(payload)); // bounded, host-side only
 push_escaped(&mut out, header(ch, payload.len()));
 push_escaped(&mut out, payload);
 push_escaped(&mut out, crc.to_le_bytes());
 Ok(out)
}

fn frame_decode(cfg: &SerialLinkConfig, buf: &mut ReadBuf) -> Result<Option<SerialFrame>, Status> {
 // Streaming decoder: scans for DLE-STX, accumulates bounded frame, verifies CRC, un-escapes.
 // CRC mismatch ⇒ drop frame + observability counter (best-effort — I-SR3), resync to next DLE-STX.
 // Timeout with partial frame ⇒ drop partial + counter.
}

// MCU side (bindings/c freestanding subset — §3.3: no heap, no recursion, statically bounded loops, stdint only)
// drm_serial_encode(cfg, ch, payload, len, out, out_cap) -> drm_status (zero allocation; out caller-owned)
// drm_serial_decode(state, byte) -> frame-or-none state machine (single byte ingress; no blocking reads)
```

## Protocols

*Host↔MCU exchange (interop gate I-SR6):*

1. Host publishes small structured topic over serial link (route config `[route.x.serial]`).
2. MCU decodes streamingly; acknowledges via its own publish (opaque `bytes[<=N]` payload per REQ-CORE-mcu-opaque-payload).
3. Host decodes MCU frames; deposit via standard ingress path (router-core §Functions `ingress`).

*MCU steady-state discipline (I-SR5):*

1. All buffers static (`static` storage / linker-placed pools); no stack-overflow-prone recursion.
2. Data path loops statically bounded (`for i in 0..MAX_FRAMES_PER_TICK`).
3. Linker-section audit runs in MCU CI: no `malloc/free/calloc/realloc/new` symbols reachable from publish/read call graph.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Burst corruption (multiple bits) | CRC mismatch ⇒ drop + resync (I-SR3) |
| Partial frame at stream end (cable pull) | timeout ⇒ drop partial + counter |
| Frame larger than bound on wire (foreign sender) | rejected at decode (resync) |
| MCU backpressure (host faster than MCU) | MCU drops oldest inbound frame + counter (serial = best-effort; shaping upstream per lanes-shaping.md) |
| `reliable` contract on serial route | verify error (V-REL) |
| Channel id exhaustion (256 serial channels) | verify rejects at deployment generation |

## Test Matrix

| Case | Expected |
|---|---|
| `serial_frame_atomicity` | I-SR1 |
| `serial_oversize` (encode + verify) | I-SR2 |
| `serial_corruption` (bit + burst injection) | I-SR3 |
| `serial_escape` property ×10k | I-SR4 |
| `mcu_zero_heap_audit` (linker) | I-SR5 |
| `mcu_host_interop` (cross-decode matrix) | I-SR6 |
| MCU steady-state suite on target | timing bounds green |
