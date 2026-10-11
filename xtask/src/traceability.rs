//! Traceability matrix generator (work-plan M23W1-E4-T1). Builds the
//! requirement → implementation → test matrix defined by
//! [qualification.md](../../docs/algorithms/program/qualification.md)
//! §Traceability (`I-QAL1`): every `docs/req/registry.csv` row is resolved to
//! its source citations (implementation artifacts) and to its test target (a
//! source path or a doc test-matrix anchor). Orphans — a requirement whose test
//! target does not resolve, or a test-bearing source file that cites no
//! requirement — are reported so the gate fails on them.
//! REQ: REQ-QAL-003, REQ-QAL-004, REQ-QAL-006.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Source roots scanned for `REQ` citations and test artifacts.
const SOURCE_ROOTS: [&str; 6] = ["crates", "xtask", "tests", "bindings", "apps", "packages"];

/// Source file extensions scanned for citations and test markers.
const SOURCE_EXTS: [&str; 7] = ["rs", "c", "h", "cpp", "hpp", "py", "ts"];

/// Directory names never descended into (build output, VCS, caches).
const SKIP_DIRS: [&str; 6] = [
    "target",
    ".git",
    "node_modules",
    "__pycache__",
    "dist",
    "build",
];

/// One parsed `docs/req/registry.csv` row (columns: `id, area, statement,
/// source_ref, phase, test, status`).
/// REQ: REQ-QAL-006.
struct RegistryRow {
    /// Requirement ID.
    id: String,
    /// Requirement area (`CORE | TOOL | ...`).
    area: String,
    /// One-line normative statement.
    statement: String,
    /// Planned verification owner (path or doc anchor list).
    test: String,
    /// Lifecycle status.
    status: String,
}

/// One matrix row: a requirement, its implementation artifacts, and the test
/// targets that resolve against the repository.
/// REQ: REQ-QAL-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRow {
    /// Requirement ID (`REQ-<AREA>-<NNN>`).
    pub id: String,
    /// Requirement area.
    pub area: String,
    /// Normative statement (REQ text).
    pub statement: String,
    /// Registry status.
    pub status: String,
    /// Source files citing the requirement (implementation artifacts).
    pub impl_artifacts: Vec<String>,
    /// Test targets that resolve to an existing path or doc anchor.
    pub test_targets: Vec<String>,
}

/// A traceability gap reported by [`Matrix`] (`I-QAL1` orphan).
/// REQ: REQ-QAL-004.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Orphan {
    /// A registry requirement whose `test` column resolves to no target.
    ReqWithoutTest(String),
    /// A test-bearing source file that cites no requirement.
    TestWithoutReq(String),
}

/// Generated traceability matrix (`I-QAL1`).
/// REQ: REQ-QAL-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    /// One row per registry requirement, in registry order.
    pub rows: Vec<TraceRow>,
    /// Requirements with no resolvable test target and test files with no REQ.
    pub orphans: Vec<Orphan>,
    /// Requirements cited in no source file (implementation leg unmapped).
    pub impl_unmapped: Vec<String>,
}

impl Matrix {
    /// Number of rows whose test target resolves.
    /// REQ: REQ-QAL-006.
    #[must_use]
    pub fn mapped(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| !r.test_targets.is_empty())
            .count()
    }

    /// Test-mapping coverage in whole percent (`100` when the registry is empty).
    /// REQ: REQ-QAL-006.
    #[must_use]
    pub fn coverage_percent(&self) -> usize {
        if self.rows.is_empty() {
            return 100;
        }
        self.mapped() * 100 / self.rows.len()
    }

    /// `I-QAL1` gate: 100% test mapping and zero orphans.
    /// REQ: REQ-QAL-006.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.coverage_percent() == 100 && self.orphans.is_empty()
    }

    /// Renders the matrix as a human-readable report (rows, orphans, summary).
    /// REQ: REQ-QAL-006.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "traceability matrix: {} rows, {} mapped, {}% coverage, {} orphans",
            self.rows.len(),
            self.mapped(),
            self.coverage_percent(),
            self.orphans.len()
        );
        for row in &self.rows {
            let impl_count = row.impl_artifacts.len();
            let tests = if row.test_targets.is_empty() {
                "-".to_string()
            } else {
                row.test_targets.join("; ")
            };
            let _ = writeln!(
                out,
                "{} | {} | {} | impl={} | test={}",
                row.id, row.area, row.status, impl_count, tests
            );
        }
        for orphan in &self.orphans {
            let _ = writeln!(out, "ORPHAN | {}", orphan.render());
        }
        let _ = writeln!(
            out,
            "implementation leg unmapped (scheduled): {}",
            self.impl_unmapped.len()
        );
        out
    }
}

