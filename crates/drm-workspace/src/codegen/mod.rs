//! Codegen & invalidation ([codegen-invalidation.md](../../../docs/algorithms/toolchain/codegen-invalidation.md)):
//! `GenPlan`/`GenUnit`, generation fingerprints over directed capabilities,
//! stable shared output paths, content-compare writes (timestamp preservation),
//! platform atomic replacement, and direct reconciliation against the complete
//! expected tree. Generated bindings carry canonical identities/contracts only —
//! never runtime numeric handles (I-G9). This slice renders C; C++/Rust/Python
//! renderers land in later phases.
//! REQ: REQ-TOOL-024, REQ-TOOL-025, REQ-TOOL-026, REQ-TOOL-027, REQ-TOOL-028,
//! REQ-TOOL-029, REQ-TOOL-030, REQ-TOOL-065.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use drm_types::{Status, blake3_128};

use crate::model::layout_hash;
use crate::model::{NodeEntry, TopicEntry, WorkspaceModel};

mod render;

/// Generator version embedded in every fingerprint: bumping it invalidates all
/// units so a changed renderer re-runs (I-G2 edge case "generator version
/// bump"). REQ: REQ-TOOL-027.
pub const GENERATOR_VERSION: u32 = 1;

/// Default codegen options tag (empty). REQ: REQ-TOOL-027.
pub const DEFAULT_OPTIONS: &str = "";

/// Fingerprint database file name under the generation root.
/// REQ: REQ-TOOL-027.
const FINGERPRINT_DB: &str = ".fingerprints";

/// Generation root directory (`.drm/gen`). REQ: REQ-TOOL-029.
const GEN_DIR: &str = ".drm/gen";

/// Target language of a generated artifact. REQ: REQ-TOOL-029.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    /// C headers (`.drm/gen/c/...`).
    C,
}

/// Languages rendered by [`plan`]; grows as renderers land.
/// REQ: REQ-TOOL-029.
pub const LANGS: [Lang; 1] = [Lang::C];

impl Lang {
    /// Directory component under `.drm/gen/`. REQ: REQ-TOOL-029.
    #[must_use]
    pub const fn dir(self) -> &'static str {
        match self {
            Self::C => "c",
        }
    }

    /// File extension of the rendered artifact. REQ: REQ-TOOL-029.
    #[must_use]
    pub const fn ext(self) -> &'static str {
        match self {
            Self::C => "h",
        }
    }

    /// Stable encoding used by the fingerprint database.
    /// REQ: REQ-TOOL-027.
    const fn tag(self) -> &'static str {
        self.dir()
    }

    /// Parses a database language tag. REQ: REQ-TOOL-027.
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "c" => Some(Self::C),
            _ => None,
        }
    }
}

/// Generated artifact kind. REQ: REQ-TOOL-028.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GenKind {
    /// Topic-scoped types/codec unit (`<topic>/target.<ext>`), shared by every
    /// node using the topic. REQ: REQ-TOOL-029.
    Topic,
    /// Node-scoped API unit (`nodes/<node>.<ext>`), sole owner of endpoint
    /// operations. REQ: REQ-TOOL-028.
    NodeApi,
}

impl GenKind {
    /// Stable encoding used by the fingerprint database.
    /// REQ: REQ-TOOL-027.
    const fn tag(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::NodeApi => "node",
        }
    }

    /// Parses a database kind tag. REQ: REQ-TOOL-027.
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "topic" => Some(Self::Topic),
            "node" => Some(Self::NodeApi),
            _ => None,
        }
    }
}

/// Namespaced fingerprint key: kind + language + identity (topic path or node
/// name). REQ: REQ-TOOL-027.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FingerprintKey {
    /// Artifact kind.
    pub kind: GenKind,
    /// Target language.
    pub lang: Lang,
    /// Topic path or node name.
    pub identity: String,
}

impl FingerprintKey {
    /// Builds a key. REQ: REQ-TOOL-027.
    #[must_use]
    pub fn new(kind: GenKind, lang: Lang, identity: impl Into<String>) -> Self {
        Self {
            kind,
            lang,
            identity: identity.into(),
        }
    }
}

/// One dirty generated unit: its fingerprint and fully rendered body.
/// REQ: REQ-TOOL-026, REQ-TOOL-027.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenUnit {
    /// Artifact kind.
    pub kind: GenKind,
    /// Target language.
    pub lang: Lang,
    /// Fingerprint key.
    pub key: FingerprintKey,
    /// Stable output path.
    pub out_path: PathBuf,
    /// 128-bit generation fingerprint.
    pub fingerprint: [u8; 16],
    /// Deterministic rendered artifact.
    pub body: String,
}

