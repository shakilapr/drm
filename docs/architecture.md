# DRM Architecture

## Requirements Language

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all capitals.

## Architecture Description Conventions

This document describes the system-of-interest (DRM middleware), its stakeholders, their concerns, and the views addressing them; rationale for significant decisions lives in `docs/adr/`. Conventions: `docs/standards.md`.

DRM is a decentralized data middleware for robots. Its primary duty is to move data between programs—such as camera frames, LiDAR point clouds, sensor measurements, and motor commands—without relying on a central broker, master node, or runtime network discovery.

Applications may be written in any language capable of binding DRM's canonical C ABI. DRM's MVP provides first-party bindings for C, C++, Rust, and Python. Applications declare what topics they send and receive; DRM handles how that data is packaged, transported, and delivered.

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
  - **Default Triple-Buffering for Latest Data**: For `@history: latest` bulk topics ($R = 1$), DRM defaults to a pool of 3 slots (one for the latest completed sample, one for the active publisher write, and one for an active reader borrow). In robotics workloads, multiple concurrent readers (e.g. perception, localization, UI) typically read the same latest frame simultaneously, sharing protection on that single slot.
  - **Configurable Pool Size (`@pool_size: N`)**: When downstream pipelines require holding multiple distinct historical frames concurrently without blocking the publisher, the topic definition can explicitly declare `@pool_size: N` per the payload pool invariant below. Control loops requiring absolute drop immunity under worst-case consumer delay with $H$ independent borrowers configure $R + H + 1$ slots (for `@history: latest`, $H + 2$).

- **Safe Handling of Pool Exhaustion**:
  If readers hold borrows across all available slots and exhaust the pool, DRM preserves real-time guarantees:
  - Active borrowed slots are never overwritten or corrupted.
  - The publisher thread never blocks waiting for slow readers.
  - The publisher drops the incoming sample, increments an atomic `pool_exhausted` counter for `drm-diag` reporting, and preserves the current latest sample. Once a reader finishes or dead-endpoint reclamation returns a slot, publishing resumes instantly.

- **Reader Cursor vs Hazard Slot**:
  - **Reader Cursor**: Tracks logical sequence progress (*Which sequence should I read next?*). Maintained locally by each consumer.
  - **Hazard Slot**: Tracks physical slot protection (*Which physical payload slot am I currently reading and therefore must not be recycled?*). Each declared zero-copy consumer record — one per declared borrower — has one dedicated, naturally aligned hazard word in shared memory: `protected[consumer_id] = slot` or `NONE`. The word is a single atomic so protection can never be observed half-written; the slot generation used for ABA rejection lives in the atomically published ring entry, not in the hazard word. A consumer endpoint has at most one outstanding borrow (one hazard word in use at a time).
  - If a consumer continues borrowing a sample after its sequence has aged out of the $R$-depth history ring, that physical slot remains protected against reuse. The publisher continues advancing using the remaining free slots in the pool.

- **Lockless Consumer Acquisition Protocol**:
  1. The consumer reads the desired sequence handle from the topic sequence ring: `entry1 = history[target_seq % R]`.
  2. The consumer publishes its protection intent: `protected[my_id] = entry1.slot`.
  3. The consumer rereads the sequence handle: `entry2 = history[target_seq % R]`.
  4. If `entry1 != entry2` or `entry2.sequence != target_seq`, the publisher rolled the slot during acquisition. The consumer clears its protection (`protected[my_id] = NONE`) and handles the sequence change or retries.
  5. If `entry1 == entry2`, the slot is safely locked against reuse. The consumer processes the immutable payload directly from shared memory.
  6. Upon leaving the borrow scope, the consumer clears its protection: `protected[my_id] = NONE`.
  The generation counter prevents ABA hazards when slot numbers wrap around.

  Steps 1–4 and the publisher's slot-retirement/reclamation scan use **sequentially consistent (SeqCst)** ordering for the control handshake, so the reader's protection publication and the publisher's retirement cannot mutually miss each other (a Release/Acquire pair does not forbid this publication/reclamation race). Ordinary payload writes and reads remain Release/Acquire: the payload is published with a release commit and read after an acquire. A consumer endpoint has **at most one outstanding borrow**; a second concurrent borrow on the same endpoint is rejected (`Busy`) rather than overwriting its own hazard slot. The slot generation counter is 64-bit. **One acquisition primitive serves all four public read operations** (`borrow_latest`/`copy_latest`/`borrow_next`/`copy_next`): each resolves a target sequence, then runs this exact SeqCst protect/recheck handshake. `latest()` chooses `write_seq − 1` as the target; no entry-passing side path exists, so the single handshake is the single proof.

  Each ring position contains two alternating, naturally aligned versioned banks, not a language-specific aggregate atomic. A bank's sequence, slot, and 64-bit generation fields are atomic; its stamp is odd while written and even when stable. Sequence lap parity selects the bank, so preparing a new handle never destroys the prior retained handle before `write_seq` commits. Readers accept a snapshot only when two stamp reads match and are even. SeqCst snapshot reads are used by the hazard handshake; ordinary lookup may use acquire ordering. Activation refuses platforms where the required 32-bit and 64-bit shared atomics are not lock-free.

- **Publisher Slot Selection**:
  The publisher owns slot reuse decisions without locks or reference counts. After acquiring the per-topic non-blocking publish guard, the current history is stable because there is one writer. It selects a slot that is not referenced by that history and is not protected by any consumer hazard word. A slot leaves the referenced set when an earlier commit replaces its ring entry; before later reuse, the publisher scans every hazard word with SeqCst ordering. A reader that observed the old entry either published protection before that scan, causing the publisher to skip the slot, or publishes protection afterward and rejects the changed entry on its SeqCst reread before touching payload bytes. The publisher then writes the selected payload, increments its 64-bit generation, commits the versioned ring cell, and release-advances the write sequence.

- **Consumer Registration and Reuse**:
  Consumer records and their lock names are fixed at activation. Attachment first acquires the endpoint's non-blocking OS registration lock, which the kernel releases on process death, then publishes a monotonically increasing registration generation, incarnation-safe process identity, and hazard word packing `{registration_generation, slot | NONE}`. No data-path lock or allocation is involved. The record lifecycle is `Vacant → Claiming → Active → Closing → Vacant`. A borrow may be created only from `Active`; a new borrow that installs its hazard concurrently with a close loses and aborts (it rechecks `state == Active && registration_generation == my_generation` after installing the hazard and before touching payload bytes). Clean close first transitions `Active → Closing` atomically — no new borrow may start behind a `Closing` record — then completes only with no outstanding borrow; a borrow observed at that point keeps the record in `Closing`, the guard's release clears the exact hazard word, and the idempotent re-close completes the transition. Registration takeover uses kernel lock release as exact owner-death/close proof. Takeover, close, and reclamation replace or clear the complete generation-bearing hazard word, so a stale guard or reclaimer can never clear a replacement owner's hazard. **Hazard protection is a property of the word, not the record state**: a word protects its slot whenever it names the record's current registration generation and a slot — a `Closing` record with a live borrow still protects; a record that died in `Claiming` carries an outdated-generation word, which protects nothing and pins nothing until a lock holder overwrites it.

