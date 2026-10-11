//! Journaled multi-file commits (I-ST8/I-ST12). No filesystem operation
//! commits several files atomically, so "atomic" means journaled:
//! validate → prepare temps → write journal → rename → mark committed →
//! remove journal. A crash between journal and rename is recovered on startup
//! by finishing the renames or rolling back to the prior contents. Recovery is
//! idempotent after every kill point.
//!
//! REQ: REQ-STUDIO-002 (conditional undo inputs), REQ-STUDIO-008 (journaled
//! multi-file commit + crash recovery), REQ-STUDIO-009 (revision guard inputs).

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::protocol::StudioError;

/// A prepared file output plus the rollback data needed by conditional undo.
/// REQ: REQ-STUDIO-002, REQ-STUDIO-008.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Destination path (renamed into place).
    pub target: PathBuf,
    /// Prepared temporary path (same directory as `target`).
    pub temp: PathBuf,
    /// Desired content.
    pub content: String,
    /// Prior content, or `None` when the file did not exist.
    pub prior: Option<String>,
}

impl Output {
    /// Hash of the prior content (the conditional-undo guard).
    /// REQ: REQ-STUDIO-002.
    #[must_use]
    pub fn prior_hash(&self) -> Option<u64> {
        self.prior.as_deref().map(|s| hash_bytes(s.as_bytes()))
    }
}

/// Journal state machine. REQ: REQ-STUDIO-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    /// Temps + journal written; renames not yet applied.
    Prepared,
    /// Renames applied; journal cleanup pending.
    Committed,
}

/// A journaled change transaction. REQ: REQ-STUDIO-008.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    /// Transaction id (also the journal file stem).
    pub id: String,
    /// Prepared outputs.
    pub outputs: Vec<Output>,
    /// Current state.
    pub state: JournalState,
}

impl ChangeSet {
    /// Builds a `Prepared` change set for `outputs` with a fresh id.
    /// REQ: REQ-STUDIO-008.
    #[must_use]
    pub fn new(outputs: Vec<Output>) -> Self {
        Self {
            id: fresh_id(),
            outputs,
            state: JournalState::Prepared,
        }
    }

    /// Expected input hashes for the conditional-undo guard (I-ST9).
    /// REQ: REQ-STUDIO-002.
    #[must_use]
    pub fn expected_input_hashes(&self) -> Vec<(PathBuf, Option<u64>)> {
        self.outputs
            .iter()
            .map(|o| (o.target.clone(), o.prior_hash()))
            .collect()
    }

    /// Whether every target still holds its expected input content (I-ST9).
    /// REQ: REQ-STUDIO-002.
    #[must_use]
    pub fn inputs_unchanged(&self) -> bool {
        self.outputs.iter().all(|o| {
            let current = fs::read_to_string(&o.target).ok();
            current.as_deref() == o.prior.as_deref()
        })
    }
}

/// Hashes bytes deterministically within and across runs.
/// REQ: REQ-STUDIO-002.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Monotonic suffix so ids are unique even within the same nanosecond.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generates a fresh transaction id. REQ: REQ-STUDIO-008.
#[must_use]
pub fn fresh_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("cs-{nanos:x}-{n:x}")
}

/// The `<root>/.drm/journal` store. REQ: REQ-STUDIO-008.
#[derive(Debug, Clone)]
pub struct JournalStore {
    dir: PathBuf,
}

