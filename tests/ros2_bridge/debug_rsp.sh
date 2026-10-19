#!/bin/bash
# Is robot_state_publisher publishing /tf at all? (independent of the bridge)
set +u
source /opt/ros/jazzy/setup.bash >/dev/null 2>&1
export ROS_DOMAIN_ID=48
URDF='<?xml version="1.0"?><robot name="probe"><link name="base_link"/><link name="arm"/><joint name="j1" type="revolute"><parent link="base_link"/><child link="arm"/><axis xyz="0 0 1"/><limit lower="-1" upper="1" effort="1" velocity="1"/></joint></robot>'

timeout 12 ros2 run robot_state_publisher robot_state_publisher \
  --ros-args -p robot_description:="$URDF" > /tmp/dbg_rsp.log 2>&1 &
RSP=$!
sleep 3
timeout 10 ros2 topic pub -r 10 /joint_states sensor_msgs/msg/JointState \
  "{name: ['j1'], position: [0.5]}" > /dev/null 2>&1 &
PUB=$!
sleep 2
echo "--- ros2 topic list ---"; timeout 5 ros2 topic list | grep -E "^/tf|joint" 
echo "--- echo /tf --once (10s budget) ---"
timeout 10 ros2 topic echo /tf --once 2>&1 | head -6
echo "--- publisher info /tf ---"; timeout 5 ros2 topic info /tf 2>&1 | head -6
wait $PUB 2>/dev/null
kill $RSP 2>/dev/null
