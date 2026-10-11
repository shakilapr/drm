//! `drm ids` — report and explicitly compact the committed handle-assignment
//! lockfile `.drm/ids.json` (handle-system.md §Data Structures; §Functions
//! `assign`/`compact`). Live-table manipulation reuses
//! [`drm_workspace::model::handles`]; this module owns IO and the canonical
//! serialization only.
//! REQ: REQ-TOOL-080 (committed lockfile, no nondeterministic metadata),
//! REQ-TOOL-038 (compact explicit only), REQ-TOOL-036 (normal builds preserve
//! assigned handles), REQ-TOOL-037 (topic path is identity).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use drm_workspace::canon::json_escape;
use drm_workspace::model::handles::{AssignmentHistory, HandleTable, ReserveReason};

use crate::args::IdsAction;
use crate::error::CliError;

/// Lockfile `format_version` this tool reads and writes (handle-system.md).
/// REQ: REQ-TOOL-080.
pub const FORMAT_VERSION: u64 = 1;

/// Workspace-relative path of the committed lockfile. REQ: REQ-TOOL-080.
pub const LOCKFILE_REL: &str = ".drm/ids.json";

/// Runs an `ids` action against `root`. REQ: REQ-TOOL-038, REQ-TOOL-080.
///
/// # Errors
/// [`CliError::Refused`] for `compact` without `--yes` (I-H4) — emitted before
/// any read or write so the lockfile is provably untouched; IO/lockfile errors
/// otherwise.
pub fn run(root: &Path, action: IdsAction) -> Result<(), CliError> {
    let path = root.join(LOCKFILE_REL);
    match action {
        IdsAction::Report => {
            let table = load(&path)?;
            print!("{}", report(&table));
            Ok(())
        }
        IdsAction::Compact { yes: false } => Err(CliError::Refused(format!(
            "refusing to compact `{}` without --yes (I-H4 / REQ-TOOL-038); \
             re-run `drm ids compact --yes` (this regenerates deployment identity)",
            path.display()
        ))),
        IdsAction::Compact { yes: true } => {
            let table = load(&path)?;
            let packed = table
                .compact(true)
                .map_err(|e| CliError::Lockfile(format!("compact: {e:?}")))?;
            write(&path, &packed)?;
            print!("{}", compact_guidance(&packed));
            Ok(())
        }
    }
}

/// Reads and parses the committed lockfile into a live [`HandleTable`].
/// REQ: REQ-TOOL-080.
///
/// # Errors
/// [`CliError::Io`] when the file is missing or unreadable (friendly message),
/// [`CliError::Lockfile`] when it is malformed or has an unknown
/// `format_version`.
pub fn load(path: &Path) -> Result<HandleTable, CliError> {
    let bytes = fs::read(path).map_err(|e| {
        CliError::Io(format!(
            "cannot read `{}` ({e}); is `{}` a DRM workspace with a committed lockfile? \
             (REQ-TOOL-080)",
            path.display(),
            path.parent()
                .and_then(Path::parent)
                .map_or_else(|| ".".to_string(), |p| p.display().to_string()),
        ))
    })?;
    parse_lockfile(&bytes).map_err(|m| CliError::Lockfile(format!("`{}`: {m}", path.display())))
}

/// Parses lockfile bytes into a [`HandleTable`]. The stored `next_free` is
/// preserved and never lowered below the derived (max live/reserved + 1) floor,
/// so a monotonic counter can never be accidentally rolled back.
/// REQ: REQ-TOOL-080, REQ-TOOL-036.
///
/// # Errors
/// A message describing the schema violation.
fn parse_lockfile(bytes: &[u8]) -> Result<HandleTable, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = value.as_object().ok_or("top level must be a JSON object")?;

    let fv = obj
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("missing or invalid `format_version`")?;
    if fv != FORMAT_VERSION {
        return Err(format!(
            "unsupported `format_version` {fv} (expected {FORMAT_VERSION}); refusing to guess"
        ));
    }

    let stored = obj
        .get("next_free")
        .and_then(serde_json::Value::as_u64)
        .ok_or("missing or invalid `next_free`")?;
    let stored = u32::try_from(stored).map_err(|_| format!("`next_free` {stored} exceeds u32"))?;

    let assignments = parse_assignments(obj.get("assignments"))?;
    let reserved = parse_reserved(obj.get("reserved"))?;

    let derived = HandleTable::from_history(&AssignmentHistory {
        assignments: assignments.clone(),
        reserved: reserved.clone(),
    })
    .next_free;

    Ok(HandleTable {
        assignments,
        reserved,
        next_free: stored.max(derived),
    })
}

