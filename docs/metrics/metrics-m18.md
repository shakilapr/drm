# Metrics — Month 18 (M18)

Normative source: [work-plan.md](../../work-plan.md) W6/W8 hardening →
qualification groundwork. Month 18 expands the fuzz fleet and chaos suites,
sets the benchmark baseline, runs the full interop/sanitizer matrix, and closes
the hardening sprint. One section per weekly tranche.

## M18W1 — Hardening: Fuzz Expansion + Chaos Suites

| Variable | Target | Measured |
|---|---|---|
| `FUZZ_TARGETS_LIVE` | schema + framing + hazard + datagram + serial targets nightly | 5/5 + verify-fixture — `tests/fuzz/fuzz_targets/{schema,framing,datagram,hazard,serial,verify}.rs` registered in `tests/fuzz/Cargo.toml`; nightly `fuzz` matrix runs all six at 3600 s, `fuzz-pr` smokes `hazard`/`serial`/`verify` at 120 s; 5000-run local smoke clean per new target |
| `CHAOS_GREEN` | kill -9 matrix + PID reuse + exhaustion storms green nightly | 1 — `cargo test -p drm-core --test stress -- --ignored stress_chaos` 3/3 (`stress_chaos_kill_matrix` every borrow stage, `stress_chaos_pid_reuse`, `stress_chaos_exhaustion_storm`); `cargo test -p drm-core --test shm_kill -- --ignored stress_shm_kill` 8/8 (repaired child invocation); `cargo test -p drm-router --test link_storm -- --ignored stress_link_storm` 2/2 |
| `FUZZ_CRASHES` | 0 untriaged | 0 — [fuzz-triage-board.md](../spec/fuzz-triage-board.md), no open findings |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Fuzz fleet** (`tests/fuzz/`): `hazard` (structured storm through the public
  publish/read/reclaim surface), `serial` (one-shot decode, streaming resync,
  COBS reversibility), `verify` (malformed manifests + `model::build` +
  `verify_lite`). `drm-core` added as a fuzz dependency; the cargo-fuzz lockfile
  regenerated.
- **Chaos suites** (`crates/drm-core/tests/stress/chaos_kill_matrix.rs`,
  `crates/drm-router/tests/link_storm.rs`): every borrow/publish stage, PID
  reuse, pool-exhaustion storms, flapping links × routers, and session flapping
  with exact gap accounting.
- **Repair** (`crates/drm-core/tests/shm_kill.rs`): the child driver is now
  invoked with `--ignored`, so the 8 real `SIGKILL` cells execute under
  `cargo xtask test --stress` (regression permanence, AGENTS #14).
- **Nightly wiring** (`.github/workflows/fuzz-nightly.yml`,
  `.github/workflows/ci.yml`): `fuzz` matrix + `chaos` job; PR smoke for the
  three new targets.
- **Inventory / board / coverage** ([chaos-inventory.md](../spec/chaos-inventory.md),
  [fuzz-triage-board.md](../spec/fuzz-triage-board.md),
  [fuzz-coverage.md](../spec/fuzz-coverage.md)); gate evidence
  [m18w1.md](../gates/m18w1.md).

Notes:

- No architecture divergence: pinned invariants only; no ADR.
- The three toolchain process-kill cells (build/deploy/clean) are recorded as
  gaps and carried to M18W2 (E1).
- `spaces/`/`index.drm` directory-layer parsing is not implemented; the
  verify-fixture target covers the byte-level entry points.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `FUZZ_TARGETS_LIVE = 5/5`; `CHAOS_GREEN = 1`; `FUZZ_CRASHES = 0`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M18W1) = PASS`**

### How to reproduce

```text
cargo build --manifest-path tests/fuzz/Cargo.toml --bins
./tests/fuzz/target/debug/hazard -runs=5000 -timeout=10
./tests/fuzz/target/debug/serial -runs=5000 -timeout=10
./tests/fuzz/target/debug/verify -runs=5000 -timeout=10
cargo test -p drm-core --test stress -- --ignored stress_chaos
cargo test -p drm-core --test shm_kill -- --ignored stress_shm_kill
cargo test -p drm-router --test link_storm -- --ignored stress_link_storm
cargo xtask audit-traceability
cargo xtask check-layering
```
