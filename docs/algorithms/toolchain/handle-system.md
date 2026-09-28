# Topic Handle System Algorithms

Scope: canonical topic identity → stable runtime handles; numeric-ID assignment across builds; committed assignment lockfile (`.drm/ids.json`) with semantic merge reconciliation (`drm ids reconcile`); explicit `drm ids compact`. Handles are runtime/deployment artifacts: generated code bindings never embed them (codegen-invalidation.md I-G9, architecture.md §Canonical Identity, Runtime Handles, and Transport Identifiers). Owner: E2 (crate `drm-workspace::model::handles`, `drm-cli::ids`). Phase 6. Depends on [workspace-model.md](workspace-model.md), [deployment-identity.md](deployment-identity.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-handle-canonical-identity` | topic path = single source of truth; handle is local optimization, never identity | `REQ-TOOL-037` |
| `REQ-TOOL-handle-runtime-integer` | compact integer for dispatch/indexing, never transmitted as identity | `REQ-TOOL-039` |
| `REQ-TOOL-handle-build-stability` | normal builds preserve assigned handles | `REQ-TOOL-036` |
| `REQ-TOOL-handle-add-no-renumber` | adding unrelated topics never renumbers existing | `REQ-TOOL-035` |
| `REQ-TOOL-handle-compact-explicit` | only `drm ids compact` recalculates; regeneration scoped to deployment artifacts | `REQ-TOOL-038` |
| `REQ-TOOL-handle-lockfile` | committed assignment lockfile; no nondeterministic metadata | `REQ-TOOL-080` |
| `REQ-TOOL-handle-reconcile` | semantic three-way merge; collision ⇒ retire + reassign both, never reuse | `REQ-TOOL-081` |
| `REQ-TOOL-handle-exhaustion` | assignment-time checked increment; exhaustion cannot overflow | `REQ-TOOL-082` |

## Data Structures

```rust
pub struct HandleTable {
 assignments: BTreeMap<TopicPath, u32>,
 reserved: BTreeMap<u32, ReserveReason>, // tombstones + merge collisions — never reused (pre-compact)
 next_free: u32,
}
pub enum ReserveReason { Deleted, MergeCollision }

/// `.drm/ids.json` — COMMITTED assignment-history lockfile (REQ-TOOL-080).
/// Generated artifact: canonical sorted output; `format_version` first.
/// Never contains timestamps, branch names, usernames, or other nondeterministic metadata.
/// Structure: { "format_version": 1, "next_free": u32,
///              "assignments": { "<topic path>": u32, ... },
///              "reserved": { "<u32>": { "reason": "deleted" | "merge_collision" } } }

pub enum AssignDelta { New(TopicPath), Removed(TopicPath), Unchanged(TopicPath) }
```

Canonical topic path is permanent identity; the handle is a deployment/runtime representation. Reassignment (reconcile/compact) changes the representation only — never the topic's identity (architecture.md §Canonical Identity, Runtime Handles, and Transport Identifiers).

## Invariants

- **I-H1 HistoryDeterministic**: same current workspace **and same assignment history** ⇒ identical `.drm/ids.json`. Prior assignments are authoritative, so history is formally part of the input. Test: `handle_determinism` ×100.
- **I-H2 AddNoRenumber**: adding topics ⇒ existing handles unchanged (append-only allocation). Test: `handle_add_stability`.
- **I-H3 DeleteReaddPolicy**: removing topic X reserves a tombstone (handle not reused) until explicit `drm ids compact`; compaction re-packs live assignments only. Test: `handle_delete_readd`.
- **I-H4 CompactExplicit**: any non-compact command leaves existing assignments untouched; compaction requires acknowledgement (CLI flag). Compaction ⇒ new deployment identity ⇒ regenerate all handle-dependent deployment artifacts ⇒ rebuild only binaries/firmware that compiled them (MCU topic tables); code bindings are never regenerated. Test: `handle_compact_gate`, `handle_compact_scope`.
- **I-H5 NoSilentReuse**: no historical number is ever silently recycled: deleted tombstones and merge-collision reservations stay reserved until explicit compaction; compaction itself never reissues an ID that appears in any surviving artifact's deployment history without a coordinated redeployment (new deployment identity). Test: `handle_no_reuse`.
- **I-H6 ExhaustionSafe**: assignment increments are checked; exhaustion is an assignment-time error, never a `u32` overflow. Test: `handle_exhaustion`.
- **I-H7 BindingsHandleFree**: generated code depends on contracts only; `ids.json` changes never trigger codegen. Test: `handle_change_no_codegen`.

## Functions

```rust
fn assign(model: &WorkspaceModel, prior: &AssignmentHistory) -> Result<(HandleTable, Vec<AssignDelta>), Status> {
 let mut t = HandleTable::from(prior); // I-H1/I-H2: prior assignments + reservations win
 for topic in model.topics.keys().sorted() { // deterministic order
 if t.assignments.contains_key(topic) { continue; }
 let id = t.next_free;
 t.next_free = id.checked_add(1).ok_or(HandleSpaceExhausted)?; // I-H6: safe at assignment time
 t.assignments.insert(topic.clone(), id);
 }
 let mut deltas = Vec::new();
 for removed in prior.assignments.keys().filter(|k| !model.topics.contains_key(k)) {
 t.reserve(*removed, ReserveReason::Deleted); // I-H3/I-H5: tombstone, not reuse
 deltas.push(AssignDelta::Removed(removed.clone()));
 }
 verify.handle_space_report(&t); // verifier may add advisory diagnostics; safety never depends on it (I-H6)
 Ok((t, deltas))
}

/// `drm ids reconcile BASE OURS THEIRS` — semantic three-way merge of committed lockfiles (REQ-TOOL-081).
/// Run automatically on conflict, or explicitly. Deterministic; fail-safe (loud, never silently renumbered).
fn reconcile(base: &HandleTable, ours: &HandleTable, theirs: &HandleTable, ack: bool) -> Result<HandleTable, Status> {
 let mut out = HandleTable::default();
 // 1. Inherited-from-base assignments are immutable in both branches; any divergence ⇒ error.
 for (topic, id) in &base.assignments {
 let o = ours.assignments.get(topic); let h = theirs.assignments.get(topic);
 if o != Some(id) || h != Some(id) { return Err(ImmutableHistoryViolation { topic }); }
 out.insert(topic, *id);
 }
 // 2. Union all reservations — reserved IDs are never reusable.
 out.reserved = union(ours.reserved, theirs.reserved);
 // 3./4. New assignments with the same ID:
 for (id, o_topic) in new_in(ours, base) {
 match new_in(theirs, base).get(&id) {
 Some(h_topic) if h_topic == o_topic => { out.insert(o_topic, id); } // same topic, both branches: keep
 Some(h_topic) => { // different topics claimed the same ID:
  out.reserved.insert(id, ReserveReason::MergeCollision); // retire the number entirely
  out.insert(o_topic, out.alloc_fresh()?); // BOTH topics get fresh IDs (I-H5):
  out.insert(h_topic, out.alloc_fresh()?); // a stale artifact from the losing branch then
 } // hits an unknown/reserved handle — loud failure, not silent corruption
 None => { out.insert(o_topic, id); } // claimed in one branch only: keep
 }
 }
 // 5. Same newly-created topic, different IDs in the two branches:
 for topic in topics_new_in_both(ours, theirs, base) {
 let chosen = min(ours.assignments[topic], theirs.assignments[topic]); // deterministic
 out.reserved.insert(other_id, ReserveReason::MergeCollision);
 out.insert(topic, chosen);
 }
 // 6. Fresh allocation starts strictly above every live/reserved ID.
 out.next_free = out.max_live_or_reserved() + 1;
 if out.assignments != base.assignments {
 deploy::new_identity_required(ack)?; // 8. Reconciliation changing assignments ⇒ new deployment identity
 }
 Ok(out) // 7. canonical sorted ids.json; 9. no silent recycling (I-H5)
}

fn compact(t: &HandleTable, ack: bool) -> Result<HandleTable, Status> {
 if !ack { return Err(InvalidArg); } // I-H4: explicit only
 let mut out = HandleTable::default();
 for (i, topic) in t.assignments.keys().sorted().enumerate() {
 out.insert(topic, i as u32); // re-pack dense; reservations dropped — coordinated redeployment makes reuse safe
 }
 out.next_free = out.assignments.len() as u32;
 deploy::new_identity_required(ack)?; // I-H4: new deployment identity
 deploy::regenerate_deployment_artifacts(&out)?; // handle tables, routing/runtime tables; MCU topic tables if compiled in
 build::rebuild_nodes_compiling_deployment_tables()?; // code bindings untouched (I-H7)
 out.record_rebuild_guidance(); // output: every binary requiring rebuild + coordinated restart instructions
 Ok(out)
}
```

## Protocols

*Normal build (`drm build` / `drm dev`):* `assign(model, prior)` → write `.drm/ids.json` only if changed; deployment artifact generation and route tables consume handles; codegen does not (I-H7) — no rebuild cascade when unchanged.

*Reconciliation (merge conflict or explicit):* `drm ids reconcile` resolves semantic conflicts per the numbered rules; assignment-changing outcomes print a new-deployment-identity notice. A Git merge driver may invoke it on `.drm/ids.json` conflicts (merge declarations + reconcile derived lock state; line-level conflict resolution is never authoritative).

*Compaction (maintenance):* operator runs `drm ids compact --yes` → re-packed table → new deployment identity → deployment artifacts regenerate → only binaries compiling deployment tables rebuild → coordinated redeployment (deployment-identity.md).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two topics colliding after rename | assignment diff reported; old handle tombstoned; no silent reuse |
| Workspace with zero topics | empty table; tools still run |
| Prior file corrupt / `format_version` unknown | rebuild fails with clear error (never silently reassigns) |
| Handle exhaustion (u32) | `HandleSpaceExhausted` at assignment time (I-H6); verifier adds advisory suggesting compaction |
| Merge: same new ID, different topics (41 = camera vs 41 = lidar) | 41 reserved `merge_collision`; both topics get fresh deterministic IDs; next_free above all (I-H5) |
| Merge: same new ID, same topic | kept |
| Merge: base assignment changed in a branch | `ImmutableHistoryViolation` — manual resolution required |
| Compaction | dense re-pack; new deployment identity; bindings never regenerated (I-H4/I-H7) |

## Test Matrix

| Case | Expected |
|---|---|
| `handle_determinism` (same workspace + history) | I-H1 |
| `handle_fresh_deterministic` (empty history, same workspace) | identical initial table |
| `handle_add_stability` | I-H2 |
| `handle_delete_readd` | I-H3 |
| `handle_compact_gate` (no `--yes`) | refuses |
| `handle_compact_scope` (bindings untouched; MCU table consumers rebuilt) | I-H4/I-H7 |
| `handle_change_no_codegen` (schema/API change never rewrites bindings) | I-H7 |
| `handle_no_reuse` (tombstone + collision reservation) | I-H5 |
| `handle_exhaustion` (u32 boundary) | I-H6 typed error, no overflow |
| `handle_reconcile_collision` (41=different topics) | both reassigned; 41 reserved |
| `handle_reconcile_history_violation` | hard error |
| lockfile round-trip (canonical sorted, no nondeterministic fields) | I-H1 |
