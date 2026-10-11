# Studio Protocol & Engine Bridge Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: the engine↔Studio bridge — generated TypeScript types + JSON-RPC schema (`packages/studio-protocol`), `drm studio --stdio` host, capability handshake, revisioned optimistic concurrency, durable journaled ChangeSets with conditional undo, format-preserving editing, run-session supervisor, command/data-plane separation, workspace-trust gating, and renderer capability model. Implements [ui-architecture.md](../../ui-architecture.md) Core Principles, §Process Architecture, §Protocol, §Transactional Mutations, §Run Session Supervisor. Owner: E2 (protocol generation, stdio host) + E4 (supervisor). Phase 32. UI state/streams: [ui-architecture.md](../../ui-architecture.md) §Semantic vs UI State, §Frontend State. Extensions: §Extension Platform. Depends on [workspace-model.md](../toolchain/workspace-model.md), [verify-engine.md](../toolchain/verify-engine.md), [handle-system.md](../toolchain/handle-system.md), [build-orchestration.md](../toolchain/build-orchestration.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-STUDIO-engine-only-mutations` | every mutation executed and validated by the Rust engine; CLI/Studio cannot disagree | `REQ-STUDIO-003` |
| `REQ-STUDIO-generated-protocol` | `studio-protocol` is codegen output; hand edits rejected (R6) | `REQ-STUDIO-006` |
| `REQ-STUDIO-handshake` | first RPC is a capability handshake — `hello {protocol_version, capabilities}`; frontend hides unavailable features | `REQ-STUDIO-007` |
| `REQ-STUDIO-typed-commands` | ui-arch typed command set (Workspace/Package/Node/Topic/Compute/Link/Build/Run) | `REQ-STUDIO-014` |
| `REQ-STUDIO-two-planes` | JSON-RPC for commands/state; binary frames for topic samples/video — never mixed | `REQ-STUDIO-013` |
| `REQ-STUDIO-capability-model` | renderer receives domain capabilities, never machine access | `REQ-STUDIO-001` |
| `REQ-STUDIO-revisioned-concurrency` | snapshot carries monotonic `revision`; mutations carry `base_revision`; stale ⇒ `CONFLICT{revision}` — no last-writer-wins | `REQ-STUDIO-009` |
| `REQ-STUDIO-journaled-changesets` | multi-file operations journaled (validate → temps → journal → rename → commit → cleanup); crash recovery on startup | `REQ-STUDIO-008` |
| `REQ-STUDIO-conditional-undo` | undo reverts the ChangeSet only when expected input hashes still match; external changes ⇒ Compare/Cancel/Force | `REQ-STUDIO-002` |
| `REQ-STUDIO-format-preserving` | engine never rewrites a file it did not author structurally (`toml_edit`/CST; comments/whitespace/order survive) | `REQ-STUDIO-005` |
| `REQ-STUDIO-run-supervisor` | run sessions outlive the Studio window; close prompts Leave-running/Stop; disconnect never orphans or kills | `REQ-STUDIO-010` |
| `REQ-STUDIO-stdio-host` | `drm studio --stdio` host; no HTTP server, no ports; Studio holds no DRM logic | `REQ-STUDIO-011` |
| `REQ-STUDIO-trust-gate` | untrusted workspace: view-only; Build/Run/extensions gated behind `[ Trust Workspace ]` | `REQ-STUDIO-012` |
| `REQ-STUDIO-ext-budgets` | extension host: wasmtime fuel + memory caps; deterministic termination | `REQ-STUDIO-004` |

## Data Structures

