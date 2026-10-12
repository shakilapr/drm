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

1. **Generated Rust does not compile for bounded strings/arrays > 32 bytes.**
   `tests/ros2_compat/translated/.drm/gen/rust/std_msgs/string/target.rs` emits
   `#[derive(Clone, Debug, Default, PartialEq)]` over `pub data: [u8; 256]`, and
   `[u8; 256]` does not implement `Default`:
   `error[E0277]: the trait bound [u8; 256]: Default is not satisfied`. Every
   bounded string and every `bytes[<=N]`/fixed array above 32 bytes produces this,
   i.e. most string-bearing topics. The fake-input round-trip harness
   (`cache/scratch/ros2_roundtrip/main.rs`) fails to compile for this reason.
2. **No gate compiles generated code, so (1) is invisible.**
   `cargo xtask gen-check` reports `clean (0 topics, 0 nodes)` — it neither sees
   these topics nor type-checks the rendered output; it compares rendered text.
3. **Two node directories with the same name silently drop one node.**
   `demos/nodes/talker` and `std_msgs/nodes/talker` coexist (different manifest
   names, `talker` and `chatter_src`); `drm info` listed only two of the three
   nodes and `drm build` failed with
   `V-SW-1: topic demos/chatter is consumed by [listener] but has no publisher`.
   Renaming the directory to `typed_pub` fixed it. No diagnostic names the
   collision.
4. **`cargo xtask fmt --check` is red on the committed tree when the secondary
   formatters are installed.** With `prettier` on `PATH` it fails with
   `Code style issues found in 225 files`. The gate is only green because
   `ruff`/`taplo`/`markdownlint` are absent here (`xtask fmt: skip … (not
   installed)`); a write-mode `cargo xtask fmt` rewrote 226 tracked files
   (markdown reflow), which was reverted.

## Not covered

- QoS: reliability, durability, history and message type live in ROS 2 call
  sites; a `drm.toml` records only `name`/`language`/`publishes`/`consumes`
  (depth maps loosely onto `@pool_size`). Services, actions, parameters, `tf`
  and discovery have no DRM equivalent.
- No ROS 2 runtime is installed here, so nothing was executed against `rclcpp` /
  `rclpy`; the comparison is source-level (message definitions and node wiring)
  plus DRM's own validator and runtime.