impl Orphan {
    /// One-line rendering of the gap.
    /// REQ: REQ-QAL-004.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::ReqWithoutTest(id) => format!("{id}: no resolvable test target"),
            Self::TestWithoutReq(file) => format!("{file}: test cites no requirement"),
        }
    }
}

/// Repository root: the `xtask` crate lives at `<root>/xtask`.
/// REQ: REQ-QAL-006.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// REQ: REQ-QAL-006 — `cargo xtask audit-traceability --matrix`: generate and
/// print the matrix, then fail when coverage is below 100% or an orphan exists.
///
/// # Errors
/// [`anyhow::Error`] when the registry is unreadable or malformed, or when the
/// generated matrix is incomplete (`I-QAL1`).
pub fn run() -> Result<()> {
    let matrix = generate(&repo_root())?;
    print!("{}", matrix.render());
    if matrix.is_complete() {
        println!(
            "traceability: 100% coverage, 0 orphans (I-QAL1); {} implementation rows scheduled",
            matrix.impl_unmapped.len()
        );
        Ok(())
    } else {
        bail!(
            "traceability incomplete (I-QAL1): {}% coverage, {} orphans",
            matrix.coverage_percent(),
            matrix.orphans.len()
        );
    }
}

/// Builds the traceability matrix for the repository rooted at `root`.
/// REQ: REQ-QAL-003, REQ-QAL-004, REQ-QAL-006.
///
/// # Errors
/// [`anyhow::Error`] when the registry cannot be read or a scanned source file
/// cannot be read.
pub fn generate(root: &Path) -> Result<Matrix> {
    let registry = parse_registry(&root.join("docs/req/registry.csv"))?;
    let citations = source_citations(root)?;
    let mut rows = Vec::with_capacity(registry.len());
    let mut orphans = Vec::new();
    let mut impl_unmapped = Vec::new();
    for row in registry {
        let impl_artifacts: Vec<String> = citations
            .get(&row.id)
            .map(|files| files.iter().cloned().collect())
            .unwrap_or_default();
        if impl_artifacts.is_empty() {
            impl_unmapped.push(row.id.clone());
        }
        let test_targets = resolve_targets(root, &row.test);
        if test_targets.is_empty() {
            orphans.push(Orphan::ReqWithoutTest(row.id.clone()));
        }
        rows.push(TraceRow {
            id: row.id,
            area: row.area,
            statement: row.statement,
            status: row.status,
            impl_artifacts,
            test_targets,
        });
    }
    for file in test_without_req(root)? {
        orphans.push(Orphan::TestWithoutReq(file));
    }
    Ok(Matrix {
        rows,
        orphans,
        impl_unmapped,
    })
}

/// Parses `registry.csv` into rows, tolerating embedded commas via the shared
/// quote-aware splitter.
/// REQ: REQ-QAL-006.
fn parse_registry(path: &Path) -> Result<Vec<RegistryRow>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut rows = Vec::new();
    for (index, line) in raw.lines().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let fields = crate::layering::split_csv(line);
        let [id, area, statement, _source, _phase, test, status] = fields.as_slice() else {
            bail!(
                "{}:{}: expected 7 columns, got {}",
                path.display(),
                index + 1,
                fields.len()
            );
        };
        rows.push(RegistryRow {
            id: id.clone(),
            area: area.clone(),
            statement: statement.clone(),
            test: test.clone(),
            status: status.clone(),
        });
    }
    Ok(rows)
}

/// Resolves every `;`-separated target in a registry `test` column; a target is
/// kept only when its file exists (and, for `file#anchor`, the anchor heading
/// exists).
/// REQ: REQ-QAL-006.
fn resolve_targets(root: &Path, test: &str) -> Vec<String> {
    let mut resolved = Vec::new();
    for part in test.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((file, anchor)) = part.split_once('#') {
            let path = root.join(file.trim());
            if path.is_file() && doc_anchor_exists(&path, anchor.trim()) {
                resolved.push(part.to_string());
            }
        } else if Path::new(part)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            if root.join(part).is_file() {
                resolved.push(part.to_string());
            }
        } else if looks_like_path(part) {
            let file = part.split("::").next().unwrap_or(part).trim();
            if root.join(file).exists() {
                resolved.push(part.to_string());
            }
        }
    }
    resolved
}

/// Whether a token looks like a repository-relative source path (so free-text
/// evidence notes are never mistaken for missing files).
/// REQ: REQ-QAL-006.
fn looks_like_path(token: &str) -> bool {
    const PREFIXES: [&str; 7] = [
        "crates/",
        "tests/",
        "xtask/",
        "bindings/",
        "apps/",
        "packages/",
        "docs/",
    ];
    PREFIXES.iter().any(|prefix| token.starts_with(prefix))
}

