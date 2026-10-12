//! ROS 2 `demo_nodes_cpp` talker, translated to a DRM node.
//!
//! ROS 2 source (reference only, references/ros2/demo_nodes_cpp/src/topics/talker.cpp):
//!   create_publisher<example_interfaces::msg::String>("chatter", QoS(KeepLast{7}));
//!   msg.data = "Hello World: " + std::to_string(count++);
//!
//! DRM translation: the type is fixed by the canonical schema
//! (`demos/chatter.drm`, `data: string[256]`); `KeepLast{7}` has no drm.toml
//! key — depth belongs to the topic (`@history`/`@pool_size`). Smoke source at
//! the same fidelity as examples/mixed_robot.

int main() { return 0; }
