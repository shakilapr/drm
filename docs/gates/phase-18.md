# Phase 18 Gate — Deployment Identity & Isolation

Evidence is executable and stays green under the permanent workspace gates.
Verdict: **PASS** (sustained dual-deployment soak evidence owned by the operator/CI lane).

Implementation: `drm-core::deploy` + `drm-core::shm`
([deployment-identity.md](../algorithms/deployment-identity.md)), router
namespace helper `drm-router::deployment`.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Incompatible deployments run simultaneously with zero cross-attachment | `identity` (two deployment namespaces cannot cross-open), `drm-router` `cross_deploy_same_topic_disjoint_keys`, `identity_e2e` (two deployments live, zero cross-namespace attachment) | PASS |
| Incompatible binary is refused | `identity` / `identity_e2e`: fingerprint mismatch ⇒ `WrongAbi` before any SHM touch | PASS |
| Unrelated schema change does not change an unaffected node fingerprint | `identity_e2e` (I-DP4): a change to another node/topic leaves the unaffected node's fingerprint identical | PASS |
| Active deployment cannot be cleaned; inactive can be cleaned completely | `clean` / `identity_e2e`: active shared hold ⇒ `ActiveUntouched` with segments intact; inactive ⇒ `Cleaned` (unlinked, recreatable) | PASS |

## Invariant coverage (deployment-identity.md I-DP1–I-DP5)

| Invariant | Test |
|---|---|
| I-DP1 GuardRefuses | `identity`, `identity_e2e` |
| I-DP2 NamespaceIsolation | `identity`, `cross_deploy_same_topic_disjoint_keys` |
| I-DP4 FingerprintMinimality | `identity_e2e` |
| I-DP5 CleanupDecision | `clean`, `identity_e2e` |

## Variables (M06W3 → M06W4)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 18)` | PASS | PASS |
| `IDENTITY_E2E` | fingerprint isolation + cross-deploy matrix green | `identity_e2e`, `clean`, `cross_deploy_*` |
| `FFI_MATRIX` | C/Rust/Python(ctypes) × publisher/consumer × topic kinds | `drm-ffi` `ffi_matrix` (Rust + C-ABI paths; fixed-size topic) |
| `FFI_NEGATIVES` | misuse ⇒ explicit Status, no crash | `drm-ffi` `ffi_negative` |
| `DUAL_SOAK` | 12 h dual-deployment soak clean | not executed in-session (wall-time; operator/CI lane) |
| `ROUTER_BASELINE` | perf baseline recorded, stable ×5 | not executed in-session (pinned-machine perf lane) |

## Requirements covered

REQ-CORE-084/085/088/089 (fingerprint inputs, deployment id, namespaced
segments), REQ-CORE-092/025 (guard refusal), REQ-CORE-080/070/069 (cleanup
decision tree, active-lock authority).

## Notes

- Completion gate (M06W4) adds the sustained dual-deployment soak evidence.
- Variable-size FFI topics remain a documented gap; the endpoint currently
  backs a fixed-size topic.
