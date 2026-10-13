# ROS 2 → DRM compatibility matrix

Question: can a *simple* translation of ROS 2 packages run on DRM? Sources and
the field-level mapping live in `references/ros2/` (gitignored, `rolling` ref;
66 message definitions from `std_msgs`, `geometry_msgs`, `sensor_msgs`,
`builtin_interfaces`, plus the `demo_nodes_cpp` talker/listener and the rclcpp /
rclpy minimal publisher/subscriber pairs). Per-message verdicts:
`references/ros2/TRANSLATION.md`; node wiring: `references/ros2/NODE_MAPPING.md`.

Verdict counts over the 66 messages: **28 clean** (1:1 after flattening nested
structs), **22 needs-bounds** (an unbounded ROS string/sequence must be given a
concrete bound), **16 cannot-map** (arrays of structs, or array depth > 1 —
in expressible in DRM).

## Result

| Fixture | Contents | `drm build` |
|---|---|---|
| `tests/ros2_compat/translated/` | adapted translation: bounds chosen, nested structs flattened to dotted prims, 13 topics + 3 nodes | **PASS** — `64 generated (4 rendered), 13 verified, 3 built [chatter_src, listener, talker]` |
| `tests/ros2_compat/mechanical/` | naive 1:1 translation (`data: string`, `ranges: f32[]`, `stamp: builtin_interfaces/Time`, `name: string[32][<=32]`) | **REJECTED** — `workspace model invalid: topic sensor_msgs/joint_state: 1 defect(s); topic sensor_msgs/laser_scan: 1 defect(s); topic std_msgs/header: 3 defect(s); topic std_msgs/string: 1 defect(s)` |

So the answer is: **not by simple translation, but yes after an explicit
adaptation step.** The adaptation is mechanical and small (choose bounds, flatten
nested messages), but it is a *semantic* decision ROS 2 never forces — the
compiler cannot make it. Every string and every sequence needs a bound before a
`.drm` file is valid, and one ROS 2 message (`JointState.name`) needs a
structure DRM forbids outright (array of strings).

## Running the translated topics in DRM

`cargo xtask perf-matrix --workspace tests/ros2_compat/translated` activates,
publishes to and reads every translated topic through the production runtime
(2026-10-12, `linux x86_64 (6.8.0-146-generic)`):

| Topic | Bytes | pub p50 | pub p99 | rd p99 | ops/s | seg B |
|---|---|---|---|---|---|---|
| `demos/chatter` | 260 | 70 | 89 | 60 | 24,839,007 | 1,432 |
| `geometry_msgs/pose` | 56 | 60 | 71 | 60 | 26,247,289 | 608 |
| `geometry_msgs/pose_stamped` | 132 | 69 | 71 | 60 | 25,822,301 | 936 |
| `geometry_msgs/transform_stamped` | 208 | 69 | 71 | 60 | 25,959,572 | 1,224 |
| `geometry_msgs/twist` | 48 | 60 | 80 | 70 | 27,251,435 | 576 |
| `sensor_msgs/compressed_image` | 1,048,704 | 22,830 | 35,711 | 35,271 | 40,123 | 2,097,784 |
| `sensor_msgs/imu` | 372 | 130 | 140 | 80 | 10,261,311 | 5,400 |
| `sensor_msgs/laser_scan` | 16,508 | 320 | 340 | 350 | 3,419,903 | 49,912 |
| `std_msgs/bool` | 1 | 60 | 71 | 60 | 27,594,646 | 376 |
| `std_msgs/float64` | 8 | 60 | 71 | 60 | 28,599,763 | 376 |
| `std_msgs/header` | 76 | 60 | 80 | 70 | 25,892,121 | 696 |
| `std_msgs/int32` | 4 | 60 | 71 | 60 | 27,589,570 | 376 |
| `std_msgs/string` | 260 | 70 | 90 | 70 | 25,144,126 | 1,432 |

