# ROS 2 compatibility issues

Status: analysis of why ROS 2 packages still fail against DRM after the bridge
work, and what each failure actually is.

Scope: the bridge (`bindings/cpp/ros2-bridge/`), the DRM schema language and
runtime, and the ROS 2 Jazzy install on this host (600 message definitions, 279
services, 41 actions).

Verdict up front: **renaming topics and patching each ROS codebase is necessary
but not sufficient.** Four of the blockers are in DRM's own model, and one of
them cannot be fixed by renaming at any cost. They are listed in
[Part 1](#part-1--blockers-in-drm-not-in-the-renaming) below; the mechanical
renaming work is [Part 2](#part-2--what-renaming-does-fix).

---

## Summary table

| # | Issue | Where | Renaming fixes it? |
|---|---|---|---|
| 1 | Topic paths cannot match: DRM requires `package/stem` lowercase, always 2 segments; ROS allows `/a/b/c` and uppercase | DRM schema + discovery | **No** — structural |
| 2 | No record type, no array-of-record, no array-of-strings | DRM schema grammar | **No** — 28.5 % of messages are inexpressible as *typed* topics at any bound (transport still works via opaque bytes) |
| 3 | No services or actions in the C ABI (279 + 41 unreachable) | DRM C ABI | **No** — missing capability |
| 4 | Payload bound is static and per-topic; ROS messages are unbounded | DRM model vs ROS | **No** — needs a bound chosen per topic, and the bound is a real memory cost |
| 5 | QoS: a RELIABLE subscriber silently receives nothing from a BEST_EFFORT publisher, which is the default for every sensor driver | bridge default | No — one flag per channel |
| 6 | tf2 cannot be replaced: `/tf` is federated (N publishers), DRM is single-writer per topic; and DRM Spatial is Rust-only, not in the C ABI | DRM runtime + ABI | No |
| 7 | A DRM application cannot decode a bridged payload — no CDR codec anywhere in the repo | DRM core | No — Phase 5 translator |
| 8 | `drm build` never runs the full verifier, so `V-ID`'s topic-canonicality rule (and all of V-MEM/V-RT/V-RPC) are unreachable | toolchain | n/a — bug |
| 9 | Every invalid `.drm` file also reports a spurious `empty-file`; the model error prints only a *count* of defects | toolchain | n/a — **fixed** in this change |

Measured, not estimated — see [Evidence](#evidence-how-each-number-was-produced).

---

## Why the existing test suite passes anyway

`tests/ros2_bridge/package_matrix.sh` reports 8 PASS + 1 correct refusal, which
reads as "ROS 2 packages are compatible". It is not evidence of that:

- **The payloads are empty.** The rows send `{transforms: []}`, `{status: []}`,
  `{markers: []}` — an `TFMessage`/`DiagnosticArray`/`MarkerArray` with **no
  elements**. An empty array is 4 bytes of CDR, so every band-D type (issue 2)
  crosses successfully as a byte blob. The row proves the *typesupport library
  resolves and an empty body fits the bound*, nothing about the data a real node
  emits.
- **The direction is not tested.** The function has a
  `# --- DRM -> ROS ---` section that deletes the segment, computes
  `to_ros=1` and then never runs a DRM→ROS exchange; the verdict branch only ever
  looks at `/tmp/ps_$pkg.log` (the ROS→DRM direction). Verified by reading the
  function: `local to_ros=0; if [ -s /tmp/ps_$pkg.log ]; then to_ros=1; fi` and
  then `: "$sub_rc" "$to_ros"`. `run_bridge_probe.sh` does cover DRM→ROS, but
  with a synthetic `std_msgs/String` peer, not a package.
- **The QoS is RELIABLE on both sides** (`ros2 topic pub` default, bridge
  default), so issue 5's BEST_EFFORT black hole never appears.
- **The bound is 4100 and the messages are small**, so issue 4 never appears.

Consequently the matrix should be read as a smoke test, not a compatibility
claim; the four gaps above are all outside what it exercises. The honest
replacement is to drive each row with a *representative instance* (a LaserScan
with ≥720 beams, a TFMessage with ≥1 transform, a MarkerArray with ≥1 marker) and
to assert both directions — at which point the failing rows become the work list
for Part 2.

---

## Part 1 — Blockers in DRM, not in the renaming

### 1. Topic identity is structurally different (cannot be reconciled)

DRM derives a topic's identity from its **path on disk**:

```
packages/<package>/topics/<stem>.drm   ->   topic "<package>/<stem>"
```

(`crates/drm-cli/src/ws.rs` `discover`). That yields **exactly two segments,
lowercase**, and `docs/algorithms/toolchain/schema-parser.md` pins it:
`[a-z0-9_]+` segments joined by `/` (`REQ-TOOL-056`), because Windows is
case-insensitive and Linux is not.

ROS 2 names are `/` -rooted, allow uppercase, and have arbitrary depth
(`/robot1/sensors/lidar`). Measured on this host:

| Name | ROS 2 | DRM |
|---|---|---|
| `/tf` | valid (1 segment) | impossible (needs `pkg/stem` → only `tf/tf`) |
| `/Cmd_Vel` | valid | must be lowercased |
| `/robot1/sensors/lidar` | valid | impossible (3 segments) |

Verified directly: a workspace with `packages/tf/topics/tf.drm` produces the
identity `tf/tf`, never `tf`. And a nested directory
(`packages/robot/topics/nested/deep/lidar.drm`) is **silently ignored** — the
build reports `0 generated`, exit code 0, no warning.

So "change their topics to match ours" means every ROS topic must be remapped to
a *different* two-segment lowercase name and every consumer taught the new name.
The bridge mapping (`drm:<canonical>=ros:<topic>@<type>`) already carries that
translation, so the rename buys nothing the mapping did not already do — and it
cannot be done at all for a name that is not already `pkg/stem` shaped.

### 2. The schema language cannot express most ROS messages

`.drm` accepts only
`prim | string[N] | bytes[<=N] | prim[N] | prim[<=N] | enum{...}`
(`docs/spec/schema-grammar.md`). There is **no record type**, and the parser
rejects every construct ROS is built from — confirmed by feeding each one to
`drm_workspace::parser::parse_text`:

| ROS construct | Parser verdict |
|---|---|
| `Transform[<=16]` (array of record — `TFMessage.transforms`) | `unknown-type` |
| `string[64][<=32]` (array of strings — `JointState.name`) | `array-depth-exceeded` |
| `pose:` block (nested record — `TransformStamped.transform`) | `unknown-type` |
| `name: string` (unbounded — `Odometry.header.frame_id`) | `unbounded-string` |
| `v: f32[]` (unbounded array — `LaserScan.ranges`) | `unbounded-array` |

Classified across all **600 installed message definitions**:

| Band | Meaning | Count | Share |
|---|---|---|---|
| A | clean — 1:1 expressible | 88 | 14.7 % |
| B | flattenable — nested records only | 158 | 26.3 % |
| C | needs-bounds — unbounded string/array only | 183 | 30.5 % |
| D | **impossible** — array-of-record or array-of-strings | 171 | **28.5 %** |

Band D is the hard one: no `.drm` file expresses it at *any* choice of bounds.
`TFMessage`, `JointState`, `PointCloud2`, `Path` and `MarkerArray` are all band D.
`LaserScan`, `Image`, `Odometry` and `CameraInfo` are band C.

**Scope of this blocker — it is not a transport blocker.** The bridge moves data
as opaque CDR bytes (`data: u8[<=N]`), so a band-D message still crosses:
measured, a `TFMessage` with 1, 10, 30 and 50 real transforms is delivered intact.
What band D blocks is the *typed* path — a DRM application declaring the topic
natively and reading its fields (issue 7) — which is what "change their topics to
match ours" is trying to achieve. For pure transport only the bound (issue 4) and
QoS (issue 5) matter.

### 3. Services and actions have no path at all

`drm.h` exports 18 symbols; none is a service, action or RPC call:

```
drm_abi_version          drm_attached_open_aligned   drm_borrow_release
drm_attached_close       drm_attached_publish        drm_consumer_close
drm_attached_open        drm_attached_read_latest    drm_consumer_open
drm_attached_read_next   drm_borrow_latest           drm_context_close
drm_context_open         drm_publish_copy            drm_publisher_close
drm_publisher_open       drm_read_latest             drm_read_next
```

This host has **279 `.srv`** and **41 `.action`** definitions. `drm-rpc` exists
(2 730 lines, `crates/drm-rpc/`) but is **not exposed in `drm-ffi` and not in
`drm.h`**, so no C, C++ or Python process can reach it. Every service-based
package — `nav2` (`ClearCostmap*`), `moveit2`, `ros2_control`, `tf2`'s own
`FrameGraph` — is therefore out of reach regardless of message compatibility.

`std_srvs/srv/Empty` in the package matrix is refused with
`not a subscribable message type`, which is the correct behaviour (I-BR5), but it
also means **the whole service surface is refused**, not just that one type.

### 4. The payload bound is static; ROS messages are unbounded

A DRM topic declares `payload_capacity` at build time and the payload pool is
`pool_size` slots of that stride, allocated once. Verified behaviour: publishing
an 8 696-byte `LaserScan` through the bridge at the fixture's declared
`payload_capacity = 4100` gives

```
bridge: bridge/from_ros: message of 8696 bytes exceeds the declared bound 4100; dropped
```

and the DRM consumer receives **0** samples. Nothing truncates — the drop is
correct and honest (I-BR3) — but it means a real sensor message is simply not
delivered until the bound is raised.

Realistic sizes against a 4 096-byte bound (CDR body, computed from the actual
`.msg` files):

| Instance | CDR bytes | vs 4 096 |
|---|---|---|
| `LaserScan` 360 ranges | 2 944 | fits |
| `LaserScan` 720 ranges | 5 824 | ×1.4 |
| `LaserScan` 1080 ranges | 8 704 | ×2.1 |
| `MarkerArray` 10 markers | 11 364 | ×2.8 |
| `Path` 50 poses | 19 632 | ×4.8 |
| `Image` 640×480 rgb8 | 921 657 | ×225 |
| `PointCloud2` 64k points | 1 048 725 | ×256 |
| `Image` 1920×1080 rgb8 | 6 220 857 | ×1 519 |

Raising the bound is possible (a 1 MiB bound builds cleanly and yields
`payload_capacity: 1048580`), but the cost is static memory, because the pool is
preallocated:

| Topic bound | stride | slots | segment |
|---|---|---|---|
| `LaserScan` 1080 (8.7 KB) | 8 704 | 16 | 0.1 MB |
| `MarkerArray` 100 (111 KB) | 113 604 | 16 | 1.8 MB |
| `Image` 640×480 (900 KB) | 921 660 | 16 | 14.7 MB |
| `PointCloud2` 64k (1 MB) | 1 048 728 | 16 | 16.8 MB |
| `Image` 1080p (5.9 MB) | 6 220 860 | 16 | 99.5 MB |

That is the real trade: DRM's zero-copy fixed-slot model versus ROS's
"allocate whatever the sensor produced". It is a legitimate design difference,
but it means bridging an image topic costs ~100 MB of pinned shared memory per
topic, and it must be declared up front.

### 5. QoS: RELIABLE silently receives nothing from BEST_EFFORT

Every ROS sensor driver publishes with the `sensor_data` profile —
**BEST_EFFORT**. The bridge defaults to RELIABLE. DDS treats that pair as
incompatible and delivers nothing, with no error at the application level.
Measured through the bridge:

| bridge QoS | publisher QoS | DRM receives |
|---|---|---|
| default (RELIABLE) | BEST_EFFORT | **0** |
| `;best_effort` | BEST_EFFORT | 3 |
| default (RELIABLE) | RELIABLE | 3 |

The bridge does log it, but only as an rclcpp warning:

```
[WARN] [drm_bridge_ros2]: New publisher discovered on topic '/qc2', offering
incompatible QoS. No messages will be sent to it.
Last incompatible policy: RELIABILITY_QOS_POLICY
```

So the first thing a real `LaserScan`/`Image`/`Imu` driver hits is a silent
black hole. The `;best_effort` flag exists and works — it just is not the
default, and nothing in the failure path says "add `;best_effort`".

### 6. tf2 cannot be replaced by DRM as it stands

Two independent problems:

**a. `/tf` is federated; DRM is single-writer.** In ROS, every node broadcasts
its own transforms on `/tf` — measured: 3 concurrent publishers on one ROS topic
is normal. In DRM, two publishers on one topic is a build error:

```
drm: V-SW-2: topic `x/t` has multiple publishers [a, b] (REQ-TOOL-064)
```

The bridge *does* absorb this correctly (it is the single DRM writer and
aggregates N ROS publishers — verified: 2 ROS publishers on `/tf` → DRM
consumer received 6 samples). Transport survives; what does not is a DRM *node*
broadcasting into `/tf` itself.

**b. There is no tf2 equivalent exposed.** `drm-spatial` *does* have the
query surface (`TransformTree::latest(target, reference)`,
`crates/drm-spatial/src/query.rs:721`) — that is the right architecture. But:

- it is **not in the C ABI** (`drm-ffi` does not depend on `drm-spatial`), so a
  bridge process cannot use it;
- there is **no adapter** from `tf2_msgs/TFMessage` to DRM Spatial edges
  (grep for `TFMessage`/`TransformStamped` in `crates/`: zero hits);
- `TFMessage` itself is band D (array-of-record), so it cannot even be a typed
  DRM topic.

Net: tf2 works today only as an opaque byte pipe, and only up to the payload
bound. Measured through the bridge at the fixture's `payload_capacity = 4100`:

| transforms in one `TFMessage` | delivered |
|---|---|
| 10 | yes |
| 30 | yes |
| 50 | yes |
| **52** | **dropped** |
| 60, 100 | dropped |

So a robot whose `robot_state_publisher` emits ~50 frames in one `/tf` message
is right at the edge, and anything larger (humanoids, multi-sensor rigs, a
namespaced fleet) is silently dropped. The CDR body is `4 + 80xN` bytes, so the
bound admits ~51 transforms.

Any DRM consumer that wants "where is `base_link` in `map`" must also parse CDR
and rebuild the tree itself. This is Phase 6 in
`docs/spec/ros2-integration-plan.md` and it is genuinely not built.

### 7. A DRM application cannot read a bridged payload

The bridged topic is `data: u8[<=4096]`. Grepping the whole repo, the only CDR
code is in a test fixture (`tests/ros2_bridge/drm_peer_pub.c`). `drm-core` has
`canon` (its own codec) and nothing else. So a DRM consumer receives bytes it
cannot interpret: it must reimplement ROS CDR per type, which is exactly the
work the bridge was meant to remove.

This is Phase 5 (build-time translator) and it is not started.

### 8. `drm build` never runs the full verifier (bug)

`docs/algorithms/toolchain/schema-parser.md` says an uppercase path segment is a
workspace-model builder error (`name-invalid`, `REQ-TOOL-056`). It is not:

```
$ drm --workspace <ws with packages/Robot/topics/CmdVel.drm> build
build: 8 generated (8 rendered), 1 verified, 1 built [p]
$ python3 -c "import json; print(list(json.load(open('.drm/deploy/deployment.json'))['topics']))"
['Robot/CmdVel']
```

The check exists (`verify::engine::is_canonical_topic`, `engine.rs:939`) but the
full verifier is only ever called from **tests** — `grep -rn "verify::verify"
crates/ xtask/` finds no caller. `drm build` runs only `verify_lite`, which
implements exactly two checks (`MultiplePublishers`, `NoPublisher`). So `V-ID`,
`V-MEM`, `V-RT`, `V-SPAT` and `V-RPC` are unreachable from any command: the
policy documents a gate that no build enforces.

Consequence: cross-platform identity is not actually protected, and a deployment
that would fail `V-MEM` (issue 4's memory blow-up) builds silently.

### 9. Diagnostics lose the reason (bug) — **fixed**

An invalid file reported a *count* of defects instead of the defects, and when
the parse yielded no fields it also reported one defect too many.

```
$ printf 't: Transform[<=16]\n' > bad.drm && drm build      # before
drm: workspace model invalid: topic `x/a`: 2 defect(s)

$ drm build                                                  # after
drm: workspace model invalid: topic `x/a`: 1 defect(s)
  1:4: error: unknown-type
```

Two problems, both closed:
- **The count was inflated when the AST came out empty.** `parse_text`
  unconditionally appended `EmptyFile` when no attribute and no field parsed
  (`parser/mod.rs:296-306`). One real defect (`unknown-type`) plus that cascading
  `empty-file` made 2. It now only reports `empty-file` when the parse itself
  found nothing, so a source that declares nothing still gets it (verified) and a
  source with a real defect does not.
  Measured per case before the fix: `t: Transform[<=16]` → 2,
  `names: string[64][<=32]` → 2, `name: string` → 2 (all inflated by the
  cascade); after: 1 each.
- **The reasons were dropped.** `ModelError::Parse`'s `Display` wrote
  `"{n} defect(s)"` and discarded the rendered diagnostics (`model/mod.rs:223`).
  It now appends every `<line>:<col>: error: <reason>`.

Pinned by `crates/drm-workspace/tests/schema_diagnostics.rs`; both assertions
were confirmed to fail when either half of the fix is reverted.

Net (before the fix): the user saw a number, never the reason. That was the
biggest obstacle to *diagnosing* compatibility — the rest of this document had to
be assembled by calling the parser directly.

---

## Part 2 — What renaming does fix

Doing the renaming work is still required for the parts that are merely
mismatched, and it does clear these:

| Item | Fix |
|---|---|
| Uppercase / deep ROS names | map to `pkg/stem` lowercase; accept that names change |
| QoS black hole (issue 5) | declare `;best_effort` on sensor channels |
| Payload bound (issue 4) | pick a bound per topic; accept the memory |
| Band B messages (26.3 %) | flatten nested records into dotted primitive fields |
| Band C messages (30.5 %) | choose a concrete bound for each string/array |
| Type identity | pin the ROS type per mapping and assert it, so a type change is a startup error rather than silent garbage |

Band A (14.7 %) needs nothing. Band B + C together are 56.8 % and are exactly
"mechanical but semantic" work: the compiler cannot choose a bound, and a
flattened `position.x`/`position.y` rename is a decision, not a derivation.

Bands A+B+C = 86.8 % of messages become expressible as **typed** DRM topics.
Band D (28.5 %) never does.

---

## Part 3 — What to do, in order

Ordered by "unblocks the most with the least architectural change".

1. **Fix the diagnostics first (issue 9).** Trivial, and every other item
   becomes visible instead of guessed at. *Done:* `empty-file` is no longer
   appended when the parse already reported a defect, and `ModelError::Parse`
   renders each `<line>:<col>: error: <reason>`
   (`crates/drm-workspace/tests/schema_diagnostics.rs`). Every failure in this
   document is now readable from `drm build` output alone.
2. **Make `drm build` run the verifier (issue 8).** The checks are written; no
   command calls them. Without this, issues 4 and 1 are unenforceable.
3. **Default sensor channels to BEST_EFFORT in the bridge (issue 5)**, or fail
   startup when the declared QoS is incompatible with the publisher's — a silent
   empty receive is the worst outcome.
4. **Expose `drm-spatial` and `drm-rpc` in the C ABI (issues 3, 6b).** Both
   exist and are tested; neither is reachable from a non-Rust process. This is
   the difference between "DRM has a tf2 equivalent" and "DRM can bridge tf2".
5. **Phase 5 translator (issue 7)** — the only way a DRM app reads a bridged
   payload without hand-writing CDR.
6. **Only then** the renaming pass (Part 2), driven per package with the bands
   as the work list.

Issues 1, 2 and 3 are the architectural ones the brief asked about: the topic
model, the type system, and the missing RPC surface. Renaming does not touch any
of them.

---

## Evidence: how each number was produced

Everything below is reproducible from the repo on this host.

- **Message classification** (600 / 88 / 158 / 183 / 171): parsed every
  `/opt/ros/jazzy/share/*/msg/*.msg` with the same field-shape rules the `.drm`
  grammar enforces. Field-level occurrences across all 600 files:
  `nested-record` 734, `unbounded-string` 252, `array-of-record` 235,
  `unbounded-array` 164, `array-of-strings` 54, `composite-prim (time/duration)`
  counted as flattenable.
- **Parser verdicts per construct**: `drm_workspace::parser::parse_text` +
  `validate` called directly on one-line sources; reasons quoted are the
  `Reason` values (`unknown-type`, `array-depth-exceeded`, `unbounded-string`,
  `unbounded-array`).
- **Payload bound drop**: `LaserScan` with 2 160 ranges published over
  `/sz_scan`; bridge at `payload_capacity = 4100` logged
  `message of 8696 bytes exceeds the declared bound 4100; dropped`; DRM consumer
  `received=0`.
- **QoS matrix**: three runs at `ROS_DOMAIN_ID=111`, publisher
  `ros2 topic pub --qos-reliability {best_effort,reliable}`, bridge with and
  without `;best_effort`; DRM consumer `received` counts 0 / 3 / 3; the
  `RELIABILITY_QOS_POLICY` warning captured from the bridge's own log.
- **`/tf` transform threshold**: `TFMessage` with N synthetic transforms
  published over `/tfn` at `payload_capacity = 4100`; N ∈ {10, 30, 50, 52, 60,
  100}; delivered for N ≤ 50, dropped for N ≥ 52 (bridge logs
  `exceeds the declared bound`).
- **Existing suite's blind spots**: read `package_matrix.sh`'s `run_case`. The
  payloads are one-element nested records (`{linear: {x: 0.5}}`) and *empty*
  arrays (`{transforms: []}`, `{status: []}`, `{markers: []}`) — so no band-D
  message is ever exercised with content. The `# --- DRM -> ROS ---` block
  computes `to_ros` and then only asserts on `/tmp/ps_$pkg.log`, i.e. the ROS→DRM
  leg. The suite reported 9/9 green in the same run in which the QoS and bound
  probes above failed.
- **Topic naming**: `packages/tf/topics/tf.drm` → `tf/tf`;
  `packages/Robot/topics/CmdVel.drm` → accepted as `Robot/CmdVel`;
  `packages/robot/topics/nested/deep/lidar.drm` → silently ignored
  (`0 generated`, exit 0).
- **Multi-publisher**: three concurrent `ros2 topic pub` on `/tf` →
  `Publisher count: 3`; two DRM nodes publishing one topic →
  `V-SW-2: topic x/t has multiple publishers`.
  Bridge absorbing 2 ROS publishers → DRM consumer `received=6`.
- **RPC surface**: `nm -D target/release/libdrm_ffi.so | grep " T "` — the 18
  symbols listed above; `grep -rn rpc crates/drm-ffi/` empty.
- **Spatial surface**: `query.rs:721 latest(target, reference)`;
  `grep -rn spatial crates/drm-ffi/` empty; `grep -rln "TFMessage" crates/` empty.
- **Verifier reachability**: `grep -rn "verify::verify" crates/ xtask/` → no
  callers outside `verify/engine.rs` itself and two test files.
- **Realistic CDR sizes**: computed from the `.msg` definitions with
  `align_up(4)` per element and the standard string encoding; instance
  parameters stated in the table (e.g. `LaserScan` 1080 beams, `Image` 1920×1080
  rgb8).
- **Memory cost**: `stride = align_up(capacity, 4)`, `slots = pool_size` (16, the
  bridge fixture's value) — the product is the segment size a deployment pins.

Retracted, corrected during this analysis: `tests/ros2_compat/README.md` says the
round-trip harness "currently fails to compile" because generated code derives
`Default` over `[u8; N]` arrays. It compiles and passes now
(`ROS2_ROUNDTRIP pass=12 fail=0`) — the README is stale.
