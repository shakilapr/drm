#!/bin/bash
# Does the bridge's subscription receive anything from these nodes?
set +u
cd /root/projects/drm || exit 1
source /opt/ros/jazzy/setup.bash >/dev/null 2>&1
export ROS_DOMAIN_ID=49 DRM_BRIDGE_DEBUG=1
export LD_LIBRARY_PATH="/root/projects/drm/target/release:${LD_LIBRARY_PATH:-}"
DEP=tests/ros2_bridge/.drm/deploy/deployment.json
ID=$(python3 -c "import json;print(json.load(open('$DEP'))['deployment_id'])")
rm -f /dev/shm/drm."$ID".* 2>/dev/null
mkdir -p /tmp/drm-bridge-locks; rm -f /tmp/drm-bridge-locks/* 2>/dev/null

./bindings/cpp/ros2-bridge/build/drm_bridge_ros2 "$DEP" /tmp/drm-bridge-locks \
  'drm:bridge/from_ros=ros:/cmd_vel@geometry_msgs/msg/Twist' --duration 12 > /tmp/dbg_bridge.log 2>&1 &
BR=$!
sleep 2
echo "--- publisher-side check: ros2 topic pub baseline ---"
timeout 4 ros2 topic pub -r 5 /cmd_vel geometry_msgs/msg/Twist "{linear: {x: 0.1}}" > /dev/null 2>&1
sleep 1
echo "--- teleop through a pty ---"
( printf 'w'; sleep 3 ) | timeout 8 script -qec "ros2 run teleop_twist_keyboard teleop_twist_keyboard" /dev/null > /tmp/dbg_tele.log 2>&1
sleep 1
wait $BR 2>/dev/null
echo "--- bridge log ---"; cat /tmp/dbg_bridge.log
