# DRM Spatial Module Architecture

DRM Spatial is an integrated spatial-temporal module within DRM (`drm-spatial`). It replaces dynamic transform trees like tf2 with a compiled spatial graph, versioned shared-memory caches, explicit clock domains, and consumer-driven history.

In robotics, coordinate transforms change over time, belong to different hardware clock domains, experience sensor latencies, and occasionally jump or reparent. DRM Spatial models both spatial geometry and temporal behavior as a high-performance library on top of the DRM data fabric.

## Core Principle

Inside a compute domain, spatial relationships are declared once, static structures are compiled away at build time, and runtime memory only stores changing transforms and the exact history requested. Across physical links, only required dynamic transforms are transmitted.

A coordinate never exists by itself. A spatial value always has a coordinate space, either known at compile time or carried at runtime.

## The Five Golden Rules of DRM Spatial

1. Every spatial quantity has a coordinate space; naked Cartesian coordinates never exist in isolation.
2. Spaces form transform-connected graphs (a forest), not one mandatory global world.
3. Points, directions, poses, and transforms are distinct mathematical types.
4. Coordinate conventions and geodetic coordinate reference systems are explicitly converted at boundaries.
5. Every coordinate conversion is strictly evaluated by value, source space, target space, and time.

## Rigid Transforms Only

DRM Spatial operates exclusively on 3D rigid transforms:
- Translation: 3-dimensional vector (X, Y, Z).
- Rotation: Unit quaternion (X, Y, Z, W).
- Scale and shear are excluded, eliminating matrix decomposition and non-rigid distortion overhead.

## Distinct Spatial Semantic Types

To eliminate mathematical programming errors, DRM distinguishes between different geometric quantities:
- **Point3**: A location in space. Transforming a point applies both rotation and translation.
- **Direction3**: A unit vector representing orientation. Transforming a direction applies rotation only; translations are ignored because positions cannot translate directions.
- **Vector3**: A displacement vector with magnitude and direction. Transforming applies rotation only.
- **Pose3**: A combined position and orientation (`Point3` + `Rotation`).
- **Transform3**: A coordinate frame transformation ($SE(3)$).
- **Twist3**: Linear and angular velocity.

## Compile-Time Fixed Spaces vs Dynamic Spaces

When a topic's coordinate space is fixed by design, DRM binds the space into the type definition:

```yaml
position:
  type: point3
  space: camera/front
```

The compiler exposes `Point3<CameraFront>`. The message payload on the wire contains only the raw numerical values without frame strings or IDs, saving memory and bandwidth.

For heterogeneous streams where objects originate from different spaces (such as tracked objects from multiple sensors), DRM provides dynamic spatial types (`SpatialPoint3`) that carry a numeric `SpatialID` — a persistent object identity, not a slot index — alongside the coordinates. Timed measurements are composed using `Stamped<T>` (such as `Stamped<Point3<CameraFront>>`).

## A Forest of Spaces (No Mandatory Universal World)

DRM does not require a single universal scene origin. Spatial graphs form a forest of independent trees:
- Local robot tree: `earth` $\rightarrow$ `map` $\rightarrow$ `odom` $\rightarrow$ `robot/base` $\rightarrow$ sensors.
- Simulation tree: `simulation/world` $\rightarrow$ simulated robots.

Disconnected graphs can coexist. Querying between disconnected trees returns a path-not-found status unless an explicit bridge transform is registered. Multi-robot systems coordinate by resolving transforms through a common ancestor frame or through explicit inter-robot spatial links.

## Canonical Conventions and Geodetic Boundaries

DRM enforces one canonical internal standard across all nodes:
- Right-handed coordinate frame.
- Metric SI units (meters, radians).
- Quaternion rotations.
- Standard robotics axes: +X Forward, +Y Left, +Z Up.

External frameworks (Unreal left-handed, Unity, OpenCV optical frames, NED) declare explicit axis conversions at boundaries.

