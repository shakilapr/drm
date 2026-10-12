//! Platform SHM backend abstraction, verbatim from
//! `docs/algorithms/runtime/shm-backend.md` §Data Structures (W1: signatures +
//! contract docs; bodies W2–W3). One implementation per OS; OS types never leak
//! through signatures (codebase.md 3.1 R8).
//! REQ: REQ-CORE-072 (exclusive create), REQ-CORE-088 (generated names),
//! REQ-CORE-098 (stale classification), REQ-CORE-073 (writer lock),
//! REQ-CORE-069 (deployment active lock), REQ-CORE-070 (cleanup),
//! REQ-CORE-090 (precommit), REQ-CORE-036 (diagnostic identity).

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;

use drm_types::Status;

#[cfg(unix)]
mod lock;
#[cfg(unix)]
mod process;
#[cfg(unix)]
mod segment;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use lock::FlockBackend;
#[cfg(unix)]
pub use process::{current_identity, identity_alive, is_alive};
#[cfg(unix)]
pub use segment::PosixShm;
#[cfg(windows)]
pub use windows::{WinLockBackend, WinShm, current_identity, identity_alive, is_alive};

/// POSIX `NAME_MAX` upper bound for `shm_open` names (shm-backend.md I0).
#[cfg(unix)]
const NAME_MAX: usize = 255;

/// Windows mapping-name length bound (`Local\` prefix inclusive); enforce at
/// verify so the real `CreateFileMappingW` never rejects a generated name.
/// REQ: REQ-CORE-078.
pub const WINDOWS_NAME_MAX: usize = 255;

/// Windows mapping namespace prefix; the remainder may not contain `\`, since
/// Windows mapping names forbid backslashes after the `Local\`/`Global\` prefix
/// (architecture §Scoped Shared-Memory Namespaces).
/// REQ: REQ-CORE-078.
pub const WINDOWS_LOCAL_PREFIX: &str = "Local\\";

/// Builds the Windows generated section name
/// `Local\drm_<deployment_id>_<topic_hash>` (windows-backends.md §Data
/// Structures). Pure string form, evaluated on every host so the naming rule is
/// unit-tested even where `CreateFileMappingW` is unavailable.
/// REQ: REQ-CORE-078, REQ-CORE-088.
#[must_use]
pub fn windows_section_name(deploy_id: &str, topic_hash: u64) -> String {
    format!("{WINDOWS_LOCAL_PREFIX}drm_{deploy_id}_{topic_hash:016x}")
}

/// Windows mapping-name conformance (I0/I-W parity): `Local\` prefix, non-empty
/// remainder with no further backslash, no NUL, within [`WINDOWS_NAME_MAX`].
/// REQ: REQ-CORE-078.
#[must_use]
pub fn windows_name_conforms(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(WINDOWS_LOCAL_PREFIX) else {
        return false;
    };
    !rest.is_empty()
        && !rest.contains('\\')
        && !name.contains('\0')
        && name.len() <= WINDOWS_NAME_MAX
}

/// POSIX `shm_open` name conformance (I0): leading slash, no further slashes,
/// no NUL, `NAME_MAX` or shorter, non-empty. Pure form so the rule is testable.
/// REQ: REQ-CORE-088.
#[cfg(unix)]
#[must_use]
pub fn posix_name_conforms(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with('/')
        && !name[1..].contains('/')
        && !name.contains('\0')
        && name.len() <= NAME_MAX
}

/// Generated backend segment name — `/drm.<deployment_id>.<topic_hash>` on POSIX,
/// `Local\drm_<deployment_id>_<topic_hash>` on Windows (architecture §Scoped
/// Shared-Memory Namespaces). Canonical topic paths never reach the kernel
/// namespace; the descriptor retains the canonical topic and validates it at
/// attach.
/// REQ: REQ-CORE-088.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SegmentName(Arc<str>);

