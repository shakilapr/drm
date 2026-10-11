# Build Orchestration & Watch Mode Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: `drm build` as high-level orchestrator (parse → handles → deployment artifacts → generate → verify → invoke native builder), native build adapters (CMake/Ninja, Cargo, custom command) with per-adapter environment hooks, `drm info` (read-only model/verify projection), and `drm dev` filesystem watch — two data event categories plus error/rescan control signals, event-loss recovery, pending-dirty retention across invalid declaration batches. Watch events are invalidation hints, not truth (architecture.md §DRM as Orchestrator, §Clear Division of Responsibilities, §Development Watch Mode, §Tiered Developer Commands). Owner: E2 (crate `drm-workspace`, `drm-cli`). Phase 25. Depends on [workspace-model.md](workspace-model.md), [codegen-invalidation.md](codegen-invalidation.md), [handle-system.md](handle-system.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-build-orchestrate` | parse → handles → deployment artifacts → generate → verify → native adapter invoke | `REQ-TOOL-012` |
| `REQ-TOOL-build-adapter-delegate` | header deps/object freshness owned by native toolchains | `REQ-TOOL-011` |
| `REQ-TOOL-dev-two-events` | two data categories (declaration / application source) + `DeclarationError`/`RescanRequired` control signals | `REQ-TOOL-016` |
| `REQ-TOOL-dev-no-source-graph` | DRM never tracks header/source dependency graphs | `REQ-TOOL-014` |
| `REQ-TOOL-dev-no-stale-code` | repeated edits never produce stale generated code | `REQ-TOOL-015` |
| `REQ-TOOL-dev-pending-dirty` | source events in invalid-declaration batches retained, dispatched after next valid reload | `REQ-TOOL-066` |
| `REQ-TOOL-dev-rescan-recovery` | watcher overflow/error ⇒ full rescan reconstructs state | `REQ-TOOL-067` |

## Data Structures

```rust
pub struct BuildRequest { workspace: PathBuf, targets: Option<SortedSet<NodeName>> }
pub enum WatchEvent {
 DrmDeclarationChanged(Vec<PathBuf>), // all declaration paths in the batch
 AppSourceChanged { node: NodeName, path: PathBuf },
 DeclarationError(Vec<ModelError>), // control signal: reload failed
 RescanRequired, // control signal: event loss/overflow — reconstruct from fs
}

pub enum BuildAdapter {
 Cmake { dir: PathBuf, target: String },
 Cargo { package: String },
 Command(Vec<String>), // custom — explicit in manifest [build].command
}

struct WatchState {
 model: WorkspaceModel, // last valid model
 pending_source: SortedSet<PathBuf>, // I-BO6: source paths awaiting a valid model
 pending_rescan: bool, // I-BO7: loss detected ⇒ rescan before trusting events
}

// Adapter hooks: the environment DRM guarantees to every native adapter so node
// builds find generated bindings and deployment metadata without guessing paths.
pub struct AdapterInvocation {
 program: String, args: Vec<String>,
 env: SortedMap<String, String>, // DRM_GEN_INCLUDE, DRM_DEPLOY_METADATA (below)
}
pub const GEN_DIR_REL: &str = ".drm/gen"; // include root (c|cpp subtrees)
pub const DEPLOY_DIR_REL: &str = ".drm/deploy"; // deployment artifact root
pub const GEN_INCLUDE_ENV: &str = "DRM_GEN_INCLUDE"; // `:`-joined include paths
pub const DEPLOY_METADATA_ENV: &str = "DRM_DEPLOY_METADATA"; // path to deployment.json

// I-BO1 sequence probe: emitted once per orchestration step, in order.
pub enum BuildStep { Parse, Model, Handles, Deploy, Codegen, Verify, Adapter(NodeName) }
```

`drm info` is a **pure projection**: it renders the last in-memory `WorkspaceModel`
plus the `drm verify` summary. It performs no mutation, no codegen, no adapter
invocation, and introduces no semantics of its own (architecture.md §Tiered
Developer Commands).

## Invariants

