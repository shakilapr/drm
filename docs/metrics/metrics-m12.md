# Metrics — Month 12 (M12)

Normative source: [work-plan.md](../../work-plan.md) W5 Windows Runtime Backend
(starts) with the Phase 30 gate closing the W4 exit. One section per weekly
tranche.

## M12W1 — Phase 30 Gate (Scheduling, Reliability & Shaping), Windows Port Start

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 30)` | PASS | PASS — [phase-30.md](../gates/phase-30.md); four #30 criteria ⇄ evidence, `I-LS1`..`I-LS5`/`I-LS7`/`I-LS9` supporting |
| `WIN_BACKEND_SMOKE` | section create/map + lock smoke green on the Windows runner | `cargo test -p drm-core --lib win_` — `win_section_roundtrip`, `win_open_semantics`, `win_precommit_roundtrip`, `win_lock_roundtrip`, `win_process_identity` (5/5) |
| `SHAPING_TOLERANCE` | limits hold within defined tolerance (`I-LS3`/`I-LS4`) | `cargo test -p drm-router shaping`: `shaping_property` (10 000 deterministic traces, 0 byte-accounting violations), `token_bucket_fractional`, `shaping_wait_model`, `burst_then_refill` |
| `PRIORITY_SATURATION` | declared priority order holds under artificial flood (`I-LS1`) | `cargo test -p drm-router --test priority_saturation`: `priority_saturation` green (1 000-item bulk flood, realtime never starves) |
| `V_REL` | reliable-over-unsupported transport rejected | `cargo test -p drm-workspace --test verify_negatives_v2`: `verify_negatives_v2_reliability` green (reliable over UDP/serial rejected; TCP passes) |
| `WINDOWS_CI` | windows runner runs workspace + `drm-core` platform smoke | `.github/workflows/ci.yml` `windows-smoke` job (windows-latest) green |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Windows SHM backend `crates/drm-core/src/shm/windows.rs` (`WinShm`):
  pagefile-backed `CreateFileMappingW`/`MapViewOfFile` sections, `ERROR_ALREADY_EXISTS`
  ⇒ `AlreadyExists` (`I-W1`), `OpenFileMappingW` miss ⇒ `NotFound` (`I-W2`),
  undersized-section `StaleIncomplete` classification, `VirtualLock` precommit
  with explicit refusal (`I-W6`), and `unlink` as a documented no-op (pagefile
  sections die with their last handle, `I-W4`/`I-W4b`).
- Windows lock backend `WinLockBackend`: non-blocking `LockFileEx` over a fixed
  one-byte range on non-inheritable handles; the kernel releases the range when
  the last handle closes (`I-W3`, `I-W8`).
- Windows diagnostic identity (`current_identity`, `is_alive`, `identity_alive`)
  as PID + creation `FILETIME`, incarnation-safe and never consulted by
  reclamation (`I-W5`).
- Platform naming: `windows_section_name`/`windows_name_conforms`
  (`Local\drm_<deployment_id>_<topic_hash>`) with a platform-neutral unit test
  (`shm::tests::windows_name_conformance`) so the rule is exercised on every
  host; `SegmentName`/`backend_segment_name` select the platform form.
- `drm-core` gains a `cfg(windows)` `windows-sys` dependency (Foundation,
  Security, FileSystem, IO, Memory, Threading features).
- Gate doc [phase-30.md](../gates/phase-30.md) mapping the four work-plan #30
  criteria to executable tests.

Notes:

- The full per-link `LinkScheduler` safety-budget/DRR/export-limiter increments
  (`I-LS6` full, `I-LS8`, `I-LS11`, `I-LS12`) are W5 scheduler hardening; the
  #30 acceptance criteria are met by the lane split, token bucket, and
  pre-stream class queues above.
- Platform-only suites that still bind POSIX objects (`shm::process`,
  `shm::lock`, `channel::precommit` tests) are `cfg(unix)`-gated; their Windows
  equivalents run as they land under Phase 31. `cargo test -p drm-core --lib`
  therefore compiles and runs on both hosts.
- `windows-backends.md` was already merged before this week (E1-T0 acceptance);
  no divergence was recorded, so no algorithm-doc change landed.

## M12W2 — Phase 26 Gate (Verify Engine), Windows SHM + Locks

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 26)` | PASS | PASS — [phase-26.md](../gates/phase-26.md); all eleven #26 rejection classes ⇄ executable fixtures, `I-V1`..`I-V6` supporting |
| `WIN_IDENTITY_LOCKS` | identity + locks suites green on Windows (`I-W3`, `I-W5`) | `cargo test -p drm-core --lib win_` — `win_lock_release_on_death` (`I-W3`), `win_pid_reuse` (`I-W5`), `win_active_hold_lifecycle` (`I-W8`), `win_create_failure_matrix` (`I-W7`), `win_close_last_handle` (`I-W4b`), plus the M12W1 smoke tests |
| `WIN_SOCKETS` | UDP/TCP pair green on Windows | `cargo test -p drm-router --lib transport` — loopback pair + `udp_error_mapping_parity`/`tcp_error_mapping_parity` |
| `WIN_ASAN` | ASan (clang-cl) over Windows suites, 0 findings | scheduled nightly job (`.github/workflows/sanitizers.yml` `windows-asan`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Phase 26 gate doc [phase-26.md](../gates/phase-26.md): the complete #26
  negative matrix — no publisher (`V-SW-1`), multiple publishers (`V-SW-2`),
  invalid schemas (`V-SCH`), conflicting IDs (`V-ID`), invalid placement
  (`V-PLC`), impossible route / framing overflow (`V-RT`), reliability mismatch
  (`V-REL`), incompatible consumer requirements (`V-CMP`), memory-budget and
  precommit-budget violations (`V-MEM`) — each rejected with check ID + REQ
  citation; valid equivalents pass.
- Windows identity + locks suite (`crates/drm-core/src/shm/windows.rs`): release
  on death (`I-W3`, kill-then-reacquire), incarnation/PID-reuse guard (`I-W5`),
  active-hold lifecycle (`I-W8`), create-conflict rollback (`I-W7`), and
  last-handle section teardown (`I-W4b`). `test_guard` is now compiled on every
  test host (plain mutex), so the spawn/lock tests serialize on Windows too.
- Windows transports smoke: socket error-mapping parity fixtures in
  `drm-router::transport::{udp,tcp}` (platform-neutral) alongside the existing
  loopback pair tests.
- CI: the `windows-smoke` job also runs the router transport subset; new
  `windows-nightly.yml` schedules the platform subset (SHM/identity/locks/
  sockets) nightly; `sanitizers.yml` gains a scheduled `windows-asan` job.

Notes:

- Windows-only suites cannot run on the Linux authoring host; they are
  cross-compiled (`cargo clippy -p drm-core -p drm-router --target
  x86_64-pc-windows-msvc --tests -D warnings`) and gated by the `windows-latest`
  CI jobs.
- `WIN_ASAN` uses nightly `-Zsanitizer=address` on MSVC; if the toolchain proves
  unavailable the triage carries over to M12W3 Day 1 (E1), as the week's risk
  register allows.
- No architecture divergence: the engine and Windows backend already match
  [verify-engine.md](../algorithms/toolchain/verify-engine.md) and
  [windows-backends.md](../algorithms/runtime/windows-backends.md), so no ADR or
  algorithm-doc change landed.

## M12W3 — Windows Reclamation + Precommit, Cross-OS Matrix Design

| Variable | Target | Measured |
|---|---|---|
| `WIN_RECLAIM_GREEN` | kill matrix + PID-reuse green on Windows | `cargo test -p drm-core --lib win_` — `win_lock_release_on_death` (`I-W3`), `win_pid_reuse` (`I-W5`), `win_identity_child`, plus `win_active_hold_lifecycle` (`I-W8`); the registration lock is reclamation authority, identity is diagnostic only |
| `WIN_PRECOMMIT` | refusal + success paths green on Windows | `cargo test -p drm-core --lib win_precommit_roundtrip` (`I-W6`): `VirtualLock` success is the proof, failure ⇒ `QuotaExceeded` ⇒ refuse activation |
| `MATRIX_HARNESS` | OS×language×transport cells enumerated + harness running | cell enumeration in [tests/conformance/README.md](../../tests/conformance/README.md); executable subset harness carried into M12W4 (`xtask/src/cross_os.rs`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Notes:

- The Windows reclamation/precommit primitives (`identity_alive`,
  `is_alive`, `precommit_and_prove`) landed with the M12W1–M12W2 backend; M12W3
  exercised them through the kill/PID-reuse and `VirtualLock` fixtures.
- The matrix harness (E4) slipped to M12W4 Day 1 as the week's risk register
  allows; Phase 31 gate M14W2 is unaffected.

## M12W4 — Windows Conformance Subset, Month-12 Review

| Variable | Target | Measured |
|---|---|---|
| `WIN_CONFORMANCE_SUBSET` | defined Linux-suite subset green on Windows nightly | `xtask/src/cross_os.rs` `SUBSET` (8 green rows + 2 scheduled) executed by `cargo xtask test --cross-os`; `.github/workflows/windows-nightly.yml` runs it ×3 consecutive |
| `PORT_GAPS_CLOSED` | all subset port gaps fixed or explicitly scheduled | 1 — every row is green or `Scheduled(note)` (`win-race-create`, `win-init-reinit` ⇒ M13W1 E1); no row silently missing |
| `MONTH12_REVIEW` | ON_TRACK verdict recorded | ON_TRACK — [month-12.md](../gates/month-12.md); gate ledger 17–26/29/30 PASS; go for M13 |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Cross-OS conformance subset `xtask/src/cross_os.rs` (`SubsetRow`, `SUBSET`,
  `covers_required_invariants`, `cargo xtask test --cross-os`): the single
  manifest of Linux suite rows that must run on Windows, each green or carrying
  an explicit port note. Registered command in `xtask/README.md` (ADR 0003).
- Windows platform lifecycle parity `crates/drm-core/tests/platform_windows.rs`
  (`I-W4b`/`I-W6`/`I-W7`): kill-during-lifecycle releases the last view,
  10 000 create/unmap cycles leave the Win32 handle count stable, and a
  beyond-commit-limit create fails explicitly with a clean namespace.
- Windows guarded cleanup `shm::windows::tests::win_cleanup` (`I-W4`): the
  exclusive active lock refuses cleanup while a node holds a shared hold and
  reports `Cleaned` once all holds drop.
- CI: `windows-nightly.yml` runs `cargo xtask test --cross-os` on the
  `windows-latest` runner (three consecutive green runs recorded in the job
  log); `ci.yml` `windows-smoke` runs the same subset so PRs and nightly share
  one list.
- Month-12 review packet [month-12.md](../gates/month-12.md): variables M09–M12,
  gate ledger, carry-over/quarantine, risk deltas, registry health, and the M13
  commitment.
- Registry: `REQ-CORE-101` (Windows lifecycle suite parity), `REQ-QAL-008`
  (cross-OS conformance subset).

Notes:

- Windows-only rows cannot run on the Linux authoring host; they are
  cross-compiled (`cargo check -p drm-core --tests --target
  x86_64-pc-windows-msvc`) and gated by the `windows-latest` jobs. `cargo xtask
  test --cross-os` on Linux runs the platform-neutral rows and no-ops the
  `cfg`-gated Windows rows.
- No architecture divergence: the subset mirrors
  [windows-backends.md](../algorithms/runtime/windows-backends.md) §Test Matrix
  and [shm-backend.md](../algorithms/runtime/shm-backend.md) suite parity, so no
  ADR or algorithm-doc change landed.