impl SegmentName {
    /// Validates the name against the platform create/open contract (I0: the real
    /// `shm_open`/`CreateFileMappingW` must accept it): POSIX — leading slash, no
    /// further slashes, no NUL, `NAME_MAX` or shorter; Windows — `Local\` prefix,
    /// no further backslashes, no NUL, length-bounded. Conformance failures mirror
    /// the platform rejection as [`ShmError::Io`] with
    /// [`std::io::ErrorKind::InvalidInput`].
    /// REQ: REQ-CORE-078, REQ-CORE-088.
    ///
    /// # Errors
    /// [`ShmError::Io`] on any conformance violation.
    pub fn new(name: &str) -> Result<Self, ShmError> {
        #[cfg(unix)]
        let conforms = posix_name_conforms(name);
        #[cfg(windows)]
        let conforms = windows_name_conforms(name);
        #[cfg(not(any(unix, windows)))]
        let conforms = false;
        if !conforms {
            return Err(ShmError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid segment name",
            )));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Borrowed backend name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SegmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Builds the platform generated segment name; length-checked at verify (I0).
/// POSIX `/drm.<deployment_id>.<topic_hash>`; Windows
/// `Local\drm_<deployment_id>_<topic_hash>`.
/// REQ: REQ-CORE-078, REQ-CORE-088.
///
/// # Errors
/// [`ShmError::Io`] (`InvalidInput`) when the deployment id or formatted hash
/// violates name conformance.
pub fn backend_segment_name(deploy_id: &str, topic_hash: u64) -> Result<SegmentName, ShmError> {
    #[cfg(unix)]
    let raw = format!("/drm.{deploy_id}.{topic_hash:016x}");
    #[cfg(windows)]
    let raw = windows_section_name(deploy_id, topic_hash);
    #[cfg(not(any(unix, windows)))]
    let raw = String::new();
    SegmentName::new(&raw)
}

/// SHM backend failures (shm-backend.md Data Structures). Mapped 1:1 into the
/// drm-types status taxonomy: `InvalidName`/`SizeMismatch` → `InvalidData`,
/// `QuotaExceeded` → `InvalidConfig`, `StaleIncomplete` → `NotReady`
/// (retryable by the caller's startup policy).
/// REQ: REQ-CORE-072, REQ-CORE-098.
#[derive(Debug, thiserror::Error)]
pub enum ShmError {
    /// Object does not exist (`open_existing` on a fresh namespace; I2).
    #[error("not found")]
    NotFound,
    /// Exclusive create lost the concurrent race; caller owns the fallback policy.
    #[error("already exists")]
    AlreadyExists,
    /// Filesystem or permission failure.
    #[error("permission denied")]
    PermissionDenied,
    /// Undocumented OS failure.
    #[error("os error: {0}")]
    Io(#[from] std::io::Error),
    /// `/dev/shm` or tmpfs quota exhausted; no partial segment (I6).
    #[error("quota exceeded")]
    QuotaExceeded,
    /// Existing object size differs from the expected size (I3).
    #[error("size mismatch")]
    SizeMismatch,
    /// Object whose creation never completed — recoverable startup artifact.
    #[error("stale incomplete object")]
    StaleIncomplete,
}

impl From<ShmError> for Status {
    fn from(e: ShmError) -> Self {
        match e {
            ShmError::NotFound => Self::NotFound,
            ShmError::AlreadyExists => Self::AlreadyExists,
            ShmError::PermissionDenied => Self::Permission,
            ShmError::Io(_) | ShmError::SizeMismatch => Self::InvalidData,
            ShmError::QuotaExceeded => Self::InvalidConfig,
            ShmError::StaleIncomplete => Self::NotReady,
        }
    }
}

/// RAII mapping view: `Drop` ⇒ munmap **for an owned kernel mapping**; the
/// descriptor closes right after mmap.
///
/// A view may also be *borrowed*: memory owned elsewhere (the in-memory test
/// backend's registry) and merely aliased here. Dropping a borrowed view must
/// not release the bytes — `munmap` on allocator-owned memory corrupts the heap
/// and faults later in `free`. See [`Mapping::borrowed_view`].
/// REQ: REQ-CORE-072.
pub struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
    /// `true` for a kernel mapping created by `mmap`/`MapViewOfFile`, whose
    /// `Drop` releases the view; `false` for a borrowed view over memory owned
    /// by the caller's backend (test shims), whose `Drop` is a no-op.
    owned: bool,
}

impl Mapping {
    /// Backend-internal constructor: POSIX [`super::segment::PosixShm`] after
    /// `mmap`, Windows [`super::windows::WinShm`] after `MapViewOfFile`. The
    /// view owns the described bytes and releases them on drop.
    /// REQ: REQ-CORE-072, REQ-CORE-078.
    pub(crate) fn from_raw_parts(ptr: NonNull<u8>, len: usize) -> Self {
        Self {
            ptr,
            len,
            owned: true,
        }
    }