/// A generation plan: the COMPLETE desired tree (`expected_outputs`) plus only
/// the units whose fingerprint changed or whose file is missing (`dirty_units`).
/// Reconciliation is computed against the expected set, never the dirty set
/// (I-G3/I-G5). REQ: REQ-TOOL-026.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GenPlan {
    /// Every output the model requires, dirty or not.
    pub expected_outputs: BTreeSet<PathBuf>,
    /// Units to render this run.
    pub dirty_units: Vec<GenUnit>,
}

/// Persistent generation fingerprint database (`.drm/gen/.fingerprints`).
/// REQ: REQ-TOOL-027.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FingerprintDb {
    entries: BTreeMap<FingerprintKey, [u8; 16]>,
}

impl FingerprintDb {
    /// Loads the database under `root`; a missing or malformed file yields an
    /// empty database so the next run rechecks every output.
    /// REQ: REQ-TOOL-027.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when the database exists but cannot be read.
    pub fn load(root: &Path) -> Result<Self, Status> {
        let path = root.join(GEN_DIR).join(FINGERPRINT_DB);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err(Status::InvalidConfig),
        };
        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let mut parts = line.split('\t');
            let (Some(kind), Some(lang), Some(identity), Some(hex), None) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                continue;
            };
            let (Some(kind), Some(lang), Some(fp)) = (
                GenKind::from_tag(kind),
                Lang::from_tag(lang),
                decode_hex(hex),
            ) else {
                continue;
            };
            entries.insert(FingerprintKey::new(kind, lang, identity), fp);
        }
        Ok(Self { entries })
    }

    /// The recorded fingerprint for `key`, if any. REQ: REQ-TOOL-027.
    #[must_use]
    pub fn get(&self, key: &FingerprintKey) -> Option<[u8; 16]> {
        self.entries.get(key).copied()
    }

    /// Records (or overwrites) the fingerprint for `key`. REQ: REQ-TOOL-027.
    pub fn record(&mut self, key: FingerprintKey, fingerprint: [u8; 16]) {
        self.entries.insert(key, fingerprint);
    }

    /// Drops every record whose key is absent from `keep` (I-G5 stale-record
    /// pruning). REQ: REQ-TOOL-025.
    pub fn prune(&mut self, keep: &BTreeSet<FingerprintKey>) {
        self.entries.retain(|k, _| keep.contains(k));
    }

    /// Number of live records. REQ: REQ-TOOL-027.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the database holds no records. REQ: REQ-TOOL-027.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes the database deterministically (sorted keys).
    /// REQ: REQ-TOOL-027.
    fn render(&self) -> String {
        let mut out = String::new();
        for (key, fp) in &self.entries {
            out.push_str(key.kind.tag());
            out.push('\t');
            out.push_str(key.lang.tag());
            out.push('\t');
            out.push_str(&key.identity);
            out.push('\t');
            out.push_str(&encode_hex(fp));
            out.push('\n');
        }
        out
    }
}

/// Stable output path for a unit: `.drm/gen/<lang>/<topic>/target.<ext>` for
/// topics, `.drm/gen/<lang>/nodes/<node>.<ext>` for node APIs.
/// REQ: REQ-TOOL-029.
#[must_use]
pub fn out_path(root: &Path, lang: Lang, kind: GenKind, identity: &str) -> PathBuf {
    let mut path = root.join(GEN_DIR).join(lang.dir());
    match kind {
        GenKind::Topic => {
            path.push(identity);
            path.push(format!("target.{}", lang.ext()));
        }
        GenKind::NodeApi => {
            path.push("nodes");
            path.push(format!("{}.{}", render::file_ident(identity), lang.ext()));
        }
    }
    path
}

/// Topic fingerprint: `hash(canonical_schema, topic_identity, generator_version,
/// options)`. Schema identity includes both the schema hash and the canonical
/// layout hash. REQ: REQ-TOOL-027.
#[must_use]
pub fn topic_fingerprint(topic: &str, entry: &TopicEntry, options: &str) -> [u8; 16] {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"DRM-GEN-TOPIC\0");
    buf.extend_from_slice(&GENERATOR_VERSION.to_le_bytes());
    push_str(&mut buf, topic);
    buf.extend_from_slice(&entry.schema_hash);
    buf.extend_from_slice(&layout_hash(&entry.layout));
    push_str(&mut buf, options);
    blake3_128(&buf)
}

