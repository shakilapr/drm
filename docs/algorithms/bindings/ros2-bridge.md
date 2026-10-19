# ROS 2 Bridge (External Middleware Bridge)

Normative home of the optional ROS 2 bridge process
(`bindings/cpp/ros2-bridge/`). Implements
[architecture.md §External Middleware Bridges](../../architecture.md#external-middleware-bridges):
a third-party middleware is reached by an explicit bridge process, never by
teaching the core its protocol. The bridge is an ordinary DRM node in the
deployment's contract graph and an ordinary peer on the ROS 2 graph.

REQ: REQ-BIND-024, REQ-BIND-025, REQ-BIND-026, REQ-BIND-027.

## Data Structures

```rust
// A bridge's channel set is explicit configuration; nothing is bridged implicitly.
struct Mapping {
  drm_topic: TopicName,      // canonical DRM topic the deployment declares
  ros_topic: RosTopicName,   // absolute ROS topic
  ros_type: RosTypeName,     // e.g. std_msgs/msg/String
  direction: ToDrm | ToRos,  // exactly one per mapping
  qos: Qos,                  // reliability + durability, declared per channel
}

// The DRM side reads its connection data from the generated artifact
// (deployment-identity.md §Data Structures) — slot, contract hashes, capacity,
// history, pool, overflow. The bridge hand-builds no identity.
struct DrmEndpoint { deployment_id, slot, schema_hash, layout_hash,
                     payload_capacity, history_depth, pool_size, overflow }
```

Mapping syntax (command line): `drm:<canonical>=ros:<topic>@<type>[;<flags>]` for
ROS→DRM and `ros:<topic>@<type>[;<flags>]=drm:<canonical>` for DRM→ROS, where
`<flags>` is a `;`-separated list of `transient_local`, `best_effort`,
`reliable`, `volatile`.

## Invariants

- **I-BR1 DeclaredOnly**: a mapping naming a topic the deployment does not
  declare is refused at startup; nothing is created implicitly. Test:
  `bridge_refuses_undeclared_topic`.
- **I-BR2 OneDirection**: a DRM topic mapped twice is refused (a loop must be two
  distinct topics). Test: `bridge_refuses_duplicate_topic`.
- **I-BR3 BoundedPayload**: a foreign message larger than the declared payload
  capacity is dropped with an explicit diagnostic, never truncated. Test:
  `bridge_rejects_oversized_message`.
- **I-BR4 ConsumerWaits**: the DRM side of a ROS→DRM channel attaches as the
  publisher (it creates the segment); the DRM side of a DRM→ROS channel attaches
  as a consumer and waits for the publisher's segment rather than creating one
  (`shm-backend.md` I2). Test: `bridge_waits_for_publisher`.
- **I-BR5 MessageTypesOnly**: a declared type that is not a message (a service or
  action interface) is refused with an explicit diagnostic instead of aborting
  the process. Test: `bridge_refuses_non_message_type`.
- **I-BR6 ExplicitQos**: reliability and durability are declared per channel; a
  latched topic must declare `transient_local` or it will not see the retained
  sample. Test: `bridge_honours_declared_qos`.

## Functions

```rust
fn main(deployment_json, lock_dir, mappings) -> ExitCode {
  let config = DeploymentConfig::load(deployment_json)?;   // strict, as the runtime does
  for m in mappings {
    let endpoint = config.topic(m.drm_topic).ok_or(Refused)?;   // I-BR1
    reject_duplicate(m.drm_topic)?;                             // I-BR2
    match m.direction {
      ToDrm => drm_attached_open(..., PUBLISHER) + create_generic_subscription(qos, publish_into_drm),
      ToRos => drm_attached_open(..., CONSUMER, wait_for_segment) + create_generic_publisher(qos),
    }
  }
  loop { for c in to_ros_channels { read_next → publish serialized }; spin_some(); }
}
```

## Protocols

1. **Startup.** Load the generated deployment config; validate every mapping
   against it; open the DRM endpoints; create the ROS endpoints. Any refusal is a
   non-zero exit with the reason on stderr — never a partial bridge.
2. **ROS→DRM.** The generic subscription receives serialized bytes; the bridge
   checks the declared bound and publishes the bytes as one DRM sample. A publish
   failure is reported and does not stop other channels.
3. **DRM→ROS.** `read_next` copies one sample into the declared capacity; the
   bridge publishes it as a serialized ROS message. `NoSample`/`Retry` are normal
   idling, not errors.
4. **Shutdown.** Every attachment is closed; the segment outlives the bridge if
   the publisher still holds it.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Deployment config absent or malformed | refuse to start (`InvalidData`/`NotFound`) |
| Mapping names an undeclared DRM topic | refuse (`I-BR1`) |
| Same DRM topic mapped twice | refuse (`I-BR2`) |
| Foreign message exceeds the payload bound | dropped with a diagnostic (`I-BR3`) |
| DRM→ROS channel starts before its publisher | consumer waits, bounded (`I-BR4`) |
| Declared type is a service/action interface | refuse with the rclcpp reason (`I-BR5`) |
| Latched ROS topic without `transient_local` | no retained sample; declare the flag (`I-BR6`) |
| One channel's read/publish fails | other channels continue (isolation) |

## Test Matrix

| Case | Expected |
|---|---|
| `bridge_ros_to_drm_round_trip` | a ROS peer's message becomes a DRM sample a DRM consumer reads |
| `bridge_drm_to_ros_round_trip` | a DRM sample is published and a ROS peer's tooling prints it |
| `bridge_refuses_undeclared_topic` | I-BR1 |
| `bridge_refuses_duplicate_topic` | I-BR2 |
| `bridge_rejects_oversized_message` | I-BR3 |
| `bridge_waits_for_publisher` | I-BR4 |
| `bridge_refuses_non_message_type` | I-BR5 |
| `bridge_honours_declared_qos` | I-BR6: `/tf_static` arrives only with `transient_local` |