/// Reads the `assignments` object: canonical topic path → handle.
/// REQ: REQ-TOOL-037.
fn parse_assignments(value: Option<&serde_json::Value>) -> Result<BTreeMap<String, u32>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let obj = value.as_object().ok_or("`assignments` must be an object")?;
    let mut out = BTreeMap::new();
    for (topic, handle) in obj {
        let n = handle
            .as_u64()
            .ok_or_else(|| format!("assignment `{topic}` must be a non-negative integer"))?;
        let n = u32::try_from(n).map_err(|_| format!("assignment `{topic}` = {n} exceeds u32"))?;
        out.insert(topic.clone(), n);
    }
    Ok(out)
}

/// Reads the `reserved` object: handle → `{ "reason": ... }`.
/// REQ: REQ-TOOL-038, REQ-TOOL-080.
fn parse_reserved(
    value: Option<&serde_json::Value>,
) -> Result<BTreeMap<u32, ReserveReason>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let obj = value.as_object().ok_or("`reserved` must be an object")?;
    let mut out = BTreeMap::new();
    for (key, entry) in obj {
        let id: u32 = key
            .parse()
            .map_err(|_| format!("reserved key `{key}` is not a u32"))?;
        let reason = entry
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("reserved `{key}` is missing a string `reason`"))?;
        let reason = match reason {
            "deleted" => ReserveReason::Deleted,
            "merge_collision" => ReserveReason::MergeCollision,
            other => return Err(format!("reserved `{key}` has unknown reason `{other}`")),
        };
        out.insert(id, reason);
    }
    Ok(out)
}

/// Deterministic human report: `next_free`, then assignments and reservations
/// in ascending key order (both are `BTreeMap`s, so the order is fixed).
/// REQ: REQ-TOOL-080.
#[must_use]
pub fn report(table: &HandleTable) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "format_version: {FORMAT_VERSION}");
    let _ = writeln!(out, "next_free: {}", table.next_free);
    out.push_str("assignments:\n");
    if table.assignments.is_empty() {
        out.push_str("  (none)\n");
    }
    for (topic, handle) in &table.assignments {
        let _ = writeln!(out, "  {topic} = {handle}");
    }
    out.push_str("reserved:\n");
    if table.reserved.is_empty() {
        out.push_str("  (none)\n");
    }
    for (handle, reason) in &table.reserved {
        let _ = writeln!(
            out,
            "  {handle} = {} ({})",
            reason_str(*reason),
            reason_desc(*reason)
        );
    }
    out
}

/// Canonical lockfile render (handle-system.md §Data Structures): `format_version`
/// first, then `next_free`, then `assignments`, then `reserved`; sorted keys;
/// 2-space indent; trailing newline. No timestamps, branch names, or usernames
/// ever appear (REQ-TOOL-080).
/// REQ: REQ-TOOL-080.
#[must_use]
pub fn render_lockfile(table: &HandleTable) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"format_version\": {FORMAT_VERSION},");
    let _ = writeln!(out, "  \"next_free\": {},", table.next_free);

    let assignments: Vec<String> = table
        .assignments
        .iter()
        .map(|(topic, handle)| format!("    \"{}\": {handle}", json_escape(topic)))
        .collect();
    out.push_str("  \"assignments\": ");
    if assignments.is_empty() {
        out.push_str("{},\n");
    } else {
        let _ = writeln!(out, "{{");
        out.push_str(&assignments.join(",\n"));
        out.push_str("\n  },\n");
    }

    let reserved: Vec<String> = table
        .reserved
        .iter()
        .map(|(handle, reason)| {
            format!(
                "    \"{handle}\": {{ \"reason\": \"{}\" }}",
                reason_str(*reason)
            )
        })
        .collect();
    out.push_str("  \"reserved\": ");
    if reserved.is_empty() {
        out.push_str("{}\n");
    } else {
        let _ = writeln!(out, "{{");
        out.push_str(&reserved.join(",\n"));
        out.push_str("\n  }\n");
    }
    out.push_str("}\n");
    out
}

/// Writes the lockfile back in canonical form, creating `.drm/` if needed.
/// REQ: REQ-TOOL-080.
///
/// # Errors
/// [`CliError::Io`] when the directory or file cannot be written.
pub fn write(path: &Path, table: &HandleTable) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| CliError::Io(format!("cannot create `{}` ({e})", parent.display())))?;
    }
    fs::write(path, render_lockfile(table))
        .map_err(|e| CliError::Io(format!("cannot write `{}` ({e})", path.display())))
}