- **Crash Recovery via OS Registration-Lock Authority**:
  If a consumer process crashes while holding a borrow, that slot is not leaked permanently. DRM avoids arbitrary timeout guesses and avoids constant polling; reclamation runs on a slow path only when needed: exhaustion or maintenance trigger → acquire the dead endpoint's registration lock non-blockingly → under that lock reread state/generation/hazard and clear the exact abandoned hazard word → release. Acquiring a registration lock proves no live DRM owner retains authority (the lock is held for the endpoint's whole registration lifetime and released by the kernel on death), so lock acquisition — not process inspection — is the sole liveness proof. Process identity (`pid` plus process start identity) stored at registration is **diagnostic metadata only**; it never gates reclamation. In particular PID reuse and cross-PID-namespace visibility (a reclaimer may see `ESRCH` for a live process in another namespace) make identity-based liveness unsafe as an authority primitive. Reclamation never runs process inspection on a hard-real-time publisher thread: an RT-profile publisher that finds the pool exhausted increments its drop counters, sets a reclamation-requested flag, and returns; a non-RT maintenance context performs the scan. General-purpose profiles may opportunistically run one bounded lock-probe scan inline as an optimization.

- **Process Lifecycle (fork)**:
  On POSIX, `flock` locks belong to the open file description, so a `fork()` child inherits every DRM lock and mapping. DRM runtime objects (publishers, registrations, borrows, guards) are valid only in the process that created them. The runtime registers `pthread_atfork` handlers that invalidate every inherited DRM handle in the child (via a per-process incarnation cookie) and close inherited lock descriptors without issuing `LOCK_UN`; all lock descriptors are opened `FD_CLOEXEC`. A guard or registration that detects an incarnation-cookie mismatch never mutates shared state. Public contract: after `fork()`, inherited DRM objects are invalid in the child — the child must `exec` or reattach before using DRM. This matters for language runtimes with implicit fork (for example Python `multiprocessing`). Hard-real-time processes must not fork after activation.

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
  - **Scoped Shared-Memory Namespaces**: All shared-memory paths are scoped by this identifier. Logical topic paths never reach kernel-object namespaces: backend object names are generated opaque names derived from the deployment ID plus the topic's **compact handle — its deployment-local topic slot** in the committed handle table (`.drm/ids.json`); topic paths are **never hashed**, so no collision can make a legitimate channel uninstantiable (`/drm.<deployment_id>.<topic_slot>` on POSIX, `Local\drm_<deployment_id>_<topic_slot>` on Windows, `/run/drm/<deployment_id>/t_<topic_slot>.*.lock` lock files), because POSIX SHM names permit only a leading slash and Windows mapping names forbid backslashes after the `Local\`/`Global\` prefix. The canonical topic string remains in the immutable descriptor for validation.
  - **Startup Compatibility Guard**: At startup, `drm-core` supplies the deployment context to the node. The node checks that its compiled contract fingerprint matches the expected fingerprint recorded in `deployment.json`. If a developer attempts to run an outdated binary against an incompatible deployment, the node refuses to start immediately.
  - **Individual Segment Validation**: When a process attaches to an individual topic segment, it performs a secondary check verifying that the segment's compiled schema and layout descriptor match its expectations.
  - **No Cross-Deployment Adoption**: A new deployment always creates its own isolated shared-memory namespace. DRM never attempts live schema migration, dynamic segment resizing, or adoption of segments from an older deployment.
  - **Safe Cleanup via OS-Backed Lock**: Each deployment directory holds an OS-backed active lock (`/run/drm/<deployment_id>/active.lock`). Running nodes hold a shared read lock. The `drm clean` command attempts an exclusive lock: if the lock succeeds, no nodes are using the namespace and it is safely unlinked; if it fails, the deployment is active and left untouched.

- **3. Publisher Epoch (Runtime Shared State)**:
  Inside a deployment namespace, each topic segment maintains an atomic `publisher_epoch` counter in shared memory.
  - When a publisher starts or restarts within the same deployment, it reopens the compatible existing segment, verifies the header descriptor, and increments the epoch (for example, from 7 to 8).
  - Before publishing its first new sample, the restarted publisher advances a shared `first_valid_seq` watermark past every sequence from the previous run. The addressable window is `max(write_seq - R, first_valid_seq)`, so samples of the previous run are never addressable as current even though sequence numbering continues monotonically for gap accounting.
  - Consumers observe the epoch change and reset local reader state, ensuring samples from the previous publisher run are never misinterpreted as observations from the new run. Each consumer **persists the observed epoch across read calls** (it is part of reader state, not a per-call local), so a restart that begins and completes entirely between two calls is detected at the start of the next call; the in-call epoch reread additionally discards any acquisition a restart crossed.

- **Single Publisher per Topic (Single-Writer, Multiple-Reader)**:
  - Within a DRM domain, every topic has exactly one publisher node. Multiple nodes publishing to the same canonical topic is disallowed and rejected at build time by `drm verify`.
  - Single-writer authority guarantees lockless sequence integrity, eliminates multi-writer arbitration in shared memory, ensures unambiguous data provenance, and makes routing fully deterministic.
  - **One in-progress write per topic**: a single publisher node may use multiple threads, but only one publish may be in progress at a time. A non-blocking `Busy` guard rejects a concurrent publish (`Status::Busy`) rather than blocking or corrupting the in-progress slot. This is what makes the payload-pool invariant `pool_size >= R + H + 1` (one in-progress write) sound.
  - When an application requires converging multiple data sources (such as dual joysticks or alternate planning nodes), sources publish to distinct topics (`control/target_teleop`, `control/target_nav`). An explicit multiplexer node selects the active input and publishes the authoritative stream (`control/target`).

- **Segment Creation and Writer Authority**:
  - The single topic publisher creates the shared-memory segment atomically (`O_CREAT | O_EXCL` or equivalent). Consumers wait and open existing segments; consumers never create topic memory.
  - **Stale-Object Classification**: creation never requires full expected size before inspecting whether a prior creation completed. On `AlreadyExists` under the publisher lock, the restart classifies the existing object first (size/bootstrap prefix), then: incomplete or undersized → recover (unlink/recreate or reinitialize in place); complete and compatible → adopt; complete and incompatible → explicit ABI/data error. No crash point in the creation sequence may leave a permanently unrecoverable object. Consumers treat a temporarily undersized or `Init` object as `NotReady` within their bounded startup retry policy, never as a permanent compatibility failure.
  - Write authority is enforced using an operating-system-backed exclusive lock (`/run/drm/<deployment_id>/t_<topic_slot>.publisher.lock`). If the publishing process terminates, the operating system kernel automatically releases the lock, allowing a restarted publisher to acquire write authority without heartbeat arbitration or dead-process takeover protocols.

