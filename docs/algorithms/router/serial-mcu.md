# Serial & MCU Runtime Algorithms

Scope: serial-link transport framing and the constrained-device (MCU) runtime path — bounded frames, CRC integrity, escaping, freestanding C subset, zero-forbidden-heap data path, host↔MCU interop. Implements architecture.md §External Device Interoperability, §Decoupled Framing and Packing (bounded serial frames), codebase §3.3 (MCU subset). Owner: E3 (crate `drm-router::serial`, `bindings/c` MCU subset). Phase 32. Depends on [router-core.md](router-core.md) (frame = one sample), [lanes-shaping.md](lanes-shaping.md) (shaping), [c-abi-surface.md](../bindings/c-abi-surface.md) (surface).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-serial-bounded-frames` | one DRM sample = one bounded serial frame | `REQ-ROUTE-025` |
| `REQ-ROUTE-serial-best-effort` | serial = best-effort; `reliable` over serial rejected at verify | `REQ-ROUTE-024` |
| `REQ-ROUTE-serial-crc` | CRC when link lacks hardware checksums | `REQ-ROUTE-026` |
| `REQ-CORE-mcu-zero-heap` | MCU data path: no heap (allocation disabled at build/link level, audit secondary), no recursion, bounded loops | `REQ-CORE-087` |
| `REQ-CORE-mcu-opaque-payload` | third-party device packets via `bytes[<=N]` opaque mapping | `REQ-CORE-086` |
| `REQ-OPS-mcu-interop` | serial framing interoperates byte-for-byte with host | `REQ-OPS-004` |
| `REQ-ROUTE-serial-blocking` | non-preemptible transmission: verify reports worst-case blocking vs deadlines | `REQ-ROUTE-040` |

## Data Structures

```rust
pub struct SerialFrame {
 header: WireHeader, // router-core fixed little-endian header; channel is generated u32 wire ID
 payload: BoundedBytes,
}
pub struct SerialLinkConfig { baud: u32, max_frame_bytes: u16 } // max_frame_bytes = decoded frame bound per link (wire size = decoded + COBS overhead)
// Wire: COBS{header, payload, optional e2e CRC32C, serial CRC32C} 0x00.
// serial CRC covers the exact decoded byte sequence preceding it: header + payload + optional e2e trailer (I-SR3).
// max_encoded = decoded_len + ceil(decoded_len / 254) + 1 delimiter — standard COBS overhead for nonempty input.
```

## Invariants

- **I-SR1 OneSampleOneFrame**: a serial frame carries exactly one DRM sample; never split (router-core I-RT2 serial row). Test: `serial_frame_atomicity`.
- **I-SR2 BoundEnforced**: frames larger than `max_frame_bytes` rejected at build (verify V-RT) and refused at encode (defense-in-depth). Test: `serial_oversize`.
- **I-SR3 CrcDetects**: the serial CRC32C covers the decoded header, payload, and optional end-to-end trailer — one canonical byte sequence shared by host and MCU encoders; mismatch ⇒ frame dropped + counter. Guaranteed detection: all single-bit errors and all bursts ≤ 32 bits (degree-32 property); longer or arbitrary multi-bit corruption is probabilistic — characterized by the corruption campaign, never claimed zero. Test: `serial_corruption`.
- **I-SR4 EscapeReversible**: arbitrary bytes, including zero, survive COBS encode→decode byte-exact. Test: `serial_escape` property (random bytes ×10k).
- **I-SR5 McuNoHeap**: MCU steady-state publish/read path allocates nothing — enforced by preventive barriers, audit secondary: (1) no heap implementation linked; (2) malloc-family symbols are link errors/traps; (3) RTOS dynamic allocation disabled where supported; (4) linker-section audit + target-test allocator hooks assert zero calls; (5) stack high-water measured and gated — for every MCU task `measured_min_free_stack ≥ configured_stack_margin`, the margin declared per platform/build profile (never a universal percentage). Test: `mcu_zero_heap_audit` (barrier set green + zero reachable allocation symbols), `mcu_stack_margin`.
- **I-SR6 ByteForByteInterop**: host encoder output decodes identically by MCU decoder and vice versa. Test: `mcu_host_interop` (cross-decode matrix).
- **I-SR7 SerialBlockingBound**: UART transmission is non-preemptible; a lower-priority frame already on the wire blocks higher priorities for ≈ `10 × encoded_bytes / baud` (8N1; factor per declared character format). This is the `T_inflight_lower_priority` term of the link safety latency bound (lanes-shaping I-LS11). Verify computes `blocking_max = wire_time(max lower-priority frame)` per link and reports it against declared deadlines; unacceptable bounds require a smaller `max_frame_bytes`, higher baud, or a dedicated link — DRM never fragments a sample. Test: `serial_blocking_estimate`.

## Functions

```rust
// Host/router side (drm-router::serial)
fn frame_encode(cfg: &SerialLinkConfig, frame: &WireFrame, out: &mut [u8]) -> Result<usize, Status> {
 let decoded_len = checked_decoded_len(frame)?;
 if decoded_len > cfg.max_frame_bytes as usize { return Err(InvalidArg); }
 let need = decoded_len.checked_add(decoded_len.div_ceil(254)).and_then(|n| n.checked_add(1)).ok_or(InvalidArg)?;
 if out.len() < need { return Err(BufferTooSmall); } // before any write
 let mut covered = encoded_header_and_payload(frame); // I-SR3 canonical sequence:
 if frame.header.flags & FLAG_E2E_CRC != 0 { covered.extend_from_slice(&e2e_trailer(frame)); } // trailer covered, from the declared integrity profile
 let crc = crc32c(&covered); // host and MCU use this exact byte sequence
 cobs_encode_with_crc_and_delimiter(frame, crc, out)
}

