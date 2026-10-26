// DRM <-> ROS 2 bridge (Phase 3 MVP, docs/architecture.md §External Middleware
// Bridges).
//
// One process, two sides:
//   - DRM side: the public attached C ABI (drm_attached_*), attaching to the
//     segment the generated deployment configuration names.
//   - ROS 2 side: rclcpp generic publisher/subscription, which carry serialized
//     messages without a compile-time message type.
//
// Every connection parameter comes from the generated deployment.json: this
// process hand-builds no identity, no hash, no capacity.
//
// REQ: REQ-TOOL-083, REQ-CORE-107, REQ-CORE-088, REQ-BRIDGE-001.
#include <rclcpp/generic_publisher.hpp>
#include <rclcpp/generic_subscription.hpp>
#include <rclcpp/qos.hpp>
#include <rclcpp/rclcpp.hpp>
#include <rclcpp/serialized_message.hpp>

#include <drm/drm.h>

#include <nlohmann/json.hpp>

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <map>
#include <memory>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

namespace {

// ---------------------------------------------------------------------------
// Generated deployment configuration (the only source of runtime identities).
// ---------------------------------------------------------------------------

struct TopicConnection {
    uint64_t slot = 0;
    std::string schema_hash;
    std::string layout_hash;
    uint64_t payload_capacity = 0;
    uint64_t slot_align = 0;
    uint64_t history_depth = 0;
    uint64_t pool_size = 0;
    std::string overflow;
};

struct DeploymentConfig {
    uint64_t format_version = 0;
    uint32_t abi_version = 0;
    std::string deployment_id;
    std::map<std::string, TopicConnection> topics;

    // Parses the artifact the toolchain emits with a real JSON parser: the shape is
    // ours to rely on, but hand-rolled scanning of it was the source of a real
    // defect (a topic block read as absent), so it is not repeated.
    static bool load(const std::string &path, DeploymentConfig *out) {
        std::ifstream in(path);
        if (!in) {
            std::cerr << "bridge: cannot read deployment config at " << path << "\n";
            return false;
        }
        nlohmann::json document;
        try {
            in >> document;
        } catch (const std::exception &e) {
            std::cerr << "bridge: deployment config is not valid JSON: " << e.what() << "\n";
            return false;
        }
        try {
            out->format_version = document.at("format_version").get<uint64_t>();
            out->abi_version = document.at("abi_version").get<uint32_t>();
            out->deployment_id = document.at("deployment_id").get<std::string>();
            for (const auto &[name, entry] : document.at("topics").items()) {
                TopicConnection connection;
                connection.slot = entry.at("slot").get<uint64_t>();
                connection.schema_hash = entry.at("schema_hash").get<std::string>();
                connection.layout_hash = entry.at("layout_hash").get<std::string>();
                connection.payload_capacity = entry.at("payload_capacity").get<uint64_t>();
                connection.slot_align = entry.at("slot_align").get<uint64_t>();
                connection.history_depth = entry.at("history_depth").get<uint64_t>();
                connection.pool_size = entry.at("pool_size").get<uint64_t>();
                connection.overflow = entry.at("overflow").get<std::string>();
                out->topics[name] = connection;
            }
        } catch (const std::exception &e) {
            std::cerr << "bridge: deployment config is missing a required field: " << e.what()
                      << "\n";
            return false;
        }
        if (out->format_version != 2) {
            std::cerr << "bridge: deployment config format_version " << out->format_version
                      << " is not understood (expected 2)\n";
            return false;
        }
        if (out->deployment_id.size() != 32) {
            std::cerr << "bridge: malformed deployment id\n";
            return false;
        }
        if (out->topics.empty()) {
            std::cerr << "bridge: deployment config declares no topics\n";
            return false;
        }
        return true;
    }
};

// ---------------------------------------------------------------------------
// One declared channel: a DRM topic <-> a ROS 2 topic, in one direction.
// ---------------------------------------------------------------------------

enum class Direction { ToDrm, ToRos };

struct Channel {
    std::string drm_topic;
    std::string ros_topic;
    std::string ros_type;
    Direction direction = Direction::ToDrm;
    TopicConnection connection;

    // Declared QoS (architecture §External Middleware Bridges: "QoS translation is
    // explicit"). Defaults are the ROS defaults; a latched topic such as /tf_static
    // must declare transient_local or it will never see the retained sample.
    bool transient_local = false;
    bool best_effort = false;

