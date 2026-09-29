//! Fuzz-seed regression (schema-parser.md I3): every corpus seed yields `Ok`
//! or `Err` — never panic, never hang. Runs on every platform so the seed set
//! stays a permanent gate even though libFuzzer itself is Linux-only.
//! REQ: REQ-TOOL-053.

use std::fs;
use std::path::{Path, PathBuf};

fn seed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fuzz/corpus/schema")
}

/// REQ: REQ-TOOL-053 — all seeds parse without panicking; valid prefixes parse.
#[test]
fn fuzz_seeds_no_panic() {
    let dir = seed_dir();
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| unreachable!("read {}: {e:?}", dir.display()))
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 30, "seed count {}", names.len());
    for name in &names {
        let bytes = fs::read(dir.join(name)).unwrap_or_else(|e| unreachable!("{name}: {e:?}"));
        // Must return — panic or hang fails the test.
        let _ = drm_workspace::parser::parse_bytes(&bytes);
    }
}

/// REQ: REQ-TOOL-053 — valid seeds still parse after corpus churn (guards
/// against seeds drifting into garbage that never exercises the accept path).
#[test]
fn fuzz_seeds_valid_prefix_accepted() {
    let dir = seed_dir();
    for name in ["21_valid_prims", "22_valid_full_attrs", "32_enum_256_valid"] {
        let bytes = fs::read(dir.join(name)).unwrap_or_else(|e| unreachable!("{name}: {e:?}"));
        let Ok(_) = drm_workspace::parser::parse_bytes(&bytes) else {
            unreachable!("valid seed {name} rejected")
        };
    }
}
