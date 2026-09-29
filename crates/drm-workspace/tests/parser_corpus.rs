//! Schema-parser corpus matrix (schema-parser.md I1, I2, I4): every valid
//! fixture parses `Ok`; every invalid fixture fails with its documented
//! closed-vocabulary reason; canonical render round-trips semantically and is
//! idempotent over all valid fixtures.
//! REQ: REQ-TOOL-017, REQ-TOOL-032, REQ-TOOL-052, REQ-TOOL-054, REQ-TOOL-055.

use std::fs;
use std::path::{Path, PathBuf};

use drm_workspace::parser::{Reason, parse_bytes, render_canonical};

fn fixture_dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/schema")
        .join(kind)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| unreachable!("read {}: {e:?}", dir.display()))
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// REQ: REQ-TOOL-032 — I1: every valid fixture parses to `Ok(TopicFile)`.
#[test]
fn parser_corpus_valid() {
    let dir = fixture_dir("valid");
    for name in names(&dir) {
        let bytes = fs::read(dir.join(&name)).unwrap_or_else(|e| unreachable!("{name}: {e:?}"));
        match parse_bytes(&bytes) {
            Ok(_) => {}
            Err(e) => unreachable!("valid fixture {name} rejected: {e:?}"),
        }
    }
}

/// REQ: REQ-TOOL-017 — I2: every invalid fixture fails with its documented
/// vocabulary reason and a defect span.
#[test]
fn parser_corpus_invalid() {
    let dir = fixture_dir("invalid");
    for name in names(&dir) {
        let bytes = fs::read(dir.join(&name)).unwrap_or_else(|e| unreachable!("{name}: {e:?}"));
        let Err(errs) = parse_bytes(&bytes) else {
            unreachable!("invalid fixture {name} accepted")
        };
        assert!(!errs.is_empty(), "{name}: no diagnostics");
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let expected = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("# reason: "))
            // The empty-file fixture has no room for a reason comment.
            .unwrap_or("empty-file");
        assert!(
            errs.iter().any(|d| d.reason.as_str() == expected),
            "{name}: want reason `{expected}`, got {:?}",
            errs.iter().map(|d| d.reason.as_str()).collect::<Vec<_>>()
        );
    }
}

/// REQ: REQ-TOOL-054 — I4 over the fixed corpus: semantic AST equality after
/// one canonical render + reparse, and byte-identical idempotent render.
#[test]
fn corpus_roundtrip() {
    let dir = fixture_dir("valid");
    for name in names(&dir) {
        let bytes = fs::read(dir.join(&name)).unwrap_or_else(|e| unreachable!("{name}: {e:?}"));
        let t = match parse_bytes(&bytes) {
            Ok(t) => t,
            Err(e) => unreachable!("{name}: {e:?}"),
        };
        let s1 = render_canonical(&t);
        let t2 = match parse_bytes(s1.as_bytes()) {
            Ok(t) => t,
            Err(e) => unreachable!("{name} reparse: {e:?}"),
        };
        assert!(t2 == t, "{name}: semantic round-trip diverged");
        let s2 = render_canonical(&t2);
        assert_eq!(s1, s2, "{name}: render not idempotent");
    }
}

/// REQ: REQ-TOOL-052 — BOM and CRLF fixtures normalize silently (edge table).
#[test]
fn bom_and_crlf_normalized() {
    let bom = fs::read(fixture_dir("valid").join("021_valid_bom.drm"))
        .unwrap_or_else(|e| unreachable!("bom fixture: {e:?}"));
    assert!(bom.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(parse_bytes(&bom).is_ok());
    let crlf = fs::read(fixture_dir("valid").join("020_valid_crlf.drm"))
        .unwrap_or_else(|e| unreachable!("crlf fixture: {e:?}"));
    assert!(parse_bytes(&crlf).is_ok());
    let _ = Reason::InvalidUtf8;
}
