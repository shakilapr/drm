# ROS 2 package compatibility on DRM

Status: analysis, 2026-10-14. Companion to
[docs/metrics/ros2-compat-matrix.md](docs/metrics/ros2-compat-matrix.md) (the
earlier 66-message source-level study) and
[results-and-benchmarks.md](results-and-benchmarks.md).

## Question

For each of the 20 most-used ROS 2 packages/frameworks: can it be integrated
with DRM, and does it work with DRM today? Where it does not, what is the **root
cause** — a deep architectural problem, a missing feature, a defect, or something
else?

## Method and evidence

- Environment: ROS 2 Jazzy on Ubuntu 24.04 (384 packages installed), including
  `rviz2` 14.1.24, `nav2_bringup`/`nav2_msgs`, `slam_toolbox`,
  `robot_localization`, `ros_gz_interfaces`, `rtabmap_msgs`,
  `control_msgs`/`trajectory_msgs`, `moveit_msgs`, `foxglove_msgs` +
  `foxglove_bridge`, `rosbridge_suite`, `image_pipeline`,
  `realsense2_camera_msgs`, `mavros_msgs`, `tf2_msgs`, `rosbag2_interfaces`.
  Upstream sources for `ros2/rviz`, `ros-navigation/navigation2`, `ros2/geometry2`,
  `ros2/common_interfaces`, `ros2/rclcpp` and others were cloned for interface
  inventory.
- Static classification: every installed interface (`/opt/ros/jazzy/share/*/msg`,
  `srv`, `action`) was classified against DRM's type grammar
  (`docs/algorithms/toolchain/schema-parser.md`, restated in
  `references/ros2/TRANSLATION.md`): prims, `string[N]`, `bytes[<=N]`,
  `prim[N]`/`prim[<=N]`, `enum`; no nested message types (nested structs are
  *flattenable* to dotted prims), no arrays of structs, no arrays of strings,
  array depth ≤ 1, and every unbounded string/sequence must be given a bound.
- Falsification: the classifier's verdicts were checked against the real
  compiler by hand-writing `.drm` files from real ROS 2 messages and running
  `drm build` (see §Evidence index). Accepted and rejected cases matched the
  classification in every case tested.
- Caveat on "works": no ROS 2 ↔ DRM integration is implemented, so **no package
  works with DRM today**. This report therefore separates three questions, which
  fail for different reasons:
  1. can the package's data even be *described* as DRM topics? (static, measured)
  2. could a bridge carry it? (design)
  3. is it the kind of component that can be bridged at all? (layer)

## Headline result

- **0 of 20 packages work with DRM today.**
- Of **896** installed interfaces: **200 clean** (map 1:1), **373 need a bound
  chosen**, **323 cannot be expressed at all** (36%).
- Of the 323 inexpressible interfaces, the causes are **226 "array of message"**,
  **96 "array of string"**, **1 recursive** — i.e. **one single root cause**: DRM
  has no form for a bounded sequence of records or of strings.

## Root causes, classified

| # | Root cause | Classification |
|---|---|---|
| A | DRM's data model has no variable-length record/string sequence | **deep architectural** (deliberate design boundary) |
| B | v1.0 had no cross-process data path (no publish/read over an attached segment) | architectural gap, **now fixed** |
| C | binding surface was effectively Rust-only; generated C/C++/Python paths are conformance stubs | architectural gap, **partially fixed** |
| D | transport/semantic impedance (DDS dynamics vs static deployment, multi-publisher vs single writer) | architectural, **resolvable in a bridge** |
| E | no bridge and no `rmw_drm` | **missing feature** |
| F | package is not a messaging node (hardware abstraction, simulator, protocol server, MCU toolchain) | **layer mismatch — not a DRM defect** |
| G | defects found during the exercise | **bugs** (2), one **missing feature** |

### A — Deep architectural: no bounded sequence of records or strings

DRM fixes a topic's shape at activation: one canonical layout per topic, slots
of fixed `sample_max`, zero per-message heap allocation, zero-copy reads. A
sample whose *shape* varies cannot be represented. ROS 2 is the opposite: message
composition is recursive and sequences are unbounded by default.

Consequences measured: 226 interfaces are `SomeMsg[]` / `SomeMsg[N]`
(`TransformStamped[]`, `PoseStamped[]`, `Point[]`, `PointField[]`,
`JointTrajectoryPoint[]`, `KeyValue[]`, …) and 96 are `string[]`
(`JointState.name`, `DiagnosticArray` fields, `Marker.text` neighbours, …).
Nested *scalar* messages are fine — they flatten to dotted prims (verified).

