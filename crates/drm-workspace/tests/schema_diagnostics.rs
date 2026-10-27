//! Schema rejection must say *why*: one diagnostic per real defect, and the
//! reason visible in the error a build prints
//! (`docs/spec/DIAGNOSTICS.md`, `REQ-TOOL-017`).
//!
//! Both halves of this were broken and are easy to break again:
//! - a cascading `empty-file` was appended whenever a defect left the AST empty,
//!   so a single real defect counted as two;
//! - `ModelError::Parse` rendered only `"{n} defect(s)"`, dropping the reasons,
//!   so an author saw a number and no cause.
//!
//! REQ: REQ-TOOL-017, REQ-TOOL-052, REQ-TOOL-053.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use drm_workspace::model::{ModelError, PackageInput, TreeInput, build};
use drm_workspace::parser::{Reason, parse_text};

/// Every rejected source reports exactly one defect, with its own reason.
///
/// REQ: REQ-TOOL-017 — no cascading diagnostic.
#[test]
fn one_defect_reports_one_reason() {
    // `(source, expected reason)` — each is a single, isolated defect.
    let cases: &[(&str, &str)] = &[
        // A message-typed field: DRM has no record type.
        ("t: Transform[<=16]\n", "unknown-type"),
        // Array of strings: arrays exist only over prims, depth <= 1.
        ("names: string[64][<=32]\n", "array-depth-exceeded"),
        // Unbounded string on the data plane.
        ("name: string\n", "unbounded-string"),
        // Unbounded array on the data plane.
        ("v: f32[]\n", "unbounded-array"),
        // Two-dimensional array.
        ("m: f32[4][<=8]\n", "array-depth-exceeded"),
    ];
    for (src, expected) in cases {
        let Err(diags) = parse_text(src) else {
            unreachable!("{src:?} must be rejected")
        };
        let reasons: Vec<&str> = diags.iter().map(|d| d.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![*expected],
            "{src:?} must report exactly its own defect, got {reasons:?}"
        );
        // The cascading diagnostic must never appear next to a real one.
        assert!(
            !diags.iter().any(|d| d.reason == Reason::EmptyFile),
            "{src:?} reported a spurious empty-file: {reasons:?}"
        );
        // The reason is positioned: a diagnostic without a span is unusable.
        let span = diags[0].span;
        assert!(
            span.line >= 1 && span.col >= 1,
            "{src:?}: diagnostic span is 1-based"
        );
    }
}

/// A source that declares nothing is still `empty-file` — and only that.
///
/// REQ: REQ-TOOL-017.
#[test]
fn empty_source_still_reports_empty_file() {
    for src in ["", "\n", "# only a comment\n"] {
        let Err(diags) = parse_text(src) else {
            unreachable!("{src:?} must be rejected")
        };
        let reasons: Vec<&str> = diags.iter().map(|d| d.reason.as_str()).collect();
        assert_eq!(reasons, vec!["empty-file"], "{src:?}");
    }
}

/// The error a build prints carries the reason and its position, not a count.
///
/// REQ: REQ-TOOL-017 — the diagnostic text is what an author acts on.
#[test]
fn model_error_renders_the_reason() {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "control".to_string(),
            topics: vec![("target".to_string(), b"t: Transform[<=16]\n".to_vec())],
        }],
        nodes: Vec::new(),
        topology: None,
    };
    let Err(errs) = build(&tree) else {
        unreachable!("an inexpressible topic must be rejected")
    };
    let text = errs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("unknown-type"),
        "the error must name the reason, got: {text}"
    );
    assert!(
        text.contains("1:4"),
        "the error must carry the position, got: {text}"
    );
    // And the count must be the real one, not inflated by a cascade.
    let parse = errs
        .iter()
        .find(|e| matches!(e, ModelError::Parse { .. }))
        .unwrap_or_else(|| unreachable!("expected a Parse error, got {text}"));
    assert!(
        parse
            .to_string()
            .starts_with("topic `control/target`: 1 defect(s)"),
        "one defect must count as one, got: {parse}"
    );
}
