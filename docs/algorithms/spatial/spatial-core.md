# Spatial Core Algorithms

Scope: `drm-spatial` foundations — distinct semantic types, compile-time fixed spaces vs dynamic spaces, `[spatial]` node contracts, static/dynamic inference, forests of spaces, canonical conventions and geodetic boundaries, temporal precision (sample/receive time), the four clock domains, runtime overlay frame creation, MCU constant codegen. Implements [spatial_architecture.md](../../spatial_architecture.md) §Five Golden Rules, §Rigid Transforms, §Distinct Types, §Compile-Time Spaces, §Forest, §Conventions, §Temporal Precision, §Dual Hierarchy, §Authoring Spaces, §Node Spatial Contracts, §Shared Memory and Microcontrollers. Owner: E2 (compiler/grammar, `crates/drm-spatial`) + E1 (runtime caches). Phases 33. Depends on [canon-codec.md](../canon-codec.md), [channel-metadata.md](../runtime/channel-metadata.md), [workspace-model.md](../toolchain/workspace-model.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-typed-quantities` | Point3/Direction3/Vector3/Pose3/Transform3/Twist3 distinct | `REQ-SPATIAL-036` |
| `REQ-SPATIAL-compile-spaces` | `space: camera/front` ⇒ `Point3<CameraFront>`; payload carries values only | `REQ-SPATIAL-005` |
| `REQ-SPATIAL-dynamic-spaces` | `SpatialPoint3` carries numeric `SpatialID` — persistent object identity, not a slot index; `Stamped<T>` composition | `REQ-SPATIAL-009` |
| `REQ-SPATIAL-forest` | forests, disconnected trees, path-not-found status | `REQ-SPATIAL-010` |
| `REQ-SPATIAL-conventions` | right-handed, SI, +X fwd/+Y left/+Z up; explicit boundary conversions | `REQ-SPATIAL-007` |
| `REQ-SPATIAL-geodetic` | WGS84→ECEF→tangent-plane→ENU→SE(3) chain | `REQ-SPATIAL-012` |
| `REQ-SPATIAL-two-timestamps` | sample time + receive time on every transform sample, stored in the slot; sample clock domain fixed per edge in channel metadata | `REQ-SPATIAL-035` |
| `REQ-SPATIAL-clock-domains` | four domains (host monotonic, device hardware, global PTP/GNSS, simulation/replay); affine mappings (reference pair, rate ratio, bounded uncertainty growth) keyed by clock identity with reset epoch; unresolved ⇒ status, never silent compare | `REQ-SPATIAL-004` |
| `REQ-SPATIAL-dual-hierarchy` | compiled base (`spaces/`) + runtime overlay (`space.create`) with `SpatialHandle{index,generation}`; overlay resolves via bounded overlay walk (`max_overlay_depth`) cached under `topology_generation` | `REQ-SPATIAL-008` |
| `REQ-SPATIAL-spatial-contracts` | `[spatial] write/read` in drm.toml; single-writer per dynamic edge enforced pre-deployment | `REQ-SPATIAL-030` |
| `REQ-SPATIAL-static-dynamic-inference` | node declares write ⇒ dynamic; transform given + no writer ⇒ static | `REQ-SPATIAL-031` |
| `REQ-SPATIAL-time-history` | rings allocated only on upstream dynamic edges with declared requirements; ordered by sample time; in-window late arrivals insert in order | `REQ-SPATIAL-033` |
| `REQ-SPATIAL-history-bound` | `history = H` guaranteed sample-time window; `max_samples = N` fixed capacity; `min_sample_period = P` producer-density contract; verify `N ≥ ⌊H/P⌋ + 2`; `expected_rate` never defines retention | `REQ-SPATIAL-037` |
| `REQ-SPATIAL-mcu-constants` | MCU receives pre-calculated static constants at build; straight-line multiply at runtime | `REQ-SPATIAL-017` |

## Data Structures

```rust
pub struct Point3 { x: f64, y: f64, z: f64 } // meters; transform applies rotation + translation
pub struct Direction3(Unit3); // unit vector: rotation-only semantics
pub struct Vector3 { x: f64, y: f64, z: f64 } // displacement: rotation-only
pub struct Pose3 { position: Point3, rotation: UnitQuat }
pub struct Transform3 { translation: [f64; 3], rotation: [f64; 4] } // SE(3); unit quat (X,Y,Z,W)
pub struct Twist3 { linear: Vector3, angular: Vector3 }

// Compile-time space binding (codegen): type Point3CameraFront = Point3 tagged CameraFront;
// wire payload = raw numerics ONLY — no frame strings or IDs.
// Dynamic heterogeneous streams: struct SpatialPoint3 { id: SpatialId, p: Point3 }
// SpatialId = persistent object identity (track id), NOT a slot index; ingestion binds
// SpatialId → SpatialHandle{index, generation}, so delayed samples never alias a recycled slot.
pub struct SpatialId(u32);

// Timed measurements: Stamped<Point3<CameraFront>> etc.
pub struct Stamped<T> { value: T, sample: TimePoint, receive: TimePoint }

// §Temporal Precision — TimePoint with clock domain id
pub struct TimePoint { value_ns: u64, clock_domain: ClockDomain }

// §Temporal Precision — the four architecture clock domains (exact set; no invented variants)
pub enum ClockDomain {
 HostMonotonic, // internal OS time
 Device(u16), // hardware timers on cameras, LiDARs, MCUs
 Global(GlobalSource), // PTP or GNSS synchronized clocks
 SimReplay, // simulation/replay virtual clocks (pause, slow, replay)
}
pub enum GlobalSource { Ptp, Gnss }

// §Temporal Precision — clock identity: domain + epoch; device reset/re-time bumps epoch,
// so an old mapping can never apply to a new clock incarnation.
pub struct ClockIdentity { domain: ClockDomain, epoch: u32 }

// Affine mapping per (from ClockIdentity, to ClockDomain): t_to = to_ref + r * (t_from - from_ref)
pub struct ClockMapping {
    from_ref_ns: i64, to_ref_ns: i64,        // reference instant pair (drift reference)
    rate_ratio: f64,                          // relative rate r
    uncertainty_at_ref_ns: u64,               // uncertainty at the reference
    uncertainty_growth: f64,                  // bounded growth (fraction of elapsed time)
    valid_from_ns: i64, valid_until_ns: i64,  // mapping validity interval
    generation: u64,
}

// §Dual Hierarchy — runtime overlay handles
pub struct SpatialHandle { index: u32, generation: u32 } // delete ⇒ old references invalidate immediately
```

## The Five Golden Rules (enforcement points)

1. **Every spatial quantity has a space** — type system: `Point3` without a space tag (compile-time or `SpatialId`) does not exist in public APIs (I-S8).
2. **Forest, not global world** — disconnected trees coexist; queries across them ⇒ `PathNotFound` status unless an explicit bridge transform is registered (I-S3).
3. **Distinct mathematical types** — I-S1.
4. **Explicit conversion at boundaries** — I-S4.
5. **Strict evaluation by value, source space, target space, time** — every conversion names all four; queries resolve at the historical sample's timestamp, not current time (I-S9).

## Invariants

- **I-S1 TypeSemantics**: transforming a Point applies rotation+translation; Direction/Vector rotation only — enforced by distinct types (naked f64 triplets never cross a transform). Test: `spatial_type_semantics`.
- **I-S2 CompileSpacePayload**: compile-time space bindings carry zero frame bytes on the wire (canon layout = raw numerics). Test: `spatial_payload_layout`.
- **I-S3 ForestDisconnected**: cross-tree query ⇒ `PathNotFound` status; multi-robot resolves via common ancestor or explicit inter-robot spatial links. Test: `spatial_forest`.
- **I-S4 ConventionBoundaries**: internal values always canonical (RH, SI, +X/+Y/+Z, quats); non-canonical frameworks convert only at declared boundaries. Test: `spatial_convention_boundary` (NED/OpenCV/Unity/Unreal fixtures).
- **I-S5 GeodeticPrecision**: WGS84→ECEF→tangent-plane→ENU chain keeps local ops at small metric offsets; never million-meter ECEF deltas. Test: `spatial_geodetic_precision`.
- **I-S6 TwoTimestamps**: every sample carries `sample` + `receive` `TimePoint`s; geometry uses sample time, staleness uses receive time. Test: `spatial_stamped_fields`.
- **I-S7 UnresolvedRejected**: comparing/mapping timestamps without a valid clock mapping ⇒ `CLOCK_UNRESOLVED` status — never a silent assumption. Test: `spatial_unresolved_clock`.
- **I-S8 NoNakedQuantities**: index/query/public APIs take frame + typed geometry, never bare coordinates. Test: compile-fail/API-audit suite.
- **I-S9 HistoricalEvaluation**: conversions evaluate the graph at the query's historical sample timestamp (never "now"). Test: `spatial_historical_evaluation`.
- **I-S10 InferenceDeterministic**: writer declared ⇒ dynamic; `transform` given + no writer ⇒ static; both ⇒ verify error; neither ⇒ verify error. Test: `spatial_inference`.
- **I-S11 SingleWriterEdges**: multiple nodes writing one dynamic edge ⇒ compiler conflict before deployment. Test: `spatial_single_writer`.
- **I-S12 OverlayGeneration**: `space.create` returns `SpatialHandle`; delete invalidates outstanding handles immediately (generation bump). Test: `spatial_overlay_generation`.
- **I-S13 McuConstant**: MCU artifacts receive collapsed static constants at build; runtime = single straight-line multiply. Test: `spatial_mcu_constant`.

## Functions

```rust
fn transform_point(t: &Transform3, p: Point3) -> Point3 { t.rotation.rotate(p) + t.translation } // I-S1
fn transform_direction(t: &Transform3, d: Direction3) -> Direction3 { t.rotation.rotate(d) }
fn transform_vector(t: &Transform3, v: Vector3) -> Vector3 { t.rotation.rotate(v) }
fn compose(a: &Transform3, b: &Transform3) -> Transform3 { /* SE(3) compose; renormalize quat */ }

// Geodetic boundary chain (declared once per CRS edge in spaces/)
fn geodetic_to_enu(wgs84: Lla, anchor: Lla) -> Enu {
 let ecef = wgs84_to_ecef(wgs84);
 tangent_plane_enu(ecef, anchor) // small-offset local frame (I-S5)
}

// Clock mapping (§Temporal Precision)
fn map_timestamp(t: TimePoint, to: ClockDomain) -> Result<TimePoint, Status> {
 match resolve_mapping(t.clock_domain, to)? { // no valid mapping ⇒ CLOCK_UNRESOLVED (I-S7)
 Some(m) => Ok(TimePoint { value_ns: apply(t.value_ns, m), clock_domain: to }),
 None => Err(ClockUnresolved),
 }
}
// Timestamps from different domains are NEVER silently compared (I-S7).

// §Dual Hierarchy — runtime overlay
fn space_create(name: &str, parent: FrameId) -> SpatialHandle; // generational handle
fn space_delete(handle: SpatialHandle); // generation bump ⇒ old refs dead (I-S12)

// §Node Spatial Contracts (drm.toml)
// [spatial]
// write = ["robot/base"]
// read = ["robot/camera/front", "robot/lidar"]

// MCU codegen (§Shared Memory and Microcontrollers)
fn emit_mcu_constants(plan: &CollapsePlan, target: McuTarget) -> CodeArtifact
// Collapsed static chains (e.g. base_to_camera = rack * mount * camera) emitted as C constants;
// runtime receives only world_to_base over serial/network ⇒ one multiply (I-S13).
```

## Protocols

*Space authoring inference (§Authoring Spaces — deterministic classification):*

1. Parse `spaces/**.drm` — frame identity = filesystem path (`robot/camera/front` from `packages/robot/spaces/camera/front.drm`).
2. If any node's `[spatial] write` declares the frame ⇒ **dynamic**.
3. Else if a `transform` (translation/rotation) is declared ⇒ **static** (nominal offset relative to `parent:`).
4. Both ⇒ conflict error; neither ⇒ conflict error (dangling frame).
5. Single-writer rule enforced over dynamic edges at model level (verify V-SPATIAL family).

*Static collapse (§Build-Time Static Collapse):*

1. Walk each tree; multiply contiguous static runs into one constant (`base_to_camera = rack * mount * camera`).
2. Emit constants into codegen artifacts per language; MCU targets receive C constants (I-S13).
3. Dynamic edges (writer exists) remain runtime cache entries — [spatial-queries.md](spatial-queries.md).

*History allocation (time-based):*

1. For each declared consumer requirement `history = H, max_samples = N, min_sample_period = P` on an upstream dynamic edge, allocate exactly `N` entries. `P` is a correctness contract: two accepted samples are never closer than P in sample time. Verification requires `N >= floor(H / P) + 2` (one predecessor below the window floor for boundary interpolation) and rejects unsatisfiable declarations — history is never silently shortened. `expected_rate` is informational only (sequence-based channel rings remain the transport layer — spatial time-history is a derived cache).
2. Rings are ordered by sample time: in-window late arrivals bounded-insert; samples older than the retained floor are rejected and counted (`LateSampleDropped`).
3. Rings allocated only where needed; static constants never stored across timestamps.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Naked coordinates across transform | compile error (I-S8) |
| Quat normalization drift | renormalize on compose; bound-checked |
| Disconnected tree query | `PathNotFound` status (I-S3) |
| Boundary type mismatch (NED→ENU direct) | explicit conversion required (I-S4) |
| Clock chain unknown domain | `CLOCK_UNRESOLVED` (I-S7) |
| Device clock reset / re-time | identity epoch bump ⇒ old mappings invalid; `CLOCK_UNRESOLVED` until re-estimated (I-S7) |
| Sample older than retained history floor | rejected + `LateSampleDropped` counter (REQ-SPATIAL-037) |
| Non-unit quat input | rejected at decode (canon-codec InvalidData) |
| write+transform both declared | verify conflict (I-S10) |
| Two writers on one dynamic edge | compiler conflict (I-S11) |
| `space.delete` then old-handle query | generation mismatch ⇒ explicit invalid-handle status (I-S12) |
| MCU query needing dynamic chain | receives dynamic edge over serial/network; single multiply (I-S13) |

## Test Matrix

| Case | Expected |
|---|---|
| `spatial_type_semantics` (all types × transforms vs oracle) | I-S1 |
| `spatial_payload_layout` | I-S2 |
| `spatial_forest` (disconnected fixtures) | I-S3 |
| `spatial_convention_boundary` (NED/ENU/OpenCV/Unity) | I-S4 |
| `spatial_geodetic_precision` | I-S5 |
| `spatial_stamped_fields` | I-S6 |
| `spatial_unresolved_clock` | I-S7 |
| API audit / compile-fail | I-S8 |
| `spatial_historical_evaluation` | I-S9 |
| `spatial_inference` matrix | I-S10 |
| `spatial_single_writer` | I-S11 |
| `spatial_overlay_generation` | I-S12 |
| `spatial_mcu_constant` | I-S13 |