- **Data Freshness vs Stored Bytes**:
  When a publisher crashes, existing consumers can finish reading their currently borrowed samples safely, but DRM does not treat the last completed sample as indefinitely current. Streams transition to `STALE` or `OFFLINE` based on timing contracts. Upon restart, the stream state resets to `NO_CURRENT_SAMPLE` until the restarted publisher produces its first complete sample under the new epoch.

- **Session-Scoped Persistence**:
  Shared-memory persistence is strictly an **opportunistic optimization** for process crash/restart recovery within an operating system session; correctness never depends on it. DRM does not guarantee shared-memory persistence across machine reboots, and platform lifetime rules differ: on Linux the POSIX name survives while mapped even if the publisher exits, while on Windows a pagefile-backed section ceases to exist when its last handle closes. If the last holder exits on a platform where the segment dies, a restarted publisher simply recreates it fresh under the same epoch rules — stale-history availability is a bonus, never a contract. Following a system reboot, publishers recreate their required segments fresh.

### Attached-Channel Data Operations

Programs that attach to a segment a different process created publish and read
through the attached data path: the reserved CCB, ring, consumer-record, slot
generation, and payload regions of the segment are addressed through the
attachment's validated offsets. Publisher authority stays singular per topic
(one writer; consumers are read-only), the transient-borrow and hazard rules
are unchanged, and a restart epoch bump invalidates pre-restart cursors. The
normative algorithm and its test matrix live in
[attached-data-path.md](algorithms/runtime/attached-data-path.md); the rationale
is ADR [0007](adr/0007-cross-process-channel-data-operations.md)
(`REQ-CORE-102`…`REQ-CORE-107`).

## Memory Architecture: Fixed Channels and Structural Isolation

DRM eliminates runtime memory allocators from the data path by exploiting its static knowledge of the robot: topic shapes, maximum payload bounds, consumer borrow rights, queue depths, and hardware placement are known before deployment.

The governing rules of DRM memory are:

> **DRM channel storage is finalized when the channel becomes active. The steady-state data path only reuses that storage.**
> **Every active channel has fixed-capacity storage. Different topics never compete for buffer capacity.**

### Storage Architecture: Generalized Bounded History
Rather than managing a general-purpose heap or maintaining separate delivery engines for latest versus sequential data, DRM maps topic storage into a single unified architecture:

- **Small Structured State**: For compact telemetry, motor commands, and control targets that fit within cache lines, storage is allocated as a single slot or small sequence ring where producers update pre-calculated byte offsets without allocation overhead.
- **Bulk Latest Topics**: For camera frames, point clouds, and high-rate sensor observations (`@history: latest`), DRM defaults to a small fixed pool of 3 slots (triple buffer). This provides one slot for the current latest sample, one slot for the in-progress publisher write, and one slot temporarily held by an active reader. Deployments requiring a mathematical guarantee that up to $H$ concurrent consumers holding different historical frames can never cause a drop may explicitly configure worst-case sizing per the payload pool invariant ($R + H + 1$ slots; for latest topics, $H + 2$).
- **Sequential Topics and Events**: For ordered streams (`@history: N`), DRM allocates a circular sequence handle ring of depth $R$ paired with a fixed payload slot pool. Long-term histories and lossless recording pipelines move immediately into application-managed storage buffers (`drm-record`). If a developer explicitly configures a large history on a multi-megabyte topic (such as 64 frames on an 8 MiB camera topic), `drm verify` reports the full memory cost (512 MiB) directly.

Every storage class obeys one payload pool invariant:

> **`pool_size >= R + 1`**: the pool must retain the $R$ declared history entries plus one slot for the publisher's in-progress write. A pool smaller than this cannot represent the declared retention semantics and fails `drm verify`.
> **Drop-free guarantee**: publisher progress is guaranteed while up to $H$ consumers hold borrows of samples already aged out of the history ring when `pool_size >= R + H + 1`. For `@history: latest` ($R = 1$, $H = 1$), this reproduces the default triple buffer of 3 slots.

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

### Fixed Slot Pools and Platform Memory Backing
Bulk topics maintain fixed virtual address layouts, but operating systems need not fully commit physical memory until actually written:

> **Bulk topics use fixed-capacity slot pools. The operating-system backend determines reservation, commitment, paging, and physical residency. DRM never performs per-page allocation or decommitment on the steady-state data path. Hard-real-time targets may explicitly request precommitted, locked memory.**

- **Fixed-Capacity Slot Pools**:
  A topic declaration like `payload: bytes[<=8388608]` establishes the legal maximum payload bound and a fixed virtual address range for each slot. Slot offsets in shared memory (`offset(slot 0)`, `offset(slot 1)`, `offset(slot 2)`) remain static constants. This eliminates pointer relocation, linked chunks, and heap fragmentation.
- **Platform-Managed Paging Behavior**:
  DRM allocates fixed-capacity shared-memory slots and exposes no paging semantics of its own. Reservation, commitment, paging, and physical residency are determined entirely by the operating-system backend (Linux `/dev/shm` on `tmpfs`; Windows pagefile-backed sections, which distinguish reservation, commit charge, and physical residency, and do not support decommitting individual pages of a mapping). Linux creation reserves the declared tmpfs extent at activation so capacity failure is explicit rather than a later `SIGBUS`; this does not lock pages into RAM. DRM performs no per-page allocation, commit tracking, or decommitment on the steady-state data path.
- **Hard Real-Time Precommitment**:
  For targets where page faults are forbidden, deployments may declare `precommit: true`. Static verification checks backend locking capability and that the configured lockable-memory budget covers the declared working set. Actual success is proven at activation: the node maps the complete fixed regions, pre-faults, memory-locks every declared precommitted region (`mlock` on Linux, `VirtualLock` on Windows), verifies residency, and refuses to enter active state on failure rather than silently continuing pageable.

### Application Allocators for Long-Lived Data
Shared zero-copy slots are transient borrowed resources. When an application needs to retain samples beyond immediate processing—such as background recording to disk, user interface rendering, or historical training logs—it copies the data (`frame = camera.copy_latest()`) or transfers it to an application-managed buffer.

Owned samples are managed entirely by the application runtime's native allocator (such as Python heap, NumPy, or C++ standard allocators). DRM shared memory is never used as a general heap or unbounded queue for user applications. The moment a recording pipeline requires persistent disk buffering, that queue belongs inside `drm-record`, keeping `drm-core` channels lean, bounded, and deterministic.

### Memory Budgets as Build-Time Verification
Memory verification is deterministic against a configured deployment budget, not against guesses about operating-system virtual-memory policy:

