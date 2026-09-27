# Studio Protocol & Engine Bridge Algorithms

Scope: the engine↔Studio bridge — generated TypeScript types + JSON-RPC schema (`packages/studio-protocol`), `drm studio --stdio` host, capability handshake, revisioned optimistic concurrency, durable journaled ChangeSets with conditional undo, format-preserving editing, run-session supervisor, command/data-plane separation, workspace-trust gating, and renderer capability model. Implements [ui-architecture.md](../../ui-architecture.md) Core Principles, §Process Architecture, §Protocol, §Transactional Mutations, §Run Session Supervisor. Owner: E2 (protocol generation, stdio host) + E4 (supervisor). Phase 32. UI state/streams: [studio-ui-runtime.md](studio-ui-runtime.md). Extensions: [studio-extensions.md](studio-extensions.md). Depends on [workspace-model.md](../toolchain/workspace-model.md), [verify-engine.md](../toolchain/verify-engine.md), [handle-system.md](../toolchain/handle-system.md), [build-orchestration.md](../toolchain/build-orchestration.md).

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

The v1 surfaces, creation flows, editors, Inspector, graph, Problems/Build/Run panels, and bounded frontend state follow [studio-ui-runtime.md](studio-ui-runtime.md). The extension platform is the designed second phase in [studio-extensions.md](studio-extensions.md). Remote serving follows desktop after protocol stability; direct listening remains disabled until TLS, origin validation, and authentication are configured.