/// Post-compaction operator guidance (I-H4): a handle change is a new
/// deployment identity requiring a coordinated rebuild/restart; schema bindings
/// are never regenerated (I-H7). Deterministic checklist output.
/// REQ: REQ-TOOL-038.
#[must_use]
pub fn compact_guidance(table: &HandleTable) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "compacted {} live assignment(s) to dense handles 0..{}; all reservations dropped (I-H4/REQ-TOOL-038).",
        table.assignments.len(),
        table.assignments.len().saturating_sub(1),
    );
    out.push_str("new deployment identity — coordinated redeploy required:\n");
    out.push_str(
        "  [ ] regenerate all handle-dependent deployment artifacts \
         (routing/runtime tables; MCU topic tables where compiled in)\n",
    );
    out.push_str("  [ ] rebuild + restart every node compiling a deployment handle table\n");
    out.push_str("  [ ] confirm every affected node restarted before traffic resumes\n");
    out.push_str("schema bindings are never regenerated (I-H7).\n");
    out
}

/// Lowercase wire form of a reservation reason. REQ: REQ-TOOL-080.
#[must_use]
fn reason_str(reason: ReserveReason) -> &'static str {
    match reason {
        ReserveReason::Deleted => "deleted",
        ReserveReason::MergeCollision => "merge_collision",
    }
}

/// Human description of a reservation reason. REQ: REQ-TOOL-080.
#[must_use]
fn reason_desc(reason: ReserveReason) -> &'static str {
    match reason {
        ReserveReason::Deleted => "tombstone",
        ReserveReason::MergeCollision => "merge collision",
    }
}

#[cfg(test)]
mod tests {
    use drm_workspace::model::handles::{HandleTable, ReserveReason};

    /// REQ: REQ-TOOL-080 — canonical order: `format_version`, `next_free`,
    /// `assignments`, `reserved`; sorted keys; no nondeterministic fields.
    #[test]
    fn render_is_canonical_and_sorted() {
        let mut t = HandleTable::default();
        t.assignments.insert("control/target".to_string(), 0);
        t.assignments.insert("planning/trajectory".to_string(), 1);
        t.reserved.insert(2, ReserveReason::Deleted);
        t.next_free = 3;
        let body = super::render_lockfile(&t);
        let expected = "{\n  \"format_version\": 1,\n  \"next_free\": 3,\n  \"assignments\": {\n    \"control/target\": 0,\n    \"planning/trajectory\": 1\n  },\n  \"reserved\": {\n    \"2\": { \"reason\": \"deleted\" }\n  }\n}\n";
        assert_eq!(body, expected);
    }

    /// REQ: REQ-TOOL-080 — parse → render round-trips byte-for-byte.
    #[test]
    fn lockfile_round_trips() {
        let src = "{\n  \"format_version\": 1,\n  \"next_free\": 4,\n  \"assignments\": {\n    \"a/t\": 0,\n    \"b/t\": 2\n  },\n  \"reserved\": {\n    \"3\": { \"reason\": \"merge_collision\" }\n  }\n}\n";
        let t = super::parse_lockfile(src.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(t.next_free, 4);
        assert_eq!(t.assignments["a/t"], 0);
        assert_eq!(t.reserved[&3], ReserveReason::MergeCollision);
        assert_eq!(super::render_lockfile(&t), src);
    }

    /// REQ: REQ-TOOL-036 — a stale stored `next_free` is raised to the derived
    /// floor, never lowered (no accidental reuse).
    #[test]
    fn next_free_never_lowered() {
        let src =
            "{\"format_version\":1,\"next_free\":0,\"assignments\":{\"a/t\":7},\"reserved\":{}}";
        let t = super::parse_lockfile(src.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(t.next_free, 8);
    }

    /// REQ: REQ-TOOL-080 — unknown `format_version` is refused.
    #[test]
    fn unknown_format_version_rejected() {
        let src = "{\"format_version\":2,\"next_free\":0,\"assignments\":{},\"reserved\":{}}";
        assert!(super::parse_lockfile(src.as_bytes()).is_err());
    }

    /// REQ: REQ-TOOL-038 — compaction re-packs dense in sorted order and drops
    /// reservations.
    #[test]
    fn compact_repack_dense() {
        let mut t = HandleTable::default();
        t.assignments.insert("c/d".to_string(), 9);
        t.assignments.insert("a/b".to_string(), 3);
        t.reserved.insert(2, ReserveReason::Deleted);
        t.next_free = 10;
        let packed = t.compact_repack();
        assert_eq!(packed.assignments["a/b"], 0);
        assert_eq!(packed.assignments["c/d"], 1);
        assert!(packed.reserved.is_empty());
        assert_eq!(packed.next_free, 2);
    }
}
