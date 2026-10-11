//! Single-writer edge authority
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §Invariants
//! `I-S11`, §Protocols; [spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Protocols slot write/snapshot; [publisher-lifecycle.md](../runtime/publisher-lifecycle.md)
//! §Invariants `I-L1` authority pattern reuse). Every dynamic edge has exactly
//! one writer. DRM enforces this twice: statically, `drm verify`'s `V-SPAT` check
//! rejects a declaration with more than one writer before any node is deployed;
//! dynamically, an OS-backed authority lock held for the writer's lifetime gives
//! held-or-refuse semantics, so a second writer that starts at runtime is refused
//! rather than silently racing. The kernel releases the lock when the holder
//! dies, so a crashed writer never wedges the edge (`I-L1`/`I-L2`).
//!
//! The lock is a std `File` advisory lock (`File::try_lock`), so the module
//! stays `#![forbid(unsafe_code)]` and brokerless — no daemon arbitrates
//! authority. Authority is per dynamic edge, scoped to a deployment directory.
//!
//! REQ: REQ-SPATIAL-030, REQ-SPATIAL-050.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::types::SpatialError;

/// A held single-writer authority over one dynamic edge (`I-S11`). Dropping the
/// token releases the OS lock, so authority never outlives its writer.
/// REQ: REQ-SPATIAL-030.
#[derive(Debug)]
pub struct EdgeAuthority {
    edge: u32,
    node: u64,
    path: PathBuf,
    file: File,
}

impl EdgeAuthority {
    /// The edge index this authority guards.
    /// REQ: REQ-SPATIAL-030.
    #[must_use]
    pub fn edge(&self) -> u32 {
        self.edge
    }

    /// The node id that holds the authority.
    /// REQ: REQ-SPATIAL-030.
    #[must_use]
    pub fn node(&self) -> u64 {
        self.node
    }

    /// The lock-file path backing this authority.
    /// REQ: REQ-SPATIAL-030.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Releases authority explicitly (equivalent to dropping the token). The OS
    /// lock is released; a later [`acquire`] can take it.
    ///
    /// # Errors
    /// [`SpatialError::WriterConflict`] when the kernel refuses the unlock.
    /// REQ: REQ-SPATIAL-030.
    pub fn release(self) -> Result<(), SpatialError> {
        self.file.unlock().map_err(|_| SpatialError::WriterConflict)
    }
}

/// The conventional authority lock path for `edge` under `dir`:
/// `<dir>/edge_<edge>.spatial.lock` (`I-S11`).
/// REQ: REQ-SPATIAL-030.
#[must_use]
pub fn authority_path(dir: &Path, edge: u32) -> PathBuf {
    dir.join(format!("edge_{edge}.spatial.lock"))
}

/// Acquires exclusive single-writer authority over `edge` under `dir` for
/// `node` (`I-S11`). Exactly one caller holds the lock; every other caller is
/// refused with [`SpatialError::WriterConflict`] (no takeover, no waiting). The
/// OS releases the lock if the holder dies, so a restart can re-acquire without
/// a heartbeat (`I-L1`/`I-L2` pattern reuse).
///
/// # Errors
/// [`SpatialError::WriterConflict`] when another writer holds the authority or
/// the lock file cannot be opened.
/// REQ: REQ-SPATIAL-030.
pub fn acquire(dir: &Path, edge: u32, node: u64) -> Result<EdgeAuthority, SpatialError> {
    let path = authority_path(dir, edge);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|_| SpatialError::WriterConflict)?;
    file.try_lock().map_err(|_| SpatialError::WriterConflict)?;
    Ok(EdgeAuthority {
        edge,
        node,
        path,
        file,
    })
}

#[cfg(test)]
mod tests {
    use super::{EdgeAuthority, acquire, authority_path};
    use crate::types::SpatialError;
    use std::path::PathBuf;

    /// A unique temporary directory per test.
    /// REQ: REQ-SPATIAL-030.
    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "drm-swat-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("tmpdir: {e}"));
        dir
    }

    /// REQ: REQ-SPATIAL-030 — the first writer acquires the authority and a
    /// second writer is refused (`I-S11`).
    #[test]
    fn second_writer_refused() {
        let dir = tmpdir("second");
        let first: EdgeAuthority = acquire(&dir, 7, 1).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(first.edge(), 7);
        assert_eq!(first.node(), 1);
        assert_eq!(
            acquire(&dir, 7, 2).err(),
            Some(SpatialError::WriterConflict)
        );
        first.release().unwrap_or_else(|e| unreachable!("{e}"));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-SPATIAL-030 — authority is per edge: writers on different edges
    /// never conflict.
    #[test]
    fn distinct_edges_independent() {
        let dir = tmpdir("distinct");
        let a = acquire(&dir, 0, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let b = acquire(&dir, 1, 2).unwrap_or_else(|e| unreachable!("{e}"));
        assert_ne!(a.edge(), b.edge());
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-SPATIAL-030 — dropping the token releases authority so a later
    /// writer takes it without a heartbeat (`I-L2` pattern).
    #[test]
    fn drop_releases_authority() {
        let dir = tmpdir("drop");
        {
            let _held = acquire(&dir, 3, 1).unwrap_or_else(|e| unreachable!("{e}"));
        }
        let reacquired = acquire(&dir, 3, 9).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(reacquired.node(), 9);
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-SPATIAL-030 — the conventional lock path is deterministic.
    #[test]
    fn lock_path_deterministic() {
        let p = authority_path(std::path::Path::new("/run/drm/dep"), 5);
        assert_eq!(
            p,
            std::path::PathBuf::from("/run/drm/dep/edge_5.spatial.lock")
        );
    }
}