- **I-BO1 OrchestrationOrder**: build always: parse → WorkspaceModel → handle assignment → deployment artifact generation (handle tables, routing tables — handle-system.md) → codegen (codegen-invalidation.md plan/execute) → `drm verify` → adapters. No step skipped; verify failure aborts before adapters. Codegen consumes contracts, not handles. Test: `build_order` (hooked sequence assert).
- **I-BO2 NoSourceTracking**: DRM never parses app sources for dependencies; adapter decides incremental work. Test: source-edit triggers exactly one adapter invocation (not DRM-driven dependency analysis).
- **I-BO3 AffectedOnlyDeclarations**: declaration change regenerates only affected artifacts (codegen I-G3) and invokes only nodes owning/using affected topics. Test: `dev_affected_nodes`.
- **I-BO4 NoStaleCode**: rapid successive edits — every regenerated artifact reflects the final state at adapter invocation time (WorkspaceModel reload per event; no stale cache). Test: `dev_rapid_edits` (debounce + final-state check).
- **I-BO5 ReloadBeforeDispatch**: within one debounced batch, declaration changes are applied (model reloaded) **before** any application-source event is mapped to an owning node; source events never use a stale model. Test: `watch_reload_before_dispatch`.
- **I-BO6 PendingDirtyPreserved**: source events in a batch whose declarations fail to reload are recorded as pending dirty paths and mapped via the **next** successfully loaded model, then dispatched once and cleared. Source events are never silently discarded. Test: `watch_pending_dirty_dispatch`.
- **I-BO7 EventsAreHints**: OS watch queues can overflow and drop events (inotify `IN_Q_OVERFLOW`, `ReadDirectoryChangesW` buffer loss, network filesystems with no usable events). On overflow, watcher error, or unpaired rename, DRM emits `RescanRequired`, performs a full declaration rescan plus a conservative node-source rescan, and reconstructs its known snapshot before resuming normal dispatch. Debounce is coalescing, never a correctness mechanism. Test: `watch_overflow_rescan` (drop events, assert recovery).
- **I-BO8 OwnershipByPath**: source ownership is decided by path only (model placement/manifest paths); file content is never decoded to classify an event. Test: binary/source file of any encoding triggers owning node.

## Functions

```rust
fn build(req: BuildRequest) -> Result<BuildReport, Status> {
 let model = workspace_model::build(tree(req.workspace))?; // workspace-model.md (I-D1)
 let ids = handle_system::assign(&model, prior_ids())?; // handle-system.md
 deploy::generate(&model, &ids)?; // deployment artifacts: handle tables, route/runtime tables (may embed handles)
 codegen::execute(&codegen::plan(&model, prev_fingerprints()))?; // codegen-invalidation.md — contracts only, no handles
 verify::run(&model, &deployment_config(req.workspace)?)?; // verify-engine.md (Phase 26; minimal set here)
 let targets = resolve_targets(req.targets.as_ref(), &model)?; // sorted, owned names
 for node in targets {
 adapter_for(node)?.invoke(node)?; // delegate (I-BO2)
 }
 Ok(BuildReport { generated, verified, built })
}

fn watch(ws: PathBuf, mut on_event: impl FnMut(WatchEvent)) -> Result<(), Status> {
 let mut st = WatchState { model: workspace_model::build(tree(&ws))?, pending_source: default(), pending_rescan: false };
 loop {
 let evs = match fs_events.next_batch_debounced(200ms) { // coalescing only (I-BO4)
 Ok(batch) => batch,
 Err(WatchOverflow | WatchError) => { st.pending_rescan = true; on_event(RescanRequired); continue; } // I-BO7
 };
 if st.pending_rescan { recover_by_rescan(&mut st, &ws)?; } // full declaration + conservative source rescan (I-BO7)

 // I-BO5: apply declaration changes (reload) BEFORE mapping source events
 let mut reload_failed = false;
 let declarations: Vec<_> = evs.iter().filter(|e| is_declaration(e)).cloned().collect();
 if !declarations.is_empty() {
 match workspace_model::build(tree(&ws)) {
 Ok(next) => { st.model = next; on_event(DrmDeclarationChanged(declarations)); }
 Err(e) => { on_event(DeclarationError(e)); reload_failed = true; } // keep last valid model
 }
 }
 // I-BO6: map source events (this batch + pending) via the current valid model
 let mut source_evs: SortedSet<PathBuf> = evs.iter().filter(|e| !is_declaration(e)).cloned().collect();
 source_evs.extend(st.pending_source.drain()); // retained dirty paths (I-BO6)
 for ev in source_evs.iter() {
 if let Some(node) = owner_of(ev, &st.model) { // ownership from the current model (I-BO8: path only)
 on_event(AppSourceChanged { node, path: ev.clone() });
 } // unmappable ⇒ retain as pending if a reload failed this batch, else drop (only two categories tracked)
 if reload_failed { st.pending_source.insert(ev.clone()); } // retry after next valid reload
 }
 }
}

fn recover_by_rescan(st: &mut WatchState, ws: &Path) -> Result<(), Status> {
 st.model = workspace_model::build(tree(&ws))?; // full declaration rescan (all declarations)
 st.pending_source = all_node_source_paths(&st.model); // conservative: dirty every owned source tree
 st.pending_rescan = false;
 Ok(())
}

// Adapter hooks (E1-T1): canonical include paths + deployment metadata path,
// exported as environment so adapters never hard-code `.drm` layout.
fn include_paths(ws: &Path) -> Vec<PathBuf> { vec![ws.join(GEN_DIR_REL).join("c"), ws.join(GEN_DIR_REL).join("cpp")] }
fn deployment_metadata_path(ws: &Path) -> PathBuf { ws.join(DEPLOY_DIR_REL).join("deployment.json") }
fn invocation_for(node: &NodeEntry, ws: &Path) -> AdapterInvocation // per BuildAdapter: cmake/cargo/command + canonical flags
fn info(model: &WorkspaceModel, verify: &[VerifyFinding]) -> String // render_model + verify summary; deterministic, read-only
```

