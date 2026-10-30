#!/bin/bash
# Regression: the bridge's subscription QoS defaults to BEST_EFFORT.
#
# Before the fix a ROS BEST_EFFORT publisher (the sensor_data profile every
# driver uses) was silently dropped by the bridge's default RELIABLE
# subscription: the DRM consumer counted 0 samples. With no QoS flag the
# bridge must now choose BEST_EFFORT and deliver the samples.
#
# Evidence-based: PASS only when the DRM peer counted samples and the bridge
# startup report names BEST_EFFORT as the subscription default.
set +u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT" || exit 1
source /opt/ros/jazzy/setup.bash >/dev/null 2>&1
export ROS_DOMAIN_ID=48
export LD_LIBRARY_PATH="$ROOT/target/release:${LD_LIBRARY_PATH:-}"

DEP=tests/ros2_bridge/.drm/deploy/deployment.json
LOCKS=/tmp/drm-bridge-locks
BRIDGE=./bindings/cpp/ros2-bridge/build/drm_bridge_ros2
ID=$(python3 -c "import json;print(json.load(open('$DEP'))['deployment_id'])")
ALIGN=$(python3 -c "import json;d=json.load(open('$DEP'));print(next(iter(d['topics'].values()))['slot_align'])")
H1=335486d6659ba456dfde687646f45dde
H2=f93b232c3f35db0e0f0b1ec8994e613b
seg() { printf '/drm.%s.%016x' "$ID" "$1"; }

if [ ! -x /tmp/drm_peer_sub ]; then
  cc -O2 -I bindings/c/include tests/ros2_bridge/drm_peer_sub.c \
    -L"$ROOT/target/release" -ldrm_ffi -lpthread \
    -Wl,-rpath,"$ROOT/target/release" -o /tmp/drm_peer_sub || exit 1
fi

rm -f /dev/shm/drm."$ID".* 2>/dev/null
mkdir -p "$LOCKS"; rm -f "$LOCKS"/* 2>/dev/null

# No QoS flag: the subscription must default to BEST_EFFORT.
"$BRIDGE" "$DEP" "$LOCKS" 'drm:bridge/from_ros=ros:/drm_probe_qos@std_msgs/msg/String' \
  --duration 16 > /tmp/qos_bridge.log 2>&1 &
BR=$!
sleep 3
/tmp/drm_peer_sub sub "$(seg 0)" bridge/from_ros "$LOCKS" "$H1" "$H2" 0 4100 8 16 3 \
  BEST-EFFORT-RX "$ALIGN" > /tmp/qos_sub.log 2>&1 &
SUB=$!
sleep 1
# BEST_EFFORT publisher, exactly what a ROS sensor driver looks like on the wire.
timeout 8 ros2 topic pub -r 5 --qos-reliability best_effort /drm_probe_qos \
  std_msgs/msg/String "{data: 'BEST-EFFORT-RX'}" > /tmp/qos_pub.log 2>&1
sleep 1
wait $SUB 2>/dev/null
wait $BR 2>/dev/null

RECEIVED=0
if grep -q "received=[1-9]" /tmp/qos_sub.log 2>/dev/null; then
  RECEIVED=1
fi
REPORTED=0
if grep -q "subscription /drm_probe_qos: BEST_EFFORT (default for a subscription)" \
    /tmp/qos_bridge.log 2>/dev/null; then
  REPORTED=1
fi

echo "== bridge QoS default check (no flag, BEST_EFFORT publisher) =="
echo "--- bridge ---"; cat /tmp/qos_bridge.log
echo "--- drm consumer ---"; cat /tmp/qos_sub.log
if [ "$RECEIVED" -eq 1 ] && [ "$REPORTED" -eq 1 ]; then
  echo "PASS  BEST_EFFORT publisher received with no QoS flag"
  exit 0
fi
echo "FAIL  BEST_EFFORT publisher not received or default not reported"
[ "$RECEIVED" -eq 1 ] || echo "      DRM consumer saw no samples (was 0 before the fix)"
[ "$REPORTED" -eq 1 ] || echo "      bridge did not report BEST_EFFORT as the subscription default"
exit 1
