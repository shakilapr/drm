# ROS 2 integration work plan

Dependency-ordered phases for making ROS 2 packages interoperate with DRM.
Companion to [ros2-package-compatibility-on-drm.md](../../ros2-package-compatibility-on-drm.md)
(the audit that motivates the order) and to
[docs/metrics/ros2-compat-matrix.md](../metrics/ros2-compat-matrix.md) (the
earlier source-level study).

Ordering rule: **build correctness and deployment provisioning come before any
bridging or type-system change.** A bridge built on an unverifiable build and
hardcoded identities would encode the defects it is meant to expose. Phases are
prerequisites, not durations; phases 4–8 may overlap once their inputs pass.

Status legend: `done` · `in progress` · `pending`.

## Phase summary

| Phase | Goal | Status |
|---|---|---|
| 0 | Build correctness: change detection + non-vacuous gates + an integration fixture | `done` |
| 1 | Deployment-to-runtime provisioning: two separately built processes connect from generated configuration | `done` |
| 2 | Complete attached-channel bindings and safety (C, C++, Rust, Python) | `pending` |
| 3 | First ROS 2 bridge MVP (generic serialized publish/subscribe) | `done` |
| 4 | Native bounded composite types (records, arrays of records, arrays of strings) | `pending` |
| 5 | Typed ROS ↔ DRM conversion (build-time translator) | `pending` |
| 6 | DRM Spatial ↔ tf2 interoperability | `pending` |
| 7 | Higher-level ROS compatibility (RViz, Nav2, MoveIt 2, ros2_control, SLAM, rosbag2/Foxglove) | `pending` |
| 8 | Hardening and performance | `pending` |

## Phase 0 — Build correctness and verification — `done`

Defects closed: **G1** (renderer identity folded into the codegen cache key —
ADR [0008](../adr/0008-generator-identity-in-the-codegen-cache-key.md), commit
`1673f6c`) and **G2** (the gate could not fail: no committed `.drm/gen` tree
existed anywhere and CI ran it against an empty model — commit `60d6d33`).

Fixture: `tests/integration/primitive` — one primitive topic (`counter/value`,
`u64`), two node declarations, and **two independently built applications**
(`apps/producer`, `apps/consumer`; separate cargo projects, workspace-excluded)
that meet on one shared-memory segment. Driver: `cargo xtask integration --check`.

Acceptance criteria, each verified:

| Criterion | Evidence |
|---|---|
| Changing a renderer invalidates affected artifacts | `gen_generator_identity_invalidates_and_caches` (topic + node units dirtied) |
| An unchanged build is correctly cached | same test: 0 dirty units on re-plan; `integration` prints `unchanged re-plan cached (0 dirty, …)` |
| Empty or missing fixtures cannot go green | `mandatory_fixture_is_never_vacuous`; `integration --check` fails on an absent fixture |
| Fixture builds from a clean checkout | generated tree + both app `target/` dirs deleted, then re-run: `generated 12 unit(s)` … `exchange verified — PRODUCER_OK published=64 … CONSUMER_OK received=64` |
| Existing ABI and schema tests still pass | full workspace suite (recorded with the phase-0 close) |

Empty workspaces remain legitimate: only the **mandatory fixture** must be
nonempty, and its absence fails CI.

## Phase 1 — Deployment-to-runtime provisioning — `done`

Goal: two separately built DRM processes communicate using the deployment DRM
actually generates — no hardcoded test identities. Closes **G4** from the
compatibility report (every existing caller builds its `Expectation` by hand;
`drm build` emits no connection information).

Generation and consumption:

- deployment instance id and platform-specific segment names;
- topic handles, schema hashes, layout hashes;
- node contract fingerprints and endpoint permissions;
- sample maximum, history depth, pool capacity, applicable consumer registrations;
- expected runtime version and attachment-validation data.

Constraint: keep the compiled-contract versus runtime-deployment distinction —
deployment identifiers are **not** compiled into application binaries.

Acceptance criteria, each verified:

| Criterion | Evidence |
|---|---|
| 1. Two separate applications | `tests/integration/primitive` (Phase 0 fixture) and the ROS 2 bridge are independent builds; `deployment_provisions_two_processes` runs the consumer as a separate process of the test binary |
| 2. Publisher and consumer in separate processes | same test: a child process attaches, the parent publishes |
| 3. Both load the same deployment configuration | `crates/drm-cli/tests/deployment_config.rs` (`emitted_config_loads_in_the_runtime`, `loaded_identity_drives_segment_names`); the Phase 1 test derives segment name, hashes, slot, capacity, depth, pool, overflow **and slot alignment** from the artifact alone |
| 4. 1,000 numbered messages, content and sequence | `deployment_provisions_two_processes`: fixture declares `@history: 1024` so the ring retains the run; consumer reports `CHILD_OK read=1000 in_order=yes` |
| 5. Mismatched contracts and unauthorized endpoints rejected | `deployment_refuses_mismatched_contracts` (another node's fingerprint ⇒ `WrongAbi`, undeployed node ⇒ `NotFound`, second live publisher ⇒ refused) |
| 6. Publisher restart, epoch handling | `crates/drm-core/tests/counters.rs` (`restart_run` bumps `publisher_epoch`, counters stay lifetime-cumulative); epoch is a property of a live publisher, not of the artifact |
| 7. Publisher crash, safe recovery | `crates/drm-core/tests/shm_kill.rs` and `tests/stress/chaos_kill_matrix.rs`; recovery is the `Freshness` classification path in `lifecycle::publisher_open` |

Known limit, recorded rather than hidden: criterion 4 needs a topic that
retains, and `examples/mixed_robot` declares `control/target` under the default
`latest` policy (depth 1). The acceptance test therefore owns a fixture with an
explicit `@history` rather than asserting an in-order replay the example's own
policy does not promise.

## Phase 2 — Complete attached-channel bindings and safety — `pending`

| Task | Requirement |
|---|---|
| Typed attached endpoints | generated code uses real shared-memory channels, not process-local stubs |
| Consumer hazards | correct registration, borrow protection, release |
| Required acknowledgements | declared acknowledgement-floor behaviour implemented |
| C ABI | stable handles, error codes, ownership rules |
| C++ and Rust | safe typed copy operations; scoped borrows only where proven safe |
| Python | owned copies, consistent with the architecture |

The attached API stays copy-capable even where zero-copy borrowing exists.
Concurrency safety (reuse races, process crashes, publisher restarts) is this
phase's exit gate; Phase 1's round trip is not evidence of it.

## Phase 3 — First ROS 2 bridge — `done`

Optional `drm-bridge-ros2` package using Jazzy's `create_generic_subscription()` /
`create_generic_publisher()` (serialized-message path). Declared topic mapping,
ROS type identity and serialization-format validation, explicit QoS, capacity
checking, loop prevention. Remains an external optional process; never a broker.

Acceptance, each verified (`tests/ros2_bridge/`):

| Criterion | Evidence |
|---|---|
| Both directions carry real serialized messages | `run_bridge_probe.sh`: DRM peer → bridge → `ros2 topic echo` prints `data: drm-97`; `ros2 topic pub` → bridge → DRM consumer reads `received=5` |
| Declared types from real packages | `package_matrix.sh`: 8 message packages pass (`std_msgs`, `geometry_msgs`, `sensor_msgs`, `nav_msgs`, `tf2_msgs`, `rosgraph_msgs`, `diagnostic_msgs`, `visualization_msgs`) |
| Real package nodes, not synthetic publishers | `real_packages.sh`: `tf2_ros`, `robot_state_publisher`, `teleop_twist_keyboard`, `rosbag2`, `rviz2` all pass |
| Non-message types refused, not aborted | `std_srvs/srv/Empty` refused with `not a subscribable message type` (I-BR5) |
| Oversized / duplicate / undeclared refused | invariants I-BR1..I-BR4 in [ros2-bridge.md](../algorithms/bindings/ros2-bridge.md) |
| Every DRM identity from the generated config | the bridge passes the deployment's `slot_align`; the C peers read the same file |

`LaserScan` is not in the matrix: its CDR body exceeds the fixture's 4100-byte
payload bound, so it is a Phase 4 (bounded composite) case, not a Phase 3 one.

## Phase 4 — Native bounded composite types — `pending`

Two language-independent capabilities: named record types (fixed shape) and
bounded sequences of records and of strings, e.g.

```
types:
  Point:
    x: f32
    y: f32
    z: f32

fields:
  points: Point[<=1024]
  names: string[64][<=32]
```

Grammar specified before implementation. Update schema parser, canonical layout
compiler, schema hashing, generated encoders/decoders, memory-capacity verifier.
Preserve fixed-capacity slots and deterministic handling of unused element
storage. Acceptance: `TFMessage`, `JointState`, `Path`, `JointTrajectory`,
`Marker` equivalents compile and exchange natively; rerun the 896-interface audit
(322 inexpressible interfaces → expressible where a finite bound can be assigned).
Recursive schemas remain separate.

## Phase 5 — Typed ROS conversion — `pending`

Build-time translator from ROS interface definitions to DRM-compatible schemas,
generating field-mapping code (no runtime reflection in `drm-core`). Handles
nested records, bounded sequences, string encoding, numeric conversions, explicit
limits. Acceptance: a native DRM application reads fields of a ROS-originated
`JointState`/`Path` without interpreting CDR; failures return explicit errors.

## Phase 6 — Spatial ↔ tf2 — `pending`

Adapter around the existing DRM Spatial design, not a second transform engine:
DRM → `/tf` and `/tf_static`; permitted ROS frame updates → DRM Spatial;
timestamp meaning and explicit clock mappings preserved; unauthorized writes and
conflicting frame ownership rejected. Depends on the corresponding DRM Spatial
runtime capability; must not block Phase 3's transform-message transport.

## Phase 7 — Higher-level ROS compatibility — `pending`

| Target | Primary requirement |
|---|---|
| RViz | TF, point clouds, markers, images |
| Nav2 | topics, transforms, lifecycle behaviour, services, actions |
| MoveIt 2 | trajectories, robot states, services, actions |
| ros2_control | adapter to controllers/hardware, not message translation alone |
| SLAM and localization | sensor streams, TF, temporal consistency |
| rosbag2 / Foxglove | recording and visualization adapters |

Services and actions need dedicated compatibility logic; topic conversion is not
framework compatibility.

## Phase 8 — Hardening and performance — `pending`

Deterministic memory budgets, allocation behaviour, cross-language layout
agreement, startup mismatches, crash recovery, consumer stalls, history gaps,
publisher restarts. For external links, every declared maximum payload must fit
the available framing and transport constraints (no arbitrary fragmentation).
Measure copying, encoding and transport overhead separately — the ROS boundary is
**not** zero-copy merely because DRM's local path is. Acceptance: no regressions
in the DRM core suite, verified memory bounds, explicit failure handling,
reproducible performance results.