    /// Backend-internal constructor for a **borrowed** view: the bytes are owned
    /// by the caller's backend, so `Drop` must not release them. Used by the
    /// in-memory test backend, whose segments live in a process-local registry
    /// backed by the Rust allocator — `munmap` there is undefined behaviour.
    /// REQ: REQ-CORE-072.
    #[cfg(test)]
    pub(crate) fn borrowed_view(ptr: NonNull<u8>, len: usize) -> Self {
        Self {
            ptr,
            len,
            owned: false,
        }
    }

    /// Base pointer of the mapping. Raw view by design: shared memory is
    /// written/read through this pointer under the borrow protocol; external
    /// I/O must copy first (transient borrow lifecycle).
    /// REQ: REQ-CORE-072, REQ-CORE-071.
    #[must_use]
    pub fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    /// Mapped byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Empty-mapping predicate (`clippy::len_without_is_empty`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(unix)]
impl Drop for Mapping {
    fn drop(&mut self) {
        // A borrowed view's bytes belong to the backend registry, not to us.
        if !self.owned || self.len == 0 {
            return;
        }
        // SAFETY: `ptr` was produced by `mmap(PROT_READ | PROT_WRITE, MAP_SHARED)`
        // over exactly `len` bytes and is exclusively owned by this `Mapping`;
        // dropping releases it exactly once.
        unsafe { libc::munmap(self.ptr.cast().as_ptr(), self.len) };
    }
}

#[cfg(windows)]
impl Drop for Mapping {
    fn drop(&mut self) {
        // A borrowed view's bytes belong to the backend registry, not to us.
        if !self.owned || self.len == 0 {
            return;
        }
        // SAFETY: `ptr` was produced by `MapViewOfFile` over a live pagefile
        // section and is exclusively owned by this `Mapping`; `UnmapViewOfFile`
        // releases the view exactly once. The section survives until its last
        // view closes (I-W4b).
        unsafe {
            windows_sys::Win32::System::Memory::UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.ptr.cast::<core::ffi::c_void>().as_ptr(),
                },
            );
        }
    }
}

impl fmt::Debug for Mapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY-free: pointer address is never exposed in diagnostics.
        f.debug_struct("Mapping")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// Platform SHM object lifecycle. Contract: exactly one concurrent
/// `create_exclusive` succeeds (I1); `open_existing` never creates (I2) and
/// validates size only after stale classification (I3); every failure after
/// exclusive creation rolls back fd and name (I7).
/// Frozen v1 (work-plan M01W3-E1-T2): trait shape and method set are stable;
/// breaking changes require a drm-core version bump. POSIX impl:
/// [`super::segment::PosixShm`]; Windows impl: [`super::windows::WinShm`]
/// (windows-backends.md).
/// REQ: REQ-CORE-072, REQ-CORE-078, REQ-CORE-098.
#[doc(alias = "shm create")]
#[doc(alias = "shm open")]
pub trait ShmBackend {
    /// `O_CREAT|O_EXCL` create + exact-size establish; loser observes
    /// [`ShmError::AlreadyExists`] with no silent fallback.
    /// REQ: REQ-CORE-072.
    ///
    /// # Errors
    /// [`ShmError`] per the create failure matrix (I6, I7).
    fn create_exclusive(&self, name: &SegmentName, size: usize) -> Result<Mapping, ShmError>;

    /// Opens the raw object and returns the descriptor and its CURRENT size without
    /// exact-size enforcement; classification is the caller's job (publisher
    /// recover path). Linux-backend contract; Windows variant per windows-backends.md.
    /// REQ: REQ-CORE-098.
    ///
    /// # Errors
    /// [`ShmError::NotFound`] when absent.
    #[cfg(unix)]
    fn open_raw(&self, name: &SegmentName) -> Result<(std::os::fd::OwnedFd, u64), ShmError>;

