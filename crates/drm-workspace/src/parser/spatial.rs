//! Phase 33 parser groundwork (feature `spatial-grammar`): spatial field-type
//! keywords, frame-path conformance, and static/dynamic binding classification
//! per `docs/spec/spatial-types-design.md` §3–§5. This module is a standalone
//! recognizer; the main `.drm` grammar is unchanged, so the Phase-2 corpus is
//! byte-for-byte unaffected (work-plan M15W1-E2-T1, non-breaking).
//! REQ: REQ-SPATIAL-036, REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-047.

/// The six distinct spatial semantic types (spatial-types-design.md §1).
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialType {
    /// Location in a space (`point3`).
    Point3,
    /// Unit vector (`direction3`).
    Direction3,
    /// Displacement (`vector3`).
    Vector3,
    /// Point + rotation (`pose3`).
    Pose3,
    /// `SE(3)` rigid transform (`transform3`).
    Transform3,
    /// Linear + angular velocity (`twist3`).
    Twist3,
}

impl SpatialType {
    /// Source keyword.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Point3 => "point3",
            Self::Direction3 => "direction3",
            Self::Vector3 => "vector3",
            Self::Pose3 => "pose3",
            Self::Transform3 => "transform3",
            Self::Twist3 => "twist3",
        }
    }

    /// Recognizes a spatial keyword.
    /// REQ: REQ-SPATIAL-036.
    #[must_use]
    pub fn from_keyword(word: &str) -> Option<Self> {
        match word {
            "point3" => Some(Self::Point3),
            "direction3" => Some(Self::Direction3),
            "vector3" => Some(Self::Vector3),
            "pose3" => Some(Self::Pose3),
            "transform3" => Some(Self::Transform3),
            "twist3" => Some(Self::Twist3),
            _ => None,
        }
    }
}

/// Compile-time vs dynamic frame binding (spatial-types-design.md §2).
/// REQ: REQ-SPATIAL-005, REQ-SPATIAL-009.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialBinding {
    /// `space: <frame-path>` — binds the frame into the generated type.
    Static {
        /// Lowercase `/`-separated frame path.
        frame: String,
    },
    /// `spatial_id: true` — dynamic object identity (`SpatialId`).
    Dynamic,
}

/// One recognized spatial field declaration.
/// REQ: REQ-SPATIAL-036.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialField {
    /// Semantic type.
    pub ty: SpatialType,
    /// Frame binding.
    pub binding: SpatialBinding,
}

/// Grammar diagnostics for the feature-gated recognizer.
/// REQ: REQ-SPATIAL-005, REQ-SPATIAL-047.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialGrammarError {
    /// The type keyword is not a spatial type.
    UnknownType(String),
    /// Neither `space:` nor `spatial_id:` was supplied (naked quantity).
    NakedQuantity,
    /// `space:` and `spatial_id:` were both supplied on one field.
    ConflictingBinding,
    /// A frame path violates path identity (lowercase ASCII, `/`-separated).
    BadFramePath(String),
}

/// Validates a frame path: lowercase ASCII segments joined by `/` (identity is
/// the filesystem path; spatial-types-design.md §3).
/// REQ: REQ-SPATIAL-047.
#[must_use]
pub fn frame_path_conforms(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    path.split('/').all(|seg| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

/// Classifies a spatial field from its type keyword and optional binding
/// (`space` frame path, or `spatial_id: true`).
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-005, REQ-SPATIAL-009.
///
/// # Errors
/// [`SpatialGrammarError`] for an unknown keyword, a naked quantity, a
/// conflicting binding, or a malformed frame path.
pub fn parse_spatial_field(
    ty_word: &str,
    space_frame: Option<&str>,
    spatial_id: bool,
) -> Result<SpatialField, SpatialGrammarError> {
    let ty = SpatialType::from_keyword(ty_word)
        .ok_or_else(|| SpatialGrammarError::UnknownType(ty_word.to_string()))?;
    let binding = match (space_frame, spatial_id) {
        (Some(_), true) => return Err(SpatialGrammarError::ConflictingBinding),
        (Some(frame), false) => {
            if !frame_path_conforms(frame) {
                return Err(SpatialGrammarError::BadFramePath(frame.to_string()));
            }
            SpatialBinding::Static {
                frame: frame.to_string(),
            }
        }
        (None, true) => SpatialBinding::Dynamic,
        (None, false) => return Err(SpatialGrammarError::NakedQuantity),
    };
    Ok(SpatialField { ty, binding })
}

#[cfg(test)]
mod tests {
    use super::{
        SpatialBinding, SpatialGrammarError, SpatialType, frame_path_conforms, parse_spatial_field,
    };

    /// REQ: REQ-SPATIAL-036 — every keyword round-trips through the recognizer.
    #[test]
    fn keywords_roundtrip() {
        for ty in [
            SpatialType::Point3,
            SpatialType::Direction3,
            SpatialType::Vector3,
            SpatialType::Pose3,
            SpatialType::Transform3,
            SpatialType::Twist3,
        ] {
            assert_eq!(SpatialType::from_keyword(ty.keyword()), Some(ty));
        }
        assert_eq!(SpatialType::from_keyword("point2"), None);
    }

    /// REQ: REQ-SPATIAL-047 — frame paths are lowercase `/`-separated segments.
    #[test]
    fn frame_paths() {
        assert!(frame_path_conforms("camera/front"));
        assert!(frame_path_conforms("robot/base_link"));
        assert!(!frame_path_conforms("Camera/Front"));
        assert!(!frame_path_conforms("camera//front"));
        assert!(!frame_path_conforms(""));
        assert!(!frame_path_conforms("camera/"));
    }

    /// REQ: REQ-SPATIAL-005, REQ-SPATIAL-009 — static and dynamic bindings
    /// classify; naked and conflicting declarations are rejected.
    #[test]
    fn field_classification() {
        let static_field = parse_spatial_field("point3", Some("camera/front"), false)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(
            static_field.binding,
            SpatialBinding::Static {
                frame: "camera/front".to_string()
            }
        );
        let dynamic =
            parse_spatial_field("pose3", None, true).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(dynamic.binding, SpatialBinding::Dynamic);
        assert_eq!(
            parse_spatial_field("point3", None, false),
            Err(SpatialGrammarError::NakedQuantity)
        );
        assert_eq!(
            parse_spatial_field("point3", Some("camera/front"), true),
            Err(SpatialGrammarError::ConflictingBinding)
        );
        assert!(matches!(
            parse_spatial_field("point3", Some("Bad/Path"), false),
            Err(SpatialGrammarError::BadFramePath(_))
        ));
        assert!(matches!(
            parse_spatial_field("quat", Some("camera/front"), false),
            Err(SpatialGrammarError::UnknownType(_))
        ));
    }
}
