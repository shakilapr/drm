# Shared-Memory Backend Algorithms

Scope: Linux OS shared-memory backend — exclusive-create (`O_CREAT|O_EXCL`), open-existing semantics, map/unmap lifecycle, OS-backed per-topic publisher locks + one per-deployment active lock, incarnation-safe process identity. Implements architecture.md §Segment Creation and Writer Authority, §Safe Cleanup via OS-Backed Lock, §Crash Recovery via OS Process Inspection. Owner: E1 (crate `drm-core::shm`). Phase 8. Windows variants: [windows-backends.md](windows-backends.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-shm-create-exclusive` | §Segment Creation — `O_CREAT \| O_EXCL`, exactly one creator | — |
| `REQ-CORE-shm-consumer-never-create` | consumers wait and open existing | — |
| `REQ-CORE-shm-writer-lock` | §Writer Authority — OS lock auto-releases on death | — |
| `REQ-CORE-shm-active-lock` | §Safe Cleanup — one deployment active.lock; shared by nodes, exclusive by clean | — |
| `REQ-CORE-shm-cleanup-unreferenced` | exclusive active lock ⇒ no holders ⇒ unlink namespace | — |
| `REQ-CORE-identity-incarnation` | §Crash Recovery — pid + process start identity | — |
| `REQ-CORE-identity-pid-reuse` | PID reuse at worst delays reclamation, never corrupts | — |

## Data Structures

```rust
pub struct SegmentName(Arc<str>); // "drm/<deployment_id>/<topic>" (deployment-identity.md scoping)
pub struct Mapping { ptr: NonNull<u8>, len: usize } // RAII: Drop ⇒ munmap

pub enum ShmError {
 NotFound, AlreadyExists, PermissionDenied, Io(std::io::Error), QuotaExceeded, SizeMismatch,
}
// ShmError → drm-types::Status: NotFound, AlreadyExists, Permission, Io→InvalidData,
// QuotaExceeded→InvalidConfig, SizeMismatch→InvalidData.

pub trait ShmBackend {
 fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError>;
 fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError>;
 fn unmap(&self, m: Mapping);
 fn unlink(&self, name: &SegmentName) -> Result<(), ShmError>;
}

pub struct LockPaths(PathBuf); // runtime lock dir: /run/drm/<deployment_id> (NOT under /dev/shm)
impl LockPaths {
 fn active(&self) -> PathBuf { self.0.join("active.lock") } // one per deployment
 fn publisher(&self, topic: &str) -> PathBuf { self.0.join(format!("{topic}.publisher.lock")) }
}

pub trait LockBackend {
 fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError>; // LOCK_EX|LOCK_NB
 fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError>; // LOCK_SH
 fn release(self, h: LockHandle); // Drop ⇒ release
}
// Single lock model (deployment-identity.md): one deployment active.lock (nodes hold shared,
// `drm clean` takes exclusive) and one per-topic publisher.lock. There is NO separate cleanup lock.

pub struct ProcessIdentity { pid: i32, start_ticks: u64 } // Linux: /proc/<pid>/stat field 22

pub enum CleanReport { Cleaned, ActiveUntouched }
```

## Invariants

- **I1 SingleCreator**: per segment lifetime, exactly one concurrent `create_exclusive` succeeds; all others observe `AlreadyExists` with no silent fallback (caller decides policy). Test: `race_create` (8 procs × 100 iters, Created == 100, conflicts 0).
- **I2 OpenNeverCreates**: `open_existing` on a nonexistent name ⇒ `NotFound`; no creation side effect. Test: `open_semantics`.
- **I3 SizeValidated**: `open_existing` with `expect_size` mismatching `fstat` size ⇒ `SizeMismatch` (no partial state). Test: `open_semantics` (mismatch case).
- **I4 LockDiesWithProcess**: publisher/cleanup lock held by a killed process is acquirable immediately after death (kernel releases fd locks). Test: `lock_release_on_death`.
- **I5 IncarnationStable**: identity of a live process is stable across reads; identity of exited process fails `is_alive`; PID reuse ⇒ different start identity ⇒ dead-for-us. Tests: `process_identity`, `process_identity_reuse`.
- **I6 QuotaExplicit**: `/dev/shm` exhaustion ⇒ `QuotaExceeded`, no partial segment. Test: `quota_edge`.

## Functions

```rust
fn create_exclusive(name: &SegmentName, size: usize) -> Result<Mapping, ShmError> {
 let fd = shm_open(name, O_CREAT | O_EXCL | O_RDWR, 0o600)?; // EEXIST ⇒ AlreadyExists (I1)
 ftruncate(fd, size)?; // ENOSPC ⇒ QuotaExceeded (I6)
 let ptr = mmap(fd, size, PROT_READ | PROT_WRITE, MAP_SHARED)?;
 Ok(Mapping { ptr, len: size }) // Drop ⇒ munmap
}

fn open_existing(name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError> {
 let fd = shm_open(name, O_RDWR)?; // ENOENT ⇒ NotFound (I2); never creates
 let st = fstat(fd)?;
 if st.st_size != expect_size { return Err(SizeMismatch); } // I3 — never trust, always check
 Ok(Mapping { ptr: mmap(fd, ...)?, len: expect_size })
}

fn remove_namespace(paths: &LockPaths, names: &[SegmentName]) -> Result<CleanReport, ShmError> {
 // Exclusive active.lock succeeds ⇒ no node holds a shared hold ⇒ namespace is unowned (architecture §Safe Cleanup).
 let _excl = match lock_backend.try_exclusive(&paths.active()) {
 Ok(h) => h,
 Err(_) => return Ok(CleanReport::ActiveUntouched), // any shared holder — leave untouched
 };
 for name in names { shm_unlink(name)?; } // remove every topic segment in the namespace
 Ok(CleanReport::Cleaned)
}

fn current_identity() -> ProcessIdentity {
 let pid = getpid();
 let start = parse_proc_field(read("/proc/self/stat"), 22); // starttime; defensive parse (comm may contain spaces)
 ProcessIdentity { pid, start_ticks: start }
}

fn is_alive(id: &ProcessIdentity) -> bool {
 // Primary: pidfd (pidfd_open + poll) — immune to PID reuse, no signal delivery (architecture §Crash Recovery).
 if let Ok(pfd) = pidfd_open(id.pid) { return poll_terminated(pfd) == false; }
 // Fallback (older kernels): kill(0) + start-ticks guard.
 match kill(id.pid, 0) {
 Ok(()) => current_start_ticks(id.pid) == Ok(id.start_ticks), // PID reuse ⇒ mismatch ⇒ dead-for-us (I5)
 Err(ESRCH) => false,
 Err(EPERM) => true, // alive-but-foreign: conservative alive
 Err(e) => Err(e)?,
 }
}
```

## Protocols

*Writer authority (publisher-lifecycle.md owns the full lifecycle; SHM supplies the primitive):*

1. Publisher: `try_exclusive(paths.publisher(topic))` = `/run/drm/<deployment_id>/<topic>.publisher.lock`; failure ⇒ another live publisher exists ⇒ refuse.
2. Hold for process lifetime; `Drop`/death ⇒ kernel releases (I4) — restarted publisher re-acquires without heartbeat or takeover.

*Cleanup probe (`drm clean`):*

1. `try_exclusive(paths.active())` = `/run/drm/<deployment_id>/active.lock`; success ⇒ no live holders.
2. `shm_unlink` every topic segment under the namespace; report per-segment result.
3. Failure (a node holds the shared active lock) ⇒ `ActiveUntouched`; nothing unlinked.

*Consumer registration identity (exhaustion-reclamation.md consumes):*

1. At consumer attach, write `ProcessIdentity` into the consumer's hazard-slot registration record (channel-metadata.md layout).
2. Reclamation later uses `is_alive(identity)` — PID reuse at worst delays reclamation (mismatch start ticks), never mis-reclaims a live reader (I5).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Concurrent create race | loser gets `AlreadyExists`; decides policy (caller retries `open_existing` or fails) |
| Truncation-while-mapped (foreign actor) | SIGBUS — documented unsupported pattern; detection-path test documents behavior |
| NFS/flock semantics | out of scope; documented exclusion in `lock.rs` |
| `/proc` parse anomalies (comm with spaces) | defensive parser + fixture-file tests (no live-proc-only tests) |
| Segment exists with wrong size | `SizeMismatch` on open (I3) |

## Test Matrix

| Case | Expected |
|---|---|
| `race_create` (8 procs × 100) | I1: Created == 100, conflicts 0, < 30 s |
| `open_semantics` | I2 + I3 |
| `lock_release_on_death` | I4 |
| `flock_multiprocess` (real `flock`, shared vs exclusive across processes) | active-lock model end-to-end |
| `process_identity` / `_reuse` | I5 |
| `quota_edge` | I6 |
| `no_fd_leak` (10k cycles) | fd count stable |
| `shm_kill` (8 scenarios × 100) | recovery each time |