```rust
// studio-protocol: generated TS types + JSON-RPC schema from Rust request/response enums (codegen-invalidation pipeline)

// Capability handshake (first RPC — never an exact-version requirement)
pub struct Hello { protocol_version: u32, client_version: String }
pub struct HelloAck { protocol_version: u32, host_version: String, capabilities: Vec<Capability> }
pub enum Capability { WorkspaceV2, ExtensionsWeb, BinaryStreamsV1, RuntimeStats, Trust }

// Typed command set (ui-architecture §Protocol)
pub enum StudioRequest {
    Hello(Hello),
    OpenWorkspace { path: PathBuf },
    Snapshot,                                  // → WorkspaceSnapshot { revision, model render, diagnostics }
    Verify,                                    // → verify-engine report (facts/estimates labeled)
    Mutate { base_revision: u64, command: ModelCommand },
    BuildStart { target: Option<NodeName> }, BuildCancel,
    RunStart { config: RunConfig }, RunStop { session: SessionId }, RunRestart { session: SessionId },
    Undo { changeset: ChangeSetId, force: bool }, // force requires explicit confirmation + trust
    SubscribeTelemetry { channels: Vec<TelemetryChannel> },
}
pub enum ModelCommand {
    PackageCreate(PackageSpec), PackageRename { from: PackageName, to: PackageName }, PackageDelete(PackageName),
    NodeCreate(NodeSpec), NodeUpdate(NodeSpec), NodeDelete(NodeName),
    TopicCreate(TopicSpec), TopicUpdate(TopicSpec), TopicRename { from: TopicPath, to: TopicPath }, TopicDelete(TopicPath),
    ComputeCreate(ComputeSpec), ComputeUpdate(ComputeSpec),
    LinkCreate(LinkSpec), LinkUpdate(LinkSpec),
}
pub enum StudioResponse {
    HelloAck(HelloAck),
    Snapshot(WorkspaceSnapshot),               // { revision: u64, model render, problems }
    MutationResult { revision: u64, render: ModelRender },
    Conflict { revision: u64 },                // base_revision stale — UI offers Review/Reload/Retry
    BuildEvent(BuildEvent), RunEvent(RunEvent),
}
pub struct WorkspaceSnapshot { revision: u64, model: ModelRender, problems: Vec<Diagnostic> }

// Journaled ChangeSet (§Journaled ChangeSets — "atomic" = journaled)
pub struct ChangeSet {
    txn_id: Uuid,
    expected_input_hashes: Vec<FileHash>,      // conditional undo guard (I-ST9)
    outputs: Vec<(PathBuf, PreparedTemp, PriorContent)>, // rollback data included
    state: JournalState,                       // Prepared | Committed | RolledBack
}

// Run session (supervisor-owned; outlives Studio window)
pub struct RunSession {
    id: SessionId, started_at: u64,
    nodes: Vec<NodeProcess>,                   // pid, stdout/stderr capture, status
    run_config: RunConfig,                     // disconnect policy lives here
}

// Frontend transport abstraction (ui-architecture §Control and Data Planes):
// desktop stdio and remote web share one React app; only the transport changes.
pub trait StudioTransport {
    fn request(&mut self, command: StudioRequest) -> Result<StudioResponse, Status>;
    fn events(&mut self) -> EventStream;
    fn open_data_stream(&mut self, request: StreamRequest) -> BinaryStream; // control plane never carries sample bytes (I-ST3)
}
pub enum EventStream {
    WorkspaceChanged, DiagnosticsChanged, BuildOutput,
    NodeStarted, NodeStopped, RuntimeStats,
}
```

## Invariants