Global geographic coordinates (WGS84, ECEF, UTM) are treated as geodetic coordinate reference systems, not rigid kinematic frames:
$$\text{WGS84} \xrightarrow{\text{geodetic}} \text{ECEF} \xrightarrow{\text{tangent-plane}} \text{Map/ENU} \xrightarrow{\text{rigid } SE(3)} \text{robot/base}$$

This preserves numerical precision: robots operate locally around small metric offsets ($x = 4.2\text{ m}$) rather than carrying million-meter Earth coordinates that degrade floating-point accuracy.

## Temporal Precision and Hardware Clocks

Every transform sample explicitly records two distinct timestamps:
- **Sample Time**: The exact moment in time when the physical pose was true.
- **Receive Time**: The moment DRM received the data on the local computer.

A timestamp in DRM carries a clock identity (`TimePoint { value, clock: ClockIdentity }` — domain plus epoch). Clock domains distinguish:
- **Host Monotonic Clock**: Internal operating system time.
- **Device Hardware Clocks**: Hardware timers on cameras, LiDARs, and microcontrollers.
- **Global Time**: PTP or GNSS synchronized clocks.
- **Simulation and Replay Clocks**: Virtual clocks that can pause, slow down, or replay recorded datasets.

DRM maintains affine clock mappings: $t_{to} = t_{to,ref} + r \cdot (t_{from} - t_{from,ref})$, with rate ratio $r$, a reference instant pair, uncertainty at the reference, and bounded uncertainty growth. Every clock has an identity (`kind`, `source`, `epoch`); a device reset or re-time bumps the epoch so an old mapping can never apply to a new clock incarnation. Timestamps from different clocks are never silently compared; if no valid clock mapping exists, queries return an unresolved clock status.

## Dual Hierarchy: Compiled Base and Runtime Overlay

DRM splits spatial frames into two distinct layers:
1. **Compiled Base**: Robot bodies, sensor racks, and joint chains declared in `spaces/`. Their static offsets are precomputed at build time for straight-line fast-path execution; queries over the base never traverse the graph at runtime.
2. **Runtime Overlay**: Dynamic world objects, detected obstacles, human tracking markers, and temporary tools created while the robot runs (`space.create("object/42")`). Runtime frames are allocated with generational handles (`SpatialHandle { index, generation }`) so that deleting an object invalidates old references immediately without recycling bugs. Overlay frames resolve through a **bounded overlay walk** (declared `max_overlay_depth`) from the overlay frame up to its compiled-base attachment frame, followed by the precompiled base plan; the resulting small prefix is cached under a `topology_generation`, and a reparent bumps the generation and re-walks once. Topology events are retained in a bounded history scoped to the retrievable transform-history horizon.

## Runtime Spatial Index

Graph queries resolve *transforms between frames*. Runtime overlay objects introduce a second question: *which tracked objects lie within a radius or region?* DRM Spatial answers proximity queries through two indexes that mirror the dual hierarchy:

- **Sparse Hash Grid (Dynamic Objects)**: Runtime overlay objects are indexed in a uniform grid anchored to a declared frame (`map` or `odom`). Cell size is declared statically, never tuned at runtime.
- **Compiled Static BVH (Static Environment)**: Static environment frames with extents (walls, shelves, keep-out zones) compile into an axis-aligned bounding-volume hierarchy at build time.

Octrees, k-d trees, and sweep-and-prune are rejected: octrees add split/merge machinery for density the grid already absorbs; k-d trees serve static point sets only; sweep-and-prune serves collision broad-phase engines, not frame graphs.

### Hash Grid for Dynamic Objects

Cells are addressed by integer coordinates hashed into a fixed-capacity open-addressing table. Steady-state indexing and queries perform zero heap allocation; all capacity is declared before deployment:

