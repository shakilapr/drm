# Topic Handle System Algorithms

Scope: canonical topic identity → stable generated runtime handles; numeric-ID assignment across builds; explicit `drm ids compact`. Implements architecture.md §Canonical Identity, Runtime Handles, and Transport Identifiers — Stability Across Normal Builds, Explicit Handle Compaction. Owner: E2 (crate `drm-workspace::model::handles`, `drm-cli::ids`). Phase 6. Depends on [workspace-model.md](workspace-model.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-handle-canonical-identity` | topic path = single source of truth | — |
| `REQ-TOOL-handle-runtime-integer` | compact integer for dispatch/indexing, never transmitted as identity | — |
| `REQ-TOOL-handle-build-stability` | normal builds preserve assigned handles | — |
| `REQ-TOOL-handle-add-no-renumber` | adding unrelated topics never renumbers existing | — |
| `REQ-TOOL-handle-compact-explicit` | only `drm ids compact` recalculates; coordinated rebuild required | — |

## Data Structures

```rust
pub struct HandleTable {
 // topic path → u32 handle; assignment history file records prior assignments
 assignments: BTreeMap<TopicPath, u32>,
 next_free: u32,
 history: AssignmentHistory, // persisted at .drm/ids.json (workspace-local, git-ignored? committed per policy: committed — stability artifact)
}

pub enum AssignDelta { New(TopicPath), Removed(TopicPath), Unchanged(TopicPath) }
```

## Invariants

- **I-H1 RebuildStable**: same workspace input tree ⇒ identical `.drm/ids.json` (deterministic assignment). Test: `handle_determinism` ×100.
- **I-H2 AddNoRenumber**: adding topics ⇒ existing handles unchanged (append-only allocation). Test: `handle_add_stability`.
- **I-H3 DeleteReaddPolicy**: removing topic X and re-adding it without compaction: X keeps a *reserved* tombstone entry (handle not reused) for the session; after explicit `drm ids compact`, handles re-pack. Test: `handle_delete_readd`.
- **I-H4 CompactExplicit**: any non-compact command leaves existing assignments untouched; compaction requires acknowledgement (CLI flag) + full-node rebuild guidance output. Test: `handle_compact_gate`.

## Functions

```rust
fn assign_handles(model: &WorkspaceModel, prior: &AssignmentHistory) -> (HandleTable, Vec<AssignDelta>) {
 let mut t = HandleTable::from(prior); // I-H2: prior assignments win
 for topic in model.topics.keys().sorted() { // deterministic order (I-H1)
 match t.assignments.get(topic) {
 Some(_) => { /* unchanged */ }
 None => { t.assignments.insert(topic, t.next_free); t.next_free += 1; }
 }
 }
 for removed in prior.keys().filter(|k| !model.topics.contains_key(k)) {
 t.tombstone(removed); // I-H3: reserved, not reused (pre-compact)
 }
 (t, deltas)
}

fn compact(t: &HandleTable, ack: bool) -> Result<HandleTable, Status> {
 if !ack { return Err(InvalidArg); } // I-H4: explicit only
 let mut out = HandleTable::default();
 for (i, topic) in t.assignments.keys().filter(|k| t.is_live(k)).sorted().enumerate() {
 out.insert(topic, i as u32); // re-pack dense
 }
 out.record_rebuild_guidance(); // output: coordinated rebuild instructions
 Ok(out)
}
```

## Protocols

*Normal build (`drm build` / `drm dev`):* `assign_handles(model, prior)` → write `.drm/ids.json` only if changed; codegen and route tables consume handles — no rebuild cascade when unchanged.

*Compaction (maintenance):* operator runs `drm ids compact --yes` → re-packed table → tool prints every node requiring rebuild → deployment regenerated (deployment-identity.md: new deployment namespace, not in-place).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two topics colliding after rename | assignment diff reported; old handle tombstoned; no silent reuse |
| Workspace with zero topics | empty table; tools still run |
| Prior file corrupt | rebuild fails with clear error (never silently reassigns) |
| Handle exhaustion (u32) | verifier reports at build; compaction suggested |

## Test Matrix

| Case | Expected |
|---|---|
| `handle_determinism` | I-H1 |
| `handle_add_stability` | I-H2 |
| `handle_delete_readd` | I-H3 |
| `handle_compact_gate` (no `--yes`) | refuses |
| `handle_compact_yes` | dense re-pack + guidance |