- **I-ST1 SingleSemantics**: every mutation goes through the engine's parse→model→verify pipeline (workspace-model + verify-engine); Studio never validates independently. Test: mutation-parity test — same edits via CLI and Studio produce identical model renders.
- **I-ST2 GeneratedOnly**: `packages/studio-protocol` regenerated by `xtask gen-check`; hand edit ⇒ CI fail (R6). Test: gen-check.
- **I-ST3 PlaneSeparation**: no topic sample bytes ever travel in JSON-RPC; no commands in binary plane. Test: protocol audit fixture.
- **I-ST4 CapabilityModel**: renderer receives capability lists (allowed topics/actions), never raw machine/SHM access. Test: renderer-import lint (no Electron/Node built-ins per §3.5).
- **I-ST5 ExtensionBudgets**: extensions run in wasmtime with fuel + memory caps; budget exhaustion ⇒ deterministic termination, engine unaffected. Test: `ext_budget` fixtures.
- **I-ST6 FormatPreserving**: the engine never rewrites a file it did not author structurally — manifest edits via `toml_edit`; `.drm` via CST or declared canonical formatting policy; comments/whitespace/field order survive. Test: `edit_preservation` fixture.
- **I-ST7 RevisionGuard**: mutations accepted only when `base_revision` == current snapshot revision; stale ⇒ `CONFLICT{revision}`; workspace-level revision first (entity-level later if needed); no CRDTs, no collaborative editing. Test: `studio_revision_conflict`.
- **I-ST8 JournalAtomicity**: no filesystem operation commits four files atomically — journaled per §Journaled ChangeSets (validate → prepare → journal → rename → commit → remove journal); crash between journal and rename ⇒ startup detects journal and finishes or rolls back. Test: deterministic kill-point fixture.
- **I-ST9 UndoConditional**: undo reverts the original ChangeSet only when expected input hashes still match; externally changed files cannot be silently overwritten (Compare/Cancel/Force). Undo operates on file transactions, never React state. Test: `studio_undo_guard`.
- **I-ST10 SupervisorOutlivesUi**: run sessions owned by the supervisor process — closing Studio prompts Leave-running/Stop; remote disconnect leaves nodes running unless run_config says otherwise; closing a window can never kill or orphan robot processes. Test: `supervisor_outlives_ui`.
- **I-ST11 TrustGatesExecution**: untrusted workspace ⇒ view manifests/browse/inspect only; Build/Run/workspace extensions/native adapters gated behind `[ Trust Workspace ]`; a cloned repository must not execute code by being opened. Test: `trust_gate` fixture.
- **I-ST12 DurableJournal**: prepared outputs and journal are flushed before rename; renamed files and parent directories are flushed before commit cleanup. Recovery is idempotent after every kill point. Test: `journal_durability`.
- **I-ST13 BoundedControlFrames**: JSON-RPC line length, collection counts, and decoded request size are bounded before allocation-heavy parsing. Test: `studio_frame_bounds`.

## Functions

```rust
// drm-cli::studio --stdio host
fn serve_stdio(mut rx: impl Read, tx: impl Write) -> Result<(), Status> {
    let first: StudioRequest = parse_bounded(rx.next_line_bounded(MAX_CONTROL_BYTES)?)?;
    let hello = handshake(first)?;                       // MUST be Hello; capability handshake
    tx.write(HelloAck(hello).to_json_line())?;
    for line in rx.json_lines_bounded(MAX_CONTROL_BYTES) {
        let req: StudioRequest = parse_bounded(line)?;
        let resp = dispatch(req)?;                       // engine calls only — single-semantics (I-ST1)
        tx.write(resp.to_json_line())?;
    }
    Ok(())
}

fn dispatch(req: StudioRequest) -> Result<StudioResponse, Status> {
    match req {
        OpenWorkspace { path } => {
            let root = authorize_workspace_root(path)?;  // remote roots are host-configured allowlists
            if trusted(&root) { recover_journals_durable(&root)?; } // untrusted open stays read-only
            let snapshot = workspace_model::build_without_execution(tree_no_hooks(&root))?;
            Ok(snapshot_rev_with_capabilities(snapshot, trusted(&path))) // untrusted remains view-only
        }
        Mutate { command, base_revision } => {
            require_trusted_workspace()?;                // I-ST11: untrusted is view-only
            expect_revision(base_revision)?;             // I-ST7: stale ⇒ CONFLICT{revision}
            validate_paths_beneath_workspace_no_symlink_escape(&command)?;
            let new = apply_edit_preserving(load()?, command)?;           // I-ST6 (toml_edit/CST)
            let report = verify::run(&new)?;             // gate mutations (verify-engine)
            let cs = journal(new_outputs)?;              // I-ST8 journaled commit
            cs.commit()?;                                // rename prepared files; remove journal
            Ok(MutationResult { revision: new.rev, render: render(new) })
        }
        Undo { changeset, force } => { require_trusted_workspace()?; undo_conditional(changeset, force) },
        BuildStart { .. } => { require_trusted_workspace()?; supervisor::build(..).map(render) },
        RunStart { config } => { require_trusted_workspace()?; supervisor::start_session(config).map(render) },
        RunStop { session } | RunRestart { session } => { require_trusted_workspace()?; supervisor::control(session) },
        SubscribeTelemetry { .. } => open_binary_stream_capability(..), // token only; bytes never JSON
    }
}

fn undo_conditional(cs: ChangeSetId, force: bool) -> Result<StudioResponse, Status> {
    let cs = journal_load(cs)?;
    if inputs_unchanged(&cs.expected_input_hashes) { cs.revert().map(render) }
    else if force && explicit_force_confirmation_present() { cs.revert().map(render) }
    else { Ok(ConflictExternalChanged) }                 // UI offers Compare/Cancel/Force (I-ST9)
}
```

