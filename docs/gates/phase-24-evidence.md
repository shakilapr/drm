# Phase 24 Evidence Pack — Code Generation & Invalidation

Evidence for the Phase 24 gate (chairs at M09W1). Every case in
[codegen-invalidation.md](../algorithms/toolchain/codegen-invalidation.md)
§Test Matrix is mapped to a test that stays green under the permanent gates.
Verdict: **PASS (I-G1..I-G6, I-G8, I-G9, R6); I-G7 partial — POSIX path green,
Windows `ReplaceFile` backend deferred to the Windows port.**

Implementation: `crates/drm-workspace/src/codegen`
(`GenPlan`/`GenUnit`, directed-capability fingerprints, stable `.drm/gen` paths,
content-compare atomic writes, expected-tree reconciliation) and
`xtask gen-check` (R6).

## Test Matrix ⇄ evidence

| Case | Invariant | Test | Result |
|---|---|---|---|
| `gen_determinism` (double-run) | I-G1 | `codegen_c::gen_determinism`, `codegen_all::gen_all_determinism` | PASS |
| `gen_mtime_preserved` | I-G2 | `codegen_c::gen_mtime_preserved` | PASS |
| `gen_mtime_preserved_fp_bump` | I-G2 | `codegen_c::gen_mtime_preserved_fp_bump` | PASS |
| `gen_affected_only` | I-G3 | `codegen_c::gen_affected_only` | PASS |
| `gen_expected_vs_dirty` | I-G3 | `codegen_c::gen_expected_vs_dirty` | PASS |
| `gen_atomicity` (kill ×20) | I-G4 | `codegen_invariants::gen_atomicity` (+ `gen_atomicity_child` harness) | PASS |
| `gen_reconciliation` | I-G5 | `codegen_c::gen_reconciliation` | PASS |
| `gen_fingerprint_prune` | I-G5 | `codegen_invariants::gen_fingerprint_prune` | PASS |
| `node_api_invalidation` | I-G6 | `codegen_invariants::node_api_invalidation` | PASS |
| `atomic_replace_windows` | I-G7 | `codegen_invariants::atomic_replace_overwrites_existing` (POSIX `rename`); Windows `ReplaceFile`/`MoveFileEx` backend deferred | PARTIAL |
| `gen_direction_swap` + metamorphic | I-G8 | `codegen_invariants::gen_direction_swap`, `gen_direction_metamorphic` | PASS |
| `gen_bindings_handle_free` | I-G9 | `codegen_invariants::gen_bindings_handle_free` | PASS |
| `gen_check` (R6) | R6 | `xtask::gencheck::detects_hand_edited_generated_file`, `codegen_invariants::gen_check_detects_hand_edit` | PASS |
| 4-lang × golden topics | compile/import | `codegen_all::{gen_cpp_topic_compiles_and_decodes_golden, gen_rust_topic_compiles_and_decodes_golden, gen_python_topic_imports_and_decodes_golden}`, `codegen_c::gen_compiles_and_decodes_golden` | PASS (toolchains present) |
| Node-scoped surfaces per language | REQ-TOOL-028 | `codegen_c::gen_node_header_compiles`, `codegen_all::{gen_cpp_node_header_compiles, gen_python_node_module_imports, gen_rust_node_surface}` | PASS |

## How to reproduce

```text
cargo test -p drm-workspace codegen
cargo test -p xtask gencheck
cargo xtask gen-check --workspace <workspace-with-committed-.drm/gen>
```

`xtask gen-check` is wired blocking (R6): it regenerates the committed
`.drm/gen` tree into a scratch root and fails on any byte difference, missing
output, or orphan, so a hand-edited generated file cannot merge. The self-test
commits a generated tree, hand-edits one artifact, and asserts the diff is
detected (red→green fixture).

## Notes

- I-G7 is the only partial: the POSIX `rename` backend is exercised on Linux;
  the Windows atomic-replace backend (`ReplaceFile`/`MoveFileEx`) lands with the
  Windows port. The fallback preserves the prior artifact on failure but is not
  the documented Windows primitive.
- Generated bindings never embed runtime handles (I-G9): handle assignment is
  provably output-neutral, and `drm ids compact` therefore never triggers
  codegen (handle-system.md I-H7).
