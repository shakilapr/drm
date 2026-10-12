# tests/ros2_compat — ROS 2 → DRM translation fixtures

Answers "can a simple translation of ROS 2 packages run on DRM?" Results and
findings: [ros2-compat-matrix.md](../../docs/metrics/ros2-compat-matrix.md).
Source set + field-level mapping: `references/ros2/` (gitignored; `rolling`).

## Layout

| Path | What it is |
|---|---|
| `translated/` | The adapted translation: bounds chosen, nested messages flattened to dotted prims. 13 topics + 3 nodes (`chatter_src`, `listener`, `talker`). `drm build` passes. |
| `mechanical/` | The naive 1:1 translation, kept as the negative fixture. `drm build` rejects all four topics, naming each defect. |
| `roundtrip/main.rs` | Fake-input round-trip harness over the generated Rust codec (`encode → bytes → decode` + field equality). **Currently fails to compile** — the generated code derives `Default` over `[u8; N]` arrays larger than 32 bytes; see finding 1 in the matrix doc. Kept as the reproduction. |

Files under `tests/ros2_compat/` are not workspace members, so `cargo` never
compiles them.

## Reproduce

```text
cargo run -p drm-cli -- --workspace tests/ros2_compat/translated build
cargo run -p drm-cli -- --workspace tests/ros2_compat/mechanical build   # expected to fail
cargo xtask perf-matrix --workspace tests/ros2_compat/translated --json target/ros2-perf-matrix.json
```
