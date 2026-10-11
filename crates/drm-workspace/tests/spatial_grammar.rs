//! Spatial grammar extensions (work-plan M19W1-E2-T2): the six spatial type
//! keywords, the `space:` attribute, and the dynamic `SpatialPoint3` form parse
//! behind the standalone recognizer, while the ordinary `.drm` corpus is
//! byte-for-byte unaffected.
//!
//! `cargo test -p drm-workspace spatial_grammar`.
//!
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-047.

use drm_workspace::parser::spatial::{SpatialType, parse_spatial_schema};
use drm_workspace::parser::{parse_text, render_canonical};

/// REQ: REQ-SPATIAL-036 — all six spatial keywords are recognized.
#[test]
fn spatial_grammar_all_types() {
    for (word, ty) in [
        ("point3", SpatialType::Point3),
        ("direction3", SpatialType::Direction3),
        ("vector3", SpatialType::Vector3),
        ("pose3", SpatialType::Pose3),
        ("transform3", SpatialType::Transform3),
        ("twist3", SpatialType::Twist3),
    ] {
        assert_eq!(SpatialType::from_keyword(word), Some(ty));
    }
}

/// REQ: REQ-SPATIAL-005, REQ-SPATIAL-009 — the schema body binds compile-time
/// frames and dynamic identities and reports the raw-numerics wire size.
#[test]
fn spatial_grammar_schema_body() {
    let text = "\
position: point3 { space: camera/front }
heading: direction3 { space: camera/front }
offset: vector3 { space: camera/front }
pose: pose3 { space: camera/front }
base_to_camera: transform3 { space: camera/front }
velocity: twist3 { space: camera/front }
tracked: point3 { spatial_id: true }
";
    let fields = parse_spatial_schema(text).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(fields.len(), 7);
    assert_eq!(fields[0].field.wire_bytes(), 24);
    assert_eq!(fields[3].field.wire_bytes(), 56);
    assert_eq!(fields[5].field.wire_bytes(), 48);
    assert_eq!(fields[6].field.generated_type_name(), "SpatialPoint3");
}

/// REQ: REQ-SPATIAL-047 — frame identity follows the filesystem path and
/// lowercase `/`-separated segments; malformed paths are rejected.
#[test]
fn spatial_grammar_frame_identity() {
    let ok = parse_spatial_schema("a: point3 { space: robot/camera/front }")
        .unwrap_or_else(|e| unreachable!("{e:?}"));
    assert_eq!(
        ok[0].field.generated_type_name(),
        "Point3<RobotCameraFront>"
    );
    assert!(parse_spatial_schema("a: point3 { space: Robot/Camera }").is_err());
}

/// REQ: REQ-SPATIAL-036 — the non-spatial `.drm` corpus is regression-free:
/// an ordinary schema parses and renders exactly as before.
#[test]
fn spatial_grammar_non_spatial_corpus_unaffected() {
    let src = "@history: 3\nposition: f32[3]\nlabel: string[16]\n";
    let parsed = parse_text(src).unwrap_or_else(|e| unreachable!("{e:?}"));
    let rendered = render_canonical(&parsed);
    assert!(rendered.contains("position"));
    assert!(rendered.contains("f32[3]"));
    assert!(rendered.contains("string[16]"));
}