All 26 budget verdicts PASS (ceiling 50 µs). Cross-process: one 1,432 B segment,
`publisher_open` 58,712 ns, epoch 1, `consumer_open` (separate process) 15,281 ns,
second publisher refused. The 1 MiB `compressed_image` topic runs at ~36 µs p99 —
within budget but the closest to it (71 %).

## Findings (defects the exercise surfaced)

Status as of the fix tranche; each item names its gate.

1. **Generated Rust did not compile for bounded strings/arrays > 32 bytes.**
   `#[derive(Clone, Debug, Default, PartialEq)]` over `pub data: [u8; 256]`
   failed with `error[E0277]: the trait bound [u8; 256]: Default is not
   satisfied` for every string-bearing topic, because `std` implements
   `Default` only for arrays of 0..=32 elements (rust-lang/rust#61415; the
   lifting PR was rejected). **Fixed** — the renderer emits an explicit
   `Default` impl with zero literals per field. Gate:
   `codegen_all::gen_rust_bounded_topic_compiles_and_round_trips` (the previous
   compile tests all used `a: u8, b: u64`, which has no array field, so the
   defect was invisible). The harness now round-trips **12/12** topics.
2. **`cargo xtask gen-check` still does not compile generated code** — it
   compares rendered text (`regen_diff`) and, run from a root without
   `packages/`, reports `clean (0 topics, 0 nodes)`. The compile regression is
   now covered by (1)'s test rather than by gen-check itself. **Partly fixed.**
3. **Two node directories with the same name silently dropped one node.**
   `demos/nodes/talker` and `std_msgs/nodes/talker` collapsed in the stem map
   before the identity check; `drm info` listed two of three nodes and the build
   failed with a misleading `V-SW-1 … has no publisher`. **Fixed** — the
   collision is now a `ModelError::DuplicateNodeDir` naming the shared
   directory. Gate: `model_nodes::nodes_duplicate_directory_rejected`, plus an
   end-to-end `drm build` that reports the duplicate instead of failing on the
   downstream symptom.
4. **`cargo xtask fmt --check` was red on the committed tree with the secondary
   formatters installed** (`Code style issues found in 225 files`), and a
   write-mode `cargo xtask fmt` would have rewritten all 40 byte-exact
   `tests/fixtures/golden` JSON vectors and reflowed the hand-wrapped markdown
   tree. **Fixed** — `.prettierignore` scopes prettier away from the goldens,
   the markdown tree and the separate `apps/studio` project (228 → 1 file), and
   the single real violation (a generated TS union arm on one over-long line)
   is emitted wrapped. `cargo xtask fmt --check` is now green *with* prettier
   present, and gen-check stays byte-clean.
5. **Documentation drift on generated artifact paths.** Both the bindings doc
   and `architecture.md` named `.drm/gen/rust/…/mod.rs` and
   `.drm/gen/cpp/types/…`; the renderer writes `…/target.rs` and
   `…/<pkg>/<topic>/target.hpp`. **Fixed.**
6. **A renderer change does not invalidate existing generated trees.** After
   fixing (1), `drm build` reported `64 generated (0 rendered)` and left the
   stale broken modules in place; only deleting `.drm/gen` forced a re-render.
   The invalidation fingerprint covers the schema and layout, not the renderer
   version. `gen-check` catches it (fresh render vs on-disk tree), but a plain
   `drm build` does not. **Open.**

## Not covered

- QoS: reliability, durability, history and message type live in ROS 2 call
  sites; a `drm.toml` records only `name`/`language`/`publishes`/`consumes`
  (depth maps loosely onto `@pool_size`). Services, actions, parameters, `tf`
  and discovery have no DRM equivalent.
- No ROS 2 runtime is installed here, so nothing was executed against `rclcpp` /
  `rclpy`; the comparison is source-level (message definitions and node wiring)
  plus DRM's own validator and runtime.
