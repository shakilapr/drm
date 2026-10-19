#!/bin/bash
# Debug: does a DRM publisher's sample reach a DRM consumer at all?
set +u
cd /root/projects/drm || exit 1
export LD_LIBRARY_PATH="/root/projects/drm/target/release:${LD_LIBRARY_PATH:-}"
DEP=tests/ros2_bridge/.drm/deploy/deployment.json
ID=$(python3 -c "import json;print(json.load(open('$DEP'))['deployment_id'])")
SEG=$(printf '/drm.%s.%016x' "$ID" 1)
H1=335486d6659ba456dfde687646f45dde
H2=f93b232c3f35db0e0f0b1ec8994e613b
rm -f /dev/shm/drm."$ID".* 2>/dev/null

/tmp/drm_peer_pub pub "$SEG" bridge/to_ros /tmp/drm-bridge-locks "$H1" "$H2" 1 4100 8 16 200 > /tmp/p.log 2>&1 &
PUB=$!
sleep 0.5
/tmp/drm_peer_sub sub "$SEG" bridge/to_ros /tmp/drm-bridge-locks "$H1" "$H2" 1 4100 8 16 3 DRM-TO-ROS > /tmp/s.log 2>&1
SUB_RC=$?
wait $PUB 2>/dev/null
echo "publisher: $(cat /tmp/p.log)"
echo "consumer(rc=$SUB_RC): $(cat /tmp/s.log)"