impl JournalStore {
    /// Opens (creating lazily) the journal directory under `root`.
    /// REQ: REQ-STUDIO-008.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            dir: root.join(".drm").join("journal"),
        }
    }

    /// Journal directory path. REQ: REQ-STUDIO-008.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Runs the six-step journaled commit (validate → temps → journal →
    /// rename → mark committed → cleanup). REQ: REQ-STUDIO-008, REQ-STUDIO-012.
    ///
    /// # Errors
    /// [`StudioError::Journal`] on any filesystem failure at a commit step.
    pub fn commit(&self, cs: &mut ChangeSet) -> Result<(), StudioError> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| StudioError::Journal(format!("create journal dir: {e}")))?;

        // 2. Prepare all outputs as temporary files (flushed before rename).
        for out in &cs.outputs {
            write_flush(&out.temp, out.content.as_bytes())?;
        }

        // 3. Write the transaction journal.
        self.write_journal(cs)?;

        // 4. Rename prepared files into place.
        for out in &cs.outputs {
            fs::rename(&out.temp, &out.target).map_err(|e| {
                StudioError::Journal(format!("rename `{}`: {e}", out.target.display()))
            })?;
            fsync_parent(&out.target)?;
        }

        // 5. Mark committed.
        cs.state = JournalState::Committed;
        self.write_journal(cs)?;

        // 6. Remove journal.
        self.remove_journal(&cs.id)?;
        Ok(())
    }

    /// Loads a persisted change set by id. REQ: REQ-STUDIO-002.
    ///
    /// # Errors
    /// [`StudioError::Journal`] when the journal is absent or unreadable.
    pub fn load(&self, id: &str) -> Result<ChangeSet, StudioError> {
        let path = self.journal_path(id);
        let text = fs::read_to_string(&path)
            .map_err(|e| StudioError::Journal(format!("read `{}`: {e}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|e| StudioError::Journal(format!("parse `{}`: {e}", path.display())))
    }

    /// Reverts a change set to its prior contents (conditional undo).
    /// REQ: REQ-STUDIO-002.
    ///
    /// # Errors
    /// [`StudioError::Journal`] when a prior content cannot be restored.
    pub fn revert(&self, cs: &ChangeSet) -> Result<(), StudioError> {
        for out in &cs.outputs {
            match &out.prior {
                Some(prior) => write_flush(&out.target, prior.as_bytes())?,
                None => {
                    let _ = fs::remove_file(&out.target);
                }
            }
        }
        self.remove_journal(&cs.id)
    }

    /// Scans journals and finishes or rolls back each pending transaction.
    /// Returns the number of transactions recovered. Idempotent.
    /// REQ: REQ-STUDIO-008.
    ///
    /// # Errors
    /// [`StudioError::Journal`] on a filesystem failure during recovery.
    pub fn recover(&self) -> Result<usize, StudioError> {
        if !self.dir.is_dir() {
            return Ok(0);
        }
        let mut ids: Vec<String> = Vec::new();
        let rd = fs::read_dir(&self.dir)
            .map_err(|e| StudioError::Journal(format!("read journal dir: {e}")))?;
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json")
                && let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str)
            {
                ids.push(stem.to_string());
            }
        }
        let mut recovered = 0;
        for id in ids {
            let cs = self.load(&id)?;
            if cs.state == JournalState::Committed {
                self.remove_journal(&id)?;
            } else if cs.outputs.iter().any(was_renamed) {
                finish_renames(&cs)?;
                self.remove_journal(&id)?;
            } else {
                rollback_temps(&cs);
                self.remove_journal(&id)?;
            }
            recovered += 1;
        }
        Ok(recovered)
    }

    /// Journal file path for an id. REQ: REQ-STUDIO-008.
    fn journal_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Writes the journal file and flushes it (I-ST12).
    /// REQ: REQ-STUDIO-008.
    fn write_journal(&self, cs: &ChangeSet) -> Result<(), StudioError> {
        let text = serde_json::to_string_pretty(cs)
            .map_err(|e| StudioError::Journal(format!("encode journal: {e}")))?;
        write_flush(&self.journal_path(&cs.id), text.as_bytes())
    }

    /// Removes a journal file if present. REQ: REQ-STUDIO-008.
    fn remove_journal(&self, id: &str) -> Result<(), StudioError> {
        let path = self.journal_path(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StudioError::Journal(format!(
                "remove `{}`: {e}",
                path.display()
            ))),
        }
    }
}

/// Whether a prepared output was already renamed into place.
/// REQ: REQ-STUDIO-008.
fn was_renamed(out: &Output) -> bool {
    match fs::read_to_string(&out.target) {
        Ok(current) => current == out.content && out.prior.as_deref() != Some(current.as_str()),
        Err(_) => false,
    }
}

/// Finishes the remaining renames of a partially committed change set.
/// REQ: REQ-STUDIO-008.
fn finish_renames(cs: &ChangeSet) -> Result<(), StudioError> {
    for out in &cs.outputs {
        if out.temp.exists() {
            fs::rename(&out.temp, &out.target).map_err(|e| {
                StudioError::Journal(format!("recover rename `{}`: {e}", out.target.display()))
            })?;
            fsync_parent(&out.target)?;
        }
    }
    Ok(())
}

/// Deletes prepared temps of an unrenamed change set (rollback).
/// REQ: REQ-STUDIO-008.
fn rollback_temps(cs: &ChangeSet) {
    for out in &cs.outputs {
        let _ = fs::remove_file(&out.temp);
    }
}

/// Writes `bytes` to `path` and flushes to disk (I-ST12).
/// REQ: REQ-STUDIO-008.
fn write_flush(path: &Path, bytes: &[u8]) -> Result<(), StudioError> {
    let mut file = File::create(path)
        .map_err(|e| StudioError::Journal(format!("create `{}`: {e}", path.display())))?;
    file.write_all(bytes)
        .map_err(|e| StudioError::Journal(format!("write `{}`: {e}", path.display())))?;
    file.sync_all()
        .map_err(|e| StudioError::Journal(format!("flush `{}`: {e}", path.display())))?;
    Ok(())
}

/// Flushes the parent directory so a rename is durable (I-ST12).
/// REQ: REQ-STUDIO-008.
fn fsync_parent(path: &Path) -> Result<(), StudioError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let dir = File::open(parent)
        .map_err(|e| StudioError::Journal(format!("open dir `{}`: {e}", parent.display())))?;
    dir.sync_all()
        .map_err(|e| StudioError::Journal(format!("flush dir `{}`: {e}", parent.display())))?;
    Ok(())
}

/// Builds an [`Output`] for `target`, reading the prior content when present.
/// REQ: REQ-STUDIO-008.
///
/// # Errors
/// [`StudioError::Io`] when an existing target cannot be read.
pub fn output_for(id: &str, target: PathBuf, content: String) -> Result<Output, StudioError> {
    let prior = match fs::read_to_string(&target) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(StudioError::Io(format!("read `{}`: {e}", target.display())));
        }
    };
    let mut temp_name = target.as_os_str().to_os_string();
    temp_name.push(format!(".drmtmp-{id}"));
    Ok(Output {
        target,
        temp: PathBuf::from(temp_name),
        content,
        prior,
    })
}