    /// Opens an existing object: `StaleIncomplete` when below the fixed
    /// `SegmentPrefix` size, `SizeMismatch` on exact-size mismatch, else a valid
    /// [`Mapping`] (I2, I3). REQ: REQ-CORE-098.
    ///
    /// # Errors
    /// [`ShmError::NotFound`], [`ShmError::StaleIncomplete`], [`ShmError::SizeMismatch`].
    fn open_existing(&self, name: &SegmentName, expect_size: usize) -> Result<Mapping, ShmError>;

    /// Unmaps a mapping; descriptor is already closed by then.
    /// REQ: REQ-CORE-072.
    fn unmap(&self, m: Mapping);

    /// Removes the named object from the namespace (`shm_unlink`).
    /// REQ: REQ-CORE-070.
    ///
    /// # Errors
    /// [`ShmError::NotFound`] when absent.
    fn unlink(&self, name: &SegmentName) -> Result<(), ShmError>;

    /// Backend-defined precommit proof: Linux `mlock` success IS the residency
    /// proof (all intersecting pages resident until munlock); never a mincore
    /// snapshot. REQ: REQ-CORE-090.
    ///
    /// # Errors
    /// [`ShmError`] on `mlock` failure (incl. `EPERM` ⇒ `RLIMIT_MEMLOCK` too small).
    fn precommit_and_prove(&self, m: &Mapping) -> Result<(), ShmError>;
}

/// Opaque held OS lock; `Drop` releases. POSIX: `flock` binds to the open file
/// description and the kernel releases it when the last duplicate closes (I4).
/// Windows: the kernel releases the `LockFileEx` range when the last handle
/// closes (I-W3). The handle owns the lock-file descriptor/handle plus an
/// optional backend release hook for backends whose lock state is not the OS
/// object itself (test backends); the hook runs before the OS close.
/// REQ: REQ-CORE-073, REQ-CORE-069, REQ-CORE-075.
pub struct LockHandle {
    #[cfg(unix)]
    fd: Option<std::os::fd::OwnedFd>,
    #[cfg(windows)]
    handle: Option<windows_sys::Win32::Foundation::HANDLE>,
    release_hook: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for LockHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("LockHandle");
        #[cfg(unix)]
        s.field("fd", &self.fd.is_some());
        #[cfg(windows)]
        s.field("handle", &self.handle.is_some());
        s.field("release_hook", &self.release_hook.is_some());
        s.finish()
    }
}

impl LockHandle {
    /// OS-descriptor constructor (POSIX backends).
    /// REQ: REQ-CORE-073.
    #[cfg(unix)]
    pub(crate) fn from_fd(fd: std::os::fd::OwnedFd) -> Self {
        Self {
            fd: Some(fd),
            release_hook: None,
        }
    }

    /// Windows lock-file handle constructor; `Drop` closes it (kernel release,
    /// I-W3).
    /// REQ: REQ-CORE-075.
    #[cfg(windows)]
    pub(crate) fn from_windows_handle(handle: windows_sys::Win32::Foundation::HANDLE) -> Self {
        Self {
            handle: Some(handle),
            release_hook: None,
        }
    }

    /// Hook constructor (in-process test backends).
    /// REQ: REQ-CORE-073.
    #[cfg_attr(not(test), expect(dead_code, reason = "test-backend constructor"))]
    pub(crate) fn from_hook(hook: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            #[cfg(unix)]
            fd: None,
            #[cfg(windows)]
            handle: None,
            release_hook: Some(hook),
        }
    }
}

