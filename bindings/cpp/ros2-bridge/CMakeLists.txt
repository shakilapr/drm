cmake_minimum_required(VERSION 3.16)
project(drm_ros2_bridge CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# DRM C ABI: the same header every binding uses, linked against the cdylib.
set(DRM_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../..")
set(DRM_INCLUDE "${DRM_ROOT}/bindings/c/include")

find_package(rclcpp REQUIRED)
find_package(std_msgs REQUIRED)

add_executable(drm_bridge_ros2 src/drm_bridge_ros2.cpp)
target_include_directories(drm_bridge_ros2 PRIVATE "${DRM_INCLUDE}")
target_link_libraries(drm_bridge_ros2 PRIVATE
  rclcpp::rclcpp
  ${rclcpp_LIBRARIES}
  "${DRM_ROOT}/target/release/libdrm_ffi.so")