This is not an oversight: expressed as a DRM topic, `TransformStamped[]` is
rejected with 2 defects and `JointState` with 6 (verified). The fix is a
canonical form for bounded sequences of records/strings — a change to the canon
codec, the layout model and the zero-allocation invariants (ADR-level, DRM core,
not a ROS adapter).

### B — v1.0 had no cross-process data path (fixed)

Measured before the fix: `Channel::activate` allocates a process-local arena
(`crates/drm-core/src/channel/mod.rs:101-152`), the only `publish`/`latest`
implementation; `publisher_open`/`consumer_open` attach a real POSIX segment but
return only checked offsets plus a raw `Mapping` (`channel/lifecycle.rs:405-420`,
`487-500`); `init_fresh` writes only prefix + descriptor + topic name
(`lifecycle.rs:260-271`) while the publisher writes lifecycle fields — not
payload — through mapped atomics (`ccb_ptr!`, `lifecycle.rs:210-246`). A
two-process probe confirmed a consumer *can* attach and read the publisher's
bytes at `payload_offset`, but there was no operation to publish or read a
sample. Every ROS 2 package (a C++/Python process) was therefore blocked
identically.

**Fixed**: `crates/drm-core/src/channel/attached.rs` (`AttachedChannel` publish /
read-latest / read-next over the segment, reusing the in-process ring/pool/
overflow helpers) and `HistoryRing::from_mapped` — commit `0081d0b`, verified by
`attached_publish_read_roundtrip`.

### C — Binding surface was effectively Rust-only (partially fixed)

`drm-ffi`'s endpoint group is in-process: `drm_context_open` calls
`Channel::activate`, one topic id, `ENDPOINT_SAMPLE_MAX = 64`
(`crates/drm-ffi/src/lib.rs`); the C header says so — "cross-process SHM
endpoints are a later phase… an export with no live hook yet … returns
`DRM_ERR_NOT_IMPLEMENTED`, never a fabricated handle"
(`bindings/c/include/drm/drm.h`). `xtask/src/cross_os.rs:521-523` records the
C/C++/Python interop cells as *scheduled until their router-transport bindings
land*. Generated node bindings call the stub read path while generated topic
headers are data-shape (`encode`/`decode`) only, so no generated C/C++ typed
path existed over a real segment.

**Partially fixed**: the attached group (`drm_attached_open`/`_close`/
`_publish`/`_read_latest`/`_read_next`, commit `088af6b`, `REQ-CORE-107`,
append-only). **Still missing**: typed codegen over the attached path,
zero-copy borrow across the ABI, attached hazard registration, required-ack
floors.

### D — Transport and semantic impedance (bridgeable)

- Dynamic discovery (DDS) vs static deployment identity.
- Per-message QoS vs DRM overflow policy + fixed history depth.
- Runtime-sized CDR serialization vs fixed canonical layouts (no per-message
  serialization in DRM).
