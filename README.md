# DRM — Decentralized Robotics Middleware

## Purpose

DRM is a decentralized, contract-driven data middleware designed for deterministic, high-throughput robotics and computer systems on Linux and Windows.

Applications declare what topics they publish and consume. DRM handles how data moves. There is no central broker, master node, daemon dependency for local IPC, or runtime discovery.

## Local Data Fabric

Every computer running DRM operates a local domain over operating-system shared memory.

All topics mapped to local nodes are available in shared memory with zero serialization overhead. DRM delivers data directly between publisher and consumer memory slots without copying every message to every process.

For high-bandwidth topics like camera frames and point clouds, DRM uses lockless zero-copy shared memory. Topics use fixed-capacity slot pools (defaulting to a 3-slot triple buffer for latest data), allowing concurrent readers to borrow frames safely while publishers continue writing without blocking.

## Communication Contracts

Each node declares its communication contract in a small manifest:

- Topics it publishes (with single-writer authority per topic)
- Topics it consumes
- Data schemas and structural bounds
- Timing expectations (deadlines, expected rates)
- Retention depth and history overflow policies

Nodes declare only their local communication interface. They do not know physical hardware placements, network topologies, or other nodes.

## Physical Boundaries and Routing

Data stays inside the local computer's shared memory unless a consumer is placed on a remote device.

Routing is automatic: DRM infers routes from the declared communication graph and physical link topology. Developers do not maintain manual export lists. When multiple physical links exist between devices, developers supply explicit routing overrides (`prefer`, `deny`, `force`) to resolve ambiguity.

The boundary router daemon (`drm-router`) handles cross-device traffic:

- Enforces single-packet limits (under MTU) for low-latency UDP datagrams
- Uses stream transports like TCP for large bulk data or reliable delivery
- Prioritizes critical control and safety commands ahead of background telemetry
- Shapes traffic to respect link bandwidth budgets
- Imports incoming network data directly into the local shared-memory domain

## Build-Time Toolchain

DRM resolves system geometry, layouts, and routing at build time rather than runtime:

- Generates cache-aligned, zero-deserialization structs conforming to a canonical C-ABI (`drm.h`)
- Generates native bindings for the four MVP languages: C, C++, Rust, and Python
- Pre-calculates fixed shared-memory slot sizes and bounded ring buffer offsets
- Verifies deterministic system properties: single-publisher constraints, schema compatibility, physical link fit, and worst-case static memory budgets
- Packages deployment metadata into a single deployment configuration

At runtime, nodes attach directly to shared memory using static offsets and execute lockless reads and writes without discovery overhead, dynamic memory allocations, or runtime schema negotiation.

## Core Principle

Inside a computer domain, DRM provides direct, zero-copy shared-memory IPC. Across physical boundaries, the boundary router automatically transports and shapes only the data required by remote consumers.

## Documentation

- [Core Architecture Specification](docs/architecture.md): Full system specification, memory model, shared history, boundary router, and build-time verification.
- [Spatial Architecture Specification](docs/spatial_architecture.md): Coordinate frames, geometric transforms, clock domains, and static transform collapse.
