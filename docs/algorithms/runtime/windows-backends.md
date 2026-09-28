# Windows Runtime Backend Algorithms

Scope: Windows implementations of the platform seams — shared-memory sections (pagefile-backed), file locks, incarnation-safe process identity, precommit locking — with the platform-divergence rules pinned. Implements architecture.md §Platform-Managed Paging, §Hard Real-Time Precommitment, §Crash Recovery (Windows variants), §Safe Cleanup. Owner: E1 (crate `drm-core::shm::windows`). Phase 31. Mirrors [shm-backend.md](shm-backend.md) invariants I1–I8.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-win-shm-sections` | pagefile-backed sections; fixed virtual layouts | `REQ-CORE-078` |
| `REQ-CORE-win-platform-paging` | OS decides reservation/commit/residency; DRM does no per-page commit tracking or decommit | `REQ-CORE-076` |
| `REQ-CORE-win-locks` | `LockFileEx` per-topic publisher + one per-deployment active lock; kernel release on death | `REQ-CORE-075` |
| `REQ-CORE-win-identity` | process handle signaled state + start identity | `REQ-CORE-074` |
| `REQ-CORE-win-precommit` | `VirtualLock`; refuse activation on failure | `REQ-CORE-077` |

## Data Structures

```rust
// Named section: Local\<deployment_id>\<topic> (namespace scoping per deployment-identity.md)
fn create_exclusive(name, size) -> Result<Mapping> {
 // CreateFileMappingW(INVALID_HANDLE_VALUE, PAGE_READWRITE, size) — pagefile-backed
 // ERROR_ALREADY_EXISTS ⇒ close returned handle, report AlreadyExists (I1 parity with O_EXCL)
 // MapViewOfFile failure closes the section handle; successful Mapping owns view + handle.
}
fn open_existing(name, expect_size) -> Result<Mapping> {
 // OpenFileMappingW(FILE_MAP_ALL_ACCESS) → ERROR_FILE_NOT_FOUND ⇒ NotFound (I2)
 // MapViewOfFile with expect_size rejects smaller sections; descriptor.segment_size and
 // layout_hash reject larger or incompatible sections during attachment.
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
- **I-W4 CleanupGuarded**: pagefile sections have no unlink operation and disappear with their last handle; exclusive cleanup safely removes deployment lock files only after all DRM node handles are gone. Test: `win_cleanup`.
- **I-W5 IncarnationSafe**: identity = PID plus creation time; dead proof requires absence, signal, or creation-time mismatch. Inspection failure is not proof. Test: `win_process_identity`, `win_pid_reuse`.
- **I-W6 PrecommitRefusal**: `VirtualLock` failure ⇒ refuse active state. Test: `win_precommit_refusal`.
- **I-W7 CreateRollback**: conflict or view failure closes every returned handle/view. Test: `win_create_failure_matrix`.
- **I-W8 ActiveHold**: every running node retains one shared active-file lock until all mappings close. Test: `win_active_hold_lifecycle`.

## Functions

```rust
fn process_identity_current() -> ProcessIdentity {
 let pid = GetCurrentProcessId();
 let (create_t, _) = GetProcessTimes(GetCurrentProcess()); // creation file time = start identity
 ProcessIdentity { pid, start: create_t }
}

fn identity_dead_proven(id: &ProcessIdentity) -> bool {
 let h = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id.pid) {
  Ok(h) => OwnedHandle(h),
  Err(ERROR_INVALID_PARAMETER) => return true, // no such PID
  Err(_) => return false, // access/inspection failure gives no proof
 };
 let Ok(create) = GetProcessTimes(h).map(|t| t.create) else { return false; };
 if create != id.start { return true; }
 WaitForSingleObject(h, 0) == WAIT_OBJECT_0
}

fn precommit_and_prove(mapping: &Mapping) -> Result<(), Status> {
 // Backend-defined proof (memory-budget.md seam — parity with Linux mlock):
 // The section was created with PAGE_READWRITE on INVALID_HANDLE_VALUE = fully committed
 // (SEC_COMMIT parity), so VirtualLock's committed-pages precondition holds by construction;
 // reservation-only ranges are invalid input, never a DRM state.
 if !VirtualLock(mapping.base, mapping.len) { return Err(LockFailed); } // I-W6: refuse activation upstream
 if !query_working_set_all_resident(mapping) { return Err(LockFailed); } // PSAPI_WORKING_SET_EX Valid|Locked
 Ok(())
}
```

## Protocols

*Cleanup (`drm clean` parity):* exclusive `LockFileEx` on
`<runtime_root>\<deployment_id>\active.lock` (runtime root: `%ProgramData%\drm\`; everything below it
derives solely from the deployment ID — deployment-identity.md, no per-deployment path config).
Success means every DRM node has closed its non-inheritable section handles, so named pagefile
sections already ceased to exist. Cleanup deletes only the deployment's lock files/directories and
reports `Cleaned`. Failure returns `ActiveUntouched`. Publisher locks use non-inheritable file
handles and a fixed byte range.

*Dead-incarnation proof:* identical contract as [exhaustion-reclamation.md](exhaustion-reclamation.md): reclaim only when `identity_dead_proven` returns true; access or query failure preserves the hazard.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Pagefile commit limit exceeded at create | explicit error (`QuotaExceeded` parity); budgets documented (memory-budget.md) |
| `VirtualLock` working-set limits | `LockFailed` ⇒ refuse activation (I-W6); runner config documented |
| Section name length limits (LPCTSTR) | deployment/topic names length-validated at verify |
| Mandatory locks absent (Windows) | `LockFileEx` = advisory parity; all DRM locks advisory by design |
| Handle inheritance into children | handles opened `bInheritHandle=false` — no accidental sharing (Linux parity: shm-backend I9 `O_CLOEXEC`) |
| Named section still openable after `Cleaned` | external/stale non-DRM holder — reported, never force-reclaimed (pagefile sections have no unlink; I-W4) |
| `LockFileEx` transiently busy right after process death | OS release lag — treated as active (`Busy`), never as justification to force-destroy the namespace |

## Test Matrix

| Case | Expected |
|---|---|
| `win_race_create` | I-W1 |
| `win_open_semantics` | I-W2 (+I3 size) |
| `win_lock_release_on_death` | I-W3 |
| `win_cleanup` | I-W4 |
| `win_process_identity` / `_pid_reuse` | I-W5 |
| `win_precommit_refusal` | I-W6 |
| `win_create_failure_matrix` | I-W7; handle count stable |
| `win_active_hold_lifecycle` | I-W8 |
| full Linux conformance subset on Windows runner | nightly green |
