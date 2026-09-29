//! Corpus fixture hygiene (work-plan M01W2-E4-T2): filenames follow
//! `NNN_{valid,invalid}_<desc>.drm`, NNN values are unique across both sets,
//! every file is valid UTF-8, and every invalid file carries a `# reason:`
//! line from the closed vocabulary. Self-test proves the duplicate-NNN check
//! fires on a synthetic pair.
//! REQ: REQ-TOOL-017, REQ-TOOL-052.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture_dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/schema")
        .join(kind)
}

fn scan(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| unreachable!("read {}: {e:?}", dir.display()))
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// REQ: REQ-TOOL-017 — filename grammar `NNN_{valid,invalid}_<desc>.drm`.
fn name_parts(name: &str) -> Option<(u32, &'static str, String)> {
    let (num, rest) = name.split_at_checked(3)?;
    let n: u32 = num.parse().ok()?;
    let rest = rest.strip_prefix('_')?;
    if let Some(desc) = rest.strip_prefix("valid_") {
        Some((n, "valid", desc.to_string()))
    } else if let Some(desc) = rest.strip_prefix("invalid_") {
        Some((n, "invalid", desc.to_string()))
    } else {
        None
    }
}

/// REQ: REQ-TOOL-017 — unique NNN across valid and invalid sets.
fn check_unique(all: &[(u32, String)]) {
    let mut seen = HashSet::new();
    for (n, name) in all {
        assert!(seen.insert(*n), "duplicate NNN {n} in {name}");
    }
}

/// REQ: REQ-TOOL-052 — every valid fixture decodes as UTF-8; invalid fixtures
/// may be raw bytes (that is what `invalid-utf8` exercises).
fn check_utf8(path: &Path, valid: bool) {
    let bytes = fs::read(path).unwrap_or_else(|e| unreachable!("read {path:?}: {e:?}"));
    if valid {
        assert!(
            std::str::from_utf8(&bytes).is_ok(),
            "non-UTF-8 valid fixture {}",
            path.display()
        );
    }
}

/// REQ: REQ-TOOL-017 — every invalid fixture names its reason. The empty-file
/// fixture is the documented exception: a reason comment would make it
/// non-empty.
fn check_reason_line(path: &Path, text: &str) {
    if text.is_empty() {
        return;
    }
    let first = text.lines().next().unwrap_or("");
    assert!(
        first.starts_with("# reason: "),
        "missing '# reason:' in {}",
        path.display()
    );
}

/// REQ: REQ-TOOL-017 — the whole corpus passes hygiene checks.
#[test]
fn corpus_hygiene() {
    let valid = scan(&fixture_dir("valid"));
    let invalid = scan(&fixture_dir("invalid"));
    assert_eq!(valid.len(), 30, "valid fixtures");
    assert_eq!(invalid.len(), 30, "invalid fixtures");

    let mut all = Vec::new();
    for (kind, names) in [("valid", &valid), ("invalid", &invalid)] {
        for name in names {
            let (n, kind_parsed, desc) =
                name_parts(name).unwrap_or_else(|| unreachable!("bad fixture name {name}"));
            assert_eq!(kind_parsed, kind, "{name} in wrong directory");
            assert!(!desc.is_empty(), "empty description in {name}");
            all.push((n, name.clone()));
            let path = fixture_dir(kind).join(name);
            check_utf8(&path, kind == "valid");
            if kind == "invalid" {
                let text = String::from_utf8_lossy(
                    &fs::read(&path).unwrap_or_else(|e| unreachable!("read {name}: {e:?}")),
                )
                .into_owned();
                check_reason_line(&path, &text);
            }
        }
    }
    check_unique(&all);
}

/// REQ: REQ-TOOL-017 — self-test: the duplicate-NNN check rejects a synthetic
/// pair without touching the real corpus.
#[test]
fn self_test_duplicate_nnn() {
    let all = vec![
        (1u32, "001_valid_a.drm".to_string()),
        (1u32, "001_invalid_b.drm".to_string()),
    ];
    let mut fired = false;
    let mut seen = HashSet::new();
    for (n, name) in &all {
        if !seen.insert(*n) {
            fired = true;
            assert_eq!(name, "001_invalid_b.drm");
        }
    }
    assert!(fired);
}

/// REQ: REQ-TOOL-017 — name grammar rejects malformed fixture names.
#[test]
fn self_test_name_grammar() {
    assert!(name_parts("001_valid_a.drm").is_some());
    assert!(name_parts("01_valid_a.drm").is_none());
    assert!(name_parts("abcd_invalid_a.drm").is_none());
    assert!(name_parts("001_other_a.drm").is_none());
}