fn frame_decode(cfg: &SerialLinkConfig, buf: &mut ReadBuf) -> Result<Option<SerialFrame>, Status> {
 // Fixed-state streaming decoder accumulates at most max_encoded bytes until 0x00.
 // It COBS-decodes into caller-owned storage, checks exact length, then CRC.
 // Any failure drops the frame and resynchronizes at the next 0x00.
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
3. Build prevents allocation: no heap implementation linked; malloc-family calls fail to link/trap; RTOS dynamic allocation off (e.g. `configSUPPORT_DYNAMIC_ALLOCATION=0`); system heap zero-sized by default where applicable.
4. Linker-section audit + allocator hooks in target CI are secondary assertions: no `malloc/free/calloc/realloc/new` reachable from the publish/read call graph; stack high-water recorded and gated against the declared per-profile margin (I-SR5).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Corruption: any 1-bit flip, any burst ≤ 32 bits | CRC mismatch guaranteed ⇒ drop + resync (I-SR3) |
| Corruption: longer/multiple bursts | probabilistic — campaign characterizes residual rate; never claimed zero (I-SR3) |
| Partial frame at stream end (cable pull) | timeout ⇒ drop partial + counter |
| Frame larger than bound on wire (foreign sender) | rejected at decode (resync) |
| Safety frame ready while lower-priority frame transmits | blocked ≤ `blocking_max` (I-SR7); verify-reported vs deadlines |
| MCU backpressure (host faster than MCU) | MCU drops oldest inbound frame + counter (serial = best-effort; shaping upstream per lanes-shaping.md) |
| `reliable` contract on serial route | verify error (V-REL) |
| Wire ID allocation exhausted (`u32`) | deployment generation fails; runtime topic handles remain unrelated |

## Test Matrix

| Case | Expected |
|---|---|
| `serial_frame_atomicity` | I-SR1 |
| `serial_oversize` (encode + verify) | I-SR2 |
| `serial_corruption` (exhaustive 1-bit, all bursts ≤ 32 bits, 2/3/4-bit patterns, randomized campaign, golden CRC vectors, e2e-trailer bit-flip covered by serial CRC) | I-SR3 |
| `serial_cobs_boundary` (decoded 253/254/255/507/508/509; host ↔ MCU byte-identical) | I-SR4/I-SR6 boundary vectors |
| `serial_escape` property ×10k | I-SR4 |
| `mcu_zero_heap_audit` (link/trap barriers + hooks) | I-SR5 |
| `mcu_stack_margin` (measured min free stack vs declared per-profile margin) | I-SR5 |
| `mcu_host_interop` (cross-decode matrix) | I-SR6 |
| `serial_blocking_estimate` (verify report vs declared deadlines) | I-SR7 |
| MCU steady-state suite on target | loop/stack bounds and allocation barriers pass |
