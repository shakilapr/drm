# Spatial Types & Clock Domains — Design Confirmation (M14W3)

Non-normative derived spec. Source of truth: [spatial_architecture.md](../spatial_architecture.md)
§Distinct Spatial Semantic Types, §Compile-Time Fixed Spaces vs Dynamic Spaces, §Temporal
Precision and Hardware Clocks, §Authoring Spaces, §Node Spatial Contracts;
[spatial-core.md](../algorithms/spatial/spatial-core.md) §Data Structures/§Functions;
[workspace-model.md](../algorithms/toolchain/workspace-model.md) §spatial (I-D6/I-D7);
[schema-parser.md](../algorithms/toolchain/schema-parser.md) §Grammar. On any conflict the
architecture docs win; on any pseudocode conflict the algorithm docs win (codebase.md §2.4).
Owner: E2. Phase 33 prep. Written M14W3 (confirms the M14W2 grammar-extension draft).

Confirms the Phase 33 (DRM Spatial) type and clock design against the normative algorithm
docs. This is a design-confirmation artifact: it introduces no requirement beyond the
architecture and records the deltas/decisions the week must pin before implementation
(M19W1).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-SPATIAL-typed-quantities` | Point3/Direction3/Vector3/Pose3/Transform3/Twist3 distinct | `REQ-SPATIAL-036` |
| `REQ-SPATIAL-compile-spaces` | `space: camera/front` ⇒ `Point3<CameraFront>`; payload values only | `REQ-SPATIAL-005` |
| `REQ-SPATIAL-dynamic-spaces` | `SpatialPoint3` carries numeric `SpatialId`; `Stamped<T>` composition | `REQ-SPATIAL-009` |
| `REQ-SPATIAL-spatial-contracts` | `[spatial] write/read` in drm.toml; single-writer per dynamic edge | `REQ-SPATIAL-030` |
| `REQ-SPATIAL-static-dynamic-inference` | write ⇒ dynamic; transform + no writer ⇒ static | `REQ-SPATIAL-031` |
| `REQ-SPATIAL-two-timestamps` | sample time + receive time on every transform sample | `REQ-SPATIAL-035` |
| `REQ-SPATIAL-clock-domains` | four domains; affine mappings; unresolved ⇒ status | `REQ-SPATIAL-004` |
| `REQ-SPATIAL-clock-affine` | affine mapping with reference pair, rate ratio, bounded uncertainty | `REQ-SPATIAL-041` |
| `REQ-SPATIAL-clock-reset-epoch` | clock identity (kind, source, epoch); reset bumps epoch | `REQ-SPATIAL-051` |
| `REQ-SPATIAL-static-relationship` | static offset relative to parent; dynamic declares parent only | `REQ-SPATIAL-048` |
| `REQ-SPATIAL-path-identity` | identity derives from filesystem path | `REQ-SPATIAL-047` |
| `REQ-SPATIAL-mcu-constants` | MCU receives collapsed static constants at build | `REQ-SPATIAL-017` |

## 1. Typed quantities (confirmed)

Six distinct semantic types; the compiler rejects naked `f64` triplets crossing a transform
(I-S1/I-S8, `REQ-SPATIAL-036`). Each transformation applies a different algebra:

| Type | Meaning | Transform semantics |
|---|---|---|
| `Point3` | location in a space | rotation + translation |
| `Direction3` | unit vector (orientation) | rotation only |
| `Vector3` | displacement | rotation only |
| `Pose3` | `Point3` + `UnitQuat` | `Point3`/rotation pair |
| `Transform3` | `SE(3)` rigid transform | `SE(3)` compose; quat renormalized |
| `Twist3` | linear + angular velocity | `SE(3)` exponential-map integration |

No delta: matches [spatial-core.md](../algorithms/spatial/spatial-core.md) §Data Structures
exactly. Wire representation is raw numerics (`f64` triples, quat `[f64; 4]` X,Y,Z,W);
scale/shear remain excluded (rigid-only, `REQ-SPATIAL-038`).

## 2. Compile-time vs dynamic space binding (confirmed)

- **Compile-time fixed space**: a `.drm` field annotated `space:` binds the frame into the
  generated type (`Point3<CameraFront>`). The wire payload carries **raw numerics only** —
  no frame strings or IDs (I-S2, `REQ-SPATIAL-005`).
- **Dynamic space**: heterogeneous streams use `SpatialPoint3 { id: SpatialId, p: Point3 }`.
  `SpatialId` is a persistent object identity (track id), never a slot index; ingress binds
  `SpatialId → SpatialHandle { index, generation }` so delayed samples never alias a recycled
  slot (`REQ-SPATIAL-009`).
- **Timed measurement**: `Stamped<T> { value, sample: TimePoint, receive: TimePoint }`
  composes with either binding, e.g. `Stamped<Point3<CameraFront>>`
  (`REQ-SPATIAL-035`).

No delta: matches §Data Structures.

## 3. Grammar extension EBNF draft (M14W2 draft, confirmed)

Field-type grammar extension over [schema-parser.md](../algorithms/toolchain/schema-parser.md)
§Grammar. Topic attributes keep the `@` sigil; spatial binding uses the existing annotation
surface so no ordinary identifier becomes reserved.

```ebnf
field-type      = scalar-type | spatial-type | bounded-type | enum-ref ;
spatial-type    = "point3" | "direction3" | "vector3" | "pose3" | "transform3" | "twist3" ;
spatial-annot   = "space" ":" frame-path ;
spatial-id-annot= "spatial_id" ":" "true" ;        (* opt-in dynamic object identity *)
frame-path      = segment { "/" segment } ;
segment         = lower-alpha { lower-alpha | digit | "_" } ;
```

Restrictions (parser diagnostics reuse the closed vocabulary in
[schema-parser.md](../algorithms/toolchain/schema-parser.md) §Diagnostics Vocabulary):

- `space:` is legal only on `point3`/`direction3`/`vector3`/`pose3`/`transform3`/`twist3`.
- `space:` and `spatial_id: true` are mutually exclusive on one field (static vs dynamic).
- `frame-path` follows path identity: lowercase ASCII segments, `/`-separated, identity is
  the filesystem path (REQ-SPATIAL-047).
- A field with neither `space:` nor `spatial_id` is a naked quantity and is rejected by the
  `V-SPAT` naked-coordinate check (`REQ-SPATIAL-030`).

No grammar-level default is invented: the dynamic/static classification is a model-build
inference (§5), not a syntax flag.

## 4. `[spatial]` contract parsing (confirmed)

Nodes declare reads/writes in `drm.toml`; the model parser fills
`NodeEntry.spatial: SpatialContract` ([workspace-model.md](../algorithms/toolchain/workspace-model.md)
§spatial):

```toml
[spatial]
write = ["robot/base"]
read  = ["robot/camera/front", "robot/lidar"]
```

- `write`/`read` are sorted `FrameId` sets; each entry is a path-identity frame.
- Per-`read` history requirements (`history`, `max_samples`, `min_sample_period`) attach to
  the read edge (`HistoryRequirement`, REQ-SPATIAL-037); `V-SPAT` enforces
  `max_samples >= floor(history / min_sample_period) + 2`.
- Model build enforces referential integrity only; the single-writer-per-dynamic-edge check
  is `drm verify` (`V-SPAT`), not model build (I-D12/I-D7).

No delta: the parsing approach matches §spatial.

## 5. Static/dynamic inference (confirmed, I-D6/I-S10)

Deterministic classification over the declared graph — no extra flags:

| Declared | Classification | Result |
|---|---|---|
| node `[spatial] write` names the frame | dynamic | `Dynamic { writer }` |
| `transform` given, no writer | static | `Static { transform }` |
| write **and** transform on one frame | conflict | model error |
| neither write nor transform | dangling frame | model error |

`REQ-SPATIAL-031`, [workspace-model.md](../algorithms/toolchain/workspace-model.md) I-D6.
Two nodes writing one dynamic edge ⇒ conflict before deployment (I-D7, REQ-SPATIAL-030).
Static relationships declare nominal offset relative to `parent:`; dynamic spaces declare
only `parent:` (REQ-SPATIAL-048). Contiguous static chains collapse to one constant at build
(REQ-SPATIAL-049), and MCU targets receive that constant (REQ-SPATIAL-017).

## 6. Clock domains (confirmed)

Four domains, exact set — no invented variants (REQ-SPATIAL-004):

| Domain | Source | Notes |
|---|---|---|
| `HostMonotonic` | internal OS monotonic time | scheduling/replay anchor |
| `Device(u16)` | hardware timers on cameras, LiDARs, MCUs | per-device timer; id in the variant |
| `Global(GlobalSource)` | `Ptp` or `Gnss` synchronized clocks | `GlobalSource { Ptp, Gnss }` |
| `SimReplay` | simulation/replay virtual clocks | may pause, slow, replay |

A timestamp is `TimePoint { value_ns: u64, clock: ClockIdentity }`; a clock identity is
`ClockIdentity { domain: ClockDomain, epoch: u32 }` (REQ-SPATIAL-051). Every transform sample
stores both a **sample** and a **receive** `TimePoint` (REQ-SPATIAL-035).

Mapping is affine, resolved per full `ClockIdentity` (domain + epoch), never per domain alone:

```text
t_to = to_ref_ns + rate_ratio * (t_from - from_ref_ns)
```

`ClockMapping` carries the reference instant pair, `rate_ratio`, `uncertainty_at_ref_ns`,
bounded `uncertainty_growth`, the validity interval, and a `generation`
(REQ-SPATIAL-041). No valid mapping ⇒ `CLOCK_UNRESOLVED` status; timestamps from different
domains are never silently compared (I-S7).

No delta: matches [spatial-core.md](../algorithms/spatial/spatial-core.md) §Temporal
Precision and §Functions `map_timestamp`.

## 7. PTP/GNSS and SimReplay sources; degradation paths (confirmed)

Dev hardware may lack a PTP/GNSS source, so the design pins the source and its degradation
for each domain. Degradation is **explicit status**, never a silent assumption (I-S7).

| Domain | Primary source | Degradation path | Result when unresolved |
|---|---|---|---|
| `Global(Ptp)` | PTP grandmaster on the LAN | no PTP grandmaster ⇒ fall back to `HostMonotonic` mapping once measured; if no measured mapping exists, leave unresolved | `CLOCK_UNRESOLVED` |
| `Global(Gnss)` | GNSS receiver fix (UTC-disciplined) | no fix / antenna loss ⇒ hold-over on the receiver's free-running clock with growing `uncertainty_growth`; beyond the declared hold-over bound, unresolved | `CLOCK_UNRESOLVED` |
| `SimReplay` | simulator/replay scheduler virtual clock | clock pause/slow/replay is legal; a mapping is only valid within its `valid_from_ns..valid_until_ns` interval and for the current epoch | `CLOCK_UNRESOLVED` |
| `Device(u16)` | device hardware timer | device reset / re-time bumps the identity epoch ⇒ prior mapping invalid until re-estimated | `CLOCK_UNRESOLVED` |
| `HostMonotonic` | OS monotonic clock | never degrades; epoch only bumps on process restart | n/a |

Ambiguity in clock semantics is routed through the ambiguity-report process
([docs/req/ambiguity-01.md](../req/ambiguity-01.md)) rather than guessed at implementation.

## 8. Confirmation status (M14W3)

- [x] Typed quantities confirmed against §Data Structures (E2).
- [x] Compile-time vs dynamic binding confirmed (raw-numerics payload).
- [x] Grammar-extension EBNF draft confirmed (M14W2 draft).
- [x] `[spatial]` contract parsing approach confirmed.
- [x] Static/dynamic inference rule (I-D6/I-S10) confirmed.
- [x] Four clock domains + affine mapping + identity epoch confirmed.
- [x] PTP/GNSS + SimReplay sources identified; degradation paths documented.

Deltas: none — the algorithm docs already state these; this spec records the confirmation.
Open items carried to M19W1: generated-type spelling per binding language, and the exact
`frame-path` diagnostic vocabulary additions.

REQ: REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-017, REQ-SPATIAL-030,
REQ-SPATIAL-031, REQ-SPATIAL-035, REQ-SPATIAL-036, REQ-SPATIAL-041, REQ-SPATIAL-047,
REQ-SPATIAL-048, REQ-SPATIAL-049, REQ-SPATIAL-051, REQ-SPATIAL-004, REQ-SPATIAL-038.
