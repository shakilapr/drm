# DRM Work Plan — 24-Month Program

Normative sources: [docs/architecture.md](docs/architecture.md), [docs/spatial_architecture.md](docs/spatial_architecture.md). This plan schedules implementation; it never overrides architecture.

## Program Parameters

- **Duration**: 24 months (M1 = program start).
- **Team**: 4 engineers. Testing is a shared duty; a rotating test lead owns each wave's exit gate.
- **Stack**: `drm-core` and the `drm` toolchain in Rust with a pure canonical C ABI; first-party bindings C, C++, Rust, Python.
- **Scope**: all 34 development phases, including DRM Spatial (Phase 33).
- **Architecture discipline**: AGENTS.md directives apply at all times (architecture-first, reference-only, clean-room, brokerless/single-writer, transient borrow lifecycle, zero data-path allocation, terse, 100% verification).

## Team and Roles

| Role | Ownership |
|---|---|
| **E1** | Runtime core: shared-memory backends, channels, rings, hazard protocol, epochs, precommit, Windows port, trace/RPC runtime |
| **E2** | Toolchain: schema grammar, type system, canonical layout, handles, WorkspaceModel, codegen, verify, spatial compiler |
| **E3** | Boundary router, transports, scheduling/shaping, serial/embedded integration |
| **E4** | Bindings (C ABI stewardship, C++, Python), CI, fuzz/property/sanitizer infrastructure, benchmarks, integration harness |

Rotating test lead order per wave: E4 → E1 → E2 → E3 → repeat.

## Requirement Discipline (Phase 1 feeding Phase 34)

1. Phase 1 mints a normative requirement registry: every normative statement in the architecture gets a unique ID.
2. Every later phase's implementation and test cites requirement IDs.
3. Phase 34 closes the program only when requirement → implementation → test → passing result coverage is 100%.

## Wave Overview

Waves overlap deliberately (pipeline). A phase is listed under the wave in which its work is performed; its completion gate may be exercised again by later waves (regression is permanent).

| Wave | Months | Phases | Lead |
|---|---|---|---|
| W1 Foundations & Specification | M1–M3 | 1, 2, 3, 7 (draft), 8 | E2 |
| W2 Core Runtime | M3–M9 | 4, 5, 6, 9–19, 7 (finalize gate) | E1 |
| W3 Bindings, Toolchain & Verification | M6–M12 | 20, 21, 22, 23, 24, 25, 26, 27, 28 | E2 |
| W4 Boundary Router & Transports | M10–M16 | 29, 30 | E3 |
| W5 Windows Runtime Backend | M14–M18 | 31 | E1 |
| W6 Operational Tooling | M15–M19 | 32 | E4 |
| W7 DRM Spatial | M19–M22 | 33 | E2 |
| W8 Whole-System Qualification & Release | M22–M24 | 34 | All |

---

## W1 — Foundations & Specification (M1–M3)

**Phase 1 — Architecture specification & normative rules**
Output: machine/reference specification defining every DRM concept, state, error and invariant.
Done when:
- Every normative statement in the architecture has a unique requirement ID.
- Ambiguous or conflicting requirements = 0.
- Every later implementation requirement traces back to one of these IDs.
- Undefined runtime states = 0.
Owner: E2 (all contribute).

**Phase 2 — Schema grammar & parser**
Output: parser for `.drm` topic definitions.
Done when:
- Every documented valid schema parses; every documented invalid schema fails with location + reason.
- Malformed/fuzz-generated input never crashes or hangs.
- Parser round-trip tests are deterministic.
Owner: E2.

**Phase 3 — DRM type system**
Output: primitive types, fixed arrays, bounded arrays/strings/bytes, enums, structural bounds.
Done when:
- Every architecture-described type has encode/decode/layout tests.
- Illegal unbounded data-plane structures are rejected.
- Boundary values (`0`, `max`, empty, full capacity) all pass.
Owner: E2.

**Phase 7 (draft) — Canonical C ABI**
Output: draft stable `drm.h`, opaque context/publisher/consumer handles, fixed int32 statuses (no `Internal`), copy publish, publisher write-loan (`write_acquire`/`commit`/`abort`), borrow/copy read operations, ABI versioning + one-way compatibility policy, fail-stop panic rule.
Done when (draft gate): plain C programs can compile against the draft header in a spike; final completion gate closes in W2.
Owner: E1 + E4.

