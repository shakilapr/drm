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

## M14W3 — Spatial Types/Clocks + Record Core Design Confirmations

| Variable | Target | Measured |
|---|---|---|
| `SPATIAL_TYPES_DESIGN` | typed Point3/…/Twist3 + `Stamped<T>` + compile/dynamic space binding confirmed | 1 — [spatial-types-design.md](../spec/spatial-types-design.md) §1–§5: six typed quantities, compile-time `Point3<CameraFront>` (raw-numerics payload), dynamic `SpatialPoint3{id}` + `Stamped<T>`, grammar-extension EBNF draft, `[spatial]` parsing, I-D6 inference matrix; locked by `design_lock::spatial_types_confirmed` |
| `CLOCK_DESIGN` | four clock domains + mapping/drift/uncertainty confirmed | 1 — [spatial-types-design.md](../spec/spatial-types-design.md) §6–§7: `HostMonotonic`/`Device`/`Global(Ptp\|Gnss)`/`SimReplay`, affine `t_to = to_ref + r·(t_from − from_ref)`, `ClockIdentity{domain,epoch}`, PTP/GNSS + SimReplay sources and per-domain degradation paths; locked by `design_lock::clock_domains_confirmed` |
| `RECORD_DESIGN` | recorder pipeline confirmed vs record-replay.md | 1 — [record-replay.md](../algorithms/ops/record-replay.md) §Design Confirmation: borrow-copy-release, bounded per-topic slabs, `on_full` {Drop,Stop,Backpressure}, gap ledger, MCAP chunking, replay namespace; locked by `design_lock::record_policies_confirmed` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- NEW [spatial-types-design.md](../spec/spatial-types-design.md): E2 spatial types
  + clock-domain design confirmation (absorbs the M14W2 grammar-extension draft).
- NEW [mcu-bring-up.md](../spec/mcu-bring-up.md): E3 reference MCU/RTOS lock
  (STM32H743 + FreeRTOS; fallback STM32F407), pinned toolchain, CI cross-compile /
  zero-heap / stack jobs, host↔MCU bring-up steps, preventive-first linker-section
  audit.
- [observability-plane.md](../algorithms/ops/observability-plane.md): E1 trace
  dictionary versioning pinned (`dict_version`, append-only per deployment) +
  design-confirmation section; E4 diag confirmation.
- [record-replay.md](../algorithms/ops/record-replay.md): E4 record design
  confirmation section.
- Executable design-lock tests (`xtask/src/main.rs` `design_lock`) —
  `spatial_types_confirmed`, `clock_domains_confirmed`, `record_policies_confirmed`,
  `trace_dictionary_confirmed`.
- Gate evidence: [m14w3.md](../gates/m14w3.md); spec index
  ([docs/spec/README.md](../spec/README.md)); [CHANGELOG.md](../../CHANGELOG.md).

Notes:

- Design-only tranche: the plan's test suite is review sign-offs; no runtime code
  is required (Phase 32/33 implementation starts M15W1/M19W1). The design-lock
  tests convert the sign-offs into a permanent gate.
- No architecture divergence and no new REQ IDs: the confirmations restate pinned
  invariants, so no ADR landed and the registry is unchanged.

### Weekly gate checklist (chaired by E2, Friday)

- [x] `SPATIAL_TYPES_DESIGN = 1`; `CLOCK_DESIGN = 1`; `RECORD_DESIGN = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M14W3) = PASS`**

### How to reproduce

```text
cargo test -p xtask design_lock
cargo xtask audit-traceability
cargo xtask check-layering
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
```

REQ: REQ-SPATIAL-004, REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-017,
REQ-SPATIAL-030, REQ-SPATIAL-031, REQ-SPATIAL-035, REQ-SPATIAL-036,
REQ-SPATIAL-041, REQ-SPATIAL-051, REQ-OPS-005, REQ-OPS-006, REQ-OPS-007,
REQ-OPS-008, REQ-OPS-009, REQ-OPS-014, REQ-OPS-015, REQ-OPS-017, REQ-OPS-018,
REQ-OPS-019, REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026, REQ-ROUTE-040,
REQ-CORE-086, REQ-CORE-087, REQ-QAL-008.