## Protocols

*Session handshake:*

1. Renderer connects over stdio; sends `Hello{protocol_version, client_version}`.
2. Host replies `HelloAck{protocol_version, host_version, capabilities}`.
3. Renderer gates features on declared capabilities — hides unavailable ones (ui-arch §Protocol Versioning).

*Mutation flow (I-ST1/I-ST6/I-ST7/I-ST8):*

1. Studio sends domain command with `base_revision` (never raw file writes).
2. Engine: trust + revision + contained-path checks → format-preserving apply → re-parse → verify.
3. Verify failure: full failure report (check-IDs + REQ citations); no partial apply, revision unchanged.
4. Success: flush temps → flush journal → rename → flush parents → mark committed → remove journal; response = new revision + canonical render.

*Crash recovery (I-ST8):*

1. Trusted startup scans workspace journals; untrusted open reports pending recovery without writing.
2. Journal found in Prepared/Committed-partial state ⇒ finish renames or roll back to input hashes.
3. Journal removed; workspace consistent.

*Run sessions (I-ST10):*

1. `RunStart` creates a supervisor-owned session (host process family).
2. Studio disconnect/close ⇒ prompt Leave-running/Stop; sessions persist per run_config.
3. `runtimeStats`/`nodeStarted`/`nodeStopped` events stream to attached UIs.

*Trust (I-ST11):*

1. First open of a workspace without a trust record ⇒ restricted capability set.
2. `[ Trust Workspace ]` records trust (machine-local); Build/Run/extensions/native adapters unlock.
3. Trust and authentication are independent questions — MVP remote access stays localhost + SSH/VPN.

## Design Confirmation (M14W4)

Studio bridge design confirmed against [ui-architecture.md](../../ui-architecture.md) §Process
Architecture/§Protocol/§Transactional Mutations and this doc's I-ST1..I-ST13; the survey is
recorded in [studio-prep-survey.md](../../spec/studio-prep-survey.md). No invariant changed.
The formal Studio doc is authored M17W2; this week locks the approach.

- [x] `drm studio --stdio` host shape: `serve_stdio` reads bounded JSON-RPC lines, requires `Hello` first, replies `HelloAck`, then dispatches engine calls; no HTTP server, no ports, Studio holds no DRM logic (I-ST1, `REQ-STUDIO-011`).
- [x] Generated TS types pipeline: `packages/studio-protocol` is codegen output regenerated by `xtask gen-check`; hand edits rejected (R6) (I-ST2, `REQ-STUDIO-006`).
- [x] Engine-only mutation flow: every `Mutate` runs trust + `base_revision` + contained-path checks, format-preserving apply (`toml_edit`/CST), re-parse, verify, then a journaled commit; Studio never validates independently (I-ST1/I-ST6/I-ST7/I-ST8, `REQ-STUDIO-003`/`REQ-STUDIO-005`/`REQ-STUDIO-009`).
- [x] Capability model: the handshake exchanges `Hello {protocol_version, capabilities}`/`HelloAck`; the renderer gates features on declared `Capability` values and receives no raw machine/SHM access (I-ST4, `REQ-STUDIO-001`/`REQ-STUDIO-007`).
- [x] Two planes stay separate: commands/state over JSON-RPC; topic samples/video over binary frames — never mixed (I-ST3, `REQ-STUDIO-013`).

Deferred scope (post-1.0): creation UI, editors, Inspector, and remote-web serving. Deltas:
none — this section records the review; implementation starts Phase 32/33 (M15W1+).