```yaml
# packages/perception/spaces/index.drm
index:
  anchor: map
  cell_size: 0.5        # meters
  max_objects: 4096
  max_cells_per_object: 27
  load_factor: 0.70     # physical cell-hash slots = ceil(M / load_factor)
  max_query_cells: 512  # enumeration bound before object-table fallback
```

The generated membership pool has checked capacity `max_objects × max_cells_per_object`; verification rejects a slack/envelope bound that can touch more cells. Cells are hashed into a fixed open-addressing table sized $\lceil M/\alpha_{max} \rceil$ with $\alpha_{max} \le 0.70$, keeping probe bounds within the real-time budget at worst-case occupancy; deletion uses backward shift, so no tombstones accumulate. Indexed objects retain generational handles, so deletion invalidates stale references.

Re-bucketing uses a declared motion slack — a fat cell envelope per object. Its center remains fixed until exit; moving it without updating memberships would create false negatives. Updates are **incremental**, serialized by a single non-blocking updater: only cells whose membership actually changes (`remove` ∪ `add`; the intersection is kept) are touched — $O(k)$ in the object's cell span, never a whole-index copy. Each object record carries its occupied cells and the dependency versions of every transform input feeding its anchor-space pose; a change to any dependency invalidates the object's index location even if its own edge did not move. A global grid epoch (even = stable, odd = updating) makes multi-cell moves transactional: queries accept candidates only when the epoch is unchanged across the scan; bounded retry exhaustion returns `CONCURRENT_UPDATE`, never a partial candidate set.

### Compiled Static BVH

Static environment geometry compiles into a fixed-layout bounding-volume hierarchy at build time. Each recalibratable subtree has two generated fixed-capacity banks; recalibration builds the inactive bank, then atomically selects it. Oversize geometry is rejected without disturbing the active bank.

### Ownership and Update Discipline

> **The spatial index is a derived cache, not authoritative state. Authoritative object state remains the versioned dynamic edges; each consumer indexes only its declared spatial reads.**

A node builds indexes exactly over the spaces it declares in `[spatial] read`. Updates ride existing version counters: the index tracks the **full dependency vector** of each object's anchor-space pose — its own edge, all ancestor edges on the conversion path, calibration slots, and the topology generation. A change to any dependency refreshes the affected buckets; an unchanged dependency vector touches nothing. No polling, no cross-node index protocol, no central registry.

### Proximity Query Semantics

Queries declare a frame and typed geometry; naked coordinates never enter the index API:

```cpp
auto hits = space.nearby("map", wrist_pt, 2.0);   // radius query
auto hits = space.region("map", region_aabb);     // region query
```

Results return candidate object handles with status metadata. The candidate set is a deliberate superset; callers perform exact distance checks on typed points through the standard conversion path. The prospective cell count is computed with checked integer arithmetic **before** enumeration; above a declared `max_query_cells` the query falls back to scanning the fixed object table, bounding worst-case work independently of radius. Candidate storage overflow returns an explicit `OutputTooSmall{required}` status — misses and silent truncation are forbidden. Proximity evaluates only the latest indexed state; per-candidate historical state resolves afterward through `at()`. `drm verify` costs the membership pool, cell table, object records, query scratch sets, BVH nodes, and both calibration banks.

## Authoring Spaces (`spaces/`)

Spatial identities are defined directly by their filesystem path. For example, `packages/robot/spaces/camera/front.drm` defines the frame `robot/camera/front`.

A static relationship defines its nominal geometric offset relative to its parent:

```yaml
parent: robot/base
translation: [0.35, 0.0, 0.72]
rotation: [0.0, 0.0, 0.0, 1.0]
```

A dynamic space simply declares its parent frame:

```yaml
parent: world
```

Static and dynamic states are inferred automatically without extra flags: if a node declares that it writes to a space, that space is dynamic; if no writer exists and a transform is given, it is static.

## Calibration Without Recompilation