**Phase 8 — Linux shared-memory backend**
Output: Linux process/shared-memory primitives, atomic mapping and locking.
Done when:
- Independent processes map the same channels correctly.
- Simultaneous startup races cannot create conflicting segments.
- Shared-memory cleanup is safe.
- Sanitizer/stress runs produce zero corruption, zero DRM-attributable leaks, zero data races, zero invalid mappings.
Owner: E1.

---

## W2 — Core Runtime (M3–M9)

**Phase 4 — Canonical binary layout**
Output: architecture-independent canonical representation.
Done when:
- Golden byte vectors are identical from C/C++/Rust/Python.
- Native compiler alignment/padding cannot change wire representation.
- Little-endian format verified.
- Random encode→decode→encode property tests reproduce identical bytes.
Owner: E2 (+E4 golden vectors).

**Phase 5 — Metadata layering**
Output: channel-, sample- and transport-level metadata formats.
Done when:
- Fixed samples contain no unnecessary length/schema/topic envelope.
- Variable samples contain required length only.
- Channel metadata validated on attachment.
- Transport metadata appears only at physical boundaries.
- Byte-layout inspection tests prove each rule.
Owner: E1 + E2.

**Phase 6 — Topic identity & numeric handle system**
Output: canonical topic paths, stable generated runtime handles, explicit ID compaction.
Done when:
- Same workspace rebuild preserves every assigned handle.
- Adding unrelated topics cannot renumber existing topics.
- Deleting/readding behaves per defined policy.
- Only `drm ids compact` changes established assignments.
Owner: E2.

**Phase 7 (finalize) — Canonical C ABI**
Full completion gate:
- Plain C programs built independently can use DRM.
- ABI compatibility suite passes against previously supported ABI versions.
- Symbol/layout ABI checker shows no accidental breaking changes.
Owner: E1 + E4.

**Phase 9 — Structured local channels**
Output: single-writer/multi-reader fixed structured communication.
Done when:
- One publisher + many readers run billions of publish/read cycles with 0 torn samples, 0 mixed generations, 0 invalid payloads, monotonically valid sequence behavior.
Owner: E1.

**Phase 10 — Shared history rings**
Output: `history: latest`, `history: N`, circular sequence history.
Done when:
- Exhaustive tests for depths 1…N.
- Wraparound occurs repeatedly without changing retained-history semantics.
- Exactly the declared `R` completed sequences remain addressable.
Owner: E1.

**Phase 11 — Reader cursors & delivery semantics**
Output: `latest`, `next`, independent reader positions, `GAP` reporting.
Done when:
- Readers at different speeds never affect each other's cursor.
- Intentional lapping verifies exact `missed: N`.
- `latest()` always resolves the newest committed sample.
- `next()` never silently skips retained sequences.
Owner: E1.

**Phase 12 — History overflow policies**
Output: `overwrite` and `reject`.
Done when:
- Overwrite produces exact gaps; reject returns `QUEUE_FULL` without secretly blocking.
- Required sequential readers correctly gate rejection.
- Publisher behavior matches specification under full history.
Owner: E1.

**Phase 13 — Bulk payload pool**
Output: fixed-capacity shared-memory payload slots.
Done when:
- Maximum payload, empty payload and all intermediate sizes operate correctly.
- Slot offsets never relocate.
- Topic pools cannot borrow from one another.
- Attempts beyond structural maximum are rejected before memory overwrite.
Owner: E1.

**Phase 14 — Zero-copy borrowing & hazard protocol**
Output: `borrow_latest`, `borrow_next`, hazard slots, generations.
Done when:
- Every race point in the acquisition algorithm is forced.
- An actively borrowed slot is never overwritten.
- ABA tests across repeated slot reuse pass.
- Borrow release makes slots reusable.
- Multiple readers of the same latest sample remain safe.
Owner: E1 (race suite: E4).

**Phase 15 — Pool exhaustion behavior**
Output: nonblocking publisher behavior, `pool_exhausted` accounting.
Done when:
- Holding every usable slot deliberately: publisher returns/drops exactly per specification and never waits for a reader.
- Current latest remains valid; counter increments exactly.
- Publishing resumes immediately when capacity returns.
Owner: E1.

