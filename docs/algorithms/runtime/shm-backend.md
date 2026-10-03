# Shared-Memory Backend Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: Linux OS shared-memory backend — backend-safe generated names, exclusive-create
(`O_CREAT|O_EXCL`), stale-object classification before exact-size attach, map/unmap lifecycle,
OS-backed per-topic publisher locks + one per-deployment active lock, fork sanitization, precommit
locking primitive. Implements architecture.md §Segment Creation and Writer Authority, §Scoped
Shared-Memory Namespaces, §Safe Cleanup via OS-Backed Lock, §Crash Recovery via OS
Registration-Lock Authority, §Process Lifecycle (fork), §Hard Real-Time Precommitment. Owner: E1
(crate `drm-core::shm`). Phase 8. Windows variants: [windows-backends.md](windows-backends.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-shm-generated-names` | backend-safe opaque names: `/drm.<dep_id>.<topic_hash>` | `REQ-CORE-088` |
| `REQ-CORE-shm-create-exclusive` | §Segment Creation — `O_CREAT \| O_EXCL`, exactly one creator | `REQ-CORE-072` |
| `REQ-CORE-shm-consumer-never-create` | consumers wait and open existing | `REQ-CORE-071` |
| `REQ-CORE-shm-stale-classification` | classify before exact-size attach; recover / adopt / reject | `REQ-CORE-098` |
| `REQ-CORE-shm-writer-lock` | §Writer Authority — OS lock auto-releases on death | `REQ-CORE-073` |
| `REQ-CORE-shm-active-lock` | §Safe Cleanup — one deployment active.lock; shared by nodes, exclusive by clean | `REQ-CORE-069` |
| `REQ-CORE-shm-cleanup-unreferenced` | exclusive active lock ⇒ no holders ⇒ unlink namespace | `REQ-CORE-070` |
| `REQ-CORE-shm-fork-sanitize` | fork invalidates inherited DRM objects in the child | `REQ-CORE-099` |
| `REQ-CORE-shm-lock-and-populate` | `precommit_and_prove`: mlock success is the residency proof | `REQ-CORE-090` |
| `REQ-CORE-identity-diagnostic` | pid + start identity recorded; diagnostic only, never authority | `REQ-CORE-036` |

## Data Structures

```rust
// POSIX SHM names: leading slash required, no further slashes, <= NAME_MAX. Canonical topic
// paths never reach the kernel namespace (deployment-identity.md); the descriptor retains the
// canonical topic and validates it at attach.
pub struct SegmentName(Arc<str>); // "/drm.<deployment_id>.<topic_hash>"
fn backend_segment_name(deploy_id: &str, topic_hash: u64) -> SegmentName; // length-checked at verify

pub struct Mapping { ptr: NonNull<u8>, len: usize } // RAII: Drop ⇒ munmap; fd closes after mmap

pub enum ShmError {
 NotFound, AlreadyExists, PermissionDenied, Io(std::io::Error), QuotaExceeded, SizeMismatch,
 StaleIncomplete, // undersized object whose creation never completed — recoverable
}
// ShmError → drm-types::Status: NotFound, AlreadyExists, Permission, Io→InvalidData,
// QuotaExceeded→InvalidConfig, SizeMismatch→InvalidData, StaleIncomplete→NotReady (retryable by
// the caller's startup policy).

pub trait ShmBackend {
 fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError>;
 /// Opens the raw object and returns `(fd, fstat size)` WITHOUT exact-size enforcement.
 /// Classification is the caller's job (publisher-lifecycle.md recover path, below).
 fn open_raw(&self, name: &SegmentName) -> Result<(OwnedFd, u64), ShmError>;
 fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError>;
 fn unmap(&self, m: Mapping);
 fn unlink(&self, name: &SegmentName) -> Result<(), ShmError>;
 /// Backend-defined precommit proof (memory-budget.md `activate_precommitted`):
 /// Linux — `mlock(MLOCK_INSERT_FAULTING_PAGES)` success IS the residency proof
 /// (all intersecting pages resident until munlock). No mincore dependency.
 fn precommit_and_prove(&self, m: &Mapping) -> Result<(), ShmError>;
}

pub struct LockPaths(PathBuf); // runtime lock dir: /run/drm/<deployment_id> (NOT under /dev/shm)
impl LockPaths {
 fn active(&self) -> PathBuf { self.0.join("active.lock") } // one per deployment
 // Lock names use the SAME generated topic identifier family as segment names
 // (deployment-identity.md) — never canonical topic strings, whose '/' hierarchy would
 // turn filenames into directories.
 fn publisher(&self, topic_id: u32) -> PathBuf {
  self.0.join(format!("t_{topic_id}.publisher.lock"))
 }
 fn consumer(&self, topic_id: u32, id: u32) -> PathBuf {
  self.0.join(format!("t_{topic_id}.{id}.consumer.lock"))
 }
}
// Topic handles, not canonical topic strings: topic paths contain '/', which would turn lock
// filenames into directory hierarchies.

pub trait LockBackend {
 fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError>; // LOCK_EX|LOCK_NB
 fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError>; // LOCK_SH
 fn release(self, h: LockHandle); // Drop ⇒ release
}
// flock(LOCK_EX/LOCK_SH) semantics: locks bind to the open file description. All lock fds are
// opened O_CLOEXEC and sanitized at fork (below). Single lock model (deployment-identity.md): one
// deployment active.lock (nodes hold shared, `drm clean` takes exclusive) and one per-topic
// publisher.lock. There is NO separate cleanup lock.

pub struct ProcessIdentity { pid: i32, start_ticks: u64 } // Linux: /proc/<pid>/stat field 22
// DIAGNOSTIC ONLY. Reclamation authority is the registration lock
// (exhaustion-reclamation.md); identity never gates a reclaim decision.

pub enum CleanReport { Cleaned, ActiveUntouched }
```

## Invariants

- **I0 NameConformance**: every generated name is accepted by the real platform create/open call
  (`shm_open`, `open(2)` on lock paths) for maximum-length topics. Test: `name_conformance`
  (generated-name matrix incl. `control/target`, `camera/front/image`, `a/b/c/d`, max length).
- **I1 SingleCreator**: per segment lifetime, exactly one concurrent `create_exclusive` succeeds; all others observe `AlreadyExists` with no silent fallback (caller decides policy). Test: `race_create` (8 procs × 100 iters, Created == 100, conflicts 0).
- **I2 OpenNeverCreates**: `open_existing` on a nonexistent name ⇒ `NotFound`; no creation side effect. Test: `open_semantics`.
- **I3 SizeValidated**: `open_existing` with `expect_size` mismatching `fstat` size ⇒ `SizeMismatch` (no partial state). Complete-object size is enforced only AFTER stale classification. Test: `open_semantics` (mismatch case).
- **I4 LockDiesWithProcess**: publisher/cleanup lock held by a killed process is acquirable immediately after death — after fork sanitization removes inherited duplicates (kernel releases fd locks when the last OFD duplicate closes). Test: `lock_release_on_death`, `fork_lock_matrix`.
- **I5 ForkSanitized**: a forked child never mutates shared DRM state and never releases an inherited lock: `pthread_atfork` child handler invalidates runtime objects (incarnation cookie) and closes lock fds **without `LOCK_UN`**; all lock fds are `O_CLOEXEC`. Tests: `fork_lock_matrix` (parent/child exit permutations with live publisher/consumer/borrow), `fork_guard_no_mutation`.
- **I6 QuotaExplicit**: `/dev/shm` exhaustion ⇒ `QuotaExceeded`, no partial segment. Test: `quota_edge`.
- **I7 CreateRollback**: every failure after exclusive creation closes the fd and unlinks the incomplete name. Test: `create_failure_matrix`.
- **I8 ActiveHold**: every running node holds one shared deployment lock for its lifetime. Test: `active_hold_lifecycle`.
- **I9 PrecommitLocked**: `precommit_and_prove` locks the full declared region or fails explicitly; no silent pageable fallback (memory-budget.md I-MB5). Test: `precommit_mlock_matrix`.

## Functions

```rust
fn create_exclusive(name: &SegmentName, size: usize) -> Result<Mapping, ShmError> {
 let fd = OwnedFd::new(shm_open(name, O_CREAT | O_EXCL | O_RDWR, 0o600)?);
 let mut rollback = UnlinkOnDrop::new(name); // armed until complete mapping exists
 ftruncate(&fd, size)?;
 posix_fallocate(&fd, 0, size).map_err(map_quota)?; // reserve tmpfs extent; no later SIGBUS
 let ptr = mmap(&fd, size, PROT_READ | PROT_WRITE, MAP_SHARED)?;
 rollback.disarm();
 Ok(Mapping { ptr, len: size }) // fd closes here; Mapping Drop munmaps
}

// Raw open: returns the fd and its CURRENT size. Never enforces expect_size — the publisher
// recovery path must classify stale/undersized objects (publisher-lifecycle.md) before any
// exact-size validation. A crash between shm_open and ftruncate leaves a zero-length object;
// that is a recoverable Init artifact, not a compatibility failure.
fn open_raw(name: &SegmentName) -> Result<(OwnedFd, u64), ShmError> {
 let fd = OwnedFd::new(shm_open(name, O_RDWR)?);
 let size = fstat(&fd)?.st_size as u64;
 Ok((fd, size))
}

fn open_existing(name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError> {
 let (fd, size) = open_raw(name)?;
 // A POSIX SHM object is born zero-length; ftruncate(size) establishes the full size in one
 // atomic step, so a live object is either 0 (creation never completed) or full. Anything
 // below the fixed 16-byte SegmentPrefix (channel-metadata.md) is unclassifiable ⇒ stale.
 if size < size_of::<SegmentPrefix>() as u64 { return Err(StaleIncomplete); }
 if size != expect_size { return Err(SizeMismatch); } // I3 — never trust, always check
 let ptr = mmap(&fd, expect_size, PROT_READ | PROT_WRITE, MAP_SHARED, 0)?;
 Ok(Mapping { ptr, len: expect_size }) // fd closes
}

// Precommit proof consumed by memory-budget.md activate_precommitted. mlock success is the
// proof; residency is never asserted from a mincore snapshot (staleable by design).
fn precommit_and_prove(m: &Mapping) -> Result<(), ShmError> {
 if mlock(m.ptr, m.len) != 0 { return Err(Io(errno)); } // incl. EPERM ⇒ RLIMIT_MEMLOCK too small
 Ok(())
}

fn enter_deployment(paths: &LockPaths) -> Result<ActiveHold, ShmError> {
 lock_backend.try_shared(&paths.active()).map(ActiveHold) // retained by node root object
}

fn remove_namespace(paths: &LockPaths, names: &[SegmentName]) -> Result<CleanReport, ShmError> {
 // Exclusive active.lock succeeds ⇒ no node holds a shared hold ⇒ namespace is unowned (architecture §Safe Cleanup).
 let _excl = match lock_backend.try_exclusive(&paths.active()) {
 Ok(h) => h,
 Err(WouldBlock) => return Ok(CleanReport::ActiveUntouched),
 Err(e) => return Err(e),
 };
 for name in names { shm_unlink(name)?; } // remove every topic segment in the namespace
 Ok(CleanReport::Cleaned)
}

// Diagnostic identity — recorded at consumer registration for drm-diag only.
fn current_identity() -> ProcessIdentity {
 let pid = getpid();
 let start = parse_proc_field(read("/proc/self/stat"), 22); // starttime; defensive parse (comm may contain spaces)
 ProcessIdentity { pid, start_ticks: start }
}
```

## Protocols

*Fork sanitization (I5; architecture §Process Lifecycle (fork)):*

1. At runtime start, register `pthread_atfork` handlers; mint a process-incarnation cookie.
2. Child handler: invalidate every DRM runtime object (publisher, registration, borrow, guard) via
   cookie mismatch; close inherited lock fds **without issuing `LOCK_UN`** (flock binds to the open
   file description — an explicit unlock from any duplicate would release the parent's authority).
3. All lock fds open `O_CLOEXEC`; a child that `exec`s starts clean.
4. `LockHandle`, `Registration`, `BorrowGuard` methods check the cookie and never mutate shared
   state on mismatch (return `InvalidData` / no-op drop without unlock).
5. Public ABI contract: after `fork()`, inherited DRM objects are invalid in the child; the child
   must `exec` or reattach. Documented for Python (`multiprocessing` fork start method).

*Writer authority (publisher-lifecycle.md owns the full lifecycle; SHM supplies the primitive):*

1. Publisher: `try_exclusive(paths.publisher(topic_id))` =
   `/run/drm/<deployment_id>/t_<topic_id>.publisher.lock`; failure ⇒ another live publisher exists ⇒ refuse.
2. Hold for process lifetime; `Drop`/death ⇒ kernel releases (I4) — restarted publisher re-acquires
   without heartbeat or takeover.

*Node activation:* acquire one shared `active.lock` before opening any topic and retain it in the node
root object until every mapping, publisher lock, and consumer registration is closed. Partial startup
unwinds mappings first and the active hold last. Precommitted nodes then run
`activate_precommitted` (memory-budget.md) before entering active state.

*Cleanup probe (`drm clean`):*

1. `try_exclusive(paths.active())` = `/run/drm/<deployment_id>/active.lock`; success ⇒ no live holders.
2. `shm_unlink` every topic segment under the namespace; report per-segment result.
3. Failure (a node holds the shared active lock) ⇒ `ActiveUntouched`; nothing unlinked.

*Consumer registration identity (diagnostics only; exhaustion-reclamation.md):*

1. At consumer attach, write `ProcessIdentity` into the consumer's registration record
   (channel-metadata.md layout) for `drm-diag` surfaces.
2. Reclamation authority is the registration lock, never this identity. Inspection errors, PID
   reuse, and cross-PID-namespace `ESRCH` cannot cause mis-reclaim because they are not consulted.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Concurrent create race | loser gets `AlreadyExists`; decides policy (caller classifies/retries) |
| Truncation-while-mapped (foreign actor) | SIGBUS — documented unsupported pattern; detection-path test documents behavior |
| NFS/flock semantics | out of scope; documented exclusion in `lock.rs` |
| `/proc` parse anomalies (comm with spaces) | defensive parser + fixture-file tests (no live-proc-only tests) |
| Segment exists with wrong size | `SizeMismatch` on open **after** classification; zero/undersized ⇒ `StaleIncomplete` (recoverable) |
| failure after exclusive create | close + unlink by rollback guard (I7) |
| fork with live publisher/consumer/borrow | child inherits nothing usable; parent authority intact (I5) |
| `mlock` EPERM (RLIMIT_MEMLOCK) | explicit `QuotaExceeded`-family failure; activation refuses (I9) |

## Test Matrix

| Case | Expected |
|---|---|
| `name_conformance` (generated names incl. slashed + max-length topics) | I0: platform calls accept every generated name |
| `race_create` (8 procs × 100) | I1: Created == 100, conflicts 0, < 30 s |
| `open_semantics` | I2 + I3 |
| `lock_release_on_death` | I4 |
| `fork_lock_matrix` (parent/child exit permutations, live borrow, inherited guard attempts) | I4 + I5: parent authority intact; child never mutates/unlocks |
| `fork_guard_no_mutation` (child drops inherited `BorrowGuard`/`Registration`) | I5: no shared-state mutation, no `LOCK_UN` |
| `flock_multiprocess` (real `flock`, shared vs exclusive across processes) | active-lock model end-to-end |
| `quota_edge` | I6 |
| `no_fd_leak` (10k cycles) | fd count stable |
| `shm_kill` (8 scenarios × 100) | recovery each time |
| `create_failure_matrix` (kill after shm_open / ftruncate / fallocate / mmap) | I7; zero-length stale object classified `StaleIncomplete`, restart recovers |
| `precommit_mlock_matrix` (success / EPERM / pressure + VmLck + fault counts) | I9: locked or refused |
| `active_hold_lifecycle` | I8 |
