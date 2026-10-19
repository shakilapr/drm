#!/bin/bash
# End-to-end DRM <-> ROS 2 bridge probe (Phase 3).
#
#   direction A (DRM -> ROS): a DRM peer publishes a CDR-serialized
#     std_msgs/String; the bridge reads DRM and republishes on ROS;
#     `ros2 topic echo` (a foreign peer with no DRM knowledge) must print it.
#   direction B (ROS -> DRM): `ros2 topic pub` publishes; the bridge writes into
#     DRM; a DRM peer must read the bytes out of shared memory.
#
# Every connection parameter is read from the generated deployment.json.
set +u
cd /root/projects/drm || exit 1
source /opt/ros/jazzy/setup.bash >/dev/null 2>&1
export ROS_DOMAIN_ID=45
export LD_LIBRARY_PATH="/root/projects/drm/target/release:${LD_LIBRARY_PATH:-}"

DEP=tests/ros2_bridge/.drm/deploy/deployment.json
LOCKS=/tmp/drm-bridge-locks
BRIDGE=./bindings/cpp/ros2-bridge/build/drm_bridge_ros2
ID=$(python3 -c "import json;print(json.load(open('$DEP'))['deployment_id'])")
H1=335486d6659ba456dfde687646f45dde
H2=f93b232c3f35db0e0f0b1ec8994e613b

rm -f /dev/shm/drm."$ID".* 2>/dev/null
mkdir -p "$LOCKS"; rm -f "$LOCKS"/* 2>/dev/null
seg() { printf '/drm.%s.%016x' "$ID" "$1"; }

echo "== deployment $ID =="

echo
echo "== direction A: DRM peer -> bridge -> ROS =="
# Order matters: the DRM publisher creates the segment (consumers never create),
# the bridge attaches and only then advertises the ROS topic, and `ros2 topic
# echo` needs an advertised topic to resolve the type. The publisher therefore
# keeps publishing throughout (400 x 50ms = 20s).
/tmp/drm_peer_pub pub "$(seg 1)" bridge/to_ros "$LOCKS" "$H1" "$H2" 1 4100 8 16 400 \
  > /tmp/peer_pub.log 2>&1 &
PUB=$!
sleep 1
"$BRIDGE" "$DEP" "$LOCKS" 'ros:/drm_probe_out@std_msgs/msg/String=drm:bridge/to_ros' \
  --duration 18 > /tmp/bridge_a.log 2>&1 &
BR=$!
sleep 3
timeout 12 ros2 topic echo /drm_probe_out --once > /tmp/ros_echo.log 2>&1
ECHO_RC=$?
wait $BR 2>/dev/null
wait $PUB 2>/dev/null
PUB_RC=$?
echo "--- ros2 echo (rc=$ECHO_RC) ---"; head -8 /tmp/ros_echo.log
echo "--- bridge ---"; cat /tmp/bridge_a.log
echo "--- drm publisher (rc=$PUB_RC) ---"; cat /tmp/peer_pub.log

echo
echo "== direction B: ROS pub -> bridge -> DRM peer =="
rm -f /dev/shm/drm."$ID".* 2>/dev/null
"$BRIDGE" "$DEP" "$LOCKS" 'drm:bridge/from_ros=ros:/drm_probe_in@std_msgs/msg/String' \
  --duration 20 > /tmp/bridge_b.log 2>&1 &
BR2=$!
sleep 3
/tmp/drm_peer_sub sub "$(seg 0)" bridge/from_ros "$LOCKS" "$H1" "$H2" 0 4100 8 16 5 ROS-TO-DRM \
  > /tmp/peer_sub.log 2>&1 &
SUB=$!
sleep 1
timeout 10 ros2 topic pub -r 5 /drm_probe_in std_msgs/msg/String "{data: 'ROS-TO-DRM'}" \
  > /dev/null 2>&1
sleep 1
wait $SUB 2>/dev/null
SUB_RC=$?
wait $BR2 2>/dev/null
echo "--- drm consumer (rc=$SUB_RC) ---"; cat /tmp/peer_sub.log
echo "--- bridge ---"; cat /tmp/bridge_b.log

echo
echo "== verdict =="
if grep -q "data:" /tmp/ros_echo.log 2>/dev/null; then
  echo "A: PASS - ROS tooling received a DRM-originated sample:"
  grep "data:" /tmp/ros_echo.log | head -2
else
  echo "A: FAIL - no ROS sample observed"
fi
if grep -q "received=[1-9]" /tmp/peer_sub.log 2>/dev/null; then
  echo "B: PASS - $(grep -o 'received=[0-9]*' /tmp/peer_sub.log) read from DRM shared memory"
else
  echo "B: FAIL - DRM consumer saw no ROS-originated sample"
fi