- **Deployment Memory Budget**: Each compute target declares a configured DRM memory budget in deployment configuration. All static checks compare against this declared budget.
- **Worst-Case Static Capacity**: During `drm verify`, the DRM toolchain calculates the maximum physical capacity required if every slot in every active channel on a compute unit were filled to its maximum declared bound simultaneously, then:
  - Hard real-time or `precommit` targets: required locked memory exceeding the declared real-time budget is a hard error. Lockable accounting is per channel (the channel's `precommit` flag), not gated on a compute-level flag, and is checked both compute-wide and **per process** — each hard-real-time node must fit its own mapped, locked working set, not merely the aggregate shared capacity. Router-owned lane buffers are counted against the same compute budget. All capacity arithmetic is overflow-checked (`CapacityOverflow`), never allowed to wrap into a passing value.
  - General targets: worst-case capacity exceeding the configured DRM memory budget is a hard error.
  - Worst-case capacity exceeding physical RAM while within the configured budget: a strong warning, not inherently invalid — simultaneous maxima may be unreachable and backing may be pageable (Windows commit limits include pagefile capacity, not physical RAM alone).
  - Estimated nominal working set: informational only.
- **Runtime Measurement**: Actual resident set size (RSS), page commitment, and cache effects are measured and reported live by `drm-diag`, not guessed by static compilers.

## Core Language Ecosystem (MVP)

In systems like ROS 2, each language requires a dedicated client library (`rclcpp`, `rclpy`, `rclc`, `rclrs`, `rclnodejs`, `ros2_dotnet`, `rclgo`) that includes heavy serialization engines (CDR) and dynamic discovery stacks.

DRM eliminates runtime serialization by generating thin, native bindings from a single canonical C-ABI specification (`drm.h`). For the initial MVP, DRM provides first-class support for four foundational robotics languages:

- **C (ISO C99/C11)**: Pure, zero-allocation C structs and direct functions (`drm_publish_control_target`, `drm_read_control_target`). Built for microcontrollers, bare-metal chips, real-time operating systems (FreeRTOS, Zephyr), and safety-critical motor controllers.
- **C++ (C++20)**: Type-safe templates, RAII memory managers, and zero-copy byte-backed accessor views (no reinterpret-cast of shared bytes) without runtime overhead.
- **Rust**: Safe zero-copy reference wrappers, compile-time thread safety, and lock-free ring buffer access.
- **Python**: Owned reads only. `latest()`, `next()`, and `.numpy()` return application-owned copies; the first-party binding exposes no DRM shared-memory payload through the Python buffer protocol, `memoryview`, NumPy views, or any other Python-visible borrowed pointer — a derived Python object can outlive any scope, so scope-pinned zero-copy cannot be made sound there. The binding may internally borrow a slot while performing a copy, but the hazard is released before control returns to Python. Workloads that cannot tolerate the copy run the intensive section in a native (Rust/C++) extension over DRM's zero-copy paths.

All other languages interface directly through the canonical C-ABI header (`drm.h`) or community-maintained bindings. DRM defines a language-neutral canonical binary layout determining the bytes, and exposes runtime access through a stable C ABI determining how foreign languages call `drm-core`; any language with foreign function interface (FFI) or raw pointer access can connect to DRM without a custom runtime engine. Because dynamic language runtimes manage their own internal heap objects and garbage collection, DRM guarantees that its native payload data path performs no per-message heap allocation in steady state.

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
  When a node attaches to a topic, it compares its complete expected descriptor and layout hash against the channel descriptor once. After successful attachment, steady-state messages do not repeat immutable compatibility checks. A topic definition change creates a new deployment namespace and requires a new attachment. A publisher restart changes only the publisher epoch; consumers reset cursor and freshness state, but do not revalidate immutable descriptor fields.
  Concurrency-control state (epoch, sequence watermarks, counters) lives in a naturally aligned control block with static offsets. Atomic fields are never placed in a packed layout; the descriptor and control block use explicit alignment with compile-time offset assertions.

- **Sample Metadata (Varies Per Sample or Drives Concurrency)**:
  Individual shared-memory samples contain only information that differs per sample or is required for lockless synchronization:
  - **Fixed-Size Topics**: Contain only the raw payload. Synchronization state (such as generation counters and sequence progression) resides in the channel control block alongside the slots, not inside the application payload.
  - **Variable-Sized Topics**: Prepend an actual byte length before the payload. Fixed-size topics never pay for a redundant length field.
  - **Sensor Topics**: Include an explicit source timestamp (such as optical shutter time or IMU sample acquisition time) only when declared by topic semantics. DRM does not force arbitrary transport timestamps onto discrete commands or state machines.
  - **Local Integrity**: Local shared memory relies on atomic generation counters and acquire-release ordering for concurrency safety rather than software CRCs.

- **Transport Metadata (Boundary Framing Across Physical Links)**:
  Wire-level framing is added only when a message crosses a physical boundary to another device:
  - **Multiplexed Channel ID**: Identifies the stream across shared physical media (such as UDP multicast or serial links); it is a transport identifier from the route binding, never the runtime topic handle.
  - **Packet Framing**: Every frame carries `{version, flags, wire_channel_id, stream_epoch, source_seq, payload_len}` in a fixed little-endian layout; `stream_epoch` is `u64` (wrap-free restart recognition). The source epoch and sequence are preserved across router hops. They let a best-effort receiver reject reordered/duplicated older frames and reset on a source restart, so an out-of-order datagram can never become the new "latest". One DRM sample crosses a link as exactly one application frame — a single UDP datagram on best-effort routes, one length-framed record on stream transports, or one COBS-delimited, length-checked, CRC-protected serial frame (raw UART is not a reliable byte stream). The router performs no DRM-level fragmentation or reassembly. Because samples are never fragmented, stream receivers enforce a generated per-route maximum record length (header + declared topic payload bound + trailers) *before* buffering; a declared length above it is a protocol error, never an allocation.
  - **Link and End-to-End Integrity**: Physical checksums protect each hop. A declared end-to-end profile appends CRC32C over `{stream_epoch, source_seq, payload_len, payload}` at the source router; transit routers may remap the hop-local wire channel but preserve those covered fields and the trailer. The final router verifies before local import.
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
- `@history: 1` (or `@history: latest`): The topic retains only the most recently published sample.
- `@history: N` (such as `@history: 32`): The topic retains the last $N$ publication sequences in a circular handle ring.

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
- `on_full: backpressure`: Pauses ingress reads so that topic-level history overflow rules (`overwrite` gaps or `reject` errors) naturally govern the publisher without holding shared-memory hazard slots pinned to disk latency. The recorder is never an implicit required sequential consumer: publisher-gating `reject` backpressure requires the recorder to be declared as a required consumer at deployment time (`drm verify` enforces this).

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
- **`drm-core`**: The ultra-lightweight runtime library linked into every process. Contains platform-native shared memory primitives, lockless sequence history rings, hazard-slot payload pools, endpoint opening validation, and C-ABI interfaces. Exposes basic atomic runtime state (`write_seq`, `drop_count`, `overrun_count`, `publisher_epoch`, `pool_exhausted`) directly on channels; tools derive the last sequence from `write_seq`. It has zero dependencies on networking code, disk storage, or compilers.
- **DRM Build Tooling (`drm` tool)**: The unified build-time engine and command-line interface. Parses `.drm` schemas and `drm.toml` manifests, validates node communication contracts, builds the in-memory `WorkspaceModel`, generates language bindings, packages deployments, and provides essential developer inspection (`drm build`, `drm verify`, `drm deploy`, `drm run`, `drm echo`, `drm info`). Implemented initially as a single tool rather than separate compiler and CLI products.
- **`drm-router`**: The boundary gateway daemon. Runs as an independent process on computers connected to physical links. Subscribes to cross-device topics in local shared memory and handles framing, priority queues, and rate throttling across external hardware.

### Next (Persistence and Diagnostic Aggregation)
- **`drm-record`**: The logging and replay subsystem. It consumes topics as a normal sequential reader, briefly borrows each sample, copies it into a bounded recorder-owned buffer, and immediately releases the DRM borrow. Independent writer threads perform chunking, compression, and disk I/O from recorder-owned memory, so slow storage never retains DRM transport buffers or delays publishers.
- **`drm-diag`**: Out-of-band monitoring daemon. Aggregates and interprets basic runtime counters, arrival deadlines, process health, and hardware link statistics without imposing overhead on the real-time data path.

### Later (Specialized Robotics Capabilities)
- **`drm-rpc`**: Request-response services and long-running, cancellable action workflows. Each client owns generated request/cancel topics; the server deterministically multiplexes those inputs and alone publishes that client's single ordered `event` stream (topic sequence = total operation order). Correlation identities include the client and session; a server session identity ensures requests never survive a restart into a new execution context. Execution is reactor/worker: the reactor polls requests, cancels, and session health and flushes bounded outboxes (lifecycle/terminal events guaranteed, progress advisory); user code runs only on a bounded worker pool with cooperative cancellation. RPC routes are required to be reliable. The library adds no core ring semantics and never creates a multi-writer topic.
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

- **Consumer (Freshness, Deadlines, Borrower Multiplicity)**:
  Consumers declare independent timing requirements based on their operational needs:
  ```toml
  [consume."control/target"]
  deadline = "20ms"
  ```
  A fast motor controller may require a tight 20 ms deadline on `/control/target`, while a diagnostic telemetry dashboard reading the same topic accepts a 500 ms deadline. DRM separates publisher capability from consumer requirement.
  Consume endpoints declare their concurrent borrower capacity (records back zero-copy borrows and required-sequential acknowledgements alike; default 1):
  ```toml
  [consume."camera/front"]
  borrowers = 4 # concurrent borrowers; default 1
  ```
  `borrowers` fixes the consumer records and hazard words reserved for this endpoint at activation. Copy-only use keeps a record's hazard word `NONE`. Per topic, the independent-borrower bound is `H = Σ declared borrowers + the reserved diagnostic-consumer range` — a statically known quantity that feeds the payload-pool drop-free guarantee (`pool_size >= R + H + 1`) and is checked by `drm verify`.

- **Runtime Reality (`drm-diag`)**:
  Actual publication frequencies (`observed_rate`), per-consumer deadline state, and latency jitter are measured continuously and non-intrusively at runtime by `drm-diag`. Deadlines are consumer properties, never channel properties: two consumers of one topic with different deadlines are tracked separately. Exact timing measurement requires the topic's channel to be built with timing instrumentation (`timing = auto` resolves it on when a consumer deadline or jitter diagnostics is declared); otherwise diag reports counters and rate estimates only. Diagnostics observe behavior; they do not warranty historical accounting — controllers needing hard staleness guarantees check sample age in their own safety logic.

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
  - **Decoupled Framing and Packing**: Physical links multiplex frames from many topics by transport identifiers, but each DRM wire frame carries exactly one sample. A UDP datagram contains one frame; stream transports concatenate length-delimited frames. Payloads exceeding a datagram route's bound are rejected at build time and belong on stream transports.

- **Stability Across Normal Builds**:
  Standard builds (`drm build`, `drm dev`) preserve previously assigned numeric handles to maintain binary compatibility and prevent cascading rebuilds across nodes.

- **Explicit Handle Compaction**:
  Only an explicit maintenance command (`drm ids compact`) recalculates and packs numeric IDs. Compaction regenerates all handle-dependent deployment artifacts (routing/runtime tables) and creates a new deployment identity; it requires a coordinated redeployment. It does **not** regenerate schema bindings (they never embed handles): only binaries/firmware that compile deployment tables (for example MCU topic tables) rebuild. `drm ids reconcile` applies the same deployment-identity rule when a committed `ids.json` merge changes assignments.

### Structural Bounds vs Semantic Annotations
DRM separates communication requirements from optional application validation:

- **Structural Bounds**: Define the maximum legal limits of a topic payload:
  ```yaml
  name: string[32]
  samples: f32[16]       # Fixed-size array
  readings: f32[<=16]    # Bounded variable array (up to 16 items)
  ```
  Structural bounds are used for schema validation, wire encoding limits, network link feasibility analysis, and hard-real-time pool sizing. A declared upper bound does not mandate that a general-purpose workstation must eagerly reserve the entire maximum theoretical capacity at process startup. On an MCU or hard-real-time node, bounds translate into fixed pre-allocated memory; on general-purpose host workstations, memory pools are allocated and reused based on configured budgets and active demand up to the declared bound.
- **Semantic Annotations**: Optional metadata used for documentation, developer tooling, and UI generation, written inline in braces:
  ```yaml
  speed: f32 {range: [0, 40], unit: m/s}
  ```
  Semantic annotations do not alter default binary encoding. Standard types remain uncompressed (`f32` stays a 4-byte IEEE float) unless explicit quantization is specifically requested for a constrained bus under an `encoding:` rule.

- **Enums**: declared inline; the underlying runtime tag is `u8` (max 256 variants):
  ```yaml
  mode: enum { disabled, manual, autonomous }
  ```

- **Two-Plane Architecture (Data Plane vs Observability Plane)**:
  DRM explicitly separates operational data transfer from system diagnostics:
  - **Data Plane (Strictly Static)**: The core robot communication path requires statically declared schemas with strict bounds. Dynamic schema-less messages are prohibited because they require runtime memory allocators and reflection, which breaks deterministic real-time execution. When an application manages variable instances (such as multiple tracked objects or dynamic game entities), it uses fixed schemas paired with an instance identifier field (`source_id: u32`).
  - **Bounded Opaque Payload Escape Hatch**: For third-party device packets, raw serial streams, or unparsed JSON pass-through, topics declare bounded byte arrays:
    ```yaml
    payload: bytes[<=4096]
    ```
    DRM treats this strictly as an opaque memory buffer with a pre-allocated maximum capacity. It performs no runtime field parsing, dynamic resizing, or string inspection.
  - **Observability Plane**: Diagnostic events and system traces run in an independent, lightweight observability plane rather than inside the critical control path. Diagnostic aggregation never executes on the data path. Optional fixed-cost timing instrumentation is compiled into channels only when declared timing semantics require it. Observability ring buffers use fixed per-producer lanes and are strictly non-blocking and best-effort; when a recorder, telemetry tool, or user interface falls behind, diagnostic events are dropped immediately (counted) rather than exerting backpressure on real-time publisher loops.
  - **Staged Implementation**: Phase 1 provides simple application logging (`drm.log(level, message)`) and standard static topics. Phase 2 introduces full compiled event tracing (`drm-trace`), where event names and format strings are compiled into a build-time metadata dictionary, emitting only small numeric event IDs and arguments at runtime.

## DRM Spatial (Integrated Module for Future Implementation)

DRM Spatial is an integrated spatial-temporal library within DRM (`drm-spatial`), designed to replace dynamic transform trees like tf2. It is planned for development following the core data middleware. It builds directly upon DRM's shared-memory data fabric and compiler toolchain:
- Spatial relationships are declared once in `spaces/`.
- Static structures are collapsed into precompiled constants at build time.
- Authoritative runtime memory stores only dynamic edges with lockless version counters.
- Consumer nodes declare their history requirements, allocating ring buffers only where required.
- Dynamic transforms cross physical links only when remote devices demand them.

Core data communication remains DRM's primary immediate focus. For the complete specification, see the dedicated [DRM Spatial Module Architecture](spatial_architecture.md).

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

`link.bandwidth` declares the **physical line capacity** of the link, not a DRM payload budget. Transport adapters convert declared wire sizes to line time (framing, framing overhead, COBS expansion, line encoding); feasibility estimation counts adapter-computed wire bytes against this capacity. Route-level `export_limit` (below) budgets DRM traffic separately from line capacity.

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
- **Forwarding Tree per Topic**: A topic with several consumers is not one route but a forwarding plan: the publisher's compute is the source, and each reached compute carries a rule `{ingress link, egress links, local delivery}`. This expresses 1→N fan-out, local and remote consumers on the same compute, and multi-hop forwarding in one structure. Path classification is polynomial (connectivity/bridge analysis), never simple-path enumeration.
- **Ambiguity Detection**: If multiple valid physical routes exist without an explicit preference, `drm verify` reports an ambiguous route error at build time rather than silently choosing a path.
- **Explicit Override Conflicts**: Contradictory explicit instructions (`force` and `deny` naming the same link, or multiple `force` routes for one topic) are a verify error (`ConflictingRouteOverride`). Explicit constraints are never resolved by precedence; the selection pipeline is config-validity → `deny` → `force` → `prefer` → automatic.
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
  - **Single-Packet UDP for Real-Time Telemetry**: To avoid reimplementing custom network stacks, reassembly, or congestion control in the MVP, UDP routes carry one DRM sample per datagram under a configured `max_datagram_payload` bound — the path MTU minus IP and UDP headers (closed-LAN deployments typically auto-suggest 1400; tunneled, VPN, or routed networks configure it explicitly). DRM performs no IP fragmentation and builds no path-MTU discovery subsystem. UDP routes provide low-latency, best-effort delivery.
  - **TCP and Dedicated Streams for Large or Reliable Data**: Payloads exceeding a datagram route's bound (such as video frames, point clouds, or recorded logs) and topics requiring guaranteed, in-order delivery across physical devices use standard stream transports (TCP or dedicated bulk sockets) that provide battle-tested fragmentation, retransmission, and flow control.
  - **Serial Framing for Raw Links**: Raw serial (UART) is a byte stream with no framing, so it never reuses the TCP length-record algorithm. Serial frames use COBS with a `0x00` delimiter, an explicit length, and CRC32C over the header, payload, and optional end-to-end trailer, so a dropped or corrupted byte resynchronizes at the next delimiter instead of desynchronizing the receiver indefinitely. A frame in transmission is non-preemptible, so `max_frame_bytes` and baud bound the worst-case priority-inversion delay (≈ `10 × bytes / baud` for 8N1); `drm verify` reports this blocking bound against declared deadlines.
  - **Ordered-Stream Traffic Classes**: A reliable ordered stream cannot reorder bytes once queued; middleware priority applies only within router queues, before data enters the stream. Topics with materially different latency classes must never share one ordered transport stream: latency-critical reliable traffic (control, status, events) uses a realtime lane per peer, bulk reliable traffic (maps, frames, files) uses a separate bulk lane — lanes per peer, never a socket per topic. Bulk bytes never queue ahead of critical reliable control messages inside DRM's own queues and streams. Shared downstream resources (kernel socket buffers, qdisc, NIC, switch queues, the remote kernel) can still add latency; isolation beyond DRM's boundary is a deployment QoS concern (socket priorities, `TCP_NOTSENT_LOWAT`, appropriate qdisc) and is measured at runtime, not guaranteed by lane separation alone.
  - **Transport Reliability Enforcement**: Reliability is an adapter capability of the physical link. Local shared memory and TCP are reliable; UDP and serial are best-effort. `reliable` means the selected transport provides ordered loss recovery while the transport session remains valid; it never means durable, exactly-once, or delivery surviving reconnects or process restarts. If a node contract declares `reliable` delivery for a remote topic, `drm verify` ensures the route maps to a reliable transport adapter; attempting to assign `reliable` delivery over raw UDP without a reliable transport adapter fails at compile time. For reliable fan-out, branches are independent: each holds a bounded per-branch queue, and a branch whose reliable lane stays exhausted (receiver stalled below the sustained declared rate) fails its transport session rather than growing memory without bound or backpressuring unrelated branches or local delivery.
- **Priority Scheduling**: Safety has a **dedicated admission budget** isolated from non-safety token state — control/status/bulk can never delay safety by consuming shared credit. When the transmitter is eligible, safety is selected strictly first, control/status receive deficit-round-robin shares, and no class starves indefinitely. A dedicated budget removes *scheduler-induced* inversion, not the physical link's: the safety worst-case latency is bounded as `T_scheduler + T_inflight_lower_priority + T_adapter_queue + T_transport + T_safety_budget_wait`, where `T_inflight_lower_priority` is one already-committed lower-priority transmission on non-preemptive transports, and is zero (bounded by the maximum non-preemptible fragment) only where link-layer preemption is available and enabled. Scheduling, prioritization, and shaping apply to every transport on a link (UDP datagrams, serial frames, stream lanes); only reliable ordered streams additionally use ordered lanes.
- **Traffic Shaping and Bandwidth Budgeting**: Applies configured token-bucket limits and priorities to DRM-generated traffic on physical links, bounding DRM's contribution to link load and reducing bursts. Shaping charges a declared per-link wire-frame byte count (encoded frame + transport framing overhead); over any window W seconds, admitted bytes are ≤ `rate_bits_per_sec·W/8 + burst_bytes`. Safety and non-safety traffic are metered by **separate buckets** with independent token state, but both are carved from the same physical capacity: `drm verify` requires `safety_rate + non_safety_rate ≤ link_sustainable_rate` and rejects any shaped bucket whose burst is smaller than its maximum charged frame — without DRM fragmentation such a frame could never accumulate enough credit. Traffic from other applications, processes, and computers sharing the medium remains outside DRM's control.
- **Two-Way Import**: When an external microcontroller or companion computer sends sensor data across a link, the local boundary router receives it and writes it directly into the local shared-memory domain. Local nodes consume it identically to any local topic without knowing it crossed a physical boundary. A validated frame is delivered locally and forwarded on each egress branch independently; a failure on one branch never suppresses local delivery or sibling branches.

Physical link disconnections affect only topics traveling across that specific link. All local processes continue communicating across shared memory with zero interruption.

## External Middleware Bridges

A third-party middleware is reached through an explicit **bridge process**, never by teaching the core about its protocol. A bridge is an ordinary DRM node from the local deployment's point of view and an ordinary peer from the foreign middleware's point of view; DRM's shared-memory domain retains exactly one publisher per topic.

**Structure.** A bridge attaches to DRM over the public attached data API ([§Attached-Channel Data Operations](#attached-channel-data-operations)) — in practice the C ABI, so one implementation serves every language a foreign middleware favors — and speaks the foreign protocol natively. It is not a broker, holds no registry, and introduces no second writer: publishing into DRM means being the deployment's declared publisher for that topic.

**Serialization is the bridge's business, not the core's.** DRM's canonical layout is its own; a foreign wire format (for example CDR) is encoded and decoded inside the bridge. The core never learns a foreign encoding, and `drm-core` gains no dependency on any external middleware.

**Declared mapping.** A bridge's channel set is explicit configuration naming, per direction, the foreign endpoint and the DRM canonical topic. Undeclared topics are never bridged, and a message whose foreign type identity or serialized size does not fit the declared DRM bounds is rejected with an explicit status — never truncated, never silently resized.

**Capacity is checked, not discovered.** A bridge honors the declared payload bound, history depth, and pool capacity of the DRM side. A foreign publisher whose message exceeds the bound is refused by the bridge; a DRM topic whose retention cannot absorb the bridge's pace follows its declared overflow policy, exactly as for a native node.

**No feedback loops.** A bridge that both reads and writes the same topic is rejected at configuration time: with one publisher per topic per direction, an echo path must be expressed as two distinct topics.

**QoS translation is explicit.** Foreign reliability and durability settings map to declared DRM delivery semantics where the mapping is real, and are refused where it is not. A bridge never claims a guarantee the DRM topic does not carry.

**Types move by serialized payload.** Until typed conversion exists, a bridge carries samples as serialized bytes and DRM applications treat that payload as opaque. Typed foreign↔DRM field conversion is a separate, later capability; a bridge must not imply it.

**Opt-in CDR decode.** A DRM application may decode a bridged ROS 2 payload with `drm-cdr`, a zero-dependency leaf codec: bounds-checked CDR_LE reads with alignment measured from the post-encapsulation origin. Decoding is explicit and per-payload; the bridge and core remain payload-opaque (REQ-BIND-028).

**Isolation.** A bridge process failing, restarting, or stalling affects only its declared topics: other topics, other bridges, and all local DRM traffic continue. The bridge holds no lock or resource that a local publisher needs.

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

The model is the single source for runtime activation expectations: per-topic descriptor expectations (schema/layout hashes), payload capacity, history depth, and payload-pool sizing are derived from the model and consumed by the runtime when it activates a channel. The runtime never parses `.drm` schemas itself, so a topic declaration change and the channel configuration cannot drift.

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
.drm/gen/rust/control/target/target.rs
.drm/gen/python/control/target/target.py
```

Each topic binding is generated once per language and shared by every node that uses it. If ten C++ nodes across different packages consume `ControlTarget`, they all compile against `.drm/gen/cpp/control/target/target.hpp`.

### Node Communication Contracts as Architectural Capabilities
A node's `publish` and `consume` declarations in its manifest are the single source of truth for its DRM interface. DRM uses these declarations as an architectural correctness mechanism rather than a separate security-permission subsystem:

- **Static Languages (C++, Rust)**: The generated API for a node exposes only the topics declared in that node's manifest. If `controller` declares `publish = ["actuator/command"]` and `consume = ["control/target"]`, calling `drm::camera::front::publish(...)` fails at compile time because that method is not included in the node's generated interface.
- **Dynamic Languages (Python)**: Runtimes expose generated topic objects matching node declarations. Attempting to publish or consume an undeclared topic raises an immediate runtime error (`ContractError`).
- **Low-Level C ABI and Endpoint Handles**: When using the raw C runtime ABI, access validation occurs once when opening an endpoint handle at startup (`drm_context_open` → `drm_publisher_open(...)` / `drm_consumer_open(...)`). If the node manifest permits the topic, an authorized handle is returned; consumer opens claim one of the declared borrower records (`Busy` when none are free). Once handles exist, steady-state calls execute with zero per-message permission overhead. The canonical operations are: `drm_publish_copy` (copy publish), `drm_write_acquire`/`drm_write_commit`/`drm_write_abort` (zero-copy publisher loan writing directly into a shared slot), `drm_borrow_latest`/`drm_borrow_next`, `drm_copy_latest`/`drm_copy_next`, `drm_borrow_release`, the close/`drm_abi_version` calls, and the best-effort `drm_log` observability export. Endpoints carry explicit per-handle thread-safety contracts: a publisher endpoint accepts concurrent publish attempts (exactly one in-progress write; losers get `Busy`), a consumer endpoint serves one borrower. A Rust panic at the C ABI is an internal invariant failure and terminates the node process — recoverable conditions always return status codes, and normal crash recovery (locks, epochs, registration-lock reclamation) applies. C status codes are a fixed `int32_t` set with explicit values. ABI compatibility is one-way: older applications always run against newer libraries; applications built against newer headers require a runtime providing the symbols they use, and node contract fingerprints record the required ABI.
- **Architectural Correctness, Not Security Sandboxing**: DRM prevents accidental communication outside a node's declared interface. This is an architectural correctness mechanism, not an operating-system security boundary against malicious code. Separate per-node `.contract` policy files are not required; compatibility fingerprints and endpoint validation tables are derived directly from the node manifest and deployment metadata.

DRM manages generation using lightweight invalidation metadata rather than content-addressed directory names:
- **Internal Fingerprints**: DRM stores a small fingerprint of generation inputs:
  `fingerprint = hash(canonical_schema, topic_identity, generator_version, options)`
  If the stored fingerprint matches and the expected output files exist on disk, regeneration is skipped. Node-scoped API fingerprints additionally hash the node's **directed** publish and consume capabilities (topic + direction + schema/layout) — direction is part of identity, never a union of topics.
- **No Runtime Handles in Generated Bindings**: Generated schema and node-API bindings carry canonical topic identities, contracts, and typed operations only — never runtime numeric handles. Runtime handles are resolved at startup from deployment artifacts; compaction therefore never regenerates bindings. Deployment-specific artifacts (for example MCU firmware topic tables) may compile handle numbers; those are deployment artifacts, rebuilt only where compiled in.
- **Atomic Writes**: Bindings are written to temporary files and atomically renamed upon completion. A canceled generator never leaves half-written or corrupt header files.
- **Timestamp Preservation**: If generated code is identical to the existing file on disk, DRM does not rewrite it. Preserving the modification timestamp prevents native build tools (CMake/Ninja, Cargo) from triggering unnecessary rebuilds of dependent application code. Byte-identity is checked by content, not by fingerprint alone: a changed fingerprint with identical rendered output leaves the file (and its mtime) untouched.
- **Direct Reconciliation**: When a topic is deleted from workspace declarations, DRM deletes its corresponding logical directory directly. There are no orphan hash directories accumulating on disk and no disk garbage collection passes required. Content-addressed storage (CAS) is deferred as an optional remote CI/cloud cache feature.

### Development Watch Mode (`drm dev`)
The filesystem watcher monitors changes without tracking header dependencies or object freshness. It distinguishes two data event categories (plus error/rescan control signals):
- **DRM Declaration Changed** (`.drm` schema, `drm.toml` manifest, topology): DRM recompiles the schema, regenerates affected language bindings and contracts, and invokes the native builder for affected nodes.
- **Application Source Changed** (`.cpp`, `.rs`, `.py`, `.go`): DRM identifies which node owns the file and immediately invokes that node's native build adapter. CMake/Ninja or Cargo decides which object files require recompilation.

Watch events are invalidation hints, not ground truth. Operating-system watch queues can overflow and drop events (inotify `IN_Q_OVERFLOW`, `ReadDirectoryChangesW` buffer exhaustion); DRM treats such loss as a control signal (`RESCAN_REQUIRED`) and reconstructs state by rescanning declarations and node source trees. Source events arriving in a batch whose declarations fail to reload are retained as pending dirty state and dispatched after the next successful reload — never discarded.

### Verification and Runtime Observation: Facts, Feasibility, and Reality
A static build tool cannot honestly prove runtime timing, execution jitter, or that a network bus will never experience transient bursts. DRM maintains a strict boundary between static proof, feasibility estimation, and runtime measurement:

> **DRM verifies facts it can prove; it estimates feasibility from declarations; it observes actual behavior at runtime.**
> **Compile-time DRM proves configuration. Runtime DRM measures behavior. Never present an estimate as a guarantee.**

- **What `drm verify` Proves (Deterministic Facts)**:
  - All topic schemas are valid and consistent across packages.
  - Every consumed topic has exactly one declared publisher across the domain (enforcing Single-Writer, Multiple-Reader).
  - Placements, compute units, and physical link topologies are valid without conflicting identifiers.
  - Cross-device topic routes exist and data shapes fit target link framing rules (e.g. encoded UDP frames fit the route's configured `max_datagram_payload`).
  - Transport adapter capabilities match delivery requirements (rejecting `reliable` contracts assigned to best-effort UDP/serial links).
  - Active DRM channels fit the configured deployment memory budget; precommitted channels additionally fit the configured lockable-memory budget.
  - Consumer delivery requirements on shared topics are mutually compatible.

- **What `drm verify` Estimates (Declared Feasibility)**:
  - Nominal link utilization based on declared `expected_rate` values and packet sizes (for example, declaring that nominal UDP telemetry consumes 12.4 Mbps of a 100 Mbps link with 87.6% headroom).
  - Feasibility flags impossible configurations (such as declared traffic exceeding 100% of link capacity) as build errors, but treats nominal link headroom as an engineering estimate, not a mathematical guarantee that runtime packet bursts or packet transmission delays will never occur.
  - Publisher `expected_rate` declarations are planning metadata used for buffer sizing and bandwidth estimation, not a compiler guarantee that user application code will sustain that rate without jitter.

- **What `drm-diag` Measures (Runtime Reality)**:
  - Runtime health, timing behavior, and link statistics are observed on running nodes by the `drm-diag` subsystem out-of-band:
  - Actual publication frequencies (`observed_rate`) versus declared planning rates (`expected_rate`).
  - Observed sample gaps and per-consumer deadline state (`WaitingFirst/Current/Late`) against each consumer's declared deadline (`deadline: 50ms`) — on channels built with timing instrumentation; other channels report counters and rate estimates only.
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

# Owned copies — every read returns application-owned memory
status = topics.robot.status.latest()
frame = topics.sensor.camera.latest()          # owned bytes
array = topics.robot.status.latest().numpy()   # NumPy-owned array, never a shared-memory view
```

In C++ (`controller`):
```cpp
// Scoped zero-copy borrow; generated types are byte-backed views with accessors (no struct cast)
if (auto target = drm::control::target::borrow_latest()) {
    float speed = target->speed();
    float steering = target->steering();
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
- Tools attach as non-publishing observers. Payload bytes are read-only by contract; the mapping permits writes only so the tool can claim its declared consumer record and update its own hazard word.
- Each deployment reserves a fixed, bounded diagnostic-consumer range per topic. Application endpoints use generated record indices; ad-hoc tools scan only that reserved range at attachment and fail `Busy` when it is full. Diagnostic attachment never adds channel storage or allocates on the sample path.
- When inspecting bulk topics, diagnostic tools take a momentary borrow (`borrow_latest()`), copy the sample to private memory, and release the hazard slot immediately. They never hold prolonged shared memory borrows, ensuring developer tools cannot freeze production robot buffers.
- Developers can view data rates, latency, and memory utilization live without deploying extra daemon processes.

## Fault Isolation and Recovery

Nodes operate independently without centralized failure cascades:

- **Crash Isolation**: If a consumer or publisher crashes, other programs continue running unaffected. An operating-system-backed writer lock immediately releases publisher authority on termination.
- **Controlled Reconnect**: A restarted publisher reconnects to its deployment-scoped shared-memory segment, increments its publisher epoch, and resumes communication. Consumers observe the epoch change and reset their sequence expectations cleanly without mistaking stale samples for new data.
- **Deployment Independence**: Deployments never adopt or migrate segments across updates. New deployments launch in their own isolated namespaces, allowing atomic cutovers without complex in-place shared-memory transformations.
- **Network Resilience**: Loss of a physical connection affects only remote topics traveling across that link. All local control loops continue without interruption.
