# Transport Survey — M01W1-E3-T1 (seed for router algorithm docs)

Numbers sourced from published protocol specifications; adapters implement them
as constants (clean-room: no code copied). Forward-referenced by
`docs/algorithms/router/router-core.md` + `lanes-shaping.md` (W2+).
REQ: REQ-ROUTE-001 (ingress topics never re-exported), REQ-ROUTE-002
(bits/bytes explicit).

## Requirements Language

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all capitals.

## Datagram overhead (fixed, per wire frame)

| Family | IP header | DRM frame header | Total overhead |
|---|---|---|---|
| IPv4 UDP | 20 B | 8 B UDP | 28 B |
| IPv6 UDP | 40 B | 8 B UDP | 48 B |

`max_datagram_payload = MTU − overhead`: 1472 B @ IPv4/MTU 1500, 1452 B @ IPv6.
Auto-suggested safe bound: **1400 B** (tunnel/encapsulation headroom).

## Kernel and fragmentation rules

- UDP kernel datagram maximum: 65507 B (IP length field − headers); DRM never
  relies on it: **no IP fragmentation, no path-MTU discovery** on the realtime
  path — a sample that exceeds the per-route datagram bound is a config error
  at verify.
- TCP: length-prefixed records (architecture §Packet Framing
  `{version, flags, wire_channel_id, stream_epoch, source_seq, payload_len}`,
  fixed little-endian); receiver enforces the generated per-route maximum
  record length BEFORE buffering.
- Serial: COBS-delimited, CRC-checked frames; raw UART is not a byte stream.

## Lanes

Two lanes per peer (`realtime`, `bulk`); lane assignment is per route class;
control/status traffic shares `realtime` under DRR shaping
(REQ-ROUTE-020: strict-first under the verified policer envelope; realtime
never starves).

## Open questions (owned by E3, ≤ 10)

1. Serial COBS overhead bound per max record (framing constant for shaping).
2. IPv6 jumbogram support — rejected for MVP; record here as exclusion.
3. Wi-Fi MTU variability per driver — use 1400 auto-suggest or probe?
