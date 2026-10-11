//! Phase 33 spatial grammar extensions: spatial field-type keywords, the
//! `space:` compile-time binding, the dynamic `SpatialPoint3` form, frame-path
//! conformance, and static/dynamic binding classification per
//! `docs/spec/spatial-types-design.md` §3–§5. The recognizer is additive — the
//! main `.drm` grammar is unchanged, so the Phase-2 corpus is byte-for-byte
//! unaffected (work-plan M15W1-E2-T1 groundwork; M19W1-E2-T2 extensions).
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

    /// Generated type name for a compile-time frame binding (`Point3`, …).
    /// REQ: REQ-SPATIAL-005.
    #[must_use]
    pub const fn generated_name(self) -> &'static str {
        match self {
            Self::Point3 => "Point3",
            Self::Direction3 => "Direction3",
            Self::Vector3 => "Vector3",
            Self::Pose3 => "Pose3",
            Self::Transform3 => "Transform3",
            Self::Twist3 => "Twist3",
        }
    }

    /// Generated type name for a dynamic (`SpatialId`) binding
    /// (`SpatialPoint3`, …). REQ: REQ-SPATIAL-009.
    #[must_use]
    pub const fn dynamic_name(self) -> &'static str {
        match self {
            Self::Point3 => "SpatialPoint3",
            Self::Direction3 => "SpatialDirection3",
            Self::Vector3 => "SpatialVector3",
            Self::Pose3 => "SpatialPose3",
            Self::Transform3 => "SpatialTransform3",
            Self::Twist3 => "SpatialTwist3",
        }
    }

    /// Canonical wire size in bytes for the raw numerics (`I-S2`).
    /// REQ: REQ-SPATIAL-005.
    #[must_use]
    pub const fn wire_bytes(self) -> u32 {
        match self {
            Self::Point3 | Self::Direction3 | Self::Vector3 => 24,
            Self::Pose3 | Self::Transform3 => 56,
            Self::Twist3 => 48,
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

impl SpatialField {
    /// Whether the field binds a dynamic object identity (`I-S2`/`I-S9`).
    /// REQ: REQ-SPATIAL-009.
    #[must_use]
    pub const fn is_dynamic(&self) -> bool {
        matches!(self.binding, SpatialBinding::Dynamic)
    }

    /// Canonical wire size in bytes (raw numerics only, `I-S2`).
    /// REQ: REQ-SPATIAL-005.
    #[must_use]
    pub const fn wire_bytes(&self) -> u32 {
        self.ty.wire_bytes()
    }

    /// Generated type spelling: `Point3<CameraFront>` for a compile-time frame,
    /// or `SpatialPoint3` for a dynamic binding.
    /// REQ: REQ-SPATIAL-005, REQ-SPATIAL-009.
    #[must_use]
    pub fn generated_type_name(&self) -> String {
        match &self.binding {
            SpatialBinding::Static { frame } => {
                format!("{}<{}>", self.ty.generated_name(), frame_type_param(frame))
            }
            SpatialBinding::Dynamic => self.ty.dynamic_name().to_string(),
        }
    }
}

/// Converts a `/`-separated frame path into its generated type parameter
/// (`camera/front` ⇒ `CameraFront`). REQ: REQ-SPATIAL-047.
#[must_use]
pub fn frame_type_param(frame: &str) -> String {
    let mut out = String::new();
    for segment in frame.split('/') {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// A named spatial field in a topic schema (grammar-extension output).
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-005.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialSchemaField {
    /// Field name.
    pub name: String,
    /// Recognized spatial field (type + binding).
    pub field: SpatialField,
}

/// Parses one spatial declaration: `name: type { space: frame }` or
/// `name: type { spatial_id: true }` (`spatial-types-design.md` §3).
///
/// # Errors
/// [`SpatialGrammarError`] for a missing field name/type, an unknown keyword, a
/// naked quantity, a conflicting binding, or a malformed frame path.
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-005, REQ-SPATIAL-009, REQ-SPATIAL-047.
pub fn parse_spatial_declaration(text: &str) -> Result<SpatialSchemaField, SpatialGrammarError> {
    let line = text.trim();
    let (name, rest) = line
        .split_once(':')
        .ok_or_else(|| SpatialGrammarError::UnknownType(line.to_string()))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(SpatialGrammarError::NakedQuantity);
    }
    let rest = rest.trim();
    let (type_word, annot) = match rest.split_once('{') {
        Some((ty, tail)) => (
            ty.trim(),
            Some(tail.trim_end().trim_end_matches('}').trim()),
        ),
        None => (rest, None),
    };
    let mut space_frame: Option<&str> = None;
    let mut spatial_id = false;
    if let Some(body) = annot {
        for entry in body.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (key, value) = entry
                .split_once(':')
                .ok_or(SpatialGrammarError::NakedQuantity)?;
            match key.trim() {
                "space" => space_frame = Some(value.trim()),
                "spatial_id" => spatial_id = value.trim() == "true",
                _ => return Err(SpatialGrammarError::UnknownType(key.trim().to_string())),
            }
        }
    }
    let field = parse_spatial_field(type_word, space_frame, spatial_id)?;
    Ok(SpatialSchemaField {
        name: name.to_string(),
        field,
    })
}

/// Parses a whole spatial schema body: one declaration per non-empty,
/// non-comment (`#`) line. Every defect is collected.
///
/// # Errors
/// `Vec<SpatialGrammarError>` — one entry per malformed line.
/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-005, REQ-SPATIAL-009.
pub fn parse_spatial_schema(
    text: &str,
) -> Result<Vec<SpatialSchemaField>, Vec<SpatialGrammarError>> {
    let mut out = Vec::new();
    let mut errs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_spatial_declaration(line) {
            Ok(field) => out.push(field),
            Err(e) => errs.push(e),
        }
    }
    if errs.is_empty() { Ok(out) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::{
        SpatialBinding, SpatialGrammarError, SpatialType, frame_path_conforms, frame_type_param,
        parse_spatial_declaration, parse_spatial_field, parse_spatial_schema,
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

    /// REQ: REQ-SPATIAL-005, REQ-SPATIAL-009 — generated type spellings bind the
    /// compile-time frame or the dynamic identity.
    #[test]
    fn generated_type_spelling() {
        let static_field = parse_spatial_field("point3", Some("camera/front"), false)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(static_field.generated_type_name(), "Point3<CameraFront>");
        assert_eq!(static_field.wire_bytes(), 24);
        assert!(!static_field.is_dynamic());

        let dynamic =
            parse_spatial_field("pose3", None, true).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(dynamic.generated_type_name(), "SpatialPose3");
        assert_eq!(dynamic.wire_bytes(), 56);
        assert!(dynamic.is_dynamic());
        assert_eq!(frame_type_param("robot/camera/front"), "RobotCameraFront");
    }

    /// REQ: REQ-SPATIAL-036, REQ-SPATIAL-005 — a spatial schema body parses into
    /// typed fields with compile-time and dynamic bindings.
    #[test]
    fn schema_body_parses() {
        let text = "\
# camera/front.drm
position: point3 { space: camera/front }
velocity: vector3 { space: camera/front }
tracked: pose3 { spatial_id: true }
";
        let fields = parse_spatial_schema(text).unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "position");
        assert_eq!(fields[0].field.generated_type_name(), "Point3<CameraFront>");
        assert_eq!(fields[2].field.generated_type_name(), "SpatialPose3");
    }

    /// REQ: REQ-SPATIAL-036 — malformed declarations are rejected.
    #[test]
    fn schema_body_rejects_naked() {
        assert_eq!(
            parse_spatial_declaration("position: point3"),
            Err(SpatialGrammarError::NakedQuantity)
        );
        assert!(parse_spatial_declaration("position point3").is_err());
    }
}
