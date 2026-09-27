# Build Orchestration & Watch Mode Algorithms

Scope: `drm build` as high-level orchestrator (parse → generate → verify → invoke native builder), native build adapters (CMake/Ninja, Cargo, custom command), and `drm dev` filesystem watch with exactly two event categories. Implements architecture.md §DRM as Orchestrator, §Clear Division of Responsibilities, §Development Watch Mode. Owner: E2 (crate `drm-workspace`, `drm-cli`). Phase 25. Depends on [workspace-model.md](workspace-model.md), [codegen-invalidation.md](codegen-invalidation.md), [handle-system.md](handle-system.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-build-orchestrate` | parse → generate → verify → native adapter invoke | `REQ-TOOL-012` |
| `REQ-TOOL-build-adapter-delegate` | header deps/object freshness owned by native toolchains | `REQ-TOOL-011` |
| `REQ-TOOL-dev-two-events` | DRM declaration changed / application source changed — nothing else | `REQ-TOOL-016` |
| `REQ-TOOL-dev-no-source-graph` | DRM never tracks header/source dependency graphs | `REQ-TOOL-014` |
| `REQ-TOOL-dev-no-stale-code` | repeated edits never produce stale generated code | `REQ-TOOL-015` |

## Data Structures

```rust
pub struct BuildRequest { workspace: PathBuf, targets: Option<SortedSet<NodeName>> }
pub enum WatchEvent { DrmDeclarationChanged(PathBuf), AppSourceChanged { node: NodeName, path: PathBuf } }

pub enum BuildAdapter {
 Cmake { dir: PathBuf, target: String },
 Cargo { package: String },
 Command(Vec<String>), // custom — explicit in manifest [build].command
}
```

## Invariants

- **I-BO1 OrchestrationOrder**: build always: parse → WorkspaceModel → codegen (codegen-invalidation.md plan/execute) → `drm verify` → adapters. No step skipped; verify failure aborts before adapters. Test: `build_order` (hooked sequence assert).
- **I-BO2 NoSourceTracking**: DRM never parses app sources for dependencies; adapter decides incremental work. Test: source-edit triggers exactly one adapter invocation (not DRM-driven dependency analysis).
- **I-BO3 AffectedOnlyDeclarations**: declaration change regenerates only affected artifacts (codegen I-G3) and invokes only nodes owning/using affected topics. Test: `dev_affected_nodes`.
- **I-BO4 NoStaleCode**: rapid successive edits — every regenerated artifact reflects the final state at adapter invocation time (WorkspaceModel reload per event; no stale cache). Test: `dev_rapid_edits` (debounce + final-state check).
- **I-BO5 ReloadBeforeDispatch**: within one debounced batch, declaration changes are applied (model reloaded) **before** any application-source event is mapped to an owning node; source events never use a stale model. Test: `watch_reload_before_dispatch`.

## Functions

```rust
fn build(req: BuildRequest) -> Result<BuildReport, Status> {
 let model = workspace_model::build(tree(req.workspace))?; // workspace-model.md (I-D1)
 let ids = handle_system::assign(&model, prior_ids())?; // handle-system.md
 codegen::execute(&codegen::plan(&model, prev_fingerprints()))?; // codegen-invalidation.md
 verify::run(&model, &deployment_config(req.workspace)?)?; // verify-engine.md (Phase 26; minimal set here)
 let targets = resolve_targets(req.targets.as_ref(), &model)?; // sorted, owned names
 for node in targets {
 adapter_for(node)?.invoke(node)?; // delegate (I-BO2)
 }
 Ok(BuildReport { generated, verified, built })
}

fn watch(ws: PathBuf, mut on_event: impl FnMut(WatchEvent)) -> Result<(), Status> {
 let mut model = workspace_model::build(tree(&ws))?;
 loop {
 let evs = fs_events.debounce(200ms).batch(); // I-BO4: batch to final state

 // I-BO5: apply declaration changes (reload) BEFORE mapping source events
 let declarations: Vec<_> = evs.iter().filter(|e| is_declaration(e)).cloned().collect();
 if !declarations.is_empty() {
 match workspace_model::build(tree(&ws)) {
 Ok(next) => { model = next; on_event(DrmDeclarationChanged(declarations)); }
 Err(e) => { on_event(DeclarationError(e)); continue; } // suspend source dispatch; keep last valid model
 }
 }
 for ev in evs.iter().filter(|e| !is_declaration(e)) {
 if let Some(node) = owner_of(ev, &model) { // ownership from the NEW model
 on_event(AppSourceChanged { node, path: ev.clone() });
 }
 // else: ignore — watch handles ONLY the two categories (I-BO3/requirement)
 }
 }
}
```

## Protocols

*Declaration-change handling (`drm dev`):*

1. Batch events (debounce) to final state.
2. Reload WorkspaceModel fully (workspace-model.md total-reload).
3. Re-run handle assignment (stability ⇒ usually no-op) + codegen plan/execute (affected-only) + verify.
4. Invoke native adapter for each affected node (per manifest `[build]`).

*Application-source change handling:*

1. Identify owning node (from model placement/manifest paths).
2. Invoke that node's adapter only — CMake/Ninja/Cargo decide object freshness (I-BO2).
3. No DRM parsing of sources, no header scanning, ever.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Verify fails during build | adapters not invoked; report lists verify errors with REQ refs |
| Adapter failure | per-node failure surfaced; other nodes continue (`--keep-going` default in dev) |
| Renamed node dir | manifest discovery resolves; old adapter target dropped |
| Non-UTF8 source file touched | ignored (not a declaration; owner mapping by path only) |
| Watch event during build | queued; coalesced and applied after the single in-flight build |
| Invalid declaration batch | emit `DeclarationError`; dispatch no source events until a later valid reload |
| Custom command missing | clear error naming manifest field |

## Test Matrix

| Case | Expected |
|---|---|
| `build_order` | sequence exact |
| `dev_affected_nodes` | only owning/using nodes rebuilt |
| `dev_rapid_edits` (50 rapid edits) | final-state artifacts |
| `watch_reload_before_dispatch` (rename + source same batch) | I-BO5 |
| `adapter_matrix` (cmake/cargo/command) | each invoked with configured args |
| no-source-tracking probe | single adapter call per source change |