    // DRM side.
    drm_attached *attached = nullptr;
    std::vector<uint8_t> buffer;

    // ROS side.
    rclcpp::GenericPublisher::SharedPtr publisher;
    rclcpp::GenericSubscription::SharedPtr subscription;
};

/// Builds the QoS a channel declared.
rclcpp::QoS qos_for(const Channel &channel) {
    rclcpp::QoS qos(10);
    if (channel.transient_local) {
        qos.transient_local();
    } else {
        qos.durability_volatile();
    }
    if (channel.best_effort) {
        qos.best_effort();
    } else {
        qos.reliable();
    }
    return qos;
}

struct Bytes {
    std::vector<uint8_t> data;
};

// Parses `drm:<canonical>=ros:<name>@<type>` (to-DRM) or
// `ros:<name>@<type>=drm:<canonical>` (to-ROS). Undeclared topics are never
// bridged, so both endpoints must be named explicitly.
bool parse_mapping(const std::string &spec, Channel *out) {
    const size_t eq = spec.find('=');
    if (eq == std::string::npos)
        return false;
    const std::string left = spec.substr(0, eq);
    const std::string right = spec.substr(eq + 1);
    auto strip = [](const std::string &s, const std::string &prefix) {
        return s.rfind(prefix, 0) == 0 ? s.substr(prefix.size()) : std::string();
    };
    auto split_type = [](const std::string &s, std::string *name, std::string *type,
                         std::string *flags) {
        const size_t at = s.find('@');
        if (at == std::string::npos)
            return false;
        *name = s.substr(0, at);
        std::string rest = s.substr(at + 1);
        const size_t semi = rest.find(';');
        if (semi == std::string::npos) {
            *type = rest;
        } else {
            *type = rest.substr(0, semi);
            *flags = rest.substr(semi + 1);
        }
        return !name->empty() && !type->empty();
    };

    std::string drm_topic;
    std::string ros_part;
    std::string ros_name;
    std::string ros_type;
    std::string flags;
    if (!(drm_topic = strip(left, "drm:")).empty()) {
        out->direction = Direction::ToDrm;
        ros_part = strip(right, "ros:");
        if (ros_part.empty() || !split_type(ros_part, &ros_name, &ros_type, &flags))
            return false;
    } else if (!(ros_part = strip(left, "ros:")).empty()) {
        out->direction = Direction::ToRos;
        drm_topic = strip(right, "drm:");
        if (drm_topic.empty() || !split_type(ros_part, &ros_name, &ros_type, &flags))
            return false;
    } else {
        return false;
    }
    // Flags: `transient_local` (latched publishers such as /tf_static) and
    // `best_effort`. Anything else is a configuration error, never silently ignored.
    size_t start = 0;
    while (start <= flags.size() && !flags.empty()) {
        const size_t end = flags.find(';', start);
        const std::string flag =
            flags.substr(start, end == std::string::npos ? std::string::npos : end - start);
        if (!flag.empty()) {
            if (flag == "transient_local") {
                out->transient_local = true;
            } else if (flag == "best_effort") {
                out->best_effort = true;
            } else if (flag != "reliable" && flag != "volatile") {
                std::cerr << "bridge: unknown QoS flag `" << flag << "`\n";
                return false;
            }
        }
        if (end == std::string::npos)
            break;
        start = end + 1;
    }
    out->drm_topic = drm_topic;
    out->ros_topic = ros_name;
    out->ros_type = ros_type;
    return true;
}

// hex helpers: deployment.json carries digests as lowercase hex; the C ABI takes
// raw 16-byte arrays.
bool unhex(const std::string &hex, uint8_t out[16]) {
    if (hex.size() != 32)
        return false;
    for (size_t i = 0; i < 16; ++i) {
        const std::string byte = hex.substr(i * 2, 2);
        char *end = nullptr;
        const long value = std::strtol(byte.c_str(), &end, 16);
        if (end == nullptr || *end != '\0')
            return false;
        out[i] = static_cast<uint8_t>(value);
    }
    return true;
}

drm_status open_channel(Channel *channel, const std::string &segment, const std::string &lock_dir,
                        uint8_t mode, const std::string &deployment_id) {
    uint8_t schema[16];
    uint8_t layout[16];
    if (!unhex(channel->connection.schema_hash, schema) ||
        !unhex(channel->connection.layout_hash, layout)) {
        std::cerr << "bridge: " << channel->drm_topic << ": malformed contract digest\n";
        return DRM_ERR_INVALID_DATA;
    }
    const uint8_t overflow = channel->connection.overflow == "reject" ? 1 : 0;
    // Consumers never create a segment: until the publisher has created it, attach
    // is NOT_FOUND. A bridge process normally starts before or alongside the
    // publisher, so a bounded wait is part of starting up, not an error
    // (shm-backend.md I2; architecture §Segment Creation and Writer Authority).
    // Publishers are single-writer: a losing start is PERMISSION and is fatal.
    const bool waiting = mode == DRM_ATTACHED_CONSUMER;
    const int attempts = waiting ? 250 : 1;
    drm_status status = DRM_ERR_NOT_FOUND;
    for (int attempt = 0; attempt < attempts; ++attempt) {
        status = drm_attached_open(segment.c_str(), channel->drm_topic.c_str(), lock_dir.c_str(),
                                   schema, layout, static_cast<uint32_t>(channel->connection.slot),
                                   channel->connection.payload_capacity,
                                   channel->connection.history_depth, channel->connection.pool_size,
                                   overflow, mode, /*timeout_ms=*/5000, &channel->attached);
        if (status != DRM_ERR_NOT_FOUND)
            break;
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    (void)deployment_id;
    if (status != DRM_OK) {
        std::cerr << "bridge: " << channel->drm_topic << ": drm_attached_open -> " << status
                  << (status == DRM_ERR_NOT_FOUND ? " (no publisher has created this segment yet)"
                                                  : "")
                  << "\n";
        return status;
    }
    channel->buffer.resize(channel->connection.payload_capacity);
    return DRM_OK;
}

// Builds the DRM segment name the runtime derives: /drm.<deployment-id>.<slot>.
std::string segment_name(const std::string &deployment_id, uint64_t slot) {
    char slot_hex[17];
    std::snprintf(slot_hex, sizeof(slot_hex), "%016llx", static_cast<unsigned long long>(slot));
    return "/drm." + deployment_id + "." + slot_hex;
}

} // namespace

int main(int argc, char **argv) {
    // argv: <deployment.json> <lock-dir> <mapping>... [--duration <s>]
    if (argc < 4) {
        std::cerr << "usage: drm_bridge_ros2 <deployment.json> <lock-dir> <mapping>... "
                     "[--duration <seconds>]\n"
                     "  mapping: drm:<canonical>=ros:<name>@<type>   (ROS -> DRM)\n"
                     "           ros:<name>@<type>=drm:<canonical>   (DRM -> ROS)\n";
        return 2;
    }
    const std::string deployment_path = argv[1];
    const std::string lock_dir = argv[2];

    DeploymentConfig config;
    if (!DeploymentConfig::load(deployment_path, &config))
        return 1;

    std::vector<Channel> channels;
    double duration_s = 0.0;
    for (int i = 3; i < argc; ++i) {
        const std::string arg = argv[i];
        if (arg == "--duration") {
            if (i + 1 >= argc) {
                std::cerr << "bridge: --duration needs a value\n";
                return 2;
            }
            duration_s = std::atof(argv[++i]);
            continue;
        }
        Channel channel;
        if (!parse_mapping(arg, &channel)) {
            std::cerr << "bridge: malformed mapping `" << arg << "`\n";
            return 2;
        }
        const auto entry = config.topics.find(channel.drm_topic);
        if (entry == config.topics.end()) {
            // Declared mapping names an undeployed topic: refuse rather than invent one.
            std::cerr << "bridge: mapping names `" << channel.drm_topic
                      << "`, which the deployment does not declare\n";
            return 3;
        }
        channel.connection = entry->second;
        channels.push_back(std::move(channel));
    }
    if (channels.empty()) {
        std::cerr << "bridge: no declared mappings\n";
        return 2;
    }
    // No feedback loops: one topic may not be bridged in both directions.
    for (size_t i = 0; i < channels.size(); ++i) {
        for (size_t j = i + 1; j < channels.size(); ++j) {
            if (channels[i].drm_topic == channels[j].drm_topic) {
                std::cerr << "bridge: `" << channels[i].drm_topic
                          << "` is mapped twice (a loop must be two distinct topics)\n";
                return 3;
            }
        }
    }

    rclcpp::init(argc, argv);
    auto node = std::make_shared<rclcpp::Node>("drm_bridge_ros2");

    // Open every DRM attachment and build the ROS endpoints.
    for (Channel &channel : channels) {
        const std::string segment = segment_name(config.deployment_id, channel.connection.slot);
        if (channel.direction == Direction::ToDrm) {
            if (open_channel(&channel, segment, lock_dir, DRM_ATTACHED_PUBLISHER,
                             config.deployment_id) != DRM_OK) {
                return 1;
            }
            // ROS -> DRM: a generic subscription carrying serialized bytes. A declared
            // type that is not a message (a service or action interface) cannot be
            // subscribed to: rclcpp throws, so it is caught and reported rather than
            // aborting the process.
            try {
                channel.subscription = node->create_generic_subscription(
                    channel.ros_topic, channel.ros_type, qos_for(channel),
                    [&channel](std::shared_ptr<rclcpp::SerializedMessage> message) {
                        const auto &rcl = message->get_rcl_serialized_message();
                        if (rcl.buffer_length > channel.connection.payload_capacity) {
                            // Capacity is checked, not discovered: never truncated.
                            std::cerr << "bridge: " << channel.drm_topic << ": message of "
                                      << rcl.buffer_length << " bytes exceeds the declared bound "
                                      << channel.connection.payload_capacity << "; dropped\n";
                            return;
                        }
                        if (std::getenv("DRM_BRIDGE_DEBUG") != nullptr) {
                            std::cerr << "bridge: received " << rcl.buffer_length << " bytes on "
                                      << channel.ros_topic << "\n";
                        }
                        uint64_t seq = 0;
                        const drm_status status = drm_attached_publish(channel.attached, rcl.buffer,
                                                                       rcl.buffer_length, &seq);
                        if (status != DRM_OK) {
                            std::cerr << "bridge: " << channel.drm_topic << ": publish -> "
                                      << status << "\n";
                        }
                    });
            } catch (const std::exception &e) {
                std::cerr << "bridge: " << channel.ros_topic << " @ " << channel.ros_type
                          << ": not a subscribable message type: " << e.what() << "\n";
                return 3;
            }
        } else {
            if (open_channel(&channel, segment, lock_dir, DRM_ATTACHED_CONSUMER,
                             config.deployment_id) != DRM_OK) {
                return 1;
            }
            try {
                channel.publisher = node->create_generic_publisher(
                    channel.ros_topic, channel.ros_type, qos_for(channel));
            } catch (const std::exception &e) {
                std::cerr << "bridge: " << channel.ros_topic << " @ " << channel.ros_type
                          << ": not a publishable message type: " << e.what() << "\n";
                return 3;
            }
        }
    }

    std::cout << "bridge: deployment " << config.deployment_id << ", " << channels.size()
              << " channel(s)\n";
    for (const Channel &channel : channels) {
        std::cout << "  " << (channel.direction == Direction::ToDrm ? "ros->drm " : "drm->ros ")
                  << channel.ros_topic << " @ " << channel.ros_type << " <-> " << channel.drm_topic
                  << "\n";
    }
    std::cout.flush();

    // Main loop: pump DRM -> ROS channels. A DRM read failure on one channel never
    // stops the others (isolation).
    const auto start = std::chrono::steady_clock::now();
    while (rclcpp::ok()) {
        for (Channel &channel : channels) {
            if (channel.direction != Direction::ToRos)
                continue;
            size_t len = 0;
            uint64_t missed = 0;
            const drm_status status = drm_attached_read_next(
                channel.attached, channel.buffer.data(), channel.buffer.size(), &len, &missed);
            if (status == DRM_OK && len > 0) {
                rclcpp::SerializedMessage message(len);
                std::memcpy(message.get_rcl_serialized_message().buffer, channel.buffer.data(),
                            len);
                message.get_rcl_serialized_message().buffer_length = len;
                channel.publisher->publish(message);
            } else if (status != DRM_ERR_NO_SAMPLE && status != DRM_ERR_RETRY) {
                std::cerr << "bridge: " << channel.drm_topic << ": read -> " << status
                          << " (missed " << missed << ")\n";
            }
        }
        rclcpp::spin_some(node);
        if (duration_s > 0.0) {
            const double elapsed =
                std::chrono::duration<double>(std::chrono::steady_clock::now() - start).count();
            if (elapsed >= duration_s)
                break;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }

    for (Channel &channel : channels) {
        if (channel.attached != nullptr)
            drm_attached_close(channel.attached);
    }
    rclcpp::shutdown();
    return 0;
}
