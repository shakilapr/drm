# DRM fuzz targets (codebase.md 4.1; work-plan M01W1-E4-T3)

cargo-fuzz project, excluded from the workspace. Run on Linux runners
(libFuzzer does not build on MSVC; Windows fuzzing rides the sanitizer matrix).

Smoke: `cargo fuzz run dummy -- -max_total_time=30`

Planned targets (each becomes a permanent blocking gate when green):

| Target | Scope | Lands |
|---|---|---|
| `dummy` | harness smoke | W1 |
| `framing` | router frame parser (router-core.md) | W2+ |
| `schema` | schema parser (schema-parser.md grammar) | W3 |
| `hazard` | hazard protocol state machine (hazard-protocol.md Phases 14–16) | M03W1 |
