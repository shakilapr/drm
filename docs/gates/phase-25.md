# Phase 25 Gate — Native Build Orchestration & `drm dev`

Chaired at M09W3 (E2, with E4 evidence). Evidence is executable and stays green
under the permanent gates. Verdict: **PASS** (`I-BO1`..`I-BO4`; the watch
dispatch core `I-BO5`..`I-BO8` and the adapter matrix are supporting evidence).

Implementation: `crates/drm-workspace/src/orchestrator.rs` and
`crates/drm-workspace/src/watch.rs`
([build-orchestration.md](../algorithms/toolchain/build-orchestration.md));
CLI facade `crates/drm-cli/src/{build,dev,info}.rs`. Owner: E2. Gate chair: E2
(E4 presents evidence).

## Criteria ⇄ evidence

| Criterion (work-plan #25) | Invariant | Artifact | Test | Result |
|---|---|---|---|---|
| DRM declaration change rebuilds exactly the affected artifacts/nodes | `I-BO1`, `I-BO3` | `orchestrator::build` step probe; `watch::affected_nodes` | `build_order::build_order`, `watch_dispatch::dev_affected_nodes` | PASS |
| Application source change delegates dependency decisions to the native builder | `I-BO2` | `Invocation` per owning node; no DRM source parse | `watch_dispatch::{dev_no_source_tracking,no_source_tracking_probe}`, `dev_smoke::dev_scripted_edit_cycles` | PASS |
| DRM never tracks source/header dependency graphs | `I-BO2` | path-only ownership (`watch::owner_of`) | `watch_dispatch::dev_no_source_tracking` | PASS |
| Repeated watch edits never produce stale generated code | `I-BO4` | model reload per declaration batch; final-state diff | `watch_dispatch::{dev_rapid_edits_final_state,dev_rapid_edits_coalesced}`, `dev_smoke::dev_scripted_edit_cycles` | PASS |
| Exact orchestration sequence, adapters last | `I-BO1` | `BuildStep` sequence probe | `build_order::build_order` | PASS |
| Verify failure aborts before adapters | `I-BO1` | `BuildError::Verify` short-circuit | `build_order::verify_failure_aborts_before_adapters` | PASS |
| Adapter matrix (CMake / Cargo / custom command) | `I-BO1` | `BuildAdapter` + `invocation_for` | `build_order::adapter_matrix` | PASS |
| Adapter environment hooks (`DRM_GEN_INCLUDE`, `DRM_DEPLOY_METADATA`) | — | `AdapterInvocation.env` | `build_order::adapter_matrix` | PASS |
| Reload before dispatch within a debounced batch | `I-BO5` | `watch::apply_batch` | `watch_dispatch::watch_reload_before_dispatch` | PASS |
| Pending dirty retained across invalid declaration batches | `I-BO6` | `WatchState.pending_source` | `watch_dispatch::watch_pending_dirty_dispatch` | PASS |
| Watcher overflow ⇒ full rescan recovery | `I-BO7` | `WatchControl::Overflow` ⇒ `RescanRequired` | `watch_dispatch::watch_overflow_rescan` | PASS |
| Path-only ownership (binary/non-UTF8 source) | `I-BO8` | `owner_of` path classification | `watch_dispatch::watch_binary_source` | PASS |
| `drm info` deterministic read-only projection | — | `orchestrator::info` | `build_order::info_projection_is_deterministic`, `example_build::example_info_deterministic` | PASS |
| Dev-mode smoke (scripted edits on a temp workspace) | `I-BO2`, `I-BO4` | `drm dev --once` + successive dev cycles | `dev_smoke::{dev_once_smoke,dev_scripted_edit_cycles}` | PASS |
| Mixed example (c + python + rust) builds green | — | `examples/mixed_robot` | `example_build::example_build_green` | PASS |

## Verify integration (Phase 26 start)

`V-RT` consumes the single route-derivation output (`topology::derive`); the
negatives v2 suite extends the catalogue's rejection matrix
([verify-engine.md](../algorithms/toolchain/verify-engine.md),
[route-derivation.md](../algorithms/toolchain/route-derivation.md)):

| Case | Test | Result |
|---|---|---|
| `V-RT` framing overflow / ambiguous route / no route | `verify_negatives_v2::{verify_negatives_v2_route,verify_route_integration_single_source}` | PASS |
| `V-REL` reliable over UDP and serial | `verify_negatives_v2::verify_negatives_v2_reliability` | PASS |
| `V-MEM` budget / lockable / pool invariant | `verify_negatives_v2::verify_negatives_v2_memory` | PASS |
| `V-CMP` `reject` without a required sequential consumer | `verify_negatives_v2::verify_negatives_v2_compatibility` | PASS |
| Collect-all, never short-circuit | `verify_negatives_v2::verify_negatives_v2_no_shortcircuit` | PASS |

## How to reproduce

```text
cargo test -p drm-workspace --test build_order
cargo test -p drm-workspace --test watch_dispatch
cargo test -p drm-workspace --test verify_negatives_v2
cargo test -p drm-cli --test dev_smoke
cargo test -p drm-cli --test example_build
```

## Notes

- Native adapter freshness (CMake/Ninja, Cargo, custom command) is delegated to
  the toolchain: DRM invokes one adapter per affected node and never parses app
  sources (`I-BO2`).
- The watch skeleton uses a dependency-free mtime poller in `drm dev`; a native
  event watcher replaces the poller without changing `watch::apply_batch`.
  Debounce is coalescing only, never a correctness mechanism (`I-BO7`).
- Handle reassignment/compaction regenerates deployment artifacts but leaves
  bindings untouched (codegen `I-G9`); only binaries embedding deployment tables
  rebuild.