## Formal Merge (M17W2)

The protocol-level doc is merged at M17W2 (E4 author, E2 review). Scope is
protocol-level only: the engine bridge, transport abstraction, handshake,
revisioned concurrency, journaled ChangeSets, conditional undo,
format-preserving editing, run-session supervisor, control/data-plane
separation, trust gate, and extension budgets. Invariants I-ST1..I-ST10 (plus
I-ST11..I-ST13) are defined above; the test matrix names each anchor. Creation
UI/editors/Inspector/remote-web are deferred — see the explicit list below.

### Deferred Post-1.0 Scope (explicit)

| Item | Disposition |
|---|---|
| Creation dialogs (wizards/forms) | deferred post-1.0 |
| Editors (text/graph) | deferred post-1.0 |
| Inspector UI | deferred post-1.0 |
| Command palette / keyboard map | deferred post-1.0 |
| DataStore UI | deferred post-1.0 |
| Remote web mode | deferred post-1.0; direct listening disabled until TLS, origin validation, authentication |
| Web sandbox | deferred post-1.0 |
| Native adapters | deferred post-1.0 |
| `.drmx` packaging | deferred post-1.0 |
| `drm-visual-schemas` | deferred post-1.0 |

## Implementation (M17W3)

The host slice lands in `crates/drm-cli/src/studio/`: `protocol` (wire types +
bounded parse), `host` (`StudioHost` + `serve_stdio`), `journal` (six-step
journaled commit + crash recovery), `edit` (`toml_edit` format-preserving
manifest mutation). `drm studio --stdio [--trust]` is the CLI entry
(`REQ-STUDIO-011`). Generated TS types live in `packages/studio-protocol`
(`REQ-STUDIO-006`); the renderer scaffold is `apps/studio`.

Implemented invariants and test anchors (`cargo test -p drm-cli studio`):
I-ST1 (`studio_host_parity_cli_vs_studio`), I-ST2 (`xtask gen-check`),
I-ST4 (`studio_host_capability_model`), I-ST6
(`studio_host_format_preservation`), I-ST7 (`studio_revision_conflict`),
I-ST8 (`journal_recovery_prepared_rolls_back`/`journal_recovery_partial_finishes`),
I-ST9 (`studio_undo_guard`), I-ST11 (`studio_trust_gate`), I-ST13
(`studio_host_frame_bounds`). Carried to M17W4: the remaining `ModelCommand`
variants (package/node/topic/compute/link create/delete beyond `NodeUpdate`),
the run-session supervisor process, and the extension host.

## Implementation (M17W4)

The run-session supervisor lands in `crates/drm-cli/src/studio/supervisor.rs`
(`REQ-STUDIO-010`). It is owned by the host process, never by a renderer
connection: a `RunSession` carries an id, start time, node processes (pid,
captured stdout/stderr, status), and the `RunConfig` whose `leave_running`
flag is the disconnect policy. `RunStart`/`RunStop`/`RunRestart` dispatch to
`Supervisor::start`/`stop`/`restart`; the stdio loop calls
`StudioHost::on_disconnect` when the renderer closes, and `Supervisor::detach`
stops only sessions whose config forbids leaving them running. A window close
with running nodes returns `CloseDecision::Prompt` so the UI can offer
Leave-running/Stop — a close can never kill or orphan a node process.

Node launching is a `ProcessRunner` seam (injectable, so tests never spawn OS
processes); `CommandRunner` is the concrete runner and captures stdout/stderr
on reader threads. Read-only inspectors (`Workspace`/`Topics`/`Nodes`/`System`)
and the Problems panel render engine snapshots only — mutations stay engine
capability only (I-ST1). The `apps/studio` renderer holds zero DRM logic.

Implemented invariants and test anchors (`cargo test -p drm-cli supervisor`):
I-ST10 (`supervisor_start_stop_lifecycle`, `supervisor_outlives_ui`,
`supervisor_disconnect_stops_when_config_says_so`,
`supervisor_close_prompts_leave_or_stop`,
`supervisor_restart_replaces_processes`). Mutation-audit evidence:
[studio-mutation-audit.md](../../spec/studio-mutation-audit.md). Carried to
M18: extension host, remaining `ModelCommand` variants, Graph/Run panels,
Electron packaging.

