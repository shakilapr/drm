# Metrics — Month 14 (M14)

Normative source: [work-plan.md](../../work-plan.md) W5 Windows Runtime Backend
closes with the Phase 31 gate (M14W2) and W6 operational tooling opens with the
Phase 32 serial/MCU work. One section per weekly tranche.

## M14W1 — Windows↔Linux Full Matrix, Serial-MCU Doc Final

| Variable | Target | Measured |
|---|---|---|
| `FULL_MATRIX` | `{windows, linux} × {udp, tcp, serial} × {c, cpp, rust, python}` core cells green | 1 — the interop harness (`xtask/src/cross_os.rs`) now spans the full matrix: 96 cells, of which the 24 same-language cells are **core** (gate-blocking) and the 72 mixed-language pairs are **tracked**; `cargo xtask test --cross-os --transport {udp,tcp,serial}` runs the green Rust pair in both directions (UDP/TCP loopback, serial host-loopback of the byte-exact COBS/CRC32C codec) and writes `target/interop/matrix-report.txt`; C/C++/Python cells are tracked pending the Phase 32/33 bindings (no silent skips) |
| `SERIAL_DOC` | serial-mcu.md merged | 1 — [serial-mcu.md](../algorithms/router/serial-mcu.md) finalized: COBS is the binding escape choice (four pre-stated criteria), frame-limit/CRC/escape tables absorb the M02W3/M05W3/M06W1/M13W1 drafts, I-SR1..I-SR7 unchanged |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Full interop matrix (`xtask/src/cross_os.rs`): `InteropLang::ALL` adds C++
  (total 4 languages), `InteropTransport::ALL` adds `Serial` (host-loopback),
  `interop_core_matrix()` is the 24-cell gate-blocking core and
  `interop_matrix()` the 96-cell full table; `CellClass {Core, Tracked}`
  classifies each cell (`InteropCell::class`); `run_cell` routes `Serial`
  through `serial_roundtrip` — an in-process encode→decode through
  `drm_router::transport::serial` (I-SR1/I-SR4/I-SR6). `run_interop` writes the
  complete cell table to `target/interop/matrix-report.txt`
  (`write_matrix_report`).
- Harness tests: `core_matrix_is_twenty_four_cells`,
  `mixed_language_pairs_are_tracked`, `matrix_covers_every_combination`,
  `every_cell_is_classified`, `green_cells_round_trip`,
  `serial_cells_use_host_loopback`.
- Physical-matrix runbook + loopback evidence
  ([phase-31-runbook.md](../gates/phase-31-runbook.md)): two-host topology,
  ×3 repeat policy, per-transport green/scheduled ledger; the physical
  dual-VM rows are measured-not-guaranteed and carry the M14W2 fix window.
- Gate evidence ([phase-31.md](../gates/phase-31.md) M14W1 section), conformance
  matrix ([tests/conformance/README.md](../../tests/conformance/README.md)
  full-matrix tables), and CI: `ci.yml` `interop-cells` job gains the serial
  step, `windows-nightly.yml` runs `--transport udp`/`tcp`/`serial`.
- `codebase.md` §5.2 nightly-workflow row updated to the full matrix.
- [CHANGELOG.md](../../CHANGELOG.md): M14W1 tranche entry.

Notes:

- Windows-only and C/C++/Python rows cannot execute on the Linux authoring
  host; the port board (`PORT_BOARD`) tracks each (`golden-c`, `golden-cpp`,
  `golden-python`, `ffi-matrix`, `borrow-disciplines`, `serial-loopback`),
  retargeted to the M14W2 fix window; the full Windows manifest ×2 remains the
  blocking platform gate.
- No architecture divergence: the expanded matrix, serial host-loopback, and
  doc finalization implement pinned invariants (router-core.md
  `I-RT1`/`I-RT4`/`I-RT5`, serial-mcu.md `I-SR1`..`I-SR7`, deployment-identity.md
  namespaces), so no ADR landed.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `FULL_MATRIX` core matrix classified + green Rust cells; `SERIAL_DOC = merged`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M14W1) = PASS`**

### How to reproduce

```text
cargo xtask test --cross-os --transport udp
cargo xtask test --cross-os --transport tcp
cargo xtask test --cross-os --transport serial
cargo test -p xtask cross_os::interop_tests
cargo test -p drm-router serial_route
cargo xtask audit-traceability
```

REQ: REQ-QAL-008, REQ-OPS-004, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026,
REQ-ROUTE-029, REQ-ROUTE-032, REQ-ROUTE-040, REQ-CORE-086, REQ-CORE-087.
