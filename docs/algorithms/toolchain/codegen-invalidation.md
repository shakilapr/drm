# Codegen & Invalidation Algorithms

Scope: language binding generators (C/C++/Rust/Python) — stable shared output paths, generation fingerprints, node-scoped API units, atomic replacement, timestamp preservation, direct reconciliation against a complete expected-output set. Implements architecture.md §Stable Shared Bindings with Internal Invalidation Fingerprints, §Node Communication Contracts. Owner: E2 (crate `drm-workspace::codegen`). Phase 24. Depends on [workspace-model.md](workspace-model.md), [canon-codec.md](../canon-codec.md), [handle-system.md](handle-system.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-gen-stable-paths` | `.drm/gen/<lang>/<topic>/<file>` shared per topic+language | `REQ-TOOL-029` |
| `REQ-TOOL-gen-fingerprint` | `hash(canonical_schema, topic_identity, generator_version, options)` | `REQ-TOOL-027` |
| `REQ-TOOL-gen-expected-outputs` | plan carries full expected tree, distinct from dirty units | `REQ-TOOL-026` |
| `REQ-TOOL-gen-node-api-unit` | node-scoped API invalidated by declaration changes alone | `REQ-TOOL-028` |
| `REQ-TOOL-gen-atomic-replace` | platform atomic replace; canceled run leaves no partial file | `REQ-TOOL-024` |
| `REQ-TOOL-gen-timestamp-preserved` | identical output ⇒ no rewrite ⇒ no rebuild cascade | `REQ-TOOL-030` |
| `REQ-TOOL-gen-direct-reconciliation` | deleted declaration ⇒ output removed; no orphan CAS | `REQ-TOOL-025` |

## Data Structures

```rust
pub struct GenPlan {
 expected_outputs: BTreeSet<PathBuf>, // COMPLETE desired generated tree (topics + node APIs)
 dirty_units: Vec<GenUnit>, // only units whose fingerprint changed or file missing
}

pub enum GenKind { Topic, NodeApi }
pub struct GenUnit {
 kind: GenKind,
 lang: Lang, // C | Cpp | Rust | Python
 key: String, // topic path or node name
 out_path: PathBuf, // .drm/gen/<lang>/<dir>/<file> (stable, predictable)
 fingerprint: [u8; 16], // see plan()
 body: String, // rendered artifact (deterministic)
}
```

## Invariants

- **I-G1 ByteIdentical**: identical inputs ⇒ byte-identical outputs (no timestamps, no absolute paths, stable iteration). Test: `gen_determinism` (double-run diff).
- **I-G2 TimestampPreserved**: unchanged fingerprint + existing file ⇒ file untouched (mtime stable). Test: `gen_mtime_preserved`.
- **I-G3 AffectedOnly**: changed topic regenerates exactly its units; an unrelated output is neither rewritten nor deleted. Reconciliation is computed against `expected_outputs`, not `dirty_units`. Test: `gen_affected_only`, `gen_expected_vs_dirty`.
- **I-G4 Atomicity**: interrupted generation (kill mid-write) leaves prior artifact intact — no partial files. Test: `gen_atomicity` (kill harness).
- **I-G5 Reconciliation**: deleted topic or removed node declaration ⇒ its output path removed same run; no orphan accumulations. `existing_generated \ expected_outputs` is exactly the deletion set. Test: `gen_reconciliation`.
- **I-G6 NodeApiInvalidated**: a node changing `publish`/`consume` without any schema change still regenerates that node's API units (fingerprint includes declarations). Test: `node_api_invalidation`.
- **I-G7 ReplacePortable**: atomic replacement uses a platform `atomic_replace()` backend (POSIX `rename`, Windows `MoveFileEx`/`ReplaceFile`); no assumption of Unix rename semantics over an existing file. Test: `atomic_replace_windows`.

## Functions

```rust
fn plan(model: &WorkspaceModel, prev: &FingerprintDb) -> GenPlan {
 let mut expected = BTreeSet::new();
 let mut dirty = Vec::new();

 // Topic-scoped artifacts
 for topic in model.topics.keys().sorted() {
 for lang in LANGS {
 let out = out_path(lang, GenKind::Topic, topic);
 expected.insert(out.clone());
 let fp = hash(canon_schema_bytes(topic), topic, GENERATOR_VERSION, options);
 if prev.get((lang, topic)) == Some(fp) && out_exists(lang, topic) { continue; }
 dirty.push(render(lang, GenKind::Topic, topic, model, fp));
 }
 }

 // Node-scoped API artifacts — invalidated by node declarations, not only schema changes
 for node in model.nodes.values().sorted() {
 for lang in LANGS {
 let out = out_path(lang, GenKind::NodeApi, &node.name);
 expected.insert(out.clone());
 let fp = hash(node.publish ∪ node.consume, // declared interfaces
 topic_interface_hashes(model, node),// relevant topic schemas/layouts
 GENERATOR_VERSION, options);
 if prev.get((lang, &node.name)) == Some(fp) && out_exists(lang, &node.name) { continue; }
 dirty.push(render(lang, GenKind::NodeApi, &node.name, model, fp));
 }
 }

 GenPlan { expected_outputs: expected, dirty_units: dirty }
}

fn execute(plan: &GenPlan, prev: &mut FingerprintDb) -> Result<(), Status> {
 let _guard = workspace_codegen_lock()?; // cross-process exclusive lock
 let mut next = prev.clone();
 for u in &plan.dirty_units {
 let tmp = unique_temp_sibling(&u.out_path)?;
 write(tmp, &u.body)?; // full content first
 fsync_file(&tmp)?;
 atomic_replace(&tmp, &u.out_path)?; // I-G7: platform atomic replace
 next.record(u.key(), u.fingerprint);
 }
 // Reconcile against the COMPLETE expected set, never the dirty set (I-G3/I-G5)
 for orphan in existing_generated().difference(&plan.expected_outputs) {
 remove(orphan)?;
 }
 save_fingerprint_db_atomic(&next)?; // commit metadata only after outputs and reconciliation
 *prev = next;
 Ok(())
}

// Node-scoped surface (per bindings-cpp-rust-python.md):
fn node_header(node: &NodeEntry, model: &WorkspaceModel) -> String
// includes ONLY topics in node.publish ∪ node.consume; undeclared topic names do not exist in the header.
```

## Protocols

*Generation run (inside `drm build` / `drm dev`):*

1. Load `FingerprintDb` (`.drm/gen/.fingerprints`).
2. `plan` → complete `expected_outputs` + `dirty_units` (topics and node APIs).
3. `execute` atomically replaces dirty units via `atomic_replace()`; unchanged units retain mtimes (I-G2).
4. Reconcile `existing_generated \ expected_outputs` (I-G5).
5. Atomically save the new fingerprint DB last. Failure leaves the prior DB, so the next run rechecks outputs.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Generator version bump | all topic + node API fingerprints differ ⇒ full regeneration (intentional) |
| Node declaration edit, schema unchanged | node API unit regenerated (I-G6) |
| Unchanged output absent from `dirty_units` | retained — reconciliation uses `expected_outputs` (I-G3) |
| Hand-edited generated file | `xtask gen-check` (R6) fails CI — regen diff detected |
| Disk full mid-write | tmp file remains; artifact intact; next run cleans tmp |
| Concurrent `drm build` instances | out-path lock (single-writer per workspace) |
| Topic renamed | old dir orphaned ⇒ reconciled; handle tombstone per handle-system.md |
| Destination exists on Windows | `atomic_replace` uses `ReplaceFile`/`MoveFileEx`; never a bare rename (I-G7) |

## Test Matrix

| Case | Expected |
|---|---|
| `gen_determinism` | I-G1 |
| `gen_mtime_preserved` | I-G2 |
| `gen_affected_only` | I-G3 |
| `gen_expected_vs_dirty` | I-G3: unchanged outputs never deleted |
| `gen_atomicity` (kill ×20) | I-G4 |
| `gen_reconciliation` | I-G5 |
| `node_api_invalidation` | I-G6 |
| `atomic_replace_windows` | I-G7 |
| `gen_check` (R6) | hand-edit detected |
| 4-lang × golden topics | outputs compile in each language |