## Implementation (M18W2)

The Run panel backend extends the studio protocol with two read-only requests:
`RunSessions` returns the supervisor-owned `RunSessionView` snapshots (id, start
time, status, `leave_running`, node processes with pid/stdout/stderr/status) and
`RuntimeStats` returns per-node observed statistics (`session`, `node`, `pid`,
`status`) — the `runtimeStats` surface (ui-arch §Protocol events). Both route
through the existing `Supervisor` (I-ST10), so sessions still outlive the UI and
a disconnect never kills or orphans a node process (`REQ-STUDIO-018`). Generated
TS types carry `RunSessionView`/`RuntimeStats`/`ProcessStatus` (I-ST2).

The renderer adds the Graph activity (a read-only route/link projection of the
deployment model; viewport/zoom/collapse are UI state, never `drm.toml`,
`REQ-STUDIO-017`) and the Run activity (start/stop/restart plus session and
`runtimeStats` rendering, `REQ-STUDIO-018`). Electron packaging (electron-builder
config, window lifecycle, bundled `drm` executable, sandboxed context-isolated
renderer with a narrow preload) is recorded in `apps/studio/electron`
(`REQ-STUDIO-019`). Test anchors: `studio_run_sessions_and_runtime_stats`
(`cargo test -p drm-cli studio`). Carried to M18W3: extension host.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Malformed JSON-RPC line | error response, connection continues |
| First RPC not `Hello` | connection rejected (handshake mandatory) |
| Edit failing verify | full failure report (check-IDs + REQ citations); no partial apply; revision unchanged |
| Stale `base_revision` | `CONFLICT{revision}` — UI offers Review/Reload/Retry (I-ST7) |
| Crash between journal and rename | startup finishes or rolls back (I-ST8) |
| Undo with externally changed inputs | Compare/Cancel/Force — never silent overwrite (I-ST9) |
| Extension fuel exhaustion | deterministic stop; engine unaffected (I-ST5) |
| Studio disconnect mid-build/run | build completes headless; run session persists (I-ST10) |
| Two Studio instances | revisioned concurrency arbitrates; single in-flight build lock per workspace |
| Untrusted workspace + Build request | refused by trust gate (I-ST11) |
| Untrusted workspace with pending journal | view-only warning; no recovery write until trusted |
| Oversize/deep/count-heavy JSON request | bounded error before dispatch (I-ST13) |

## Test Matrix

| Case | Expected |
|---|---|
| `studio_parity` (CLI vs Studio edits) | I-ST1 identical renders |
| `gen_check` on studio-protocol | I-ST2 |
| plane-audit fixture | I-ST3 |
| renderer import lint | I-ST4 |
| `ext_budget` ×{fuel, memory} | I-ST5 |
| `edit_preservation` (toml_edit round-trip) | I-ST6 |
| `studio_revision_conflict` (concurrent mutations) | I-ST7 |
| journal kill-point fixture ×20 | I-ST8 |
| `studio_undo_guard` (external change) | I-ST9 |
| `supervisor_outlives_ui` (close/disconnect scenarios) | I-ST10 |
| `trust_gate` (untrusted restrictions) | I-ST11 |
| `journal_durability` (flush/rename kill matrix) | I-ST12 |
| `studio_frame_bounds` | I-ST13 |
| handshake compat (older client) | capability-driven feature hiding |
| e2e: open→edit→verify→build→run→stop | full flow green |

## Staging

The v1 surfaces, creation flows, editors, Inspector, graph, Problems/Build/Run panels, and bounded frontend state follow [ui-architecture.md](../../ui-architecture.md) (§Application Layout, §Creation and Editors, §Inspector, §System Graph, §Problems Panel, §Semantic vs UI State, §Frontend State). The extension platform is the designed second phase ([ui-architecture.md](../../ui-architecture.md) §Extension Platform; post-1.0). Remote serving follows desktop after protocol stability; direct listening remains disabled until TLS, origin validation, and authentication are configured.
