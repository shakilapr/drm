# DRM fuzz targets (codebase.md 4.1; work-plan M01W1-E4-T3)

cargo-fuzz project, excluded from the workspace. Run on Linux runners
(libFuzzer does not build on MSVC; Windows fuzzing rides the sanitizer matrix).

Smoke: `cargo fuzz run --fuzz-dir tests/fuzz schema -- -max_total_time=120 -timeout=5`

Live targets (work-plan M18W1-E4-T1, `FUZZ_TARGETS_LIVE = 5/5` core targets;
each becomes a permanent blocking gate when green):

| Target | Scope | Source |
|---|---|---|
| `dummy` | harness smoke | `dummy.rs` |
| `schema` | schema parser (schema-parser.md grammar) | `schema.rs` |
| `framing` | router record/multiplex decoder (router-core.md) | `framing.rs` |
| `datagram` | UDP datagram + wire-header decode (router-core.md) | `datagram.rs` |
| `hazard` | structured hazard-protocol storm (hazard-protocol.md Phases 14–16) | `hazard.rs` |
| `serial` | serial framing + streaming COBS decoder (serial-mcu.md I-SR3/I-SR4) | `serial.rs` |
| `verify` | malformed workspace manifests + model build (workspace-model.md) | `verify.rs` |

Nightly wiring, triage board, and the coverage report live in
[docs/spec/fuzz-triage-board.md](../../docs/spec/fuzz-triage-board.md),
[docs/spec/fuzz-coverage.md](../../docs/spec/fuzz-coverage.md), and the
`fuzz-nightly` workflow.
