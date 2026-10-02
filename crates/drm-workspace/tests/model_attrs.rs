//! Model attribute extraction tests (work-plan M02W2-E2-T3): `@history`,
//! `@pool_size`, `@overflow` carry into `TopicAttrs`; grammar rejections
//! stay parser-side. REQ: REQ-TOOL-031, REQ-TOOL-013.

use drm_workspace::model::{PackageInput, TreeInput, attrs_of, build};
use drm_workspace::parser::parse_bytes;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// REQ: REQ-TOOL-031 — all attribute keys normalize into typed model data.
#[test]
fn model_attrs_all_keys() {
    let schema = "@history: 64\n@pool_size: 128\n@overflow: reject\nv: u32\n";
    let f = fixture(parse_bytes(schema.as_bytes()), "parse");
    let a = attrs_of(&f);
    assert_eq!(a.history_depth, Some(64));
    assert!(!a.history_latest);
    assert_eq!(a.pool_size, Some(128));
    assert!(!a.overflow_overwrite);
    assert!(a.overflow_reject);
}

/// REQ: REQ-TOOL-031 — `@history: latest` and `@overflow: overwrite`.
#[test]
fn model_attrs_latest_overwrite() {
    let schema = "@history: latest\n@overflow: overwrite\nv: u32\n";
    let f = fixture(parse_bytes(schema.as_bytes()), "parse");
    let a = attrs_of(&f);
    assert!(a.history_latest);
    assert_eq!(a.history_depth, None);
    assert!(a.overflow_overwrite);
    assert!(!a.overflow_reject);
}

/// REQ: REQ-TOOL-031 — a topic without attributes yields defaults.
#[test]
fn model_attrs_absent() {
    let f = fixture(parse_bytes(b"v: u32\n"), "parse");
    let a = attrs_of(&f);
    assert_eq!(a.history_depth, None);
    assert!(!a.history_latest);
    assert_eq!(a.pool_size, None);
    assert!(!a.overflow_overwrite);
    assert!(!a.overflow_reject);
}

/// REQ: REQ-TOOL-031 — build carries normalized attrs per topic (model-level,
/// corpus-style fixture tree).
#[test]
fn model_attrs_through_build() {
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics: vec![
                (
                    "bounded".to_string(),
                    "@history: 32\n@pool_size: 8\n@overflow: reject\nv: u32\n"
                        .to_string()
                        .into_bytes(),
                ),
                (
                    "latest".to_string(),
                    "@history: latest\nv: u32\n".to_string().into_bytes(),
                ),
            ],
        }],
    };
    let m = fixture(build(&tree), "build");
    let bounded = &m.topics["pkg/bounded"];
    assert_eq!(bounded.attrs.history_depth, Some(32));
    assert_eq!(bounded.attrs.pool_size, Some(8));
    assert!(bounded.attrs.overflow_reject);
    let latest = &m.topics["pkg/latest"];
    assert!(latest.attrs.history_latest);
    assert_eq!(latest.attrs.history_depth, None);
    assert_eq!(latest.attrs.pool_size, None);
}
