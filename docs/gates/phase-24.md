# Phase 24 Gate — Code Generation & Invalidation

Chaired at M09W1 (E4). Evidence is executable and stays green under the
permanent gates. Verdict: **PASS** (I-G1..I-G6, I-G8, I-G9, R6; I-G7 POSIX
green, Windows `ReplaceFile`/`MoveFileEx` backend deferred to the Windows port).

Implementation: `crates/drm-workspace/src/codegen`
([codegen-invalidation.md](../algorithms/toolchain/codegen-invalidation.md)) and
`xtask gen-check` (R6). Full evidence pack:
[phase-24-evidence.md](phase-24-evidence.md). Owner: E2. Gate chair: E4.

## Criteria ⇄ evidence

| Criterion | Artifact | Test | Result |
|---|---|---|---|
| I-G1 byte-identical outputs | `codegen::{plan,execute}`, stable paths | `codegen_c::gen_determinism`, `codegen_all::gen_all_determinism` | PASS |
| I-G2 timestamp preservation | content-compare write | `codegen_c::{gen_mtime_preserved,gen_mtime_preserved_fp_bump}` | PASS |
| I-G3 affected-only regeneration | `expected_outputs` vs `dirty_units` | `codegen_c::{gen_affected_only,gen_expected_vs_dirty}` | PASS |
| I-G4 atomicity under kill | `atomic_replace` | `codegen_invariants::gen_atomicity` (×20) | PASS |
| I-G5 direct reconciliation | orphan removal + fingerprint prune | `codegen_c::gen_reconciliation`, `codegen_invariants::gen_fingerprint_prune` | PASS |
| I-G6 node-API invalidation | directed-capability fingerprint | `codegen_invariants::node_api_invalidation` | PASS |
| I-G7 portable replace | POSIX `rename` backend | `codegen_invariants::atomic_replace_overwrites_existing` | PARTIAL |
| I-G8 direction-sensitive fingerprint | ordered directed capabilities | `codegen_invariants::{gen_direction_swap,gen_direction_metamorphic}` | PASS |
| I-G9 no handles in bindings | canonical identities only | `codegen_invariants::gen_bindings_handle_free` | PASS |
| R6 gen-check blocking | `xtask gen-check` + self-test | `xtask::gencheck::detects_hand_edited_generated_file` | PASS |
| 4-language outputs compile | C/C++/Rust/Python renderers | `codegen_all::{gen_cpp_topic_compiles_and_decodes_golden,gen_rust_topic_compiles_and_decodes_golden,gen_python_topic_imports_and_decodes_golden}`, `codegen_c::gen_compiles_and_decodes_golden` | PASS |

## Orchestration slice (Phase 25 start)

The codegen gate is exercised end-to-end by the new `drm build` orchestrator
([build-orchestration.md](../algorithms/toolchain/build-orchestration.md)):

| Case | Test | Result |
|---|---|---|
| I-BO1 exact sequence (parse→model→handles→deploy→codegen→verify→adapters) | `build_order::build_order` | PASS |
| Verify failure aborts before adapters | `build_order::verify_failure_aborts_before_adapters` | PASS |
| Adapter matrix + environment hooks | `build_order::adapter_matrix` | PASS |
| `drm info` deterministic projection | `build_order::info_projection_is_deterministic`, `example_build::example_info_deterministic` | PASS |
| Mixed example (c + python + rust) builds green | `example_build::example_build_green` | PASS |

## How to reproduce

```text
cargo test -p drm-workspace codegen
cargo test -p xtask gencheck
cargo test -p drm-workspace --test build_order
cargo test -p drm-cli --test example_build
cargo xtask gen-check --workspace <workspace-with-committed-.drm/gen>
```

## Notes

- I-G7 remains the only partial: the POSIX `rename` backend is exercised on
  Linux; the Windows atomic-replace backend lands with the Windows port. The
  fallback preserves the prior artifact on failure.
- Generated bindings never embed runtime handles (I-G9); `drm ids compact`
  therefore never triggers codegen.
- `xtask gen-check` is wired blocking: it regenerates into a scratch root and
  fails on any byte difference, missing output, or orphan, so a hand-edited
  generated file cannot merge.
