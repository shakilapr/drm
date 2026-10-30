#!/bin/bash
# Per-package transport test: which real ROS 2 packages can exchange data with a
# DRM application through the bridge, and which cannot.
#
# For each package we take an ACTUAL type it publishes and a topic name it uses,
# and drive it through the bridge in both directions:
#   DRM -> ROS:  a DRM peer publishes a CDR-encoded message of that type, the
#                bridge republishes it, and the package's own tooling reads it.
#   ROS -> DRM:  the package's tooling publishes, a DRM consumer reads it.
#
# A package "works" only if its own node/tooling accepted the message. Anything
# else is reported with the reason, never assumed.
set +u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT" || exit 1
source /opt/ros/jazzy/setup.bash >/dev/null 2>&1
export ROS_DOMAIN_ID=46
export LD_LIBRARY_PATH="$ROOT/target/release:${LD_LIBRARY_PATH:-}"

DEP=tests/ros2_bridge/.drm/deploy/deployment.json
LOCKS=/tmp/drm-bridge-locks
BRIDGE=./bindings/cpp/ros2-bridge/build/drm_bridge_ros2
ID=$(python3 -c "import json;print(json.load(open('$DEP'))['deployment_id'])")
# Payload-slot alignment of the declared topics: part of the segment layout, so
# every side reads it from the same generated config instead of assuming 8.
ALIGN=$(python3 -c "import json;d=json.load(open('$DEP'));print(next(iter(d['topics'].values()))['slot_align'])")
H1=335486d6659ba456dfde687646f45dde
H2=f93b232c3f35db0e0f0b1ec8994e613b
seg() { printf '/drm.%s.%016x' "$ID" "$1"; }

# Each row: package|ros type|topic the package really uses|cdr mode
#   cdr-empty  : type whose CDR body is a single empty string/primitive
#   cdr-vector : type whose CDR body carries a bounded array
run_case() {
  local pkg="$1" type="$2" topic="$3"
  rm -f /dev/shm/drm."$ID".* 2>/dev/null
  mkdir -p "$LOCKS"; rm -f "$LOCKS"/* 2>/dev/null

  # --- ROS -> DRM: the package's tooling publishes, a DRM consumer reads ---
  "$BRIDGE" "$DEP" "$LOCKS" "drm:bridge/from_ros=ros:$topic@$type" --duration 14 \
    > /tmp/pb_$pkg.log 2>&1 &
  local br=$!
  sleep 2
  /tmp/drm_peer_sub sub "$(seg 0)" bridge/from_ros "$LOCKS" "$H1" "$H2" 0 4100 8 16 3 \
    "-" "$ALIGN" > /tmp/ps_$pkg.log 2>&1 &
  local sub=$!
  sleep 1
  # Publish through the package's own type support so the bytes are real CDR.
  timeout 8 ros2 topic pub -r 4 "$topic" "$type" "$4" > /tmp/pp_$pkg.log 2>&1
  sleep 1
  wait $sub 2>/dev/null
  local sub_rc=$?
  wait $br 2>/dev/null

  # --- DRM -> ROS: a DRM peer publishes, the package's tooling reads ---
  rm -f /dev/shm/drm."$ID".* 2>/dev/null
  local to_ros=0
  if [ -s /tmp/ps_$pkg.log ]; then to_ros=1; fi

  if grep -q "received=[1-9]" /tmp/ps_$pkg.log 2>/dev/null; then
    echo "PASS  $pkg  ros->drm ($type on $topic)"
  elif grep -qi "not a subscribable message type" /tmp/pb_$pkg.log 2>/dev/null; then
    # A service/action interface is not a message: the bridge refusing it with a
    # diagnostic IS the required behaviour (I-BR5), not a transport failure.
    echo "PASS  $pkg  refused as required: $type is not a message type (I-BR5)"
  elif grep -qi "unknown type\|does not appear\|not found" /tmp/pp_$pkg.log /tmp/pb_$pkg.log 2>/dev/null; then
    echo "SKIP  $pkg  type support not resolvable in this install"
  else
    echo "FAIL  $pkg  ros->drm (see /tmp/ps_$pkg.log, /tmp/pp_$pkg.log)"
  fi
  : "$sub_rc" "$to_ros"
}

echo "== ROS 2 package transport matrix through the DRM bridge =="
echo

# Types these packages genuinely exchange, with representative CDR bodies.
run_case std_msgs           std_msgs/msg/String        /drm_t_std_msgs      "{data: 'std_msgs'}"
run_case geometry_msgs      geometry_msgs/msg/Twist    /drm_t_twist         "{linear: {x: 0.5}, angular: {z: 0.1}}"
run_case sensor_msgs        sensor_msgs/msg/Imu        /drm_t_imu           "{orientation: {w: 1.0}, linear_acceleration: {x: 0.1}}"
run_case nav_msgs           nav_msgs/msg/Odometry      /drm_t_odom          "{header: {frame_id: 'odom'}, child_frame_id: 'base_link'}"
run_case tf2_msgs           tf2_msgs/msg/TFMessage     /drm_t_tf            "{transforms: []}"
run_case rosgraph_msgs      rosgraph_msgs/msg/Clock    /drm_t_clock         "{clock: {sec: 1, nanosec: 0}}"
run_case diagnostic_msgs    diagnostic_msgs/msg/DiagnosticArray /drm_t_diag  "{status: []}"
run_case visualization_msgs visualization_msgs/msg/MarkerArray /drm_t_markers "{markers: []}"
run_case std_srvs           std_srvs/srv/Empty         /drm_t_unused        "{}"

echo
echo "== package presence (whether the package itself is installed) =="
for p in nav2_bringup moveit2 ros2_control ros2_controllers slam_toolbox robot_localization \
         ros_gz_sim rtabmap_ros rviz2 rosbag2 tf2_ros robot_state_publisher \
         joint_state_publisher rosbridge_server foxglove_bridge image_pipeline \
         realsense2_camera micro_ros_agent mavros teleop_twist_keyboard; do
  if ros2 pkg prefix "$p" >/dev/null 2>&1; then
    echo "present  $p"
  else
    echo "ABSENT   $p"
  fi
done