impl Drop for LockHandle {
    fn drop(&mut self) {
        if let Some(hook) = self.release_hook.take() {
            hook();
        }
        #[cfg(windows)]
        if let Some(handle) = self.handle.take() {
            // SAFETY: `handle` is an owned `LockFileEx` lock-file handle opened
            // by `WinLockBackend` and closed exactly once; closing it releases
            // the locked range.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        }
    }
}

/// OS-backed advisory lock lifecycle (`flock` family: `LOCK_EX|LOCK_NB`,
/// `LOCK_SH`, release). Locks bind to the open file description; all lock
/// descriptors open `O_CLOEXEC` and are sanitized at fork (I5). NFS/flock
/// semantics are out of scope (documented exclusion).
/// Frozen v1 (work-plan M01W3-E1-T2): trait shape and method set are stable;
/// breaking changes require a drm-core version bump. POSIX impl:
/// [`super::lock::FlockBackend`]; Windows impl: [`super::windows::WinLockBackend`]
/// (windows-backends.md).
/// REQ: REQ-CORE-073, REQ-CORE-069, REQ-CORE-075.
#[doc(alias = "shm lock")]
pub trait LockBackend {
    /// Exclusive, non-blocking acquisition of the lock file at `path`.
    /// REQ: REQ-CORE-073.
    ///
    /// # Errors
    /// [`ShmError::AlreadyExists`]-family busy failure when another holder is live.
    fn try_exclusive(&self, path: &Path) -> Result<LockHandle, ShmError>;

    /// Shared, non-blocking acquisition of the lock file at `path`.
    /// REQ: REQ-CORE-069.
    ///
    /// # Errors
    /// [`ShmError`]-family failure when the path is not lockable.
    fn try_shared(&self, path: &Path) -> Result<LockHandle, ShmError>;

    /// Explicit release; `Drop` on the handle also releases.
    /// REQ: REQ-CORE-073.
    fn release(self, h: LockHandle);
}

/// Runtime lock directory — `/run/drm/<deployment_id>` on POSIX,
/// `%ProgramData%\drm\<deployment_id>` on Windows; never under the segment
/// namespace. Lock file names use the same generated topic identifier family as
/// segment names (never canonical topic strings, whose separator hierarchy
/// would turn filenames into directories).
/// REQ: REQ-CORE-069, REQ-CORE-075.
#[derive(Debug, Clone)]
pub struct LockPaths(PathBuf);

impl LockPaths {
    /// Wraps the per-deployment lock directory.
    /// REQ: REQ-CORE-069.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    /// One per deployment: `active.lock` (nodes hold shared, `drm clean` takes
    /// exclusive). REQ: REQ-CORE-069.
    #[must_use]
    pub fn active(&self) -> PathBuf {
        self.0.join("active.lock")
    }

    /// Per-topic publisher authority lock
    /// `t_<topic_id>.publisher.lock`. REQ: REQ-CORE-073.
    #[must_use]
    pub fn publisher(&self, topic_id: u32) -> PathBuf {
        self.0
            .join(OsStr::new(format!("t_{topic_id}.publisher.lock").as_str()))
    }

    /// Per-consumer registration lock `t_<topic_id>.<id>.consumer.lock`
    /// (reclamation authority; exhaustion-reclamation.md). REQ: REQ-CORE-098.
    #[must_use]
    pub fn consumer(&self, topic_id: u32, id: u32) -> PathBuf {
        self.0.join(OsStr::new(
            format!("t_{topic_id}.{id}.consumer.lock").as_str(),
        ))
    }
}

/// pid + process-start identity; DIAGNOSTIC ONLY, recorded at consumer
/// registration for drm-diag surfaces. Reclamation authority is the registration
/// lock, never this identity. REQ: REQ-CORE-036.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    /// OS process id.
    pub pid: i32,
    /// Process start time in platform ticks (Linux: `/proc/<pid>/stat` field 22;
    /// Windows: creation `FILETIME` as 100 ns units since 1601-01-01).
    /// REQ: REQ-CORE-036, REQ-CORE-074.
    pub start_ticks: u64,
}

/// `drm clean` result: the exclusive deployment active lock decides whether the
/// namespace is unowned. REQ: REQ-CORE-070.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanReport {
    /// Exclusive active lock acquired; every topic segment unlinked.
    Cleaned,
    /// A node still holds the shared active lock; nothing was unlinked.
    ActiveUntouched,
}

#[cfg(test)]
mod tests {
    use super::{CleanReport, ShmError, windows_name_conforms, windows_section_name};
    #[cfg(unix)]
    use super::{LockPaths, SegmentName, backend_segment_name};
    use drm_types::Status;