- Multi-publisher topics in ROS vs DRM's **single writer per topic**
  (AGENTS.md #4): a ROS topic with several publishers needs an explicit DRM
  multiplexer node.
- Type identity: ROS type names/hashes vs DRM contract fingerprints.
- External I/O may never retain a borrowed sample (AGENTS.md #5): every crossing
  must copy — which is exactly why the attached path is copy-form first.

None of this is a defect; it is where a bridge does its work.

### E — Missing feature: no bridge, no `rmw_drm`

Nothing in DRM speaks ROS 2's API or wire protocol. Two routes: a sidecar bridge
(a process translating declared topics, days) or `rmw_drm` (rclcpp/rclpy apps run
natively on DRM; needs per-message typesupport generation, months).

### F — Layer mismatch (not a DRM defect)

`ros2_control`/`ros2_controllers` (real-time hardware abstraction and controller
plugins), `ros_gz` (simulator transport), `rosbridge_suite`/`foxglove_bridge`
(websocket/JSON servers), `micro_ros_setup` (MCU build toolchain). "Integrate
with DRM" here means porting a subsystem, and DRM's own MCU story would be a DRM
agent rather than micro-ROS.

### G — Defects found (bugs) and one provisioning gap

- **G1 (bug, open):** `drm build` does not invalidate an existing generated tree
  when the renderer changes. `plan_with` skips any unit whose fingerprint
  matches and whose file exists
  (`crates/drm-workspace/src/codegen/mod.rs:351-368`) and `GENERATOR_VERSION` is
  hand-maintained (`mod.rs:25-28`), so after a renderer fix the tree keeps the
  old modules — observed as `64 generated (0 rendered)` over a tree containing
  modules from the previous renderer. Silent staleness; either derive the
  generator identity (renderer digest) or gate the version bump.
- **G2 (bug in a gate, open):** `xtask gen-check` passes vacuously — with no
  `packages/` it prints "nothing to check" and returns `Ok`, and a tree with zero
  topics reports `clean (0 topics, 0 nodes)` (`xtask/src/gencheck.rs:24-36`). A
  gate that verified nothing reports green.
- **G3 (bug, fixed earlier):** SIGSEGV in `crates/drm-core/src/shm/mod.rs`
  (`Mapping` ownership) — fixed with `owned`/`borrowed_view()`.
- **G4 (missing feature, open):** no deployment→runtime provisioning of a
  segment's expectation. `publisher_open`/`consumer_open` take
  `Expectation` + `SegmentName` + `LockPaths`, and every existing caller uses a
  synthetic expectation (`schema_hash = [7;16]`, `layout_hash = [9;16]`,
  `topic_handle = 42` in `xtask/src/perf.rs`). `drm build` does not yet emit the
  segment name, hashes and handle a node must attach with, so two *built* apps
  cannot meet on one segment. This is the next prerequisite for any bridge.

## Per-package verdicts

Legend: **A** data model, **B/C** DRM runtime/binding gap, **D** bridge
semantics, **E** no bridge, **F** layer, **G** defect. Counts are
`checked = clean / needs-bounds / cannot-map` over the interfaces on the DRM
boundary.

| # | Package | Boundary interfaces | Main blocker | Classification | Verdict |
|---|---|---|---|---|---|
| 1 | Navigation2 | `nav2_msgs` 51 = 9/27/15, `nav_msgs` 12 = 1/6/5 | `Path` = `PoseStamped[]`; costmap/pose arrays | A + D + E | ❌ fails |
| 2 | MoveIt 2 | `moveit_msgs` 81 = 7/21/**53** | planning/trajectory message arrays | A (+E) | ❌ fails (worst in set) |
| 3 | ros2_control | `control_msgs` 36 = 2/14/**20**, `trajectory_msgs` 4 = 0/1/3 | trajectory point arrays | A + F | ❌ fails |
| 4 | ros2_controllers | same family | same | A + F | ❌ fails |
| 5 | slam_toolbox | 11 = 7/4/**0** | none in data model | C + D + E | ⚠️ integrable, not bridged |
| 6 | robot_localization | 8 = 4/3/1 | none blocking | C + D + E | ⚠️ integrable, not bridged |
| 7 | ros_gz | `ros_gz_interfaces` 28 = 3/20/5 | simulator transport | F (+A) | ❌ different layer |
| 8 | rtabmap_ros | `rtabmap_msgs` 39 = 11/9/**19** | map/graph message arrays | A + E | ❌ fails |
| 9 | rviz2 | `visualization_msgs` 13 = 1/4/**8** | `Marker` (`Point[]`, `string`), `PointCloud2` | A + D + E | ❌ fails |
| 10 | rosbag2 | `rosbag2_interfaces` 19 = 14/5/**0** | records DDS; DRM has its own recorder | E (different mechanism) | ❌ not integrated |
| 11 | tf2_ros | `tf2_msgs` 4 = 0/3/1 | `TFMessage` = `TransformStamped[]` → **2 defects (verified)** | A | ❌ fails |
| 12 | robot_state_publisher | TF + JointState | both inexpressible | A | ❌ fails |
| 13 | joint_state_publisher | `sensor_msgs/JointState` | `name: string[]` → **6 defects (verified)** | A | ❌ fails |
| 14 | rosbridge_suite | `rosapi_msgs` 30 = 1/9/**20**, `rosbridge_msgs` 2 | schema mirrors arbitrary ROS types | A + F | ❌ fails |
| 15 | foxglove_bridge | `foxglove_msgs` 43 = 7/18/**18** | as 14 | A + F | ❌ fails |
| 16 | image_pipeline | `sensor_msgs` 28 = 3/19/6 | `Image` = bounded blob (workable); `CameraInfo` arrays | A (partial) + E | ⚠️ partially integrable |
| 17 | realsense2_camera | `realsense2_camera_msgs` 15 = 1/14/**0** | none in data model | C + E + hardware | ⚠️ integrable, needs device |
| 18 | micro_ros_setup | toolchain, no interfaces | not a node | F | ❌ different layer |
| 19 | MAVROS | `mavros_msgs` 129 = 37/83/**9** | mostly prims; needs a MAVLink link + FCU | C + D + E | ⚠️ mostly integrable |
| 20 | teleop_twist_keyboard | `geometry_msgs/Twist` = 6×`f64` | none — **accepted by `drm build` (verified)** | E only | ⚠️ data contract already exists |

Verdict distribution: **14 fail**, **6 are integrable in principle** (5–6, 10,
16, 17, 19, 20) but none are bridged; **5 of the 20** are not messaging nodes at
all (3, 4, 7, 14, 15, 18).

Notable specifics:

- **rviz2** ships no `.msg` of its own; every one of its display types comes from
  `visualization_msgs`/`sensor_msgs`/`tf2_msgs`. Its most-used displays
  (`Marker`, `PointCloud2`, TF) are all inexpressible, so it cannot be fed from
  DRM even after the bridge — the blocker is the data model, not the transport.
- **tf2** is the ecosystem's coordination backbone; its whole contract is one
  message array. Nothing in the ROS 2 navigation/visualization stack works
  without it, which is why group **A** cannot be "worked around" for the common
  packages.
- **MAVROS** is far more mappable than expected (37 clean, only 9 inexpressible):
  the blocker there is the MAVLink link and a flight-controller peer, not the
  data model.
- **OccupancyGrid** (flattened, `data: i8[<=4096]`) is **accepted** by
  `drm build` (verified) — maps and occupancy grids are representable; it is the
  *sequences of records* that are not.

## What is already implemented (DRM side)

| Commit | Content | Verification |
|---|---|---|
| `0081d0b` | `AttachedChannel` copy-form publish/read over a segment; `HistoryRing::from_mapped`; REQ-CORE-102/103 | `attached_publish_read_roundtrip` (parent publishes 50 samples, second process reads newest + retained window in order) |
| `088af6b` | C ABI attached group (`REQ-CORE-107`), append-only; header + opaque `drm_attached` | `attached_abi_publish_then_read` |

Still open on the DRM side: G1, G2, G4, typed codegen over the attached path,
zero-copy borrow across the ABI, attached hazard registration, required-ack
floors.

## The two decisions this report forces

1. **Bridge + provisioning (unblocks ≈ 6 packages).** Sidecar bridge over the new
   C ABI plus the missing deployment→runtime provisioning (G4), then prove it end
   to end: `Twist` (clean) → `ros2 topic echo`; bounded `LaserScan` → `rviz2`
   headless. Scope: ROS-adapter work, no change to DRM's core.
2. **Sequence support in DRM's grammar (unblocks 322 inexpressible interfaces).**
   A canonical bounded sequence of records and of strings — the single root cause
   under 226 + 96 interfaces, and the only path to tf2, rviz2, Nav2, MoveIt 2,
   ros2_control and the schema-mirroring servers. Scope: DRM core (canon codec,
   layout model, zero-allocation invariants), ADR-level.

Doing (1) alone leaves the most-used packages (tf2, rviz2, Nav2) unusable; (2)
without (1) removes the type obstacle but still transports nothing.

## Evidence index

- Static audit over 896 interfaces (`/opt/ros/jazzy/share`):
  200 clean / 373 needs-bounds / 323 cannot-map; break-down 226 array-of-message,
  96 array-of-string, 1 recursive.
- Compiler verdicts (`drm build`, this host):
  - accepted: `geometry_msgs/Twist` (6×`f64`); flattened `nav_msgs/Odometry`;
    bounded `sensor_msgs/LaserScan` (`ranges: f32[<=2048]`); flattened
    `nav_msgs/OccupancyGrid` (`data: i8[<=4096]`); flattened single transform —
    each `4-12 generated, N verified`.
  - rejected: `tf2_msgs/TFMessage` (`TransformStamped[]`) 2 defects;
    `sensor_msgs/JointState` (`name: string[]`) 6 defects; naive
    `nav_msgs/Path` 4 defects; naive `sensor_msgs/PointCloud2` 6 defects.
- Cross-process probe (pre-fix): a second process attached to a publisher's
  segment and read its bytes at `payload_offset = 440` (mapping 696 B) through
  the public API, with no publish/read operation available.
- Code references: `crates/drm-core/src/channel/mod.rs:101-152,291,554`;
  `channel/lifecycle.rs:210-246,260-271,405-420,487-500`;
  `crates/drm-ffi/src/lib.rs` (endpoint group, `drm_context_open`);
  `bindings/c/include/drm/drm.h`; `xtask/src/cross_os.rs:521-523`;
  `xtask/src/gencheck.rs:24-36`; `crates/drm-workspace/src/codegen/mod.rs:25-28,351-368`;
  `xtask/src/perf.rs` (synthetic expectation).
