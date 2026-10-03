# DRM Studio (UI Architecture)

## Requirements Language

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all capitals.

## Architecture Description Conventions

Conventions: `docs/standards.md`.

DRM Studio is the visual control surface for the DRM workspace: create and edit packages, nodes, topics, computes, and links; assign publishers and consumers; inspect routes, memory budgets, and verification results; build and run targets; visualize the system. It is explicitly **not an IDE**: source code remains in VS Code or any editor, and workspace files remain the only source of truth.

Related: [DRM Architecture](architecture.md).

## Core Principles

> **The GUI never implements DRM rules.** Every mutation is executed and validated by the Rust workspace engine. CLI and Studio cannot disagree because there is exactly one implementation of DRM semantics.
> **Workspace files remain authoritative.** Studio keeps no hidden database and offers zero capability absent from the CLI; headless workflows never start Studio.
> **The renderer receives domain capabilities, not machine access.**
> **Simple by default.** Internal implementation knobs (`pool_size`, hazard slots, transport handles, sequence counters) stay behind Advanced disclosure. A visual dump of every internal knob is worse than a config file.
> **Control and data travel separate planes.** Commands and low-frequency state use JSON-RPC; topic samples and video stream as binary frames.
> **The engine edits documents, not just semantics.** Hand-written comments, whitespace, and ordering survive GUI mutations.

ROS 2's friction is scaffolding divergence per language (`package.xml`, `CMakeLists.txt`, `setup.py`) and CLI ergonomics, not CLI existence. DRM Studio removes that friction because node manifests are already the source of truth and the DRM tool already owns parsing, generation, validation, and build orchestration.

## Process Architecture

One Rust engine, two frontends, and a process family beneath the UI:

```text
                .drm / drm.toml files
                          │
              ┌───────────┴───────────┐
            CLI                  Studio host
         `drm ...`          (`drm studio --stdio`)
                                      │
        ┌──────────────┼──────────────┴─────────────┐
   Workspace      Run Session              Extension Host
     Engine       Supervisor                  Process
        │                                        │
   Files/Git                               WASM/native
```

- **drm-workspace** (Rust library): parse, validate, mutate, scaffold, build, run — the single implementation of DRM semantics shared by CLI and Studio.
- **Studio host**: long-lived JSON-RPC mode of the existing DRM executable keeping the `WorkspaceModel` alive. No HTTP server, no ports, no localhost authentication, no WebSocket for desktop.
- **Run session supervisor**: owns run sessions independently of any UI lifetime (below).
- **Extension host process**: executes third-party WASM/native code outside the authoritative process (below).
- **File watcher owned by the Studio host**: VS Code saving `drm.toml` → filesystem event → reparse → revalidate → `workspaceChanged` → React refreshes. Unsaved draft conflict: Reload / Compare / Keep draft.

### Control and Data Planes

> **High-rate data never travels the control plane.**