**Phase 16 — Dead-reader recovery**
Output: registration-lock authority, abandoned hazard reclamation, fork sanitization.
Done when:
- Killing a reader at every possible borrow stage: slots eventually recovered only after the endpoint's registration lock is acquired and state/generation/hazard reread under it.
- Simulated PID reuse and cross-PID-namespace visibility can never reclaim a live reader's slot (identity is diagnostic only).
- Hard-RT publisher path runs zero inspection syscalls on exhaustion; maintenance context reclaims via `reclaim_requested`.
- Fork with live publisher/consumer/borrow: child mutates nothing, releases no lock; parent authority intact.
Owner: E1.

**Phase 17 — Publisher lifecycle & epochs**
Output: publisher locks, restart semantics, `publisher_epoch`.
Done when:
- Racing two publishers: exactly one acquires authority.
- Killing the active publisher: OS releases authority.
- Restart increments epoch exactly once; consumers discard old cursor assumptions.
- No pre-restart sample is interpreted as post-restart data.
Owner: E1.

**Phase 18 — Deployment identity & isolation**
Output: interface fingerprints, deployment IDs, namespaces, attachment validation, cleanup.
Done when:
- Incompatible deployments run simultaneously with zero cross-attachment.
- Incompatible binary is refused.
- Unrelated schema change does not change unaffected node fingerprint.
- Active deployment cannot be cleaned; inactive deployment can be cleaned completely.
Owner: E1 + E2.

**Phase 19 — Memory architecture & precommit**
Output: fixed per-topic partitions, memory-budget verifier, optional locked/pre-faulted operation.
Done when:
- Exact worst-case capacity calculation matches independently calculated reference values.
- Invalid `pool_size < R+1` fails.
- One topic can exhaust itself without affecting another.
- `precommit` either successfully pre-faults/locks the complete declared working set or refuses activation.
Owner: E1 + E2.

**W2 exit demo**: three processes (C, Rust, Python) exchange zero-copy frames locally; kill -9 reclamation, epoch restart, and pool-exhaustion suites green.

---

## W3 — Bindings, Toolchain & Verification (M6–M12)

**Phase 20 — C++ binding**
Output: type-safe generated C++ API, RAII borrows whose owning object is also the accessor (no detached view/span escape).
Done when:
- Every schema maps correctly.
- Borrow automatically releases on every scope-exit path including exceptions.
- No accessor returns a pointer, reference, or span into the slot; retention requires copy.
- Undeclared operations are absent from the generated node API; the shared types layer exposes types/codecs only.
- C++↔C/Rust/Python interoperability matrix passes.
Owner: E4.

**Phase 21 — Rust binding**
Output: type-safe Rust API, scoped zero-copy references, encoded `!Send + !Sync` guards.
Done when:
- Borrow lifetimes prevent use-after-release in safe code.
- `PhantomData<*mut ()>` makes guards `!Send + !Sync`; compile-fail tests cover cross-thread move/share.
- Compile-fail tests cover forbidden lifetime/contract cases.
- Generated layouts match golden bytes.
- Interoperability matrix passes.
Owner: E1.

**Phase 22 — Python binding**
Output: generated Python API, owned-default reads only (no zero-copy borrow API, no buffer-protocol exposure).
Done when:
- `latest()`/`next()`/`.numpy()` return application-owned memory; no Python-visible object references DRM shared memory (hazard-free after every call).
- PyO3 classes are `Send + Sync`; per-`Topic` native locks keep cursor calls coherent.
- Free-threaded and GIL builds pass the full suite.
- Forced GC/exceptions cannot pin slots beyond a call.
- Contract violations raise `ContractError`.
Owner: E4.

**Phase 23 — Workspace & manifest model**
Output: packages, nodes, topics, compute units, links, imports, local contracts in `WorkspaceModel`.
Done when:
- Every valid workspace structure builds exactly one deterministic model.
- Duplicate/conflicting identities are rejected.
- Filesystem ordering cannot change model output.
- Parse→model results are reproducible.
Owner: E2.

**Phase 24 — Code generation & invalidation**
Output: C/C++/Rust/Python generators, node interfaces, fingerprints, stable `.drm/gen/...` paths.
Done when:
- Identical inputs produce byte-identical outputs.
- Unchanged artifacts retain timestamps.
- Changed inputs regenerate exactly affected artifacts.
- Interrupted generation leaves no partial file.
- Deleted declarations leave no stale generated API.
Owner: E2.

