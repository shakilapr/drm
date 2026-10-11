# Metrics — Month 17 (M17)

Normative source: [work-plan.md](../../work-plan.md) W6 operational tooling →
Studio transition. Phase 32 close-out (introspection/trace/RPC gates) opens the
month; the Phase 32 gate and the Studio protocol doc land at M17W2. One section
per weekly tranche.

## M17W1 — Phase 32 Close-Out: Introspection + Trace + RPC Gates

| Variable | Target | Measured |
|---|---|---|
| `INTROSPECT_GATE` | echo/hz/log read-only, no-pin (I-OB5), rate tolerance (I-OB3) | PASS — `drm-diag` 10/10: `introspect_no_pin` (a continuous `copy_latest` observer holds zero hazard slots — `rec0.hazard == HAZARD_NONE` — and the pool stays exhaustible: holding the sole free slot makes `publish` return `PoolExhausted`, proving the observer consumed no capacity), `diag_observed_rate` (10/100/1000 Hz within 1%; a 100-sample burst reports its own high rate and decays to idle), `diag_no_overhead` (sampling leaves every counter untouched), `observer_copies_owned_bytes`/`log_level_filter` (read-only copy + filter) |
| `TRACE_GATE` | never-blocks, drop transparency (I-OB1/I-OB6) | PASS — `drm-core --test trace` 4/4: `trace_never_blocks` (1/2/4/16 producers, stopped reader, tail cost bounded < 60 s; `dropped = producers × (N − CAP)` exactly), `trace_drop_counter` (full lane retains exactly capacity 4 of 10, `total_dropped = 6`), `trace_lane_exhaustion` (`no_lane_drops`, no dynamic growth), `trace_dict_roundtrip` |
| `RPC_GATE` | correlation, cancel, timeout (I-RPC1..I-RPC3) | PASS — `drm-rpc` 12/12: `dispatch_concurrent_no_cross_correlation` (`C ∈ 1..=16`, every client gets its own response, zero stale), `cancel_at_each_phase_boundary` (`k ∈ 0..4`, exactly one `CancelAccepted`, no phase after the token, `Terminal(Cancelled)`), `dispatch_timeout_expires_and_late_terminal_stale` (`RpcTimeout` + best-effort `Cancel`; late terminal dropped stale), NEW `rpc_capacity_busy_gap_forward` (client table full ⇒ `RpcError::Busy`; server table full consumes nothing; retained request forwarded once a slot frees — I-RPC6/I-RPC7) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **RPC capacity + gap-forwarding** (`crates/drm-rpc/src/tests.rs`
  `rpc_capacity_busy_gap_forward`): the `rpc_capacity` cell of the rpc-actions
  test matrix — a full client correlation table returns `RpcError::Busy`; a full
  server operation table leaves the request published; after the admitted
  operation completes, the retained request is admitted on the next reactor pass
  and served (`publish ≠ accepted`, `I-RPC6`/`I-RPC7`).