/// Whether `anchor` (GitHub heading slug) names a heading in the doc at `path`.
/// REQ: REQ-QAL-006.
fn doc_anchor_exists(path: &Path, anchor: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines()
        .filter_map(|line| line.strip_prefix('#'))
        .map(|heading| slug(heading.trim_start_matches('#').trim()))
        .any(|slug| slug == anchor)
}

/// GitHub-style heading slug: lower-case, spaces to hyphens, drop punctuation.
/// REQ: REQ-QAL-006.
fn slug(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    for ch in heading.chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    out
}

/// Maps every cited `REQ` ID to the source files that cite it.
/// REQ: REQ-QAL-006.
fn source_citations(root: &Path) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in source_files(root) {
        let text =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        let relative = relative_path(root, &file);
        for id in req_tokens(&text) {
            map.entry(id).or_default().insert(relative.clone());
        }
    }
    Ok(map)
}

/// Source files that carry a test marker but cite no requirement.
/// REQ: REQ-QAL-004.
fn test_without_req(root: &Path) -> Result<Vec<String>> {
    let mut offenders = Vec::new();
    for file in source_files(root) {
        let text =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        if has_test_marker(&text) && req_tokens(&text).is_empty() {
            offenders.push(relative_path(root, &file));
        }
    }
    offenders.sort();
    Ok(offenders)
}

/// Whether `text` contains a Rust test marker.
/// REQ: REQ-QAL-004.
fn has_test_marker(text: &str) -> bool {
    text.contains("#[test]") || text.contains("#[cfg(test)]") || text.contains("::test]")
}

/// All scanned source files under the source roots, sorted for determinism.
/// REQ: REQ-QAL-006.
fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = SOURCE_ROOTS
        .iter()
        .map(|dir| root.join(dir))
        .filter(|dir| dir.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                if !SKIP_DIRS.iter().any(|skip| name == *skip) {
                    stack.push(path);
                }
            } else if is_source(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Whether `path` has a scanned source extension.
/// REQ: REQ-QAL-006.
fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SOURCE_EXTS.contains(&ext))
}

/// Repository-relative, forward-slash display path.
/// REQ: REQ-QAL-006.
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Extracts valid `REQ-<AREA>-<NNN>` tokens from arbitrary text.
/// REQ: REQ-QAL-006.
fn req_tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] == b"REQ-" {
            let mut end = index + 4;
            while end < bytes.len() {
                let byte = bytes[end];
                if byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-' {
                    end += 1;
                } else {
                    break;
                }
            }
            let token = &text[index..end];
            if valid_req_token(token) {
                out.insert(token.to_string());
            }
            index = end;
        } else {
            index += 1;
        }
    }
    out
}