**Phase 25 — Native build orchestration & `drm dev`**
Output: CMake/Ninja, Cargo, custom build adapters; filesystem watch mode.
Done when:
- DRM declaration change rebuilds exactly the affected DRM artifacts/nodes.
- Application source change delegates dependency decisions to the native builder.
- DRM never starts tracking source/header dependency graphs.
- Repeated watch edits never produce stale generated code.
Owner: E2.

**Phase 26 — Static verification engine**
Output: `drm verify` deterministic configuration checks.
Done when — full negative-test matrix proves rejection of:
- No publisher; multiple publishers; invalid schemas; conflicting IDs; invalid placement; impossible route; framing overflow; reliability mismatch; incompatible consumer requirements; memory-budget violations; precommit-budget violations.
- All valid equivalents pass.
Owner: E2.

**Phase 27 — Feasibility estimation**
Output: link-utilization, expected-rate, capacity reports.
Done when:
- Independently calculated reference networks match DRM estimates exactly.
- >100% declared link demand rejected where architecture requires.
- Estimates clearly distinguished from runtime guarantees; no estimated timing result reported as proven.
Owner: E2.

**Phase 28 — Topology & automatic routing**
Output: publisher→consumer graph routing, local elimination, overrides, external endpoints.
Done when — exhaustive topology fixtures prove:
- Local topics never leave the compute unit.
- Unique paths route automatically.
- Multiple valid paths without policy fail as ambiguous.
- `prefer`, `force`, `deny` produce exact routes.
- External endpoints participate in the same graph without a parallel export model.
Owner: E2 + E3.

**W3 exit demo**: full workspace builds, verifies, deploys; four-language interop matrix green; `drm dev` watch cycle clean.

---

## W4 — Boundary Router & Transports (M10–M16)

**Phase 29 — Boundary router & transports**
Output: `drm-router`, UDP, TCP, framing, import/export, connection handling.
Done when:
- Two physical hosts exchange every supported topic class.
- UDP never uses DRM fragmentation; oversized UDP sample rejected by verification.
- TCP reconstructs exact samples.
- Received external data is indistinguishable from local topic data to consumers.
- Disconnect affects remote routes only.
Owner: E3.

**Phase 30 — Router scheduling, reliability & shaping**
Output: realtime/bulk TCP lanes, priority queues, token-bucket shaping, reliability capability enforcement.
Done when:
- Under bulk saturation, realtime reliable traffic is never queued behind bulk on the same ordered lane.
- Configured rate limits hold within defined tolerance.
- Reliable-over-unsupported transport is rejected.
- Priority ordering follows declared policy under artificial saturation.
Owner: E3.

**W4 exit demo**: two-machine deployment — control loop on realtime lane, camera frames on bulk lane, token buckets enforced, link-cut leaves local loops uninterrupted.

---

## W5 — Windows Runtime Backend (M14–M18)

**Phase 31 — Windows runtime backend**
Output: Windows shared memory, process identity, locks, mapped sections, networking, lifecycle.
Done when:
- The entire core conformance suite from Linux runs on Windows.
- Same generated bytes and API semantics.
- Windows process crash recovery works.
- Windows↔Linux communication matrix passes for every supported transport and language combination.
Owner: E1 (+E4 matrix harness).

---

## W6 — Operational Tooling (M15–M19)

**Phase 32 — Operational tooling: inspect, diagnostics, record/replay, logging, trace, RPC/actions, embedded/serial**
Output: `drm echo`, `hz`, `log`, `diag`, `record`, replay, tracing, RPC/actions, constrained-device runtime.
Done when — each submodule has its own conformance suite:
- Introspection is read-only and cannot pin bulk samples.
- Diagnostics detects every injected architecture-defined condition.
- Recorder never performs disk I/O from borrowed DRM memory.
- Replay preserves samples/order/timestamps/gaps.
- RPC responses never cross-correlate; cancellation is deterministic.
- Trace buffers never block publishers.
- MCU steady-state path performs zero forbidden heap allocation.
- Serial framing interoperates byte-for-byte with host.
Sub-owners: introspection/logging E4 · diag/record/replay E4 · trace/RPC E1 · serial/MCU E3.

---

## W7 — DRM Spatial (M19–M22)

