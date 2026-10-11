# Studio Prep Survey (M14W4)

Non-normative derived spec. Source of truth: [studio-protocol.md](../algorithms/studio/studio-protocol.md)
(§Invariants I-ST1..I-ST13, §Functions, §Protocols), [ui-architecture.md](../ui-architecture.md)
(§Core Principles, §Process Architecture, §Protocol, §Transactional Mutations), [codebase.md](../../codebase.md)
§3.1 R6 (generated code). On any conflict the architecture docs win; on any pseudocode conflict
the algorithm docs win (codebase.md §2.4). Owner: E2 (protocol generation, stdio host) + E4
(supervisor). Phase 32 prep. Written M14W4. The formal Studio doc is authored M17W2.

Surveys and locks the `drm studio --stdio` host shape, the generated TypeScript types pipeline,
the engine-only mutation flow, and the capability model. Records the deferred-scope list so the
scope cannot creep before 1.0.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-STUDIO-engine-only-mutations` | every mutation executed and validated by the Rust engine | `REQ-STUDIO-003` |
| `REQ-STUDIO-generated-protocol` | `studio-protocol` is codegen output; hand edits rejected (R6) | `REQ-STUDIO-006` |
| `REQ-STUDIO-handshake` | first RPC is a capability handshake | `REQ-STUDIO-007` |
| `REQ-STUDIO-typed-commands` | ui-arch typed command set | `REQ-STUDIO-014` |
| `REQ-STUDIO-two-planes` | JSON-RPC for commands/state; binary frames for samples/video | `REQ-STUDIO-013` |
| `REQ-STUDIO-capability-model` | renderer receives domain capabilities, never machine access | `REQ-STUDIO-001` |
| `REQ-STUDIO-revisioned-concurrency` | snapshot `revision` + mutation `base_revision`; stale ⇒ `CONFLICT` | `REQ-STUDIO-009` |
| `REQ-STUDIO-journaled-changesets` | multi-file operations journaled; crash recovery on startup | `REQ-STUDIO-008` |
| `REQ-STUDIO-conditional-undo` | undo reverts only when expected input hashes still match | `REQ-STUDIO-002` |
| `REQ-STUDIO-format-preserving` | engine never rewrites a file it did not author structurally | `REQ-STUDIO-005` |
| `REQ-STUDIO-run-supervisor` | run sessions outlive the Studio window | `REQ-STUDIO-010` |
| `REQ-STUDIO-stdio-host` | `drm studio --stdio` host; no HTTP server, no ports | `REQ-STUDIO-011` |
| `REQ-STUDIO-trust-gate` | untrusted workspace view-only; Build/Run/extensions gated | `REQ-STUDIO-012` |
| `REQ-STUDIO-ext-budgets` | extension host: wasmtime fuel + memory caps | `REQ-STUDIO-004` |

## 1. `drm studio --stdio` host shape (locked)

`drm studio --stdio` is a line-oriented JSON-RPC host over stdin/stdout: no HTTP server, no
ports, and Studio holds no DRM logic (`REQ-STUDIO-011`).

```rust
fn serve_stdio(mut rx: impl Read, tx: impl Write) -> Result<(), Status> {
    let first: StudioRequest = parse_bounded(rx.next_line_bounded(MAX_CONTROL_BYTES)?)?;
    let hello = handshake(first)?;                       // MUST be Hello
    tx.write(HelloAck(hello).to_json_line())?;
    for line in rx.json_lines_bounded(MAX_CONTROL_BYTES) {
        let req: StudioRequest = parse_bounded(line)?;
        let resp = dispatch(req)?;                       // engine calls only
        tx.write(resp.to_json_line())?;
    }
    Ok(())
}
```

- Line length, collection counts, and decoded request size are bounded **before** allocation-heavy
  parsing (I-ST13).
- The first RPC must be `Hello`; any other first request rejects the connection (I-ST7 handshake
  gate). Malformed lines produce an error response and the connection continues.
- Commands/state travel over JSON-RPC; topic samples/video travel over binary frames — never mixed
  (I-ST3, `REQ-STUDIO-013`).

## 2. Generated TypeScript types pipeline (locked)

- `packages/studio-protocol` is **generated** from the Rust request/response enums; hand edits are
  rejected (R6, `REQ-STUDIO-006`).
- Generation is an extension of the `xtask gen-check` pipeline: the generated TS types + JSON-RPC
  schema carry a `@generated` banner and are re-generated and diffed in CI (AGENTS #13). A stale or
  hand-edited artifact fails the build.
- The typed command set mirrors [ui-architecture.md](../ui-architecture.md) §Protocol:
  Workspace / Package / Node / Topic / Compute / Link / Build / Run (`REQ-STUDIO-014`).

## 3. Engine-only mutation flow (locked)

Every mutation goes through the engine's parse → model → verify pipeline; Studio never validates
independently (I-ST1, `REQ-STUDIO-003`).

1. Studio sends a domain command with `base_revision` (never raw file writes).
2. Engine: trust + revision + contained-path checks → format-preserving apply (`toml_edit`/CST,
   I-ST6) → re-parse → verify.
3. Verify failure: full failure report (check-IDs + REQ citations); no partial apply; revision
   unchanged.
4. Success: flush temps → flush journal → rename → flush parents → mark committed → remove journal;
   response = new revision + canonical render (I-ST8, `REQ-STUDIO-008`).

Stale `base_revision` ⇒ `CONFLICT{revision}` — UI offers Review/Reload/Retry; no last-writer-wins
(I-ST7, `REQ-STUDIO-009`). Undo reverts a ChangeSet only when expected input hashes still match;
external changes offer Compare/Cancel/Force (I-ST9, `REQ-STUDIO-002`).

## 4. Capability model (locked)

- The handshake is `Hello {protocol_version, client_version}` → `HelloAck {protocol_version,
  host_version, capabilities}`; it is never an exact-version requirement (`REQ-STUDIO-007`).
- `Capability` values (e.g. `WorkspaceV2`, `ExtensionsWeb`, `BinaryStreamsV1`, `RuntimeStats`,
  `Trust`) drive feature hiding in the renderer.
- The renderer receives domain capabilities (allowed topics/actions), never raw machine/SHM
  access (I-ST4, `REQ-STUDIO-001`).
- Untrusted workspaces stay view-only; Build/Run/workspace extensions/native adapters gate behind
  `[ Trust Workspace ]` (I-ST11, `REQ-STUDIO-012`).

## 5. Deferred scope (recorded, post-1.0)

| Item | Disposition |
|---|---|
| Creation UI (wizards/forms) | deferred post-1.0 |
| Editors (text/graph) | deferred post-1.0 |
| Inspector panels | deferred post-1.0 |
| Remote-web serving | deferred post-1.0; direct listening disabled until TLS, origin validation, authentication |
| Extension platform | designed second phase (wasmtime fuel + memory caps) |

## 6. Acceptance (M14W4)

- [x] `drm studio --stdio` JSON-RPC host shape locked (bounded, handshake-first, no ports).
- [x] Generated TS types pipeline locked (R6 gen-check extension).
- [x] Engine-only mutation flow locked (trust/revision/path → format-preserving → verify → journal).
- [x] Capability model locked; renderer receives capabilities, never machine access.
- [x] Deferred-scope list recorded.

Deltas: none to studio-protocol.md; this survey fixes the concrete approach. Implementation lands
in Phase 32/33 (M15W1+); the formal doc is authored M17W2.

REQ: REQ-STUDIO-001, REQ-STUDIO-002, REQ-STUDIO-003, REQ-STUDIO-004, REQ-STUDIO-005,
REQ-STUDIO-006, REQ-STUDIO-007, REQ-STUDIO-008, REQ-STUDIO-009, REQ-STUDIO-010,
REQ-STUDIO-011, REQ-STUDIO-012, REQ-STUDIO-013, REQ-STUDIO-014.