When sensors are recalibrated, application code does not need to be recompiled. DRM decomposes static transforms into:
$$\text{Effective Transform} = \text{Nominal Static Transform} \times \text{Calibration Correction}$$

The compiler embeds the nominal geometry and links it to a shared calibration slot with an effective timestamp. Updating a camera's calibration invalidates dependent caches live without restarting nodes or rebuilding binaries.

## Node Spatial Contracts

Nodes declare their spatial reads and writes in `drm.toml`:

```toml
[node]
language = "cpp"
publish = ["localization/status"]
consume = ["imu/data"]

[spatial]
write = ["robot/base"]
read = ["robot/camera/front", "robot/lidar"]
```

DRM enforces a single-writer rule per dynamic edge by default. If multiple nodes attempt to write to `robot/base`, the compiler reports a conflict before deployment.

## Build-Time Static Collapse

Traditional transform trees repeatedly multiply long chains of static coordinate frames at runtime. DRM collapses contiguous chains of static transforms at build time into a single static constant:

- Authoring chain: `world` $\rightarrow$ `base` (dynamic) $\rightarrow$ `sensor_rack` (static) $\rightarrow$ `camera_mount` (static) $\rightarrow$ `camera` (static).
- Compiler computation: DRM multiplies the three static offsets into a single constant: `base_to_camera = rack * mount * camera`.
- Runtime query: Calculating camera world position requires only one runtime multiplication: `world_to_camera = world_to_base * base_to_camera`.

## Versioned Local Cache

Authoritative runtime state consists only of dynamic edges. Each dynamic edge in shared memory has a numeric ID, transform, sample and receive timestamps, and version counter.

Each process maintains an internal cache for its declared spatial reads. When a node queries `camera.world()`, it verifies whether the dependency versions of `world_to_base` have changed:
- If the versions match, the cached transform **geometry** is returned immediately without recomputation; freshness, staleness, and clock uncertainty are always derived per query from timestamps — version equality never implies `VALID`.
- If any version has changed, DRM executes the precompiled transform plan, updates the cache, and records the new versions.

## Query Semantics and History

DRM provides explicit query operations:
- **`latest(target, reference)`**: Returns the transform at the newest timestamp for which every time-varying input on the selected path can be evaluated coherently: with input intervals $I_i = [t_{oldest,i}, t_{newest,i}]$ converted to one clock, $t^* = \max(\bigcap_i I_i) = \min_i(t_{newest,i})$. Every dynamic edge is then evaluated at that same $t^*$ — the composed transform corresponds to a single physical instant, which the result reports as its sample time; if the input intervals have no common intersection, the query returns `OUT_OF_HISTORY`.
- **`at(target, reference, timestamp)`**: Returns the state valid at the specified historical time. Translations are interpolated linearly and rotations via spherical linear interpolation (SLERP) between bracketing samples in the sample-time-ordered history.
- **`predict(target, reference, max_horizon)`**: Explicit forward extrapolation from the newest coherent state. Extrapolation integrates a declared body-frame twist on $SE(3)$ via the exponential map, $T(t+\Delta) = T(t)\operatorname{Exp}(\xi_b \Delta)$; twist frame conventions and adjoint-based conversion between frames are explicit. Historical queries (`at`) never silently extrapolate beyond the newest sample.
- **`delayed_view(offset)`**: An intentionally delayed view trailing real-time by a fixed duration for smooth rendering and remote monitoring displays; resolves `at(now − offset)`.

These operations resolve state between named frames. Proximity queries (`nearby`, `region`) form the second query family, executing over the Runtime Spatial Index and returning candidate object handles; per-candidate state is then resolved through the operations above.