JSON-RPC carries commands, snapshots, events, and low-frequency diagnostics. Topic samples, point clouds, and video move over binary framed streams — OS/named pipe locally, binary WebSocket remotely, dedicated media path later. A 5 MiB point cloud base64-encoded through JSON at 20 Hz is an architectural mistake; continuous streams also demand bounded retention (Foxglove's documented OOM lesson for 60-fps panel data).

One frontend abstraction hides Electron versus browser:

```ts
interface StudioTransport {
  request<T>(command: Command): Promise<T>;
  events(): EventStream;
  openDataStream(request: StreamRequest): BinaryStream;
}
```

The React application, command model, stores, and panels are identical in both shells; only the transport beneath `StudioTransport` changes.

### Protocol Versioning

The first RPC is a capability handshake, never an exact-version requirement:

```text
→ hello { protocol_version: 3, client_version: "1.7.0" }
← hello { protocol_version: 3, host_version: "1.4.2",
          capabilities: ["workspace.v2", "extensions.web",
                         "binaryStreams.v1", "runtime.stats"] }
```

The frontend depends on declared capabilities and hides unavailable features.

### Run Session Supervisor

Run sessions outlive the Studio window:

```text
Studio UI → Studio Host → Run Session Supervisor → node processes
```

A session carries an ID, start time, node processes, stdout/stderr capture, status, and stop/restart operations. Closing Studio with running nodes prompts Leave running / Stop session; a remote disconnect leaves nodes running unless the run configuration says otherwise. Closing a window can never accidentally kill — or orphan — a real robot's processes.

### Protocol

Typed commands:

```text
Workspace.open  Workspace.snapshot  Workspace.verify
Package.create  Package.rename  Package.delete
Node.create     Node.update     Node.delete
Topic.create    Topic.update    Topic.rename    Topic.delete
Compute.create  Compute.update
Link.create     Link.update
Build.start     Build.cancel
Run.start       Run.stop        Run.restart
```

Events: `workspaceChanged`, `diagnosticsChanged`, `buildOutput`, `nodeStarted`, `nodeStopped`, `runtimeStats`.

## Transactional Mutations

The UI sends domain commands, never raw file writes:

```text
renameTopic("control/target", "control/command")
```

### Revisioned Optimistic Concurrency

Every `WorkspaceSnapshot` carries a monotonic `revision`. Mutations include `base_revision`; the engine accepts the mutation only if the revision is current, otherwise returns `CONFLICT { revision }` and the UI offers Review changes / Reload / Retry. Desktop Studio, VS Code, and remote browsers therefore coexist without last-writer-wins data loss. Workspace-level revision first; entity-level revisions later if needed. No CRDTs, no collaborative editing.

### Journaled ChangeSets

No filesystem operation commits four independent files atomically, so "atomic" is defined as journaled. A `ChangeSet` carries a transaction ID, expected input hashes, and desired outputs:

```text
1. Validate base revisions
2. Prepare all outputs as temporary files
3. Write the transaction journal
4. Rename prepared files into place
5. Mark committed
6. Remove journal
```

A crash between steps 3–5 → next startup detects the journal and finishes or rolls back the transaction. Significant operations preview affected files (`Rename Topic: 3 files will change. [Show changes]`) before commit.

### Conditional Undo

Undo reverts the original `ChangeSet` only when its expected input hashes still match. Files changed externally since the edit cannot be silently overwritten:

```text
Cannot undo cleanly: controller/drm.toml changed externally.
[Compare] [Cancel] [Force revert]
```

Undo/redo operates on file transactions, never React state.

### Format-Preserving Editing

> **The engine never rewrites a file it did not author structurally.**

The engine maintains a semantic model (what nodes exist) and a concrete document representation (exact bytes). Manifest edits use format-preserving mutation (`toml_edit`): comments, whitespace, and field order survive GUI changes. `.drm` schemas use a CST-based parser or a declared canonical formatting policy — hand formatting is never silently destroyed by deserialize → rewrite.

## Desktop Shell (Electron)

- Renderer sandboxed and context-isolated, no Node.js access. Preload exposes narrow domain functions only (`window.drm.createNode(...)`, `verify()`, `build()`, `run()`, `openSource(...)`), never `exec`, raw `fs`, or `ipcRenderer`.
- Packaging: pnpm + Electron Builder + bundled DRM executable. No Python child backend, no FastAPI, no Nuitka layer.

## Remote Web Mode

The same React application runs in two shells:

```text
            React Studio UI (browser-only)
                        │
            ┌───────────┴───────────┐
         Desktop                 Web
         Electron              Browser
            │                       │
        stdio RPC              HTTPS/WSS
            └───────────┬───────────┘
              DRM Studio Host (Rust)
                        │
               actual workspace files
```

- **Browser is presentation/input; host is authority/execution.** A remote Create Node edits the host's workspace; Build, Run, and Verify execute on the host.
- Default binding `127.0.0.1`; remote access via SSH tunnel or VPN/WireGuard. Later: `--listen`, `--tls-cert`, `--tls-key`, `--auth-token` for controlled LAN use. An unauthenticated workspace editor is never exposed to the network.
- Remote source coding: Copy Path / Copy SSH Command / Open VS Code Remote (`code --remote ssh-remote+host ...`). Studio never recreates VS Code Remote.

## Application Layout

Fixed regions in v1; resizable and collapsible, no docking framework:

```text
┌────────────────────────────────────────────────────────────┐
│ DRM Studio   workspace          Verify ✓   Build   ▶ Run   │
├──┬────────────────┬──────────────────────────┬─────────────┤
│A │ PRIMARY        │ EDITOR (tabs)            │ INSPECTOR   │
│  │ SIDEBAR        │                          │ selected    │
│  │ per activity   │                          │ object      │
├──┴────────────────┴──────────────────────────┴─────────────┤
│ PROBLEMS │ OUTPUT │ BUILD │ RUN │ DIAGNOSTICS  (collapsed)   │
└────────────────────────────────────────────────────────────┘
```

Resizable left/center/right columns, collapsible bottom panel (`react-resizable-panels`). Persisted: column widths, bottom height, active tab, expanded tree nodes. Editor regions are fixed — no drag-anywhere docking anywhere. Visualization layouts use a recursive split tree instead (see Extension Platform).

## Activities

```text
◫ Workspace     full hierarchical tree (packages → topics/nodes, computes, links)
● Nodes         searchable/filterable node list; group by package/language/compute
◇ Topics        searchable list; tree and table modes (topic, publisher, consumers,
                history, status)
⎇ System        compute units (platform, nodes, DRM memory budget, realtime memory),
                links (transport, bandwidth, max_datagram_payload, derived routes),
                external endpoints
⌘ Graph         communication/deployment/packages views
▶ Run           run configurations; running/stopped nodes with PID, published and
                consumed observed rates, stop/restart/open source
⬡ Extensions    (phase 2, designed from start — see below)
⚙ Settings      appearance, workspace, editor, graph, build, run, remote
```

Route assignment remains automatic; System displays the derived result and exposes override controls only where inference is ambiguous. Diagnostics (observed Hz, gaps, pool state) arrive later via `drm-diag`.

## Creation and Editors

Creation is available everywhere (`+` button, context menu, command palette, from related objects — e.g. a Node's `+ Add Topic` searches existing topics or creates one and attaches it in one flow):

- **New Package**: name, location, description. Nothing else — build system, QoS, networking, and memory policy are not package concerns.
- **New Node**: package, name, language, template, publish/consume. Creates `drm.toml` + template source.
- **New Topic**: path, schema field table (field, type, bounds, unit), collapsed Delivery (history, overflow) and Advanced sections.

Editors:

- **Node editor**: language/build, publish/consume lists, placement; `[ Open in VS Code ]`, `[ Reveal Folder ]`, `[ Copy Path ]` for source.
- **Topic editor**: schema table, derived publisher/consumers, delivery; collapsed Timing/Memory/Route/Advanced.
- **Raw editing**: every object offers `Open Raw` (CodeMirror 6 for `.drm`/`drm.toml` — small modular core, not the VS Code engine; Monaco only if Studio ever becomes a code editor). Source files open in VS Code; Studio embeds no LSP, debugger, Git, terminal, or marketplace.

## Inspector

A universal right panel following selection (Node/Topic/Compute/Link), like Godot's Scene Tree → Inspector model. Property classification:

| Style | Meaning | Example |
|---|---|---|
| Editable | user-authored declaration | history, overflow, placement |
| Derived | compiler-derived | publisher, routes, memory budget |
| Live | runtime-observed | observed rate `99.92 Hz LIVE` |

A displayed value is never automatically a form field. Inline diagnostics come from the Rust verifier, never duplicated UI logic: a topic exceeding a UDP bound shows the error plus a suggested fix (`[ Use TCP ]`) computed by the engine.

## System Graph

> **The graph is a projection for understanding, not the authoring model.**

Used for understand / inspect / navigate / spot errors / see routing. React Flow is isolated behind the Graph activity; large workspaces collapse by package or compute unit, focus a node's neighborhood, and filter by topic/package/link — never render 2,000 topics expanded. Graph coordinates, zoom, and collapsed state are UI state, never `drm.toml`. Allowed gestures only: drop node onto compute (re-place), connect topic to consumer (add consume declaration).

## Problems Panel

First-class, VS Code-like; click opens the offending editor and highlights the property:

```text
ERROR  controller      No publisher for localization/pose
ERROR  camera/front    Encoded frame 4096 B exceeds max_datagram_payload 1400 B
WARN   jetson          Expected memory capacity exceeds physical RAM
WARN   planner         Expected rate exceeds declared route budget
```

Severity classes mirror the verify model — ERROR (proven), WARNING (feasibility estimate), RUNTIME (observed), INFO — and are never collapsed into generic red warnings.

## Semantic vs UI State

- **Workspace semantics**: files under version control; the single source of truth.
- **Studio preferences** (theme, panel sizes, tabs, table columns, recents): `~/.config/drm/studio/`.
- **Workspace-local presentation** (graph layout): Git-ignored `.drm/studio.local.json`; an opt-in shareable `.drm/studio.json` may come later. Robot architecture and "Bob likes a 360px sidebar" never mix.

## Frontend State

Three state classes, three stores — 60 Hz telemetry never passes through React global state:

| Store | Contents | Mechanics |
|---|---|---|
| WorkspaceStore | node/topic/compute semantics, diagnostics | revisioned snapshots and deltas (TanStack Query) |
| UIStore | selection, tabs, panel sizes, graph viewport | Zustand |
| DataStore | telemetry rates, point clouds, video | direct `BinaryStream` subscriptions, ring buffers, rAF-throttled rendering |

A camera-rate update must not wake the workspace tree, and a 3D panel consumes its data stream directly instead of triggering workspace re-renders 60 times a second. Retention in the DataStore stays strictly bounded.

## Commands and Keyboard

One central command registry unifies every invocation path — toolbar buttons, context menus, palette, keybindings, and extension menu items all dispatch the same primitive:

```text
Command { id, title, icon, enabledWhen, visibleWhen, execute }
```

```text
drm.topic.create   drm.topic.rename   drm.node.build
drm.node.run       drm.workspace.verify
```

- `Ctrl+Shift+P` command palette (`cmdk`): every command; entries with CLI equivalents show them subtly (`drm topic create control/target`) — the GUI teaches the CLI instead of creating a second world.
- `Ctrl+P` quick open across nodes, topics, computes.
- A Keybindings screen lists and rebinds commands (`Rename Topic F2`, `Verify Workspace Ctrl+Shift+V`, `Build Current Node Ctrl+B`) — no scattered hard-coded shortcuts.
- `Ctrl+B` sidebar, `Ctrl+J` bottom panel, per-activity shortcuts.

## Extension Platform

Extensions are a first-class architectural requirement: robotics visualization needs (proprietary codecs, thermal/event cameras, radar, LiDAR, SLAM maps, CAN tools, vendor SDKs) cannot be predicted. The model borrows Foxglove's panel + converter system, VS Code's extension hosts and contribution points, and game-engine editor plugins — without implementing VS Code extension compatibility itself.

> **Extensions extend DRM Studio (visualization, import/export, vendor tooling), never `drm-core`.** The runtime data plane — schemas, topics, shared memory, routing — stays minimal and extension-free, so a buggy LiDAR visualizer can never become part of the robot's critical middleware.

### Contribution Points

v1: `panels`, `commands`, `sidebarViews`, `inspectors`, `topicRenderers`, `schemaConverters`, `settings`. Later: `workspaceTemplates`, `statusItems`, `dataImporters`, `fileViewers`. **Never Studio extension APIs**: `runProviders`, `buildProviders`, `deviceAdapters` — they change system execution and belong to the DRM tooling/package ecosystem as headless plugins; Studio may provide their UI, but the capability itself must exist in the CLI/tooling (preserving zero capability absent from the CLI). Never exposed: internal React components, UI stores, Electron APIs, Rust RPC objects. The extension API is stable; internal UI may change freely.

Declarative manifest with API versioning, lazy activation, auto-generated settings:

```json
{
  "id": "acme.lidar-viewer",
  "engines": { "drmStudio": "^1.0" },
  "permissions": ["topics.read", "storage.extension"],
  "activationEvents": ["onPanel:acme.lidar3d", "onSchema:acme.LidarPacket"],
  "contributes": {
    "panels": [{ "id": "lidar3d", "name": "3D LiDAR" }],
    "schemaConverters": [{ "from": "acme.LidarPacket", "to": "drm.visual.PointCloud" }],
    "configuration": { "lidar.pointSize": { "type": "number", "default": 2 } }
  },
  "entrypoints": { "web": "./dist/web.js", "host": "./dist/host.wasm" }
}
```

### Panels and Visualization Layouts

Extensions register panels; a visualization tab hosts a **recursive split tree** — not a docking engine: split left/right/up/down, move, resize, close, maximize.

```text
VisualizationLayout
└── horizontal
    ├── Camera
    └── vertical
        ├── LiDAR
        └── Plot
```

That is enough for robotics dashboards — no floating windows, arbitrary tab docking, or window manager. Editor documents and visualization layouts coexist as tabs. Layouts reference panel IDs; a missing extension renders a placeholder with `[ Find Extension ]` and preserves settings.

### Topic Access

```ts
const sub = ctx.topics.subscribe(
  "sensor/lidar",
  { mode: "latest", maxRate: 20, maxBytesPerSecond: 100_000_000 },
  (frame) => render(frame)
);
```

- Samples are binary (`ArrayBuffer` typed views) with a schema descriptor — never JSON-encoded frames for point clouds or images.
- The Studio host is authoritative on delivery: it throttles (60 Hz robot camera → 10 Hz to a remote browser) and enforces budgets; extensions declare desired/maximum only. The extension cannot tell whether data flowed local SHM → host → panel or host → network → browser.
- Compressed video uses a media path (`ctx.media.open(topic, { preferredCodec: "h264" })`) routing to hardware/browser decode; WebRTC for remote low-latency video is optional and not required in v1.

### Three Extension Types

| Type | Runtime | Isolation | Use |
|---|---|---|---|
| Web (default) | TS/JS, browser APIs only | sandboxed iframe (opaque origin, strict CSP) + Web Workers | panels, renderers, converters, views |
| Host processor | WASM/WASI | separate `drm-extension-host` process; wasmtime resource limiters (memory cap, fuel/epoch CPU interruption, topic bandwidth budget) | heavy decode/conversion next to robot data |
| Native adapter | vendor SDK executables | separate process spawned by the extension host | unavoidable vendor SDKs; crash kills adapter, not Studio |

Third-party WASM/native code never executes inside the authoritative Studio host — even WASM runs in the extension host process under explicit budgets, so a buggy converter can starve an extension, never workspace mutations, verification, or run supervision. Third-party web extensions never run inside the Electron renderer with Node access; communication happens only through a controlled message channel, making desktop and browser behavior nearly identical. Non-visual computation runs in Web Workers so a misbehaving plugin cannot freeze the UI. Native adapters require an explicit install warning (native code execution, permissions list).

### Converters and Visual Schemas

Schema converters normalize vendor formats (`ouster.RawPacket`) into a small standard visualization vocabulary — `drm.visual.PointCloud`, `Image`, `CompressedImage`, `Pose`, `Transform`, `Marker`, `Grid`, `Path`, `Mesh`, `NumericSeries` — kept in `drm-visual-schemas`, never in DRM core. One vendor converter unlocks every generic panel; no per-vendor viewer panels needed.

### Trust, Capabilities, Packaging

- **Capability enforcement**: web extensions run under a unique opaque origin and strict CSP (`default-src 'none'; script-src 'self'; connect-src 'none'`). Direct `fetch`/XHR is impossible, so `network.connect` is real, not decorative — network access happens only through permission-checked `ctx.network.fetch(...)`.
- **Workspace trust**: opening an untrusted workspace is restricted — view manifests, browse topics, inspect graph only; Build, Run, workspace extensions, and native adapters stay gated behind `[ Trust Workspace ]`. A cloned repository must not execute code merely by being opened. Trust and authentication are independent questions: authentication gates who reaches Studio; trust gates what a workspace may execute. MVP remote access stays localhost + SSH/VPN; direct network listening only after TLS, origin validation, and authentication are designed.
- **Capabilities**: `topics.read`, `topics.publish`, `workspace.read/modify`, `runtime.observe/control`, `network.connect`, `storage.extension`, `process.spawn`. Minimal by default; a color-map extension gets no `workspace.modify`.
- **Packaging**: `.drmx` (zip: manifest, `web/`, `host/`, assets). MVP installs from file/folder; a registry with signatures, hashes, and a block list comes later, with the format designed to admit signing.
- **Split execution**: the `web` entrypoint renders in the browser; the optional `host` entrypoint (WASM) executes next to the robot data — a proprietary LiDAR decode runs on the robot host and ships normalized binary to the browser panel.

## Technology Stack

| Layer | Choice | Reason |
|---|---|---|
| Desktop | Electron | developer-tool fit; proven process/fs model |
| UI | React + strict TypeScript + Vite | |
| Components | shadcn/Radix + Tailwind | |
| Layout | react-resizable-panels | fixed VS-Code-like regions, no docking framework |
| Graph | React Flow | system graph only; memoized, collapsed |
| Raw editor | CodeMirror 6 | `.drm`/TOML editing; far smaller scope than Monaco |
| Forms | React Hook Form + Zod | draft/input validation |
| Async state | TanStack Query | workspace queries, mutations, build/run ops |
| UI state | Zustand | selection, tabs, panel sizes, graph viewport |
| Engine | `drm-workspace` (Rust) | one implementation of DRM semantics |
| Bridge | `drm studio --stdio` JSON-RPC | no HTTP server, no duplicate backend |
| Persistence | workspace files authoritative | no hidden DB |
| Packaging | pnpm + electron-builder + bundled DRM executable | no Python/Nuitka |
| Tests | Rust + property tests, Vitest, Playwright, fixture workspaces | engine and GUI both validated |

Removed from earlier proposals: Python child backend, FastAPI, Pydantic authority, uv, Nuitka, uPlot — none solve a DRM Studio problem.

## Product Boundary

```text
DOES                                     DOES NOT
──────────────────────────────────────   ──────────────────────────────────
Create workspace/packages/topics/nodes   Write algorithms or controllers
Assign publish/consume/placement         Provide a full C++/Rust/Python editor
Configure computes, links, deployment    Provide LSP, debugger, Git, terminal
Inspect routes, memory, verification     Replace VS Code, SSH, or CLI workflows
Build basic targets, run/stop nodes      Replace drm-record/drm-diag/CRM runtime
Visualize architecture                   Extend drm-core (extensions aside)
```

## MVP Scope

1. Workspace, Topics, Nodes, Graph, and the Problems/Build/Run bottom panel — five surfaces, with creation flows as dialogs.
2. Extension platform lands as the designed-in second phase; the API boundary and isolation model are fixed before any frontend code ships.
3. Remote `drm studio serve` follows desktop once the host protocol is stable.

The next design input after MVP is measurement — fixture workspaces, stress tests, and real usage — not further UI speculation.