## Protocols

*Declaration-change handling (`drm dev`):*

1. Batch events (debounce) to final state.
2. Reload WorkspaceModel fully (workspace-model.md total-reload). On failure: emit `DeclarationError`, retain batch source events as pending (I-BO6), keep last valid model.
3. Re-run handle assignment (stability ⇒ usually no-op) + codegen plan/execute (affected-only) + verify.
4. Invoke native adapter for each affected node (per manifest `[build]`).

*Application-source change handling:*

1. Identify owning node by path from the current model (I-BO8).
2. Invoke that node's adapter only — CMake/Ninja/Cargo decide object freshness (I-BO2).
3. No DRM parsing of sources, no header scanning, ever.

*Rescan recovery (I-BO7):*

1. Trigger: watcher overflow, watcher error, or unpaired rename on a platform with lossy rename events.
2. Full declaration rescan rebuilds the model; conservative node-source rescan marks every owned source tree dirty.
3. Adapters run once per affected node; known snapshot reconstructed; normal dispatch resumes.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Verify fails during build | adapters not invoked; report lists verify errors with REQ refs |
| Adapter failure | per-node failure surfaced; other nodes continue (`--keep-going` default in dev) |
| Renamed node dir | manifest discovery resolves; old adapter target dropped |
| Non-UTF8 / binary source file touched | owning node dirtied — ownership is path-based (I-BO8); content never decoded |
| Watch event during build | queued; coalesced and applied after the single in-flight build |
| Invalid declaration batch | emit `DeclarationError`; batch source events + pending retained (I-BO6); dispatched after next valid reload via that model |
| Watcher overflow / error / unpaired rename | `RescanRequired`; full declaration rescan + conservative source rescan (I-BO7) |
| Handle reassignment / compaction | deployment artifacts regenerate; bindings untouched (codegen I-G9); only binaries embedding deployment tables rebuild |
| Custom command missing | clear error naming manifest field |

## Test Matrix

| Case | Expected |
|---|---|
| `build_order` | sequence exact |
| `dev_affected_nodes` | only owning/using nodes rebuilt |
| `dev_rapid_edits` (50 rapid edits) | final-state artifacts |
| `watch_reload_before_dispatch` (rename + source same batch) | I-BO5 |
| `watch_pending_dirty_dispatch` (source + invalid decl same batch; fix decl; source dispatched via new model) | I-BO6 |
| `watch_overflow_rescan` (injected event drop) | I-BO7 recovery, no lost build |
| `watch_binary_source` (non-UTF8 file) | I-BO8 owning node invoked |
| `adapter_matrix` (cmake/cargo/command) | each invoked with configured args |
| `adapter_hooks` | invocation carries `DRM_GEN_INCLUDE` + `DRM_DEPLOY_METADATA` |
| `info_projection` | deterministic model render + verify summary; no mutation |
| no-source-tracking probe | single adapter call per source change |
| handle reassignment → only deployment-table consumers rebuild | I-BO1/I-G9 boundary |