- **MCU regression wiring** (`.github/workflows/ci.yml` `mcu` job;
  `.github/workflows/mcu-cross.yml` `push: [main]`): the host parity self-test
  and the zero-forbidden-heap symbol audit are now permanent blocking gates on
  every PR/push (regression permanence, AGENTS #14).
- **Gate evidence** ([m17w1.md](../gates/m17w1.md)): I-OB1/I-OB3/I-OB5/I-OB6 and
  I-RPC1..I-RPC3 re-run as close-out evidence; Phase 32 tracker
  [phase-32.md](../gates/phase-32.md) finalized with the deferred post-1.0 Studio
  UI scope list; [CHANGELOG.md](../../CHANGELOG.md) M17W1 tranche.

Notes:

- No architecture divergence: pinned invariants only (observability-plane
  I-OB1/I-OB3/I-OB5/I-OB6, rpc-actions I-RPC1/I-RPC2/I-RPC3/I-RPC6/I-RPC7); no
  ADR.
- The 2 h MCU target soak remains deferred to the `mcu-cross` hardware lane; host
  parity and the zero-forbidden-heap audit are green in-session and now required
  in CI.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `INTROSPECT_GATE = PASS`; `TRACE_GATE = PASS`; `RPC_GATE = PASS`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M17W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-diag
cargo test -p drm-core --test trace
cargo test -p drm-rpc
make -C bindings/c/mcu host-test host-audit
cargo xtask audit-traceability
cargo xtask check-layering
```

## M17W2 — Phase 32 Gate + Studio Protocol Doc

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 32)` | PASS — every submodule conformance suite green | PASS — all Phase 32 suites re-run in-session: `drm-diag` 10/10 (I-OB1..I-OB5/I-OB7), `drm-core --test trace` 4/4 (I-OB1/I-OB2/I-OB6), `drm-rpc` 12/12 (I-RPC1..I-RPC3/I-RPC6/I-RPC7), `drm-record` 18/18 (I-RC1..I-RC6), `drm-router` serial 15/15 (I-SR1..I-SR6), `drm-workspace` handles 10/10 + verify 12/12 (I-H3/I-H4/I-H7), MCU host parity + zero-forbidden-heap audit; `cargo xtask test` green |
| `STUDIO_PROTOCOL_DOC` | merged (protocol-level scope) | merged — [studio-protocol.md](../algorithms/studio/studio-protocol.md): `StudioTransport` + `EventStream`; capability handshake; typed command set; revisioned concurrency (`base_revision`/`CONFLICT`); journaled ChangeSets; conditional undo; format-preserving editing; run-session supervisor; control/data-plane separation; trust gate; extension budgets; I-ST1..I-ST13; explicit post-1.0 deferred list |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Phase 32 gate** ([m17w2.md](../gates/m17w2.md)): criteria → evidence across
  every Phase 32 submodule suite; signed by E4. Tracker
  [phase-32.md](../gates/phase-32.md) status → **PASS** (M17W2); the stale
  "RPC/actions, full diag daemon pending" row resolved to the completed M16/M17
  evidence.
- **Studio protocol doc** ([studio-protocol.md](../algorithms/studio/studio-protocol.md)
  §Formal Merge (M17W2)): protocol-level scope locked; `StudioTransport`
  abstraction and named events (`workspaceChanged`, `diagnosticsChanged`,
  `buildOutput`, `nodeStarted`, `nodeStopped`, `runtimeStats`); I-ST1..I-ST10
  (plus I-ST11..I-ST13) defined; explicit deferred post-1.0 list (creation
  dialogs, editors, Inspector UI, command palette/keyboard, DataStore UI, remote
  web mode, web sandbox, native adapters, `.drmx`, `drm-visual-schemas`).
- **Registry + changelog**: [CHANGELOG.md](../../CHANGELOG.md) M17W2 tranche;
  [registry.csv](../req/registry.csv) `REQ-STUDIO-*` rows already map the doc's
  test matrix (protocol-level, `specified`).

Notes:

- No architecture divergence: pinned invariants only; no ADR.
- Studio scope stays protocol-level; UI surfaces are deferred post-1.0. Studio
  implementation (`drm studio --stdio` host, JSON-RPC dispatch, TS codegen)
  starts M17W3.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `GATE_STATUS(Phase 32) = PASS` (E4 signs)
- [x] `STUDIO_PROTOCOL_DOC = merged`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(Phase 32) = PASS`**

### How to reproduce

```text
cargo test -p drm-diag
cargo test -p drm-core --test trace
cargo test -p drm-rpc
cargo test -p drm-record
cargo test -p drm-router --lib transport::serial
cargo test -p drm-workspace --test handle_stability --test verify_engine
make -C bindings/c/mcu host-test host-audit
cargo xtask audit-traceability
cargo xtask check-layering
```

## M17W3 — Studio Protocol Implementation (Host + Scaffold)

| Variable | Target | Measured |
|---|---|---|
| `STUDIO_PROTOCOL_GREEN` | handshake + typed JSON-RPC round-trip engine↔renderer green (I-ST1..I-ST4) | PASS — `cargo test -p drm-cli studio_host` 6/6 plus `studio_snapshot_reports_problems`: `studio_host_handshake_and_dispatch` (`Hello` → `HelloAck` → `OpenWorkspace` → `Snapshot`/`Verify` over stdio), `studio_host_handshake_required` (non-`Hello` first request rejected), `studio_host_parity_cli_vs_studio` (CLI `assign` and Studio `NodeUpdate` produce identical renders, I-ST1), `studio_host_capability_model` (`Trust`/`ExtensionsWeb` only when trusted, I-ST4), `studio_host_frame_bounds` (oversize frame rejected pre-parse, I-ST13), `studio_host_format_preservation` (comments/order survive, I-ST6) |
| `MUTATIONS_REVISIONED` | `base_revision`/`CONFLICT` semantics green (I-ST7) | PASS — `studio_revision_conflict` (stale `base_revision` ⇒ `Conflict{revision}`, revision unchanged), `studio_undo_guard` (external change ⇒ `ConflictExternalChanged`; `force` restores prior bytes verbatim, I-ST9), `journal_recovery_prepared_rolls_back`/`journal_recovery_partial_finishes`/`journal_commit_clean` (I-ST8/I-ST12) |
| `STUDIO_SCAFFOLD` | React 19 + Vite + TS strict app boots; renderer import boundaries enforced | PASS — `apps/studio` (Vite + React 19 + TS strict + Tailwind/Radix shell; Zustand `UiState`), ESLint flat config bans Electron main + Node built-ins in `src/**`; `packages/studio-protocol` generated TS |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Studio host** (`crates/drm-cli/src/studio/`): `protocol` (typed request/
  response enums, `parse_bounded` I-ST13), `host` (`StudioHost` + `serve_stdio`,
  trust + revision guards), `journal` (six-step journaled `ChangeSet` commit +
  idempotent crash recovery, I-ST8/I-ST12), `edit` (`toml_edit` format-preserving
  manifest mutation, I-ST6). `drm studio --stdio [--trust]` wired in
  `args.rs`/`lib.rs` (REQ-STUDIO-011).
- **Generated TS protocol** (`packages/studio-protocol/src/index.ts`,
  `@generated by drm-studio-codegen v1`): `xtask gen-check` extended via
  `xtask/src/studio_ts.rs`; a hand edit fails the check (REQ-STUDIO-006).
- **Renderer scaffold** (`apps/studio`): Vite + React 19 + TS strict, Tailwind +
  Radix shell, Zustand UI store (presentation only), typed JSON-RPC client over
  the stdio transport, boot UI rendering a workspace snapshot.
- **Gate evidence** ([m17w3.md](../gates/m17w3.md)); [CHANGELOG.md](../../CHANGELOG.md)
  M17W3 tranche.

Notes:

- No architecture divergence: pinned invariants only; no ADR.
- Carried to M17W4: remaining `ModelCommand` variants, run-session supervisor
  process, extension host.

### Weekly gate checklist (chaired by E2, Friday)

- [x] `STUDIO_PROTOCOL_GREEN = 1`; `MUTATIONS_REVISIONED = 1`; `STUDIO_SCAFFOLD = 1`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M17W3) = PASS`**

### How to reproduce

```text
cargo test -p drm-cli studio
cargo run -p xtask -- gen-check
cargo xtask audit-traceability
cargo xtask check-layering
```

## M17W4 — Studio Inspectors + Run Supervisor, Month-17 Review

| Variable | Target | Measured |
|---|---|---|
| `STUDIO_MVP` | read-only inspectors functional over RPC (no GUI DRM logic) | 1 — `apps/studio` `WorkspaceActivity`/`TopicsActivity`/`NodesActivity`/`SystemActivity` + `ProblemsPanel` render engine snapshots via `rpc/queries.ts` (TanStack Query); `rpc/model.ts` is display-only; mutation audit [studio-mutation-audit.md](../spec/studio-mutation-audit.md) clean (I-ST1, `REQ-STUDIO-003`/`REQ-STUDIO-015`) |
| `RUN_SUPERVISOR` | sessions outlive UI; close prompts Leave-running/Stop (I-ST10) | 1 — `cargo test -p drm-cli supervisor` 7/7: `supervisor_start_stop_lifecycle` (id/start time/node pid/stdout/status), `supervisor_outlives_ui` (disconnect leaves running; close ⇒ `Prompt`; Stop ends), `supervisor_disconnect_stops_when_config_says_so`, `supervisor_restart_replaces_processes`, `supervisor_unknown_session_is_error`, `supervisor_close_clear_when_idle`, plus host `supervisor_outlives_ui` |
| `MONTH17_REVIEW` | ON_TRACK | ON_TRACK — [month-17.md](../gates/month-17.md) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Run-session supervisor** (`crates/drm-cli/src/studio/supervisor.rs`):
  `Supervisor` owns `RunSession`s (id, start time, node processes with
  pid/stdout/stderr/status, run config) independently of any renderer
  connection. `detach` stops only sessions whose `leave_running` is false;
  `close_decision` returns `Prompt` with running session ids so the UI offers
  Leave-running/Stop; `apply_close` never kills on Leave-running. Node launching
  is a `ProcessRunner` seam (`CommandRunner` captures output on reader threads)
  so tests spawn no OS processes. `StudioHost` routes `RunStart`/`RunStop`/
  `RunRestart` and calls `on_disconnect` when the renderer closes
  (`REQ-STUDIO-010`).
- **Read-only inspectors** (`apps/studio/src/activities/`): Workspace tree,
  Topics list/table, Nodes list, System (computes/links/placements), all
  rendering the engine snapshot through TanStack Query; zero mutation logic
  (`REQ-STUDIO-015`).
- **Problems panel** (`apps/studio/src/components/ProblemsPanel.tsx`): verify
  diagnostics grouped by severity class; the wire `Diagnostic.severity` is a
  typed `Severity` (`error`/`warning`/`runtime`/`info`) in
  `packages/studio-protocol`, classified by `severity_for_check`
  (`REQ-STUDIO-016`).
- **Mutation audit** ([studio-mutation-audit.md](../spec/studio-mutation-audit.md)):
  every GUI-visible request maps to exactly one engine operation; zero
  renderer-side DRM logic (I-ST1).
- **Gate evidence** ([m17w4.md](../gates/m17w4.md)) + month review
  ([month-17.md](../gates/month-17.md)); [CHANGELOG.md](../../CHANGELOG.md)
  M17W4 tranche.

Notes:

- No architecture divergence: pinned I-ST10 plus ui-arch §Activities/§Problems
  Panel; no ADR.
- Deferred post-1.0 unchanged: creation UI, editors, Inspector forms, command
  palette/keyboard map, DataStore UI, remote web mode, native adapters, `.drmx`,
  `drm-visual-schemas`.

### Weekly gate checklist (chaired by E3, Friday)

- [x] `STUDIO_MVP = 1`; `RUN_SUPERVISOR = 1`; `MONTH17_REVIEW = ON_TRACK`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M17W4) = PASS`**

### How to reproduce

```text
cargo test -p drm-cli supervisor
cargo test -p drm-cli studio
cargo run -p xtask -- gen-check
cargo xtask audit-traceability
cargo xtask check-layering
```
