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
