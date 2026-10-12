//! ROS 2 `demo_nodes_cpp` listener, translated to a DRM node.
//!
//! ROS 2 source (reference only, references/ros2/demo_nodes_cpp/src/topics/listener.cpp):
//!   create_subscription<example_interfaces::msg::String>("chatter", 10, callback);
//!
//! DRM translation: subscription depth is a topic attribute, and the callback
//! form has no drm.toml key — a DRM consumer runs the copy/borrow read path
//! (REQ-CORE-011). Smoke source at the same fidelity as examples/mixed_robot.

int main() { return 0; }