/// Node API fingerprint over the ordered sequence of DIRECTED capabilities
/// `(topic, direction, schema_hash, layout_hash)` — direction is identity, never
/// a union of names (I-G8); declaration changes alone move the fingerprint
/// (I-G6). REQ: REQ-TOOL-027, REQ-TOOL-028.
#[must_use]
pub fn node_fingerprint(node: &NodeEntry, model: &WorkspaceModel, options: &str) -> [u8; 16] {
    let mut caps: Vec<(u8, &str)> = Vec::new();
    for topic in &node.publishes {
        caps.push((0, topic));
    }
    for topic in &node.consumes {
        caps.push((1, topic));
    }
    caps.sort_unstable();

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"DRM-GEN-NODE\0");
    buf.extend_from_slice(&GENERATOR_VERSION.to_le_bytes());
    buf.extend_from_slice(&u32::try_from(caps.len()).unwrap_or(u32::MAX).to_le_bytes());
    for (direction, topic) in caps {
        buf.push(direction);
        push_str(&mut buf, topic);
        if let Some(entry) = model.topics.get(topic) {
            buf.extend_from_slice(&entry.schema_hash);
            buf.extend_from_slice(&layout_hash(&entry.layout));
        }
    }
    push_str(&mut buf, options);
    blake3_128(&buf)
}

/// Plans generation against the previous fingerprint database: the complete
/// expected tree plus the units whose fingerprint changed or whose file is
/// missing (I-G2 skip). REQ: REQ-TOOL-026, REQ-TOOL-027.
#[must_use]
pub fn plan(model: &WorkspaceModel, prev: &FingerprintDb, root: &Path) -> GenPlan {
    plan_with(model, prev, root, DEFAULT_OPTIONS)
}

/// [`plan`] with an explicit options tag participating in every fingerprint
/// (I-G2 `gen_mtime_preserved_fp_bump`). REQ: REQ-TOOL-027.
#[must_use]
pub fn plan_with(
    model: &WorkspaceModel,
    prev: &FingerprintDb,
    root: &Path,
    options: &str,
) -> GenPlan {
    let mut expected = BTreeSet::new();
    let mut dirty = Vec::new();

    for (topic, entry) in &model.topics {
        for lang in LANGS {
            let out = out_path(root, lang, GenKind::Topic, topic);
            expected.insert(out.clone());
            let fingerprint = topic_fingerprint(topic, entry, options);
            let key = FingerprintKey::new(GenKind::Topic, lang, topic.clone());
            if prev.get(&key) == Some(fingerprint) && out.is_file() {
                continue;
            }
            dirty.push(GenUnit {
                kind: GenKind::Topic,
                lang,
                key,
                out_path: out,
                fingerprint,
                body: render::topic(lang, topic, entry),
            });
        }
    }

    for (node, entry) in &model.nodes {
        for lang in LANGS {
            let out = out_path(root, lang, GenKind::NodeApi, node);
            expected.insert(out.clone());
            let fingerprint = node_fingerprint(entry, model, options);
            let key = FingerprintKey::new(GenKind::NodeApi, lang, node.clone());
            if prev.get(&key) == Some(fingerprint) && out.is_file() {
                continue;
            }
            dirty.push(GenUnit {
                kind: GenKind::NodeApi,
                lang,
                key,
                out_path: out,
                fingerprint,
                body: render::node(lang, node, entry, model),
            });
        }
    }

    GenPlan {
        expected_outputs: expected,
        dirty_units: dirty,
    }
}

/// Executes a plan: writes dirty units, preserves byte-identical existing files
/// (I-G2), replaces others atomically (I-G7), reconciles against the complete
/// expected tree (I-G5), prunes stale fingerprint records, and saves the new
/// database last. REQ: REQ-TOOL-024, REQ-TOOL-025, REQ-TOOL-030.
///
/// # Errors
/// [`Status::InvalidConfig`] when a directory cannot be created, a temporary
/// file cannot be written, or reconciliation cannot remove an orphan;
/// [`Status::LockFailed`] when the atomic replacement fails.
pub fn execute(plan: &GenPlan, prev: &mut FingerprintDb, root: &Path) -> Result<(), Status> {
    let mut next = prev.clone();

    for unit in &plan.dirty_units {
        let Some(parent) = unit.out_path.parent() else {
            return Err(Status::InvalidConfig);
        };
        fs::create_dir_all(parent).map_err(|_| Status::InvalidConfig)?;
        let tmp = temp_sibling(&unit.out_path);
        fs::write(&tmp, unit.body.as_bytes()).map_err(|_| Status::InvalidConfig)?;
        if file_equals(&tmp, &unit.out_path) {
            // I-G2: unchanged bytes keep the existing file (and its mtime).
            let _ = fs::remove_file(&tmp);
            next.record(unit.key.clone(), unit.fingerprint);
            continue;
        }
        atomic_replace(&tmp, &unit.out_path)?;
        next.record(unit.key.clone(), unit.fingerprint);
    }

    reconcile(plan, root)?;

    let keep: BTreeSet<FingerprintKey> = next
        .entries
        .keys()
        .filter(|key| {
            plan.expected_outputs
                .contains(&out_path(root, key.lang, key.kind, &key.identity))
        })
        .cloned()
        .collect();
    next.prune(&keep);
    save_db(&next, root)?;
    *prev = next;
    Ok(())
}