    /// REQ: REQ-CORE-088 — name conformance (I0); violations mirror the kernel's EINVAL.
    #[cfg(unix)]
    #[test]
    fn segment_name_validates() {
        assert!(SegmentName::new("/drm.dep-1.0102").is_ok());
        assert!(matches!(SegmentName::new(""), Err(ShmError::Io(_))));
        assert!(matches!(SegmentName::new("drm/x"), Err(ShmError::Io(_))));
        assert!(matches!(SegmentName::new("/drm/a/b"), Err(ShmError::Io(_))));
        assert!(matches!(SegmentName::new("/drm\0x"), Err(ShmError::Io(_))));
        let max = 255;
        assert!(SegmentName::new(&format!("/{}", "d".repeat(max - 1))).is_ok());
        assert!(matches!(
            SegmentName::new(&format!("/{}", "d".repeat(max))),
            Err(ShmError::Io(_))
        ));
    }

    /// REQ: REQ-CORE-088 — POSIX generated name family format.
    #[cfg(unix)]
    #[test]
    fn backend_name_format() {
        let name = backend_segment_name("dep-1", 0x0102_0304_0506_0708);
        assert!(
            name.as_ref()
                .is_ok_and(|n| n.as_str() == "/drm.dep-1.0102030405060708"),
            "{name:?}"
        );
    }

    /// REQ: REQ-CORE-069 — lock file name family.
    #[cfg(unix)]
    #[test]
    fn lock_path_format() {
        let paths = LockPaths::new("/run/drm/dep-1");
        assert_eq!(
            paths.active(),
            std::path::PathBuf::from("/run/drm/dep-1/active.lock")
        );
        assert_eq!(
            paths.publisher(7),
            std::path::PathBuf::from("/run/drm/dep-1/t_7.publisher.lock")
        );
        assert_eq!(
            paths.consumer(7, 3),
            std::path::PathBuf::from("/run/drm/dep-1/t_7.3.consumer.lock")
        );
    }

    /// REQ: REQ-CORE-078, REQ-CORE-088 — Windows generated section name family
    /// and conformance (I0/I-W parity); pure form, evaluated on every host.
    #[test]
    fn windows_name_conformance() {
        let name = windows_section_name("dep-1", 0x0102_0304_0506_0708);
        assert_eq!(name, "Local\\drm_dep-1_0102030405060708");
        assert!(windows_name_conforms(&name));
        assert!(!windows_name_conforms("drm_dep-1_0102"));
        assert!(!windows_name_conforms("Local\\"));
        assert!(!windows_name_conforms("Local\\a\\b"));
        assert!(!windows_name_conforms("Local\\a\0b"));
        let max = super::WINDOWS_NAME_MAX - super::WINDOWS_LOCAL_PREFIX.len();
        let at_limit = format!("Local\\{}", "d".repeat(max));
        assert_eq!(at_limit.len(), super::WINDOWS_NAME_MAX);
        assert!(windows_name_conforms(&at_limit));
        let over = format!("Local\\{}", "d".repeat(max + 1));
        assert!(!windows_name_conforms(&over));
    }

    /// REQ: REQ-CORE-098 — `ShmError` → `Status` mapping (`shm-backend.md` Data Structures);
    /// invalid names surface as the kernel's `EINVAL`-family `Io` branch.
    #[test]
    fn shm_error_status_map() {
        let io = ShmError::Io(std::io::Error::other("x"));
        let cases = [
            (ShmError::NotFound, Status::NotFound),
            (ShmError::AlreadyExists, Status::AlreadyExists),
            (ShmError::PermissionDenied, Status::Permission),
            (ShmError::QuotaExceeded, Status::InvalidConfig),
            (ShmError::SizeMismatch, Status::InvalidData),
            (ShmError::StaleIncomplete, Status::NotReady),
        ];
        for (e, s) in cases {
            assert_eq!(Status::from(e), s);
        }
        assert_eq!(Status::from(io), Status::InvalidData);
    }

    /// REQ: REQ-CORE-070 — clean report variants exist for the `drm clean` contract.
    #[test]
    fn clean_report_variants() {
        assert_ne!(CleanReport::Cleaned, CleanReport::ActiveUntouched);
    }
}
