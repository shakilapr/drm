# Metrics — Month 13 (M13)

Normative source: [work-plan.md](../../work-plan.md) W5 Windows Runtime Backend
continues with Phase 31 completion (gate M14W2) and Phase 32 serial-transport
prep. One section per weekly tranche.

## M13W1 — Windows Platform Completion, Serial Transport Implementation

| Variable | Target | Measured |
|---|---|---|
| `WIN_PLATFORM_FULL` | lifecycle + channels + hazard on Windows green | 1 — Windows publisher/consumer lifecycle over `LockFileEx` + pagefile sections (`win_lifecycle_win_roundtrip`, `win_consumer_never_creates`, `win_epoch_restart_parity` ×20 restarts), authority race (`win_race_create`, I-W1/I-L1), in-place `Init` recovery (`win_init_reinit`, I-W1/I-L8), and reclamation-authority/identity subset (`win_reclaim_authority_subset`, `win_identity_diagnostic_subset`, I-E5/I-W5); all `cfg(windows)` rows in the cross-OS subset are green |
| `SERIAL_FRAMES` | frame encode/decode + CRC + escaping green ×10k loopback | 1 — `cargo test -p drm-router --lib transport::serial` 13/13: `serial_escape` (×10k COBS round-trip), `serial_corruption` (all single-bit + bursts ≤32 bits + 2 000 random multi-bit), `serial_cobs_boundary`, `serial_crc_mismatch_rejected`, and the new `serial_loopback_x10k` socketpair exchange (10 000 frames reassembled by `0x00` delimiter, byte-exact) |
| `SPATIAL_REQ_MINT_START` | ≥ 25 `REQ-SPATIAL` rows extracted | 37 — `REQ-SPATIAL-001..037` in [registry.csv](../req/registry.csv) cover `spatial_architecture.md` §1–§40 (golden rules, typed geometry, spaces, forests, conventions, geodetic chain, query algebra); registry lint 0 findings |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Windows in-place `Init` recovery (`crates/drm-core/src/channel/lifecycle.rs`,
  `reinit_in_place`): pagefile sections have no unlink, so an exact-size `Init`
  section is reinitialized under the held publisher authority — zero the CCB
  region, rewrite prefix/descriptor/name, then publish `Ready` — matching
  [publisher-lifecycle.md](../algorithms/runtime/publisher-lifecycle.md)
  §Protocols "Init crash recovery" and
  [windows-backends.md](../algorithms/runtime/windows-backends.md) §Protocols.
  POSIX keeps unlink + recreate (`#[cfg(unix)]`); undersized Windows sections
  stay retryable (`StaleIncomplete` → `NotReady`) until their last handle closes.
- Windows lifecycle/channel suite
  (`crates/drm-core/src/channel/lifecycle/tests/windows.rs`): real `WinShm` +
  `WinLockBackend` fixtures for publisher/consumer open, 8-thread authority
  race, in-place `Init` recovery, consumer open-only, 20-cycle restart epoch
  parity with a live consumer mapping holding the section (I-W4b), plus the
  reclamation-authority and identity-diagnostic subsets (E1-T1/E1-T2).
- Serial loopback (`crates/drm-router/src/transport/serial.rs`,
  `serial_loopback_x10k`): a Unix socketpair carries 10 000 COBS/CRC32C frames;
  the reader reassembles each by its delimiter and decodes byte-exact (I-SR1/
  I-SR4/I-SR6, REQ-OPS-004).
- Cross-OS subset (`xtask/src/cross_os.rs`): the two M12W4 carry-overs
  `win-race-create` and `win-init-reinit` are now green; new `serial-codec`
  (green) and `serial-loopback` (scheduled — socketpair is unix-only, Windows
  COM/named-pipe fixture lands with the M14 matrix) rows. `PORT_GAPS_CLOSED = 1`.
- [tests/conformance/README.md](../../tests/conformance/README.md): transport/
  language matrix marks the Rust TCP/serial cells `R`, and the Windows subset
  table records the green/scheduled states above.
- CI: `ci.yml` `windows-smoke` and `windows-nightly.yml` run the one cross-OS
  subset, now including the Windows lifecycle and serial-codec rows (green ×3
  nightly).

Notes:

- Windows-only rows cannot execute on the Linux authoring host; they are
  cross-compiled clean (`cargo check -p drm-core --tests --target
  x86_64-pc-windows-msvc`) and gated by the `windows-latest` jobs. The full
  Windows crash/kill matrix is M14 (Phase 31 gate M14W2); this week closes the
  scheduled subset rows.
- No architecture divergence: the implementation matches the pinned platform
  divergence rules in windows-backends.md and publisher-lifecycle.md, so no ADR
  or algorithm-doc change landed.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `WIN_PLATFORM_FULL = 1`; `SERIAL_FRAMES = 1`; `SPATIAL_REQ_MINT_START = 37 ≥ 25`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M13W1) = PASS`**

### How to reproduce

```text
cargo xtask test --cross-os
cargo test -p drm-core --lib win_                       # Windows runner
cargo test -p drm-router --lib transport::serial
cargo xtask audit-traceability --registry
cargo check -p drm-core --tests --target x86_64-pc-windows-msvc
```

REQ: REQ-CORE-101, REQ-CORE-098, REQ-CORE-054, REQ-CORE-071, REQ-CORE-074,
REQ-CORE-075, REQ-CORE-057, REQ-QAL-008, REQ-ROUTE-025, REQ-ROUTE-026,
REQ-OPS-004.