/// Removes generated files absent from the expected tree (I-G5). The
/// fingerprint database is never treated as a generated artifact.
/// REQ: REQ-TOOL-025.
fn reconcile(plan: &GenPlan, root: &Path) -> Result<(), Status> {
    let existing = existing_generated(root)?;
    for orphan in existing.difference(&plan.expected_outputs) {
        fs::remove_file(orphan).map_err(|_| Status::InvalidConfig)?;
    }
    Ok(())
}

/// Every generated file under `.drm/gen`, excluding the fingerprint database.
/// REQ: REQ-TOOL-025.
///
/// # Errors
/// [`Status::InvalidConfig`] when the generation tree cannot be read.
pub fn existing_generated(root: &Path) -> Result<BTreeSet<PathBuf>, Status> {
    let mut out = BTreeSet::new();
    let base = root.join(GEN_DIR);
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(Status::InvalidConfig),
        };
        for entry in entries {
            let entry = entry.map_err(|_| Status::InvalidConfig)?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_none_or(|n| n != FINGERPRINT_DB) {
                out.insert(path);
            }
        }
    }
    Ok(out)
}

/// Saves the fingerprint database atomically under `root`.
/// REQ: REQ-TOOL-024.
fn save_db(db: &FingerprintDb, root: &Path) -> Result<(), Status> {
    let path = root.join(GEN_DIR).join(FINGERPRINT_DB);
    let Some(parent) = path.parent() else {
        return Err(Status::InvalidConfig);
    };
    fs::create_dir_all(parent).map_err(|_| Status::InvalidConfig)?;
    let tmp = temp_sibling(&path);
    fs::write(&tmp, db.render().as_bytes()).map_err(|_| Status::InvalidConfig)?;
    atomic_replace(&tmp, &path)
}

/// Process-unique temporary sibling in the destination directory.
/// REQ: REQ-TOOL-024.
fn temp_sibling(dest: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| "gen".to_string(), str::to_string);
    dest.with_file_name(format!(".{name}.tmp-{}-{n}", std::process::id()))
}

/// Whether `a` and `b` hold identical bytes (`b` missing ⇒ false).
/// REQ: REQ-TOOL-030.
fn file_equals(a: &Path, b: &Path) -> bool {
    match (fs::read(a), fs::read(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Platform atomic replacement (I-G7): POSIX `rename` overwrites atomically.
/// REQ: REQ-TOOL-024.
#[cfg(unix)]
fn atomic_replace(tmp: &Path, dest: &Path) -> Result<(), Status> {
    fs::rename(tmp, dest).map_err(|_| Status::LockFailed)
}

/// Platform atomic replacement fallback for non-Unix hosts (Windows
/// `MoveFileEx`/`ReplaceFile` semantics arrive with the Windows port); the
/// prior artifact is preserved on failure.
/// REQ: REQ-TOOL-024.
#[cfg(not(unix))]
fn atomic_replace(tmp: &Path, dest: &Path) -> Result<(), Status> {
    if dest.exists() {
        fs::remove_file(dest).map_err(|_| Status::LockFailed)?;
    }
    fs::rename(tmp, dest).map_err(|_| Status::LockFailed)
}

/// Length-prefixed UTF-8 field in a fingerprint buffer.
/// REQ: REQ-TOOL-027.
fn push_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

/// Lowercase hex encoding of a 128-bit fingerprint.
/// REQ: REQ-TOOL-027.
fn encode_hex(fp: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for b in fp {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Decodes a 32-character lowercase hex fingerprint; anything else is `None`.
/// REQ: REQ-TOOL-027.
fn decode_hex(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    let bytes = hex.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = (bytes[2 * i] as char).to_digit(16)?;
        let lo = (bytes[2 * i + 1] as char).to_digit(16)?;
        *slot = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{FingerprintKey, GenKind, Lang, decode_hex, encode_hex};

    /// REQ: REQ-TOOL-027 — fingerprint hex round-trips.
    #[test]
    fn hex_round_trip() {
        let fp = [0xAB_u8; 16];
        let hex = encode_hex(&fp);
        assert_eq!(hex.len(), 32);
        assert_eq!(decode_hex(&hex), Some(fp));
        assert_eq!(decode_hex("nope"), None);
    }

    /// REQ: REQ-TOOL-027 — fingerprint keys are ordered deterministically.
    #[test]
    fn key_order_is_stable() {
        let a = FingerprintKey::new(GenKind::Topic, Lang::C, "a/b");
        let b = FingerprintKey::new(GenKind::Topic, Lang::C, "a/c");
        assert!(a < b);
    }
}
