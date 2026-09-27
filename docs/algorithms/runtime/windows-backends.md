# Windows Runtime Backend Algorithms

Scope: Windows implementations of the platform seams — shared-memory sections (pagefile-backed), file locks, incarnation-safe process identity, precommit locking — with the platform-divergence rules pinned. Implements architecture.md §Platform-Managed Paging, §Hard Real-Time Precommitment, §Crash Recovery (Windows variants), §Safe Cleanup. Owner: E1 (crate `drm-core::shm::windows`). Phase 31. Mirrors [shm-backend.md](shm-backend.md) invariants I1–I6.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-win-shm-sections` | pagefile-backed sections; fixed virtual layouts | — |
| `REQ-CORE-win-platform-paging` | OS decides reservation/commit/residency; DRM does no per-page commit tracking or decommit | — |
| `REQ-CORE-win-locks` | `LockFileEx` per-topic publisher + one per-deployment active lock; kernel release on death | — |
| `REQ-CORE-win-identity` | process handle signaled state + start identity | — |
| `REQ-CORE-win-precommit` | `VirtualLock`; refuse activation on failure | — |

## Data Structures

```rust
// Named section: Local\<deployment_id>\<topic> (namespace scoping per deployment-identity.md)
fn create_exclusive(name, size) -> Result<Mapping> {
 // CreateFileMappingW(INVALID_HANDLE_VALUE, PAGE_READWRITE, size) — pagefile-backed
 // ERROR_ALREADY_EXISTS ⇒ AlreadyExists (I1 parity with O_EXCL)
 // MapViewOfFile(FILE_MAP_ALL_ACCESS) — fixed offsets from PoolLayout (canon-codec I6)
}
fn open_existing(name, expect_size) -> Result<Mapping> {
 // OpenFileMappingW(FILE_MAP_ALL_ACCESS) → ERROR_FILE_NOT_FOUND ⇒ NotFound (I2)
 // VirtualQuery size check ⇒ SizeMismatch (I3)
}
```

**Platform divergence rules (architecture §Platform-Managed Paging — pinned):**
1. Windows sections distinguish reservation / commit charge / physical residency; DRM performs **no** per-page commit tracking, no decommit, no high-water tracking — a partially-filled slot consumes backing exactly as the platform decides.
2. Linux tmpfs `/dev/shm` differs (no over-commit of tmpfs pages); behavior differences are *platform facts*, not DRM features — never papered over.
3. Hard-RT targets use `precommit: true` on both platforms; general targets accept platform paging.

## Invariants (parity with shm-backend.md)

- **I-W1 SingleCreator**: `ERROR_ALREADY_EXISTS` ⇒ exactly-one creator semantics. Test: `win_race_create` (8 procs × 100).
- **I-W2 OpenNeverCreates**: `OpenFileMappingW` miss ⇒ `NotFound`. Test: `win_open_semantics`.
- **I-W3 LockDiesWithProcess**: `LockFileEx` held by killed process released by kernel. Test: `win_lock_release_on_death`.
- **I-W4 CleanupGuarded**: exclusive cleanup lock + no shared holds ⇒ unlink via section-handle close (`DRM_CLEAN` equivalent: all handles closed ⇒ section destroyed). Test: `win_cleanup`.
- **I-W5 IncarnationSafe**: identity = (pid, process creation time via `GetProcessTimes`); `is_alive` = handle signaled check + creation-time equality ⇒ PID reuse safe. Test: `win_process_identity`, `win_pid_reuse`.
- **I-W6 PrecommitRefusal**: `VirtualLock` failure ⇒ refuse active state. Test: `win_precommit_refusal`.

## Functions

```rust
fn process_identity_current() -> ProcessIdentity {
 let pid = GetCurrentProcessId();
 let (create_t, _) = GetProcessTimes(GetCurrentProcess()); // creation file time = start identity
 ProcessIdentity { pid, start: create_t }
}

fn is_alive(id: &ProcessIdentity) -> bool {
 let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id.pid).ok()?; // None ⇒ dead
 if GetProcessTimes(h).create != id.start { return false; } // PID reuse ⇒ different incarnation ⇒ dead-for-us
 WaitForSingleObject(h, 0) == WAIT_TIMEOUT // signaled ⇒ terminated ⇒ false
}

fn precommit(mapping: &Mapping) -> Result<(), Status> {
 touch_all_pages(mapping)?; // pre-fault: commit charge realized
 if !VirtualLock(mapping.base, mapping.len) { return Err(LockFailed); } // I-W6: refuse activation upstream
 Ok(())
}
```

## Protocols

*Cleanup (`drm clean` parity):* exclusive `LockFileEx` on `<lock_dir>\<deployment_id>\active.lock` (lock dir: `%ProgramData%\drm\`). Success ⇒ no shared holder ⇒ close/unmap + `DeleteFileW` on file-backed sections (pagefile sections die with last handle — report `Cleaned` when handle count reaches zero). Failure ⇒ `ActiveUntouched`. Publisher locks live in the same lock dir (`<topic>.publisher.lock`); there is no separate cleanup lock.

*Dead-incarnation proof (hazard reclamation parity):* identical slow-path contract as [exhaustion-reclamation.md](exhaustion-reclamation.md) — reclaim only after `is_alive` proves the exact incarnation dead; PID reuse delays only.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Pagefile commit limit exceeded at create | explicit error (`QuotaExceeded` parity); budgets documented (memory-budget.md) |
| `VirtualLock` working-set limits | `LockFailed` ⇒ refuse activation (I-W6); runner config documented |
| Section name length limits (LPCTSTR) | deployment/topic names length-validated at verify |
| Mandatory locks absent (Windows) | `LockFileEx` = advisory parity; all DRM locks advisory by design |
| Handle inheritance into children | handles opened `bInheritHandle=false` — no accidental sharing |

## Test Matrix

| Case | Expected |
|---|---|
| `win_race_create` | I-W1 |
| `win_open_semantics` | I-W2 (+I3 size) |
| `win_lock_release_on_death` | I-W3 |
| `win_cleanup` | I-W4 |
| `win_process_identity` / `_pid_reuse` | I-W5 |
| `win_precommit_refusal` | I-W6 |
| full Linux conformance subset on Windows runner | nightly green |
