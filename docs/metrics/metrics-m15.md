# Metrics — Month 15 (M15)

Normative source: [work-plan.md](../../work-plan.md) W6 operational tooling.
Phase 32 opens with wave A (introspection, trace, diag, MCU start). One section
per weekly tranche.

## M15W1 — Introspection Tools (echo/hz/log), Diag Daemon Start

| Variable | Target | Measured |
|---|---|---|
| `ECHO_GREEN` | `drm echo` read-only, momentary borrow+copy (I-OB5) | 1 — `drm-cli` `echo_copies_and_reports_empty` copies the newest sample and leaves the diagnostic hazard word `HAZARD_NONE`; `drm-diag` `introspect_no_pin` runs 10 000 `copy_latest` reads with zero pins and the pool still exhausts/recover |
| `HZ_GREEN` | `drm hz` observed rates match injected knowns (I-OB3) | 1 — `drm-diag` `diag_observed_rate` matches 10/100/1000 Hz injected sequences within 1%; `drm-cli` `hz_tolerance` reports a 100 Hz schedule within 1% |
| `DIAG_SKELETON` | out-of-band attach + counter read green (I-OB2) | 1 — `diag_sample` epoch sandwich (`diag_epoch_sandwich`), no data-path mutation (`diag_no_overhead`), deadline matrix (`diag_deadline_matrix`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Trace runtime** (`crates/drm-core/src/trace/mod.rs`): wait-free per-producer
  SPSC lanes (`TraceRingSet`/`TraceLane`/`TraceEvent`), `trace_emit`,
  `trace_register_thread`; drop counters (`I-OB6`). Tests
  `trace_never_blocks` (1/2/4/16 producers, stopped reader; exact drop counts,
  bounded tail cost), `trace_lane_exhaustion`, `trace_drop_counter`.
- **Trace dictionary**: append-only `TraceDict` with
  `dict_version = (format_version, BLAKE3(dict bytes)[..16])`; unknown id renders
  raw hex + version-mismatch marker. Test `trace_dict_roundtrip`.
- **Diagnostics** (`crates/drm-diag`, ops): `ChannelStats`, `ConsumerStats`,
  `DeadlineState`, `LinkStats`, `diag_sample` (epoch sandwich), `deadline_tick`,
  `RateWindow`, `Observer`, `LogCatalog`. Tests `diag_no_overhead`,
  `diag_epoch_sandwich`, `diag_observed_rate`, `diag_deadline_matrix`,
  `introspect_no_pin`, `observer_copies_owned_bytes`, `log_level_filter`,
  `fresh_channel_stats`.
- **Introspection tools** (`drm echo`/`hz`/`log`): read-only subcommands over
  `drm-diag`, resolving deployment metadata and attaching as non-publishing
  observers. Tests `render_modes`, `echo_copies_and_reports_empty`,
  `hz_tolerance`, `render_and_filter`, `parse_levels`, `introspection_shapes`.
- **Spatial parser groundwork**: `parser/spatial.rs` behind feature
  `spatial-grammar` (six typed keywords, frame-path conformance, static/dynamic
  classification); corpus byte-for-byte unaffected.
- **MCU bring-up**: `bindings/c/mcu/` hello-world + Makefile for the two locked
  targets; `.github/workflows/mcu-cross.yml` cross-compile + zero-forbidden-heap
  symbol audit.
- Gate evidence [m15w1.md](../gates/m15w1.md); phase tracker
  [phase-32.md](../gates/phase-32.md); [CHANGELOG.md](../../CHANGELOG.md) M15W1
  tranche.

Notes:

- No architecture divergence: wave A implements pinned invariants
  (observability-plane I-OB1..I-OB7); no ADR landed.
- Live SHM attach for the tools is deferred to the diag session; without a
  running publisher they report `NotFound` (namespace scoping).
- MCU cross-compile executes in CI; the Linux authoring host lacks the pinned
  `arm-none-eabi` toolchain, so the local suite carries the skeleton.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `ECHO_GREEN = 1`; `HZ_GREEN = 1`; `DIAG_SKELETON = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M15W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-core --test trace
cargo test -p drm-diag
cargo test -p drm-cli --lib
cargo test -p drm-workspace --features spatial-grammar spatial
cargo xtask audit-traceability
cargo xtask check-layering
```