**Phase 33 — DRM Spatial**
Output: complete spatial-temporal subsystem per [docs/spatial_architecture.md](docs/spatial_architecture.md).
Done when — all Spatial requirements pass:
- Typed `Point3/Direction3/Vector3/Pose3/Transform3/Twist3`.
- Compile-time and dynamic spaces; forests/disconnected graphs.
- Axis/geodetic conversions; sample/receive time.
- Affine clock-domain mapping (reference epoch, rate ratio, bounded uncertainty, identity epoch); unresolved clocks rejected.
- Static-chain collapse; versioned cache (geometry-only); coherent `latest` (common timestamp); `at`/`predict` (SE(3) exp-map)/`delayed_view`; provable history window (`min_sample_period`).
- SLERP/linear interpolation; no interpolation over discontinuity (per-edge marks).
- Historical reparenting with Preserve World/Local/Snap; live calibration.
- Generational runtime handles; bounded runtime overlay plans; single-writer spatial edges.
- Incremental dynamic hash grid (dependency-vector invalidation, grid epoch); compiled BVH; proximity candidate completeness (explicit `OutputTooSmall`).
- Exact spatial memory verification.
- Host/MCU paths all pass against mathematical reference implementations.
Owner: E2 (compiler/graph) + E1 (runtime caches); tests E4.

---

## W8 — Whole-System Qualification & Release (M22–M24)

**Phase 34 — Whole-system architecture qualification**
Output: complete DRM implementation and architecture conformance suite.
Done when:
- Every requirement ID from Phase 1 maps to: implementation + automated test + passing result.
- Architecture coverage = 100%; requirement with no implementation = 0; requirement with no test = 0.
- All OS×language×transport compatibility cells declared supported pass.
- All crash/fault-injection suites pass.
- All fuzz/property/sanitizer suites pass.
- Long-duration soak completes with zero corruption/deadlock.
- Deterministic builds pass.
- Memory/runtime benchmarks show no architecture invariant violation.
- Documentation/examples execute in CI.
- No unresolved P0/P1 correctness defect remains.
Owner: all.

**Release close-out (parallel, M23–M24)**: pilot robot integration, docs completion, packaging (crates + system packages), API freeze, v1.0 tag + schedule buffer.

---

## Standing Practices (all waves)

- **CI matrix**: Linux x86_64 + aarch64, Windows x86_64; every merge.
- **Continuous gates**: fuzz, property, sanitizer suites run from first code, not deferred to W8.
- **Zero-allocation audits**: steady-state data path checked per release of each runtime phase.
- **Traceability**: requirement registry updated in the same change as implementation; untraceable code rejected in review.
- **Architecture feedback**: implementation may falsify architecture. On falsification: stop, capture failure evidence, compare ≥2 alternatives on pre-stated criteria, record ADR, update architecture + algorithm doc in the same PR, then implement (AGENTS R1). Weekly plans carry `SPIKE`/`EXP` tasks wherever risk is unresolved (weekly conventions §Experiments).
- **Regression permanence**: a phase's completion suite becomes a permanent CI gate once green.

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| Hazard-protocol subtlety (ABA, race windows) | Data corruption in core | Phase 14 exhaustive race forcing; protocol-level model tests; permanent CI gates |
| Linux/Windows SHM semantic divergence (paging, commit, locks) | W5 slips | Backend trait abstraction frozen in W1; Windows spike kept alive from M1; parity tests per phase, not deferred |
| Requirement drift between architecture edits and registry | False 100% coverage | Registry diff required in same PR as any architecture change; W8 re-audit |
| Router lane/shaping interactions under real saturation | Priority inversion on links | Phase 30 artificial saturation suite; two-host soak in W4 exit |
| Spatial scope (largest single subsystem) compressing W8 | Qualification squeeze | W7 starts M19 while W5/W6 wind down; spatial compiler reuses W3 verify/codegen infrastructure; W8 buffer is 2 months |
| Serial/MCU target diversity (toolchains, RTOS) | Long tail | Fix supported MCU targets at W6 start; zero-alloc gate enforced by linker section checks, not review only |
| Architecture assumption falsified during implementation | Divergent local hacks, silent drift | Timeboxed `SPIKE`/`EXP` tasks with ≥2 candidates and decision criteria; ADR + same-PR architecture update; regression permanence keeps fixes honest |
