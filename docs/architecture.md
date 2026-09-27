# DRM Architecture

DRM is a decentralized data middleware for robots. Its primary duty is to move data between programs—such as camera frames, LiDAR point clouds, sensor measurements, and motor commands—without relying on a central broker, master node, or runtime network discovery.

Programs can be written in any language (C, C++, Rust, Python, TypeScript, Go, C#, Zig). Applications declare what topics they send and receive. DRM handles how that data is packaged, transported, and delivered.

## Core Principle

Inside a single computer, all topics are available to all programs. Across physical connections (cables, wireless links, buses), data only moves when nodes on different devices need it.

Because every topic belongs to the local domain by default, no `local: true` setting is ever needed.

## The Duty of DRM: How Data Is Sent and Received

DRM provides two distinct data paths based on the nature of the data:

### Structured Data (Commands, States, Small Sensors)
For high-frequency or small messages—such as motor commands, battery status, IMU readings, and control targets:
- **Build-Time Binary Encoding**: DRM compiles topic definitions into fixed, compact byte layouts. A command message takes only the exact bytes needed (for example, 7 bytes instead of 80 bytes of JSON).
- **Generated Compact Handles**: Topic paths like `control/target` are canonical identities. Where compact integers improve transport or routing efficiency (such as over a wire or serial link), DRM generates compact numeric handles so strings are never transmitted.
- **Fast Packing**: When a program publishes data, it calls a generated function that writes values straight into fixed byte offsets. The receiving program reads those exact offsets with zero string parsing or field lookups.

### Bulk Data (Cameras, LiDAR, Large Point Clouds)
For large sensor payloads—such as 4K video frames, depth maps, and high-density point clouds:
- **Zero-Copy Shared Memory**: The publishing program writes the image or point cloud directly into a pre-allocated shared memory slot.
- **No Duplication**: DRM delivers a borrowed view backed by a shared memory slot to local consumers. If multiple programs read the same camera feed, the data is stored in memory once and read simultaneously. It is never copied redundantly.
- **No Raw Pointers Across Processes**: Processes map shared memory at different virtual memory addresses. DRM uses relocatable sample handles containing `{ slot, generation, sequence }` rather than passing raw pointers across process boundaries.
- **Reuse-First Memory Management**: In steady state, DRM avoids per-message heap allocation on the data path. Pre-allocated or reusable shared memory buffers are acquired, filled, published, and recycled without invoking general-purpose operating system allocators.

## Local Data Delivery

On a single computer, DRM runs as a local data fabric:

- **Available to All, Copied to None**: Every topic exists in the local domain, but DRM only delivers data to programs that explicitly asked to consume it. If nobody listens to a camera, no CPU or memory bandwidth is spent reading it.
- **Multi-Language Access**: A microcontroller driver in C, a computer vision pipeline in Python, a state estimator in Rust, and a user interface in TypeScript all read the exact same memory buffers.
- **Instant Connection**: Programs attach directly to the pre-allocated shared memory segments using static IDs. There is no connection handshake or discovery delay.

### Zero-Copy Memory Lifecycle: Bounded Transient Borrows
The primary challenge of zero-copy shared memory is knowing when a publisher may safely reuse a slot without corrupting consumers or deadlocking when a consumer stalls or crashes.

Common approaches fail in real-world robotics:
- Shared reference counting creates heavy atomic lock contention across processes and leaks memory permanently if a consumer crashes.
- Fixed lease timeouts (such as a 100 ms expiration) cause severe bugs: if an operating system deschedules a reader thread during high load, the publisher assumes it is dead, reclaims the slot, and corrupts memory while the reader is actively using it. A slow reader must never have its memory overwritten; a dead reader's memory can be reclaimed. Time alone cannot distinguish the two.

DRM eliminates this complexity through a core architectural rule:

> **Zero-copy samples are transient borrowed resources. DRM guarantees their validity while borrowed; applications that need longer or unbounded lifetime must own a copy.**
> **External I/O may never own or retain a DRM borrowed sample.**

A borrow exists strictly for bounded in-memory computation by the consumer (such as running neural network inference, evaluating a control loop, or computing coordinate transforms), never for latency governed by external subsystems. Filesystem writes, database commits, cloud uploads, GPU operations with unbounded completion, and blocking third-party device APIs must copy data into application-owned buffers before initiating I/O. This guarantees that operating system storage stalls, NVMe pauses, or network congestion never pin shared-memory hazard slots or stall real-time publishers.

DRM implements this using a generalized bounded history pool, independent reader cursors, and per-consumer hazard slots:

- **The Dual-Layer Architecture: Sequence History and Payload Pool**:
  - **Sequence History Ring**: A circular ring of depth $R$ storing compact sequence handles: `sequence -> {slot_index, generation}`.
  - **Default Triple-Buffering for Latest Data**: For `history: latest` bulk topics ($R = 1$), DRM defaults to a pool of 3 slots (one for the latest completed sample, one for the active publisher write, and one for an active reader borrow). In robotics workloads, multiple concurrent readers (e.g. perception, localization, UI) typically read the same latest frame simultaneously, sharing protection on that single slot.
  - **Configurable Pool Size (`pool_size: N`)**: When downstream pipelines require holding multiple distinct historical frames concurrently without blocking the publisher, nodes can explicitly configure `pool_size = N` in the manifest. For control loops requiring absolute drop immunity under worst-case consumer delay with $H$ independent borrowers, the manifest configures $H + 2$ slots.

- **Safe Handling of Pool Exhaustion**:
  If readers hold borrows across all available slots and exhaust the pool, DRM preserves real-time guarantees:
  - Active borrowed slots are never overwritten or corrupted.
  - The publisher thread never blocks waiting for slow readers.
  - The publisher drops the incoming sample, increments an atomic `pool_exhausted` counter for `drm-diag` reporting, and preserves the current latest sample. Once a reader finishes or dead process cleanup reclaims a slot, publishing resumes instantly.

- **Reader Cursor vs Hazard Slot**:
  - **Reader Cursor**: Tracks logical sequence progress (*Which sequence should I read next?*). Maintained locally by each consumer.
  - **Hazard Slot**: Tracks physical slot protection (*Which physical payload slot am I currently reading and therefore must not be recycled?*). Each declared zero-copy consumer has dedicated slots in shared memory: `protected[consumer_id] = {slot, generation}` or `NONE`.
  - If a consumer continues borrowing a sample after its sequence has aged out of the $R$-depth history ring, that physical slot remains protected against reuse. The publisher continues advancing using the remaining free slots in the pool.

- **Lockless Consumer Acquisition Protocol**:
  1. The consumer reads the desired sequence handle from the topic sequence ring: `entry1 = history[target_seq % R]`.
  2. The consumer publishes its protection intent: `protected[my_id] = entry1`.
  3. The consumer rereads the sequence handle: `entry2 = history[target_seq % R]`.
  4. If `entry1 != entry2` or `entry2.sequence != target_seq`, the publisher rolled the slot during acquisition. The consumer clears its protection (`protected[my_id] = NONE`) and handles the sequence change or retries.
  5. If `entry1 == entry2`, the slot is safely locked against reuse. The consumer processes the immutable payload directly from shared memory.
  6. Upon leaving the borrow scope, the consumer clears its protection: `protected[my_id] = NONE`.
  The generation counter prevents ABA hazards when slot numbers wrap around.

- **Publisher Slot Selection**:
  The publisher owns slot reuse decisions without locks or reference counts. When publishing a new frame, it selects a free slot that is neither referenced by an active history entry nor protected by any consumer hazard slot (`no protected[consumer].slot == slot`). It writes the payload, increments the slot generation, updates the sequence history ring, and advances the write sequence with release ordering.

- **Crash Recovery via OS Process Inspection**:
  If a consumer process crashes while holding a borrow, that slot is not leaked permanently. DRM avoids arbitrary timeout guesses. Instead, dead processes are identified using native operating-system process validation (checking `kill(pid, 0)` or `pidfd` on Linux, and querying process handle signaled state on Windows). Once the OS confirms the process has terminated, its hazard slot is reset to `NONE` and returned to the pool.

- **Orthogonal Sample Access (API Operations, Not Declared Modes)**:
  `view`, `snapshot`, and `stream` are not separate delivery modes. Topics define shared bounded history; consumers choose their reader position and memory ownership through standard API calls:
  - **`borrow_latest()`**: Zero-copy transient borrow of the newest sample in shared memory.
  - **`copy_latest()`** (or **`latest()`**): Application-owned copy of the newest sample with arbitrary lifetime.
  - **`borrow_next()`**: Zero-copy transient borrow of the next sequential sample advancing the consumer cursor.
  - **`copy_next()`** (or **`next()`**): Application-owned copy of the next sequential sample advancing the consumer cursor.
  - When an unread sequence rolls out of the history ring under an `overwrite` policy, sequential readers receive an explicit `GAP { missed: N }` rather than silently missing data.

### Shared-Memory Lifetime: Three Identities and Deployment Isolation
Decentralized shared-memory middlewares often fail when attempting to manage complex cross-process cleanup, resource inheritance, and live segment migration after crashes. DRM eliminates this overhead by separating immutable build artifacts from runtime deployment instances across three distinct identities:

> **Compile what changes program compatibility. Configure what changes runtime instantiation.**

- **1. Node Interface Identity (Compiled into Binary)**:
  Each node binary compiles an immutable contract fingerprint derived strictly from its own DRM interfaces:
  - DRM ABI version
  - Schemas and memory layouts of topics published or consumed by this specific node
  - Declared publish and consume communication interfaces
  Unrelated schema edits (such as changing a camera topic) do not alter the contract fingerprint of unaffected nodes (such as a motor controller). Node binaries remain reusable across any number of compatible deployments without recompilation or relinking.

- **2. Deployment Instance ID (Runtime Configuration in `deployment.json`)**:
  Every deployment generates a random 128-bit identifier (such as `91ca8fa4-9e2b-4f48-a25c-7ea98dcb84fe`) recorded in `.drm/deploy/deployment.json`.
  - **Never Compiled into Binaries**: Deployment IDs are runtime configuration. Modifying hardware placement, physical routing tables, bus bandwidths, or deployment IDs updates only `deployment.json` without recompiling any application binary.
  - **Scoped Shared-Memory Namespaces**: All shared-memory paths are scoped by this identifier: `drm/<deployment_id>/<topic>`.
  - **Startup Compatibility Guard**: At startup, `drm-core` supplies the deployment context to the node. The node checks that its compiled contract fingerprint matches the expected fingerprint recorded in `deployment.json`. If a developer attempts to run an outdated binary against an incompatible deployment, the node refuses to start immediately.
  - **Individual Segment Validation**: When a process attaches to an individual topic segment, it performs a secondary check verifying that the segment's compiled schema and layout descriptor match its expectations.
  - **No Cross-Deployment Adoption**: A new deployment always creates its own isolated shared-memory namespace. DRM never attempts live schema migration, dynamic segment resizing, or adoption of segments from an older deployment.
  - **Safe Cleanup via OS-Backed Lock**: Each deployment directory holds an OS-backed active lock (`/run/drm/<deployment_id>/active.lock`). Running nodes hold a shared read lock. The `drm clean` command attempts an exclusive lock: if the lock succeeds, no nodes are using the namespace and it is safely unlinked; if it fails, the deployment is active and left untouched.

- **3. Publisher Epoch (Runtime Shared State)**:
  Inside a deployment namespace, each topic segment maintains an atomic `publisher_epoch` counter in shared memory.
  - When a publisher starts or restarts within the same deployment, it reopens the compatible existing segment, verifies the header descriptor, and increments the epoch (for example, from 7 to 8).
  - Consumers observe the epoch change and reset local reader state, ensuring samples from the previous publisher run are never misinterpreted as observations from the new run.

- **Single Publisher per Topic (Single-Writer, Multiple-Reader)**:
  - Within a DRM domain, every topic has exactly one publisher node. Multiple nodes publishing to the same canonical topic is disallowed and rejected at build time by `drm verify`.
  - Single-writer authority guarantees lockless sequence integrity, eliminates multi-writer arbitration in shared memory, ensures unambiguous data provenance, and makes routing fully deterministic.
  - When an application requires converging multiple data sources (such as dual joysticks or alternate planning nodes), sources publish to distinct topics (`control/target_teleop`, `control/target_nav`). An explicit multiplexer node selects the active input and publishes the authoritative stream (`control/target`).

- **Segment Creation and Writer Authority**:
  - The single topic publisher creates the shared-memory segment atomically (`O_CREAT | O_EXCL` or equivalent). Consumers wait and open existing segments; consumers never create topic memory.
  - Write authority is enforced using an operating-system-backed exclusive lock (`/drm/<deployment>/<topic>.publisher.lock`). If the publishing process terminates, the operating system kernel automatically releases the lock, allowing a restarted publisher to acquire write authority without heartbeat arbitration or dead-process takeover protocols.

- **Data Freshness vs Stored Bytes**:
  When a publisher crashes, existing consumers can finish reading their currently borrowed samples safely, but DRM does not treat the last completed sample as indefinitely current. Streams transition to `STALE` or `OFFLINE` based on timing contracts. Upon restart, the stream state resets to `NO_CURRENT_SAMPLE` until the restarted publisher produces its first complete sample under the new epoch.

- **Session-Scoped Persistence**:
  Shared-memory persistence is strictly an optimization for process crash/restart recovery within an operating system session. DRM does not guarantee shared-memory persistence across machine reboots. Following a system reboot, publishers recreate their required segments fresh.

## Memory Architecture: Fixed Channels and Structural Isolation

DRM eliminates runtime memory allocators from the data path by exploiting its static knowledge of the robot: topic shapes, maximum payload bounds, consumer borrow rights, queue depths, and hardware placement are known before deployment.

The governing rules of DRM memory are:

> **DRM channel storage is finalized when the channel becomes active. The steady-state data path only reuses that storage.**
> **Every active channel has fixed-capacity storage. Different topics never compete for buffer capacity.**

### Storage Architecture: Generalized Bounded History
Rather than managing a general-purpose heap or maintaining separate delivery engines for latest versus sequential data, DRM maps topic storage into a single unified architecture:

- **Small Structured State**: For compact telemetry, motor commands, and control targets that fit within cache lines, storage is allocated as a single slot or small sequence ring where producers update pre-calculated byte offsets without allocation overhead.
- **Bulk Latest Topics**: For camera frames, point clouds, and high-rate sensor observations (`history: latest`), DRM defaults to a small fixed pool of 3 slots (triple buffer). This provides one slot for the current latest sample, one slot for the in-progress publisher write, and one slot temporarily held by an active reader. Deployments requiring a mathematical guarantee that up to $H$ concurrent consumers holding different historical frames can never cause a drop may explicitly configure worst-case sizing ($H + 2$ slots).
- **Sequential Topics and Events**: For ordered streams (`history: N`), DRM allocates a circular sequence handle ring of depth $R$ paired with a fixed payload slot pool. Long-term histories and lossless recording pipelines move immediately into application-managed storage buffers (`drm-record`). If a developer explicitly configures a large history on a multi-megabyte topic (such as 64 frames on an 8 MiB camera topic), `drm verify` reports the full memory cost (512 MiB) directly.

Only topics assigned to active nodes on a given compute unit receive backing storage. Unused topics defined in packages consume zero memory at runtime.

### Structural Isolation (No Cross-Topic Contention)
Every topic channel owns its own storage partition. A high-bandwidth camera stream can never exhaust or starve memory allocated to LiDAR, state estimators, or safety-critical motor commands.

Because channels do not compete for memory, DRM completely eliminates:
- Runtime pool growth and shrinking
- Cross-process memory remapping
- Free-list fragmentation and coalescing
- Cross-topic fairness and priority borrowing rules
- Allocation-time deadlocks and crash recovery in shared heaps

Multiple fixed channel partitions may be mapped inside a single shared-memory region for operating system efficiency, but offsets are fixed during setup. Sharing an OS mapping does not require sharing a runtime allocator.

### Fixed Slot Pools and Demand-Backed Paging
Bulk topics maintain fixed virtual address layouts, but operating systems need not fully commit physical memory until actually written:

> **Bulk topics use fixed-capacity slot pools. On general-purpose 64-bit operating systems, physical pages are backed on demand as written, letting native OS paging manage memory rather than complex user-space tracking. Hard-real-time targets may explicitly precommit the full worst-case working set.**

- **Fixed-Capacity Slot Pools**:
  A topic declaration like `payload: bytes[<=8388608]` establishes the legal maximum payload bound and a fixed virtual address range for each slot. Slot offsets in shared memory (`offset(slot 0)`, `offset(slot 1)`, `offset(slot 2)`) remain static constants. This eliminates pointer relocation, linked chunks, and heap fragmentation.
- **Demand-Backed Operating System Paging**:
  On 64-bit operating systems (Linux `/dev/shm` backed by `tmpfs` and standard Windows pagefile-backed shared memory), physical RAM pages are committed by the OS kernel when written. If a camera produces a 3.2 MiB frame into an 8 MiB slot, only the pages touched are committed. DRM does not attempt complex user-space page decommitment or manual high-water mark tracking; it relies on the operating system's native virtual memory subsystem.
- **Hard Real-Time Precommitment**:
  For safety-critical or bare-metal RTOS targets where page faults are forbidden, deployments may declare `precommit: true`. DRM pre-faults and memory-locks (`mlock` on Linux, `VirtualLock` on Windows) the full physical capacity at startup, guaranteeing zero runtime page faults.

### Application Allocators for Long-Lived Data
Shared zero-copy slots are transient borrowed resources. When an application needs to retain samples beyond immediate processing—such as background recording to disk, user interface rendering, or historical training logs—it copies the data (`frame = camera.copy_latest()`) or transfers it to an application-managed buffer.

Owned samples are managed entirely by the application runtime's native allocator (such as Python heap, NumPy, or C++ standard allocators). DRM shared memory is never used as a general heap or unbounded queue for user applications. The moment a recording pipeline requires persistent disk buffering, that queue belongs inside `drm-record`, keeping `drm-core` channels lean, bounded, and deterministic.

### Memory Budgets as Build-Time Verification
Memory limits are verified at compile time rather than managed by a runtime resource scheduler:

- **Worst-Case Static Capacity Budget**: During `drm verify`, the DRM toolchain calculates the maximum physical capacity required if every slot in every active channel on a compute unit were filled to its maximum declared bound simultaneously. This worst-case budget is validated against the target machine's hardware memory ceiling before the robot runs.
- **Advisory Nominal Usage**: DRM reports the estimated nominal working set based on declared payload expectations as an advisory planning metric.
- **Runtime Measurement**: Actual resident set size (RSS), page commitment, and cache effects are measured and reported live by `drm-diag`, not guessed by static compilers.

## Core Language Ecosystem (MVP)

In systems like ROS 2, each language requires a dedicated client library (`rclcpp`, `rclpy`, `rclc`, `rclrs`, `rclnodejs`, `ros2_dotnet`, `rclgo`) that includes heavy serialization engines (CDR) and dynamic discovery stacks.

DRM eliminates runtime serialization by generating thin, native bindings from a single canonical C-ABI specification (`drm.h`). For the initial MVP, DRM provides first-class support for four foundational robotics languages:

- **C (ISO C99/C11)**: Pure, zero-allocation C structs and direct functions (`drm_publish_control_target`, `drm_read_control_target`). Built for microcontrollers, bare-metal chips, real-time operating systems (FreeRTOS, Zephyr), and safety-critical motor controllers.
- **C++ (C++17/20)**: Type-safe templates, RAII memory managers, and zero-copy pointer views without runtime overhead.
- **Rust**: Safe zero-copy reference wrappers, compile-time thread safety, and lock-free ring buffer access.
- **Python**: Safe default access returns an owned copy (`frame = camera.copy_latest()`, or shorthand `camera.latest()`). Explicit zero-copy uses scoped borrows (`with camera.borrow_latest() as frame: detect(frame.numpy())`), providing direct NumPy buffer compatibility without allowing unmanaged references to hold shared memory slots indefinitely.

All other languages interface directly through the canonical C-ABI header (`drm.h`) or community-maintained bindings. Because the underlying layout is fixed and standardized on the C-ABI, any language with foreign function interface (FFI) or raw pointer access can connect to DRM without a custom runtime engine. Because dynamic language runtimes manage their own internal heap objects and garbage collection, DRM guarantees that its native payload data path performs no per-message heap allocation in steady state.

## Canonical Binary Layout and Layered Metadata

DRM does not copy raw programming language memory structures directly onto the wire or into shared memory. Different compilers and processor architectures use different padding, alignment, and byte order.

Instead, DRM defines a canonical binary format generated at build time:
- Standard integer and floating-point sizes (such as 4-byte integers and 4-byte IEEE floats).
- Consistent little-endian byte ordering across all processors.
- Generated encoders and decoders for every language that produce and read identical bytes.

### No Universal Envelope: Metadata at the Narrowest Scope
A universal message wrapper that attaches channel IDs, timestamps, lengths, checksums, and schema hashes to every single message applies a network-packet mindset to local shared memory.

In local shared memory, the memory location of the ring or slot itself identifies the topic and data layout. Repeating static identifiers and computing CRC checksums across the same physical RAM wastes CPU cycles and memory bandwidth without providing concurrency safety.

DRM eliminates universal envelopes by dividing metadata into three distinct layers:

- **Channel Metadata (Validated Once at Attachment)**:
  Properties that never change while running are stored once in a channel descriptor within the shared memory segment:
  - Channel identifier and topic name
  - Schema content hash and version
  - Fixed payload size or maximum capacity
  - Queue depth and delivery policy
  - Channel epoch
  When a node attaches to a topic, it compares its expected schema hash against the channel descriptor once. After a successful attachment, steady-state messages are processed without checking schema hashes. If a topic definition changes or a publisher restarts, the channel epoch advances, triggering re-validation.

- **Sample Metadata (Varies Per Sample or Drives Concurrency)**:
  Individual shared-memory samples contain only information that differs per sample or is required for lockless synchronization:
  - **Fixed-Size Topics**: Contain only the raw payload. Synchronization state (such as generation counters and sequence progression) resides in the channel control block alongside the slots, not inside the application payload.
  - **Variable-Sized Topics**: Prepend an actual byte length before the payload. Fixed-size topics never pay for a redundant length field.
  - **Sensor Topics**: Include an explicit source timestamp (such as optical shutter time or IMU sample acquisition time) only when declared by topic semantics. DRM does not force arbitrary transport timestamps onto discrete commands or state machines.
  - **Local Integrity**: Local shared memory relies on atomic generation counters and acquire-release ordering for concurrency safety rather than software CRCs.

- **Transport Metadata (Boundary Framing Across Physical Links)**:
  Wire-level framing is added only when a message crosses a physical boundary to another device:
  - **Multiplexed Channel ID**: Identifies the stream across shared physical media (such as UDP multicast or serial links).
  - **Packet Framing**: Adds transport sequence numbers, packet lengths, and fragmentation offsets.
  - **Link and End-to-End Integrity**: Uses physical hardware checksums (such as Ethernet frame check sequences or packet CRCs) or optional end-to-end integrity profiles for safety-critical topics requiring end-to-end fault detection.
  When the boundary router receives external frames and imports them into a local shared-memory domain, wire headers are stripped and samples are deposited into local memory in their native format.

The DRM compiler generates the exact framing and headers automatically from topic shapes, timing rules, and network links. Developers declare what their data means rather than configuring individual header fields manually.

## Delivery Semantics: Shared History and Reader Cursors

In distributed robotics, per-subscriber delivery pipelines create severe architectural complexity: independent queues, conflicting backpressure rules, and duplicate network transmissions.

DRM eliminates this complexity by decoupling consumer read positions from shared storage:

> **One topic has one shared history in local memory, consumed through lightweight reader cursors.**
> **A DRM topic has one storage and retention contract per local domain. Consumers choose whether to read the latest sample or sequential samples from that shared history, and whether to borrow or copy, but they do not create independent payload queues or independent reliability engines.**

DRM organizes delivery around five orthogonal questions:

### 1. Retention: What Does the Topic Retain?
Topic definitions declare history retention depth directly:
- `history: 1` (or `history: latest`): The topic retains only the most recently published sample.
- `history: N` (such as `history: 32`): The topic retains the last $N$ publication sequences in a circular handle ring.

Queue semantics—such as consuming or popping items—do not exist at the middleware layer. The topic maintains a shared bounded sequence history. Sizing is known statically before execution.

### 2. Reader Position: Which Retained Sample Does a Consumer Want?
Each consumer navigates the shared history using a lightweight local cursor:
- **`latest()`**: Returns the newest retained sequence currently available. Requires no backlog tracking. Used for real-time perception, control loops, state estimators, and live user interfaces.
- **`next()`**: Advances the consumer's local cursor to the next unread sequence in the history ring. Used by ordered consumers to process every retained sample sequentially.

### 3. Memory Ownership: Does the Application Borrow or Own the Memory?
Memory access is an explicit API choice on each read call rather than a declared system mode:
- **Borrowing (`borrow_latest()`, `borrow_next()`)**: Acquires a transient, zero-copy pointer directly into the shared-memory payload pool protected by a hazard slot. The payload is accessed in-place with zero memory allocation.
- **Copying (`copy_latest()`, `copy_next()`)**: Copies the sample into application-managed memory (such as a local variable, heap object, or NumPy array). The shared-memory slot is released immediately, and the application retains the data indefinitely.

When logging consumers (such as `drm-record`) write topics to persistent storage, they follow an explicit borrow-copy-release lifecycle: an ingress thread borrows the sample (`borrow_next()`), copies it into a bounded recorder-owned memory buffer, and immediately releases the DRM borrow. Background disk threads perform chunking, compression, MCAP framing, and filesystem I/O entirely from recorder-owned memory. The disk thread never touches a DRM shared-memory slot. If slow storage causes the recorder buffer to fill, the recorder enforces its configured policy:
- `on_full: drop`: Discards incoming samples and tracks gap counts prominently in diagnostics, allowing recording to continue across temporary storage bandwidth deficits.
- `on_full: stop`: Aborts recording immediately with an explicit error (`RECORDING_FAILED reason: BUFFER_EXHAUSTED`), preventing corrupted or partially truncated datasets.
- `on_full: backpressure`: Pauses ingress reads so that topic-level history overflow rules (`overwrite` gaps or `reject` errors) naturally govern the publisher without holding shared-memory hazard slots pinned to disk latency.

### 4. History Overflow: What Happens When Retention Cannot Advance?
Because all consumers read from a single shared history, overflow policy belongs strictly to the topic definition:
- **`overwrite`**: The history ring rolls continuously. When capacity is reached, the oldest sequence entry is retired. If a slow sequential consumer falls behind and is lapped by the publisher, DRM returns an explicit `GAP { missed: N }` on the next read. The consumer decides whether to skip to the oldest retained sample or record an overrun diagnostic. This is the default for cameras, LiDAR, IMU, and robot state.
- **`reject`**: Required sequential consumers gate history advancement. If a declared required sequential consumer has not advanced and the history ring is full, `publish()` fails immediately with `QUEUE_FULL`. The publisher decides whether to drop, throttle, or raise an alarm. DRM favors explicit error return over secretly stalling real-time threads.

### 5. Transport Reliability: What Happens Across Physical Links?
Reliability is a property of a physical network route, not a local shared-memory consumer.

Within a single computer, shared memory does not experience packet loss. Delivery integrity is guaranteed structurally by the bounded history ring and hazard slots without running an artificial packet retransmission engine.

When a topic crosses a network boundary (between two computers or microcontrollers):
- The boundary router (`drm-router`) transmits the topic across the link under that route's declared transport contract (`best_effort` or `reliable`).
- Upon arrival at the destination computer, the boundary router writes the samples into that machine's local shared history once.
- All local consumers read from that shared history. DRM never sends duplicate parallel streams across physical wires to satisfy multiple local readers.

## The Core Concepts

Developers manage four foundational concepts:

- **Package**: A folder containing related nodes, topics, and definitions.
- **Node**: An executable program (written in C, C++, Rust, Python, etc.).
- **Topic**: A canonical named channel carrying a statically defined data schema and its shared channel-level retention and overflow rules.
- **Link**: A physical connection between two computers or microcontrollers.

All binary layouts, numeric IDs, memory buffers, routing tables, and language code are generated automatically.

## Modular System Architecture and Implementation Stages

DRM defines modules according to clear separation of responsibility. Having clean architectural boundaries does not mean all modules must be built simultaneously; rather, strong modularity enables a minimal, lightweight core that functions independently while optional subsystems are developed in stages:

### MVP (Minimum Viable Product)
The MVP proves the core DRM thesis end-to-end (independent processes communicating over local shared memory and automatic cross-device links without central brokers or runtime discovery):
- **`drm-core`**: The ultra-lightweight runtime library linked into every process. Contains platform-native shared memory primitives, lockless sequence history rings, hazard-slot payload pools, endpoint opening validation, and C-ABI interfaces. Exposes basic atomic runtime counters (`last_sequence`, `drop_count`, `overrun_count`, `publisher_epoch`, `pool_exhausted`) directly on channels. It has zero dependencies on networking code, disk storage, or compilers.
- **DRM Build Tooling (`drm` tool)**: The unified build-time engine and command-line interface. Parses `.drm` schemas and `drm.toml` manifests, validates node communication contracts, builds the in-memory `WorkspaceModel`, generates language bindings, packages deployments, and provides essential developer inspection (`drm build`, `drm verify`, `drm deploy`, `drm run`, `drm echo`, `drm info`). Implemented initially as a single tool rather than separate compiler and CLI products.
- **`drm-router`**: The boundary gateway daemon. Runs as an independent process on computers connected to physical links. Subscribes to cross-device topics in local shared memory and handles framing, priority queues, and rate throttling across external hardware.

### Next (Persistence and Diagnostic Aggregation)
- **`drm-record`**: The logging and replay subsystem. It consumes topics as a normal sequential reader, briefly borrows each sample, copies it into a bounded recorder-owned buffer, and immediately releases the DRM borrow. Independent writer threads perform chunking, compression, and disk I/O from recorder-owned memory, so slow storage never retains DRM transport buffers or delays publishers.
- **`drm-diag`**: Out-of-band monitoring daemon. Aggregates and interprets basic runtime counters, arrival deadlines, process health, and hardware link statistics without imposing overhead on the real-time data path.

### Later (Specialized Robotics Capabilities)
- **`drm-rpc`**: Request-response services and long-running, cancellable action workflows. Implemented as a clean library layer built on top of correlated topic pairs without complicating the core ring buffers.
- **`drm-trace`**: Structured diagnostic event tracing through non-blocking, best-effort ring buffers without stalling publishers.
- **`drm-spatial`**: The spatial-temporal transform library (rigid $SE(3)$ transforms, clock domains, static collapse, and versioned local caches). Dedicated to high-rate geometric transforms and deferred until the core communication toolchain is complete.

## Topic Definitions: Data Shape and Bounds

Topic definitions primarily describe the shape of the data. The file path defines the topic name.

For example, a file at `packages/control/topics/target.drm` defines the topic `control/target`:

```yaml
speed: f32
steering: f32
enabled: bool
```

### Separating Data Identity from Timing
A topic defines what data channel exists, not how frequently it is published. Data identity, producer cadence, route constraints, consumer freshness, and runtime measurement are five independent concepts:

> **A topic defines what data channel exists. Publishers describe how they expect to produce it. Routes describe how it is budgeted or throttled across physical links. Consumers describe how fresh they need it. Runtime tells you what actually happened.**

- **Topic (Channel Definition)**:
  Owns canonical identity (`control/target`), schema layout, structural bounds, shared history retention depth, and overflow policy (`overwrite` or `reject`). Topics do not declare update rates. Event-driven topics (such as fault triggers, emergency stops, button presses, or state changes) publish sporadically or in bursts and have no natural frequency; forcing a rate field onto topic definitions creates misleading metadata.

- **Publisher (Nominal Planning Cadence)**:
  Publishers declare their expected generation cadence as planning metadata in node manifests:
  ```toml
  [publish."control/target"]
  expected_rate = "100Hz"
  ```
  `expected_rate` represents nominal intention used by build tools to estimate buffer depths and link bandwidth. It is not an enforceable compiler guarantee. Hard capacity bounds (`max_rate` or `min_interval`) are recognized only if physical hardware or gateway shapers enforce them.

- **Route (Transport Throttling and Budgets)**:
  Physical links may budget or throttle topics independently without modifying topic definitions or local publisher rates. For example, a boundary router can export a 60 Hz local camera feed across a low-bandwidth radio link at `export_limit = "10Hz"`.

- **Consumer (Freshness and Deadlines)**:
  Consumers declare independent timing requirements based on their operational needs:
  ```toml
  [consume."control/target"]
  deadline = "20ms"
  ```
  A fast motor controller may require a tight 20 ms deadline on `/control/target`, while a diagnostic telemetry dashboard reading the same topic accepts a 500 ms deadline. DRM separates publisher capability from consumer requirement.

- **Runtime Reality (`drm-diag`)**:
  Actual publication frequencies (`observed_rate`), deadline misses, and latency jitter are measured continuously and non-intrusively at runtime by `drm-diag`.

### Canonical Identity, Runtime Handles, and Transport Identifiers
DRM operates primarily as a high-performance intra-computer data fabric on Linux and Windows, connecting to other computers and embedded devices over standard network links (Ethernet UDP, TCP, serial). It separates data identity from local dispatch and physical wire addressing across three distinct concepts:

> **Identity says what the data is. Transport configuration decides how that data travels across a physical link.**

- **The Three Distinct Concepts**:
  - **Topic Identity**: The canonical string path (such as `control/target` or `camera/front`). It defines what the data channel represents across the entire robot and serves as the single source of truth for schema matching and contract verification.
  - **Runtime Handle**: A generated compact integer (such as `0x0124`) used internally for fast local runtime dispatch, shared-memory indexing, or multiplexed network stream identification without string lookups.
  - **Transport Identifier / Address**: A wire-specific framing identifier determined by physical network backends (such as UDP port and stream numbers, TCP socket endpoints, or serial frame channel headers).

- **Decoupling Wire Addresses from Topic Handles**:
  Transport-specific identifiers are independent of DRM topic handles. A DRM topic retains its canonical topic identity regardless of transport. Physical transport backends assign network streams or serial channels as part of deployment routing. DRM never conflates generic runtime handles with physical wire addresses, ensuring an arbitrary topic number never dictates network routing or priority.
  - **External Device Interoperability**: When interfacing with external hardware devices or existing protocols with fixed wire IDs (such as an embedded microcontroller streaming over serial or UDP), fixed transport addresses can be declared in deployment route configuration (`[route."motor/command".serial] channel = 2`). This explicitly denotes an external protocol requirement rather than an internal DRM topic identity.
  - **Decoupled Framing and Packing**: A wire frame does not mandate a one-to-one relationship with a single DRM topic. The router backend can multiplex multiple small DRM topics into a single network packet, or fragment a larger topic payload across multiple sequenced network datagrams.

- **Stability Across Normal Builds**:
  Standard builds (`drm build`, `drm dev`) preserve previously assigned numeric handles to maintain binary compatibility and prevent cascading rebuilds across nodes.

- **Explicit Handle Compaction**:
  Only an explicit maintenance command (`drm ids compact`) recalculates and packs numeric IDs, requiring a coordinated rebuild and deployment across all nodes.

### Structural Bounds vs Semantic Annotations
DRM separates communication requirements from optional application validation:

- **Structural Bounds**: Define the maximum legal limits of a topic payload:
  ```yaml
  name: string[32]
  samples: f32[16]      # Fixed-size array
  history: f32[<=16]    # Bounded variable array (up to 16 items)
  ```
  Structural bounds are used for schema validation, wire encoding limits, network link feasibility analysis, and hard-real-time pool sizing. A declared upper bound does not mandate that a general-purpose workstation must eagerly reserve the entire maximum theoretical capacity at process startup. On an MCU or hard-real-time node, bounds translate into fixed pre-allocated memory; on general-purpose host workstations, memory pools are allocated and reused based on configured budgets and active demand up to the declared bound.
- **Semantic Annotations**: Optional metadata used for documentation, developer tooling, and UI generation:
  ```yaml
  speed:
    type: f32
    range: [0, 40]
    unit: m/s
  ```
  Semantic annotations do not alter default binary encoding. Standard types remain uncompressed (`f32` stays a 4-byte IEEE float) unless explicit quantization is specifically requested for a constrained bus under an `encoding:` rule.

- **Enums**:
  ```yaml
  mode:
    enum:
      - disabled
      - manual
      - autonomous
  ```

- **Two-Plane Architecture (Data Plane vs Observability Plane)**:
  DRM explicitly separates operational data transfer from system diagnostics:
  - **Data Plane (Strictly Static)**: The core robot communication path requires statically declared schemas with strict bounds. Dynamic schema-less messages are prohibited because they require runtime memory allocators and reflection, which breaks deterministic real-time execution. When an application manages variable instances (such as multiple tracked objects or dynamic game entities), it uses fixed schemas paired with an instance identifier field (`source_id: u32`).
  - **Bounded Opaque Payload Escape Hatch**: For third-party device packets, raw serial streams, or unparsed JSON pass-through, topics declare bounded byte arrays:
    ```yaml
    payload: bytes[<=4096]
    ```
    DRM treats this strictly as an opaque memory buffer with a pre-allocated maximum capacity. It performs no runtime field parsing, dynamic resizing, or string inspection.
  - **Observability Plane**: Diagnostic events and system traces run in an independent, lightweight observability plane rather than inside the critical control path. Observability ring buffers are strictly non-blocking and best-effort; when a recorder, telemetry tool, or user interface falls behind, diagnostic events are dropped immediately rather than exerting backpressure on real-time publisher loops.
  - **Staged Implementation**: Phase 1 provides simple application logging (`drm.log(level, message)`) and standard static topics. Phase 2 introduces full compiled event tracing (`drm-trace`), where event names and format strings are compiled into a build-time metadata dictionary, emitting only small numeric event IDs and arguments at runtime.

## DRM Spatial (Integrated Module for Future Implementation)

DRM Spatial is an integrated spatial-temporal library within DRM (`drm-spatial`), designed to replace dynamic transform trees like tf2. It is planned for development following the core data middleware. It builds directly upon DRM's shared-memory data fabric and compiler toolchain:
- Spatial relationships are declared once in `spaces/`.
- Static structures are collapsed into precompiled constants at build time.
- Authoritative runtime memory stores only dynamic edges with lockless version counters.
- Consumer nodes declare their history requirements, allocating ring buffers only where required.
- Dynamic transforms cross physical links only when remote devices demand them.

Core data communication remains DRM's primary immediate focus. For the complete specification, see the dedicated [DRM Spatial Module Architecture](file:///e:/doc/drm/docs/spatial_architecture.md).

## Workspace Layout and Node Declarations

Developers edit small local manifests. They never maintain a central registry manually.

```text
my_robot/
├── drm.toml          # Hardware topology and link definitions
└── packages/
    ├── control/
    │   ├── drm.toml  # Package metadata
    │   ├── topics/
    │   │   └── target.drm
    │   └── nodes/
    │       └── controller/
    │           ├── drm.toml
    │           └── src/
    └── planning/
        ├── topics/
        │   └── trajectory.drm
        └── nodes/
            └── planner/
                ├── drm.toml
                └── src/
```

A node contains only its own local communication requirements:

```toml
# packages/planning/nodes/planner/drm.toml
[node]
language = "python"
publish = ["control/target", "planning/trajectory"]
consume = ["localization/pose"]
```

```toml
# packages/control/nodes/controller/drm.toml
[node]
language = "cpp"
consume = ["control/target"]
publish = ["actuator/command"]
```

Developers can modify these manifests by hand or through the CLI (`drm assign control/target --consume controller`). The node manifests remain the single source of truth.

## Hardware Topology and Physical Boundaries

Data never leaves a computer unless it needs to cross a physical link to another device.

The hardware topology is defined in `drm.toml`:

```toml
[compute.jetson]
nodes = ["planner"]

[compute.robot_pc]
nodes = ["controller"]

[links."jetson <-> robot_pc"]
transport = "ethernet"
bandwidth = "1Gbps"
```

### Automatic Routing and Explicit Overrides
DRM has one routing model: routes are derived automatically from the declared communication graph and physical topology:

> **Infer normal behavior from the declared system graph. Add configuration only where inference cannot express intent.**

For each topic, DRM traces the graph from publisher placement to consumer placements across the physical link topology:
- **Automatic Route Derivation**: If a single viable physical path connects publisher and consumer, DRM routes the topic automatically. If both publisher and consumer reside on the same compute unit, the topic remains strictly local in shared memory. No manual route or export declarations are required.
- **Explicit Overrides for Multi-Path Ambiguity**: When multiple physical paths exist between devices (for example, primary Ethernet and a secondary Wi-Fi or serial link), explicit configuration exists only to constrain or override automatic routing when intent cannot be inferred uniquely:
  ```toml
  [routing]
  prefer."control/target" = "ethernet"
  deny."camera/front" = ["wifi"]
  force."diagnostics" = "ethernet"
  ```
- **Ambiguity Detection**: If multiple valid physical routes exist without an explicit preference, `drm verify` reports an ambiguous route error at build time rather than silently choosing a path.
- **External Endpoints**: When sending data to or receiving data from unmanaged third-party devices (such as an embedded microcontroller streaming over serial or an external telemetry tool), the remote device is modeled explicitly as an external endpoint:
  ```toml
  [external.dashboard]
  link = "ethernet"
  consume = ["diagnostics"]
  ```
  This integrates third-party devices directly into the unified communication graph without maintaining parallel export lists or separate routing tables.

## The Boundary Router

When data travels between separate physical devices, the **`drm-router`** daemon runs on each computer to bridge local shared memory with external physical links:

- **Topic Filtering**: Transmits only topics mapped to remote consumers across that specific physical link.
- **Framing and Transport Capabilities**:
  - **Single-Packet UDP for Real-Time Telemetry**: To avoid reimplementing custom network stacks, reassembly, or congestion control in the MVP, UDP topics are strictly sized to fit within a single physical network datagram ($\le$ link MTU, typically 1400–1500 bytes). UDP routes provide low-latency, best-effort delivery.
  - **TCP and Dedicated Streams for Large or Reliable Data**: Payloads larger than MTU (such as video frames, point clouds, or recorded logs) and topics requiring guaranteed, in-order delivery across physical devices use standard stream transports (TCP or dedicated bulk sockets) that provide battle-tested fragmentation, retransmission, and flow control.
  - **Transport Reliability Enforcement**: Reliability is an adapter capability of the physical link. Local shared memory and TCP are reliable; UDP and serial are best-effort. If a node contract declares `reliable` delivery for a remote topic, `drm verify` ensures the route maps to a reliable transport adapter; attempting to assign `reliable` delivery over raw UDP without a reliable transport adapter fails at compile time.
- **Priority Scheduling**: Prioritizes mission-critical safety and actuator commands ahead of telemetry and diagnostics.
- **Traffic Shaping and Bandwidth Budgeting**: Enforces configured rate limits and token buckets on physical links to prevent unmanaged packet bursts from congesting network interfaces.
- **Two-Way Import**: When an external microcontroller or companion computer sends sensor data across a link, the local boundary router receives it and writes it directly into the local shared-memory domain. Local nodes consume it identically to any local topic without knowing it crossed a physical boundary.

Physical link disconnections affect only topics traveling across that specific link. All local processes continue communicating across shared memory with zero interruption.

## The DRM Compiler Architecture

DRM separates communication dependencies from application compilation dependencies. The foundational rule is:

> **DRM understands communication dependencies, generated DRM artifacts, placement, and deployment. It does not understand application compilation dependencies.**
>
> **DRM compiles DRM declarations, not application source code.**

By delegating application compilation entirely to native build systems (Cargo, CMake/Ninja, Go toolchain, MSBuild), DRM avoids reinventing source-file tracking, header scanning, compiler flags, and incremental object-file dependency engines.

### In-Memory Workspace Model
Instead of maintaining complex persistent binary caches, DRM parses lightweight manifests on demand into an in-memory `WorkspaceModel`:
- Topics and their data schemas
- Nodes and declared publish/consume contracts
- Packages and logical groupings
- Compute units and execution placement
- Physical links and hardware topology

Because DRM manifests are small, parsing the entire workspace model takes milliseconds. During development watch sessions (`drm dev`), the `WorkspaceModel` remains alive in memory, eliminating cache versioning bugs, cache corruption, and stale file invalidation issues.

### Primary Domain Relationships
DRM discards artificial compilation edges (`BUILD_REQUIRES`, `USES_SCHEMA`, `USES_BINDING`). It maintains only core domain relationships:
- `Node --publishes--> Topic`
- `Node --consumes--> Topic`
- `Node --placed_on--> Compute`
- `Compute <--link--> Compute`
- Package containment and schema imports

Generated bindings, node communication interfaces, and boundary routes are computed directly from these core relationships when needed. Because communication edges are completely independent of compilation, communication loops between nodes do not create build cycles.

### Clear Division of Responsibilities
DRM strictly bounds its responsibilities:

- **What DRM Owns**:
  - Parsing `.drm` schemas and validating topic types
  - Generating language bindings and node communication interfaces
  - Computing boundary routing tables and verifying link bandwidth
  - Generating deployment artifacts and hardware bus configurations
  - Providing unified developer commands (`drm build`, `drm dev`, `drm deploy`)

- **What Native Build Systems Own**:
  - Header dependencies (`.cpp` to `.h`), Rust crate dependencies, and Go imports
  - Determining which object files need recompilation
  - Compiler flags, optimization levels, and native toolchains
  - Linking binaries and native build caching

### DRM as Orchestrator (`drm build`)
Commands like `drm build` act as high-level orchestrators rather than compiler engines:
1. Parse DRM declarations into the in-memory `WorkspaceModel`.
2. Generate or update affected DRM artifacts (language bindings, contracts, routing tables).
3. Verify communication contracts and compatibility.
4. Invoke the node's native build adapter.

Node manifests configure their native build tool via simple adapters:

```toml
# packages/control/nodes/controller/drm.toml
[node]
language = "cpp"
consume = ["control/target"]
publish = ["actuator/command"]

[build]
system = "cmake"
target = "controller"
```

For Rust nodes:
```toml
[build]
system = "cargo"
package = "controller"
```

Custom or unusual environments can supply an explicit command directly:
```toml
[build]
command = ["my-custom-builder", "--target", "controller"]
```

### Stable Shared Bindings with Internal Invalidation Fingerprints
Generated bindings use stable, predictable, topic- and language-based directory structures:

```text
.drm/gen/c/control/target/target.h
.drm/gen/cpp/control/target/target.hpp
.drm/gen/rust/control/target/mod.rs
.drm/gen/python/control/target/target.py
```

Each topic binding is generated once per language and shared by every node that uses it. If ten C++ nodes across different packages consume `ControlTarget`, they all compile against `.drm/gen/cpp/control/target/target.hpp`.

### Node Communication Contracts as Architectural Capabilities
A node's `publish` and `consume` declarations in its manifest are the single source of truth for its DRM interface. DRM uses these declarations as an architectural correctness mechanism rather than a separate security-permission subsystem:

- **Static Languages (C++, Rust)**: The generated API for a node exposes only the topics declared in that node's manifest. If `controller` declares `publish = ["actuator/command"]` and `consume = ["control/target"]`, calling `drm::camera::front::publish(...)` fails at compile time because that method is not included in the node's generated interface.
- **Dynamic Languages (Python)**: Runtimes expose generated topic objects matching node declarations. Attempting to publish or consume an undeclared topic raises an immediate runtime error (`ContractError`).
- **Low-Level C ABI and Endpoint Handles**: When using the raw C runtime ABI, access validation occurs once when opening an endpoint handle at startup (`drm_publisher_open(...)`). If the node manifest permits the topic, an authorized handle is returned. Once the handle exists, steady-state message publication (`drm_publish(handle, bytes)`) executes with zero per-message permission overhead.
- **Architectural Correctness, Not Security Sandboxing**: DRM prevents accidental communication outside a node's declared interface. This is an architectural correctness mechanism, not an operating-system security boundary against malicious code. Separate per-node `.contract` policy files are not required; compatibility fingerprints and endpoint validation tables are derived directly from the node manifest and deployment metadata.

DRM manages generation using lightweight invalidation metadata rather than content-addressed directory names:
- **Internal Fingerprints**: DRM stores a small fingerprint of generation inputs:
  `fingerprint = hash(canonical_schema, topic_identity, generator_version, options)`
  If the stored fingerprint matches and the expected output files exist on disk, regeneration is skipped.
- **Atomic Writes**: Bindings are written to temporary files and atomically renamed upon completion. A canceled generator never leaves half-written or corrupt header files.
- **Timestamp Preservation**: If generated code is identical to the existing file on disk, DRM does not rewrite it. Preserving the modification timestamp prevents native build tools (CMake/Ninja, Cargo) from triggering unnecessary rebuilds of dependent application code.
- **Direct Reconciliation**: When a topic is deleted from workspace declarations, DRM deletes its corresponding logical directory directly. There are no orphan hash directories accumulating on disk and no disk garbage collection passes required. Content-addressed storage (CAS) is deferred as an optional remote CI/cloud cache feature.

### Development Watch Mode (`drm dev`)
The filesystem watcher monitors changes without tracking header dependencies or object freshness. It handles only two distinct event categories:
- **DRM Declaration Changed** (`.drm` schema, `drm.toml` manifest, topology): DRM recompiles the schema, regenerates affected language bindings and contracts, and invokes the native builder for affected nodes.
- **Application Source Changed** (`.cpp`, `.rs`, `.py`, `.go`): DRM identifies which node owns the file and immediately invokes that node's native build adapter. CMake/Ninja or Cargo decides which object files require recompilation.

### Verification and Runtime Observation: Facts, Feasibility, and Reality
A static build tool cannot honestly prove runtime timing, execution jitter, or that a network bus will never experience transient bursts. DRM maintains a strict boundary between static proof, feasibility estimation, and runtime measurement:

> **DRM verifies facts it can prove; it estimates feasibility from declarations; it observes actual behavior at runtime.**
> **Compile-time DRM proves configuration. Runtime DRM measures behavior. Never present an estimate as a guarantee.**

- **What `drm verify` Proves (Deterministic Facts)**:
  - All topic schemas are valid and consistent across packages.
  - Every consumed topic has exactly one declared publisher across the domain (enforcing Single-Writer, Multiple-Reader).
  - Placements, compute units, and physical link topologies are valid without conflicting identifiers.
  - Cross-device topic routes exist and data shapes fit target link framing rules (e.g. UDP payloads $\le$ MTU).
  - Transport adapter capabilities match delivery requirements (rejecting `reliable` contracts assigned to best-effort UDP/serial links).
  - Worst-case static capacity budgets across all active channels fit target machine physical memory limits.
  - Consumer delivery requirements on shared topics are mutually compatible.

- **What `drm verify` Estimates (Declared Feasibility)**:
  - Nominal link utilization based on declared `expected_rate` values and packet sizes (for example, declaring that nominal UDP telemetry consumes 12.4 Mbps of a 100 Mbps link with 87.6% headroom).
  - Feasibility flags impossible configurations (such as declared traffic exceeding 100% of link capacity) as build errors, but treats nominal link headroom as an engineering estimate, not a mathematical guarantee that runtime packet bursts or packet transmission delays will never occur.
  - Publisher `expected_rate` declarations are planning metadata used for buffer sizing and bandwidth estimation, not a compiler guarantee that user application code will sustain that rate without jitter.

- **What `drm-diag` Measures (Runtime Reality)**:
  - Runtime health, timing behavior, and link statistics are observed on running nodes by the `drm-diag` subsystem out-of-band:
  - Actual publication frequencies (`observed_rate`) versus declared planning rates (`expected_rate`).
  - Observed sample gaps and violations of declared consumer deadlines (`deadline: 50ms`).
  - Observed link utilization, packet bursts, retries, hardware errors, and queue overruns.

### Tiered Developer Commands
- **`drm build`**: Fast local build that compiles DRM declarations, generates topic bindings, and invokes the node's native build adapter.
- **`drm verify`**: System-wide static validation that proves configuration correctness and reports link feasibility estimates without claiming to prove runtime timing.
- **`drm deploy`**: Executes full static verification, packages node binaries, and emits hardware routing tables.

## Application Code Experience

Because the build toolchain generates tailored APIs for each node, application code remains simple, strongly typed, and safe.

In C (microcontroller actuator driver):
```c
struct ControlTarget target;
if (drm_read_control_target(&target)) {
    apply_motor_speed(target.speed);
}
```

In Python (`planner`):
```python
from drm import topics

# Publish control target
target = topics.control.target(speed=5.2, steering=0.1)
target.publish()

# Safe owned copy
status = topics.robot.status.latest()

# Scoped zero-copy borrow
with topics.sensor.camera.borrow_latest() as frame:
    detect(frame.numpy())
```

In C++ (`controller`):
```cpp
// Scoped zero-copy borrow
if (auto target = drm::control::target::borrow_latest()) {
    float speed = target->speed;
    float steering = target->steering;
}

// Or an owned copy
auto target_copy = drm::control::target::copy_latest();
```

In Rust (`state_estimator`):
```rust
// Scoped zero-copy borrow of latest sample
if let Some(target) = drm::control::target::borrow_latest() {
    let speed = target.speed;
}

// Sequential cursor read with explicit gap detection
match drm::control::target::borrow_next() {
    Ok(sample) => {
        let speed = sample.speed;
    }
    Err(drm::Gap { missed }) => {
        eprintln!("Missed {missed} samples");
    }
}
```

The generated API knows the canonical topic path, data type, binary layout, and declared communication interfaces. Transport-specific handles and wire IDs belong to deployment routing tables and the boundary router rather than application binaries. Undeclared communication operations are prevented by the generated API in static languages or rejected during endpoint opening. Programmers never manually handle channels, binary offsets, serialization, or network protocols.

## Tooling and Introspection

Because runtime has no master node or discovery registry, debugging tools (`drm echo`, `drm hz`, `drm log`) use the generated deployment metadata in `.drm/deploy/deployment.json` to inspect the running system:

- Tools read topic definitions, binary structures, and shared memory names directly from deployment metadata.
- Tools attach to shared memory as read-only observers without altering system behavior or adding network overhead.
- When inspecting bulk topics, diagnostic tools take a momentary borrow (`borrow_latest()`), copy the sample to private memory, and release the hazard slot immediately. They never hold prolonged shared memory borrows, ensuring developer tools cannot freeze production robot buffers.
- Developers can view data rates, latency, and memory utilization live without deploying extra daemon processes.

## Fault Isolation and Recovery

Nodes operate independently without centralized failure cascades:

- **Crash Isolation**: If a consumer or publisher crashes, other programs continue running unaffected. An operating-system-backed writer lock immediately releases publisher authority on termination.
- **Controlled Reconnect**: A restarted publisher reconnects to its deployment-scoped shared-memory segment, increments its publisher epoch, and resumes communication. Consumers observe the epoch change and reset their sequence expectations cleanly without mistaking stale samples for new data.
- **Deployment Independence**: Deployments never adopt or migrate segments across updates. New deployments launch in their own isolated namespaces, allowing atomic cutovers without complex in-place shared-memory transformations.
- **Network Resilience**: Loss of a physical connection affects only remote topics traveling across that link. All local control loops continue without interruption.
