# Phase 31 Physical Matrix Runbook — M14W1-E3-T1

Two-host runbook for the Windows↔Linux communication matrix (work-plan #31
criterion 4, Phase 31) plus the local loopback fallback evidence. Normative
algorithm: [router-core.md](../algorithms/router/router-core.md) §Protocols
(two-host rows), [serial-mcu.md](../algorithms/router/serial-mcu.md)
(host↔MCU/byte-exact interop), [deployment-identity.md](../algorithms/deployment-identity.md)
(per-endpoint namespace). Owner: E3. Chair: E4 (M14W1), E1 (M14W2 gate).
REQ: REQ-QAL-008, REQ-OPS-004, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026,
REQ-ROUTE-029, REQ-ROUTE-032, REQ-ROUTE-040.

## Hosts and topology

| Role | Host | OS | Notes |
|---|---|---|---|
| `A` | lab-01 | Linux x86_64 | router + publisher |
| `B` | lab-02 | Windows x86_64 | router + consumer |

- Link: dedicated 1 GbE, static addresses, no switch QoS; shaping on
  ([lanes-shaping.md](../algorithms/router/lanes-shaping.md), `shaping=on`).
- Serial: USB-UART pair between the hosts' serial ports; when no serial pair is
  wired the serial cell uses the in-process host-loopback codec (below).
- Each endpoint runs under a deployment namespace `drm.<os>-<lang>-<os>-<lang>-<transport>`
  from [`InteropCell::namespace`](../../xtask/src/cross_os.rs); identities never
  derive from paths (deployment-identity.md).

## Cell matrix

`{windows, linux} × {udp, tcp, serial} × {c, cpp, rust, python}` — 24
gate-blocking **core** cells (same-language pairs); mixed-language pairs are
**tracked**, not gate-blocking. Machine-readable source:
`xtask/src/cross_os.rs` (`interop_core_matrix`); generated table:
`target/interop/matrix-report.txt`.

## Repeat and failure policy

- Every cell runs **×3** (`repeat=3`); 3/3 must agree on byte-exact payload.
- A single green run is insufficient evidence; a flapping cell is quarantined
  (regression permanence) and owner-tagged.
- Red core cell ⇒ M14W2 Day 1 fix window; the Phase 31 gate holds only with
  core-cell evidence (work-plan M14W1 Risks).

## Runbook (per host pair)

```text
# 1. Provision both hosts; firewall: UDP/TCP ports from the route config, serial pair.
# 2. Build: cargo build --release -p drm-cli -p drm-router
# 3. Start host B (consumer), then host A (publisher), shaping on.
# 4. For transport in udp tcp serial:
#      for repeat in 1 2 3:
#        cargo xtask test --cross-os --transport $transport    # per-cell evidence
# 5. Archive target/interop/*.txt + target/interop/matrix-report.txt per run.
```

## Evidence — M14W1 (loopback fallback)

No physical dual-VM pair is booked at M14W1; the blocking executable evidence is
the same manifest run on one host over loopback (the documented two-netns
simulation fallback), with serial exercised as an in-process host-loopback of
the byte-exact codec. Physical dual-VM rows are reported as **measured, not
guaranteed** (lanes-shaping.md `I-LS1`); they are scheduled for the M14W2 fix
window rather than silently claimed.

| Transport | Green cells | Scheduled cells | Evidence |
|---|---|---|---|
| udp | 2 (Rust↔Rust, both directions) | 30 | `target/interop/*-udp.txt` |
| tcp | 2 (Rust↔Rust, both directions) | 30 | `target/interop/*-tcp.txt` |
| serial (host-loopback) | 2 (Rust↔Rust, both directions) | 30 | `target/interop/*-serial.txt` |
| **total** | **6** | **90** (96 cells = 24 core + 72 tracked) | `target/interop/matrix-report.txt` |

The core-cell ledger: 24 core cells total (2 OS directions × 4 languages × 3
transports); 6 Rust same-language cells are green on the fallback today, the 18
C/C++/Python same-language cells are tracked pending the Phase 32/33 bindings
and the physical runner.

## How to reproduce

```text
cargo xtask test --cross-os --transport udp
cargo xtask test --cross-os --transport tcp
cargo xtask test --cross-os --transport serial
cargo test -p xtask cross_os::interop_tests
cargo test -p drm-router serial_route
```

REQ: REQ-QAL-008, REQ-OPS-004, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026,
REQ-ROUTE-029, REQ-ROUTE-032, REQ-ROUTE-040.