/// Whether `token` is exactly `REQ-<LETTERS>-<DIGITS>`.
/// REQ: REQ-QAL-006.
fn valid_req_token(token: &str) -> bool {
    let mut parts = token.split('-');
    let (Some("REQ"), Some(area), Some(number), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !area.is_empty()
        && area.chars().all(|c| c.is_ascii_uppercase())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// REQ: REQ-QAL-003, REQ-QAL-004, REQ-QAL-006 — token extraction, target
/// resolution, orphan detection, and the live repository matrix.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Orphan, generate, has_test_marker, relative_path, req_tokens, resolve_targets, slug,
        test_without_req, valid_req_token,
    };

    /// Creates a fresh temp directory for one test.
    /// REQ: REQ-QAL-006.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("drm-trace-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("tmpdir: {e}"));
        dir
    }

    /// Writes `contents` to `root/rel`, creating parent directories.
    /// REQ: REQ-QAL-006.
    fn write(root: &std::path::Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| unreachable!("mkdir: {e}"));
        }
        std::fs::write(&path, contents).unwrap_or_else(|e| unreachable!("write {rel}: {e}"));
    }

    /// REQ: REQ-QAL-006 — heading slugs match GitHub anchors, including
    /// punctuation and multi-word headings.
    #[test]
    fn heading_slug_matches_anchor() {
        assert_eq!(slug("Test Matrix"), "test-matrix");
        assert_eq!(slug("REQ Bindings"), "req-bindings");
        assert_eq!(
            slug("Soak protocol (three ledgers)"),
            "soak-protocol-three-ledgers"
        );
    }

    /// REQ: REQ-QAL-006 — only well-formed `REQ-AREA-NNN` tokens are extracted;
    /// placeholders, malformed, and lower-case variants are ignored.
    #[test]
    fn req_token_extraction() {
        let text = "REQ-CORE-042 and REQ-TOOL-007; not REQ-<AREA>-<NNN>, req-core-1, REQ-CORE-";
        let tokens = req_tokens(text);
        assert!(tokens.contains("REQ-CORE-042"));
        assert!(tokens.contains("REQ-TOOL-007"));
        assert_eq!(tokens.len(), 2, "only valid ids: {tokens:?}");
        assert!(valid_req_token("REQ-QAL-006"));
        for bad in ["REQ-CORE-", "REQ--1", "REQ-core-1", "REQ-CORE-0x"] {
            assert!(!valid_req_token(bad), "must reject {bad}");
        }
    }

    /// REQ: REQ-QAL-006 — a target resolves only when its file exists, and a
    /// `file#anchor` target only when the anchor heading exists.
    #[test]
    fn target_resolution_checks_existence_and_anchor() {
        let root = temp_dir("resolve");
        write(
            &root,
            "docs/algorithms/x.md",
            "# X\n\n## Test Matrix\n\nbody\n",
        );
        write(&root, "crates/a/src/lib.rs", "// REQ: REQ-QAL-006\n");
        let targets = resolve_targets(
            &root,
            "docs/algorithms/x.md#test-matrix; docs/algorithms/x.md#missing; crates/a/src/lib.rs; not a path",
        );
        assert_eq!(
            targets,
            vec![
                "docs/algorithms/x.md#test-matrix".to_string(),
                "crates/a/src/lib.rs".to_string(),
            ],
            "only existing targets resolve"
        );
        std::fs::remove_dir_all(&root).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-QAL-004 — a test-bearing file without a REQ citation is an
    /// orphan; the same file with a citation is not.
    #[test]
    fn detects_test_without_req() {
        let root = temp_dir("orphan");
        write(
            &root,
            "crates/a/src/lib.rs",
            "// REQ: REQ-QAL-006\n#[cfg(test)]\nmod t {}\n",
        );
        write(&root, "crates/a/tests/b.rs", "#[test]\nfn t() {}\n");
        let offenders = test_without_req(&root).unwrap_or_else(|e| unreachable!("scan: {e}"));
        assert_eq!(offenders, vec!["crates/a/tests/b.rs".to_string()]);
        std::fs::remove_dir_all(&root).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-QAL-004 — the test marker detector covers `#[test]`,
    /// `#[cfg(test)]`, and attribute-path variants.
    #[test]
    fn test_marker_detection() {
        assert!(has_test_marker("#[test]\n"));
        assert!(has_test_marker("#[cfg(test)]\n"));
        assert!(has_test_marker("#[tokio::test]\n"));
        assert!(!has_test_marker("fn main() {}\n"));
    }

    /// REQ: REQ-QAL-003, REQ-QAL-004, REQ-QAL-006 — an end-to-end synthetic
    /// repository flags a requirement without a test and a test without a
    /// requirement, and clears once both are mapped.
    #[test]
    fn matrix_reports_and_clears_orphans() {
        let root = temp_dir("matrix");
        write(&root, "docs/algorithms/x.md", "# X\n\n## Test Matrix\n");
        write(
            &root,
            "docs/req/registry.csv",
            "id,area,statement,source_ref,phase,test,status\n\
             REQ-QAL-001,QAL,covered,src,qal,docs/algorithms/x.md#test-matrix,specified\n\
             REQ-QAL-002,QAL,orphan,src,qal,docs/algorithms/x.md#missing,specified\n",
        );
        write(&root, "crates/a/src/lib.rs", "// REQ: REQ-QAL-001\n");
        write(&root, "crates/a/tests/loose.rs", "#[test]\nfn t() {}\n");
        let matrix = generate(&root).unwrap_or_else(|e| unreachable!("generate: {e}"));
        assert_eq!(matrix.rows.len(), 2);
        assert_eq!(matrix.mapped(), 1);
        assert_eq!(matrix.coverage_percent(), 50);
        assert!(!matrix.is_complete());
        assert!(
            matrix
                .orphans
                .contains(&Orphan::ReqWithoutTest("REQ-QAL-002".to_string()))
        );
        assert!(matrix.orphans.contains(&Orphan::TestWithoutReq(
            "crates/a/tests/loose.rs".to_string()
        )));
        assert_eq!(matrix.impl_unmapped, vec!["REQ-QAL-002".to_string()]);
        std::fs::remove_dir_all(&root).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }

    /// REQ: REQ-QAL-006 — the live repository registry is 100% test-mapped with
    /// zero orphans (the M23W1 gate criterion).
    #[test]
    fn repository_matrix_is_complete() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
        let matrix = generate(&root).unwrap_or_else(|e| unreachable!("generate: {e}"));
        assert!(
            matrix.is_complete(),
            "coverage {}%: orphans {:?}",
            matrix.coverage_percent(),
            matrix
                .orphans
                .iter()
                .map(Orphan::render)
                .collect::<Vec<_>>()
        );
        assert!(!matrix.rows.is_empty(), "registry must not be empty");
        assert_eq!(
            relative_path(&root, &root.join("docs/req/registry.csv")),
            "docs/req/registry.csv"
        );
    }
}
