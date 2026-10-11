# Studio Mutation Audit (M17W4)

Scope: audit every GUI-visible mutation and confirm it maps to exactly one
engine operation, and that the renderer holds zero DRM logic (I-ST1,
`REQ-STUDIO-003`). Non-normative; the architecture docs win conflicts. Normative
sources: [ui-architecture.md](../ui-architecture.md) Core Principles/§Protocol,
[studio-protocol.md](../algorithms/studio/studio-protocol.md) I-ST1.

## Method

1. Enumerate every `StudioRequest` the renderer can send.
2. Map each to the single `drm-workspace` engine call that executes/validates it.
3. Confirm the renderer (`apps/studio`) issues no mutation and duplicates no
   validation: inspectors and the Problems panel call `snapshot`/`verify` only.

## Request → engine operation

| Renderer request | Engine operation | Kind | REQ |
|---|---|---|---|
| `hello` | capability handshake (`HelloAck`) | read-only | `REQ-STUDIO-007` |
| `open_workspace` | `ws::discover` + `ws::build_model`; trusted ⇒ journal recovery | read-only | `REQ-STUDIO-003`/`REQ-STUDIO-008` |
| `snapshot` | `ws::build_model` + `render_model` + `verify_lite` | read-only | `REQ-STUDIO-014` |
| `verify` | `verify_lite` | read-only | `REQ-STUDIO-003` |
| `mutate` (`node_update`) | `edit::apply_node_update` → model rebuild → `verify_lite` → journaled `ChangeSet` commit | mutation | `REQ-STUDIO-003`/`REQ-STUDIO-005`/`REQ-STUDIO-008` |
| `build_start`/`build_cancel` | build gate; headless completion is supervisor-owned | gated action | `REQ-STUDIO-012` |
| `run_start` | `supervisor::Supervisor::start` | gated action | `REQ-STUDIO-010` |
| `run_stop` | `supervisor::Supervisor::stop` | gated action | `REQ-STUDIO-010` |
| `run_restart` | `supervisor::Supervisor::restart` | gated action | `REQ-STUDIO-010` |
| `undo` | conditional journal revert (input-hash guard) | mutation | `REQ-STUDIO-002` |
| `subscribe_telemetry` | binary-plane token only; bytes never JSON | read-only | `REQ-STUDIO-013` |

## Renderer-side check

- `apps/studio/src/activities/*` and `components/ProblemsPanel.tsx` render
  `snapshot.model` / `snapshot.problems` and issue no `mutate`/`undo` request;
  `rpc/model.ts` is a display-only projection of engine output and performs no
  validation (`REQ-STUDIO-003`).
- The Problems panel styles the engine-provided severity classes
  (`error`/`warning`/`runtime`/`info`) and computes no diagnostics of its own
  (`REQ-STUDIO-014`).

## Verdict

Every GUI-visible mutation maps to exactly one engine operation; zero
renderer-side DRM logic. Audit clean (`REQ-STUDIO-003`).

REQ: REQ-STUDIO-002, REQ-STUDIO-003, REQ-STUDIO-005, REQ-STUDIO-007,
REQ-STUDIO-008, REQ-STUDIO-010, REQ-STUDIO-012, REQ-STUDIO-013,
REQ-STUDIO-014.