History ring buffers are allocated only on upstream dynamic edges and live calibration slots needed by declared historical reads. `history = H` is a **guaranteed sample-time queryability window** behind the newest accepted sample; `max_samples = N` is the fixed memory capacity; and a declared `min_sample_period = P` is the structural producer-density contract — two accepted samples are never closer than P in sample time (a closer sample is rejected and counted). Verification requires $N \ge \lfloor H/P \rfloor + 2$ (one predecessor retained below the window floor for boundary interpolation) and rejects declarations that cannot satisfy it; history is never silently shortened. Rings are ordered by sample time: in-window late arrivals insert in order, and anything older than the retained floor is rejected and counted (`LateSampleDropped`). `expected_rate` is planning metadata and never sizes correctness-critical history. Static constants are never repeated across timestamps. `at()` selects the time-scoped hierarchy plan and resolves every dynamic/calibration input at the requested timestamp before composition.

## Discontinuities and Teleports

When a localization algorithm relocalizes or resets, the transform jumps discontinuously. DRM samples can be flagged as discontinuous. DRM never interpolates across a discontinuity boundary; queries spanning the jump surface a discontinuity flag with no cross-jump value rather than computing an invalid intermediate path.

## Transactional Reparenting

When an object is picked up or moved, its parent frame changes (`space.reparent(object, gripper, Preserve::World)`). DRM supports:
- **Preserve World**: Keeps global position constant and adjusts the local transform.
- **Preserve Local**: Keeps the local offset constant and updates global position.
- **Snap**: Snaps the object directly to the new parent's origin.

Reparenting is temporal: changing a parent at time $t_1$ does not alter history before $t_1$. Queries for earlier timestamps traverse the original parent hierarchy. Hierarchy mutations are executed atomically so readers never observe partially updated trees.

Discontinuities are marked on the local edges whose values jump, not on the object as a whole. `Preserve World` computes $T_{newParent,obj} = T_{anchor,newParent}^{-1} \cdot T_{anchor,obj}$ at $t_1$, so the object's world trajectory remains continuous across the reparent; only local edge values change.

## Transform Status and Quality

Queries return a result object containing the calculated transform along with explicit status metadata:
- Hard outcome: a transform, or one `error` — `PATH_NOT_FOUND`, `CLOCK_UNRESOLVED`, `OUT_OF_HISTORY`, `CONCURRENT_UPDATE`.
- Combinable quality flags: `STALE`, `EXTRAPOLATED`, `DISCONTINUITY_NEARBY`.
- Sample time, receive age, and clock synchronization uncertainty — derived per query, never cached.

## Shared Memory and Microcontrollers

- **Host System**: Dynamic edge slots are stored in shared memory with lockless sequence counters, allowing Python, Rust, C++, and TypeScript nodes to read transforms simultaneously without mutex contention. Each slot stores the transform plus both its sample and receive timestamps; the sample clock identity (domain and epoch) is tracked per edge in channel metadata — a device reset bumps the epoch rather than rewriting per-sample bytes.
- **Physical Boundaries**: Only changing dynamic edges are transmitted across external links.
- **Microcontrollers**: An MCU needing camera coordinates receives the pre-calculated `base_to_camera` constant at build time. When it receives the dynamic `world_to_base` transform over serial or network links, it executes a single straight-line multiplication with zero heap allocation, zero string lookups, and zero tree traversal.

## Application Code Experience

In C++:
```cpp
auto camera_res = space.latest("world", "robot/camera/front");
if (!camera_res.error) {
    auto pose = camera_res.transform;
}

Point3<CameraFront> local_pt(1.0, 0.0, 0.5);
auto world_pt = space.convert(local_pt, "map");

for (auto hit : space.nearby("map", world_pt, 2.0)) {
    auto obj = space.latest(hit.handle, "map");
}
```

In Python:
```python
res = space.latest("world", "robot/camera/front")
if res.ok():
    pos = res.transform.translation

for hit in space.nearby("map", gripper_pt, radius=2.0):
    obj = space.latest(hit.handle, "map")
```

In C (microcontrollers):
```c
struct Transform3 world_camera;
if (drm_spatial_latest_world_camera(&world_camera)) {
    aim_gimbal(world_camera.translation);
}
```
