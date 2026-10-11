//! Build-time spatial model and static collapse v0
//! ([workspace-model.md](../../../docs/algorithms/toolchain/workspace-model.md)
//! §spatial; [spatial-core.md](../../../docs/algorithms/spatial/spatial-core.md)
//! §Protocols `Static collapse`). `spaces/**.drm` files are parsed, each edge is
//! classified deterministic (`I-D6`/`I-S10`), and contiguous static chains fold
//! into one `SE(3)` constant per frame (`I-S13`/`REQ-SPATIAL-049`) emitted as a
//! codegen artifact. Dynamic edges stay runtime cache entries.
//!
//! The toolchain layer may not depend on the `drm-spatial` runtime crate
//! (codebase.md 2.1), so this module carries a minimal rigid-transform type for
//! the build-time fold; the runtime consumes the raw `[f64; 3]`/`[f64; 4]`
//! constants unchanged.
//!
//! REQ: REQ-SPATIAL-047, REQ-SPATIAL-048, REQ-SPATIAL-049, REQ-SPATIAL-031,
//! REQ-SPATIAL-017.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

/// Canonical frame identity: `/`-separated lowercase segments derived from the
/// filesystem path ([spatial-core.md] §Authoring Spaces). REQ: REQ-SPATIAL-047.
pub type FrameId = String;

/// A rigid `SE(3)` transform: translation `[X, Y, Z]` plus unit quaternion
/// `[X, Y, Z, W]` (raw numerics only, `I-S2`).
/// REQ: REQ-SPATIAL-038, REQ-SPATIAL-048.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform3 {
    /// Translation, meters.
    pub translation: [f64; 3],
    /// Unit quaternion `X, Y, Z, W`.
    pub rotation: [f64; 4],
}

impl Transform3 {
    /// The identity transform.
    /// REQ: REQ-SPATIAL-049.
    pub const IDENTITY: Self = Self {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    /// Composes `self` then `other` (`self ∘ other`), renormalizing the
    /// quaternion (`I-S1`). REQ: REQ-SPATIAL-049.
    #[must_use]
    pub fn compose(self, other: Self) -> Self {
        let r = quat_rotate(self.rotation, other.translation);
        let q = quat_mul(self.rotation, other.rotation);
        Self {
            translation: [
                self.translation[0] + r[0],
                self.translation[1] + r[1],
                self.translation[2] + r[2],
            ],
            rotation: q,
        }
    }
}

impl Default for Transform3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Rotates a `[f64; 3]` vector by a unit quaternion `[X, Y, Z, W]`.
/// REQ: REQ-SPATIAL-038.
fn quat_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let qv = [q[0], q[1], q[2]];
    let t = cross(qv, v);
    let t = [t[0] * 2.0, t[1] * 2.0, t[2] * 2.0];
    let c = cross(qv, t);
    [
        v[0] + q[3] * t[0] + c[0],
        v[1] + q[3] * t[1] + c[1],
        v[2] + q[3] * t[2] + c[2],
    ]
}

/// Hamilton product of two unit quaternions, renormalized.
/// REQ: REQ-SPATIAL-038.
fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let q = [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ];
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n <= f64::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// Cross product of two `[f64; 3]` vectors.
/// REQ: REQ-SPATIAL-038.
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A parsed `spaces/**.drm` declaration (before classification).
/// REQ: REQ-SPATIAL-047, REQ-SPATIAL-048.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpaceFile {
    /// Frame identity derived from the filesystem path.
    pub frame: FrameId,
    /// Parent frame, if any.
    pub parent: Option<FrameId>,
    /// Nominal static offset relative to the parent, if declared.
    pub transform: Option<Transform3>,
}

/// Edge classification (`I-D6`/`I-S10`).
/// REQ: REQ-SPATIAL-031.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeKind {
    /// A root frame with no parent (identity anchor).
    Root,
    /// A static relationship: nominal offset relative to the parent.
    Static {
        /// Nominal offset.
        transform: Transform3,
    },
    /// A dynamic edge owned by a single writer node.
    Dynamic {
        /// The sole writer.
        writer: String,
    },
}

/// One classified frame.
/// REQ: REQ-SPATIAL-031.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEntry {
    /// Parent frame, if any.
    pub parent: Option<FrameId>,
    /// Classified edge.
    pub kind: EdgeKind,
}

/// The classified spatial model (`I-D1`: sorted maps).
/// REQ: REQ-SPATIAL-031.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpatialModel {
    /// Frame identity → entry.
    pub frames: BTreeMap<FrameId, FrameEntry>,
}

/// A model/collapse error with the offending frame.
/// REQ: REQ-SPATIAL-031, REQ-SPATIAL-049.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialModelError {
    /// A `spaces/` file was malformed.
    BadSpaceFile {
        /// Source path.
        path: String,
        /// Defect description.
        message: String,
    },
    /// A non-root frame declared neither a writer nor a transform.
    DanglingFrame {
        /// Offending frame.
        frame: FrameId,
    },
    /// A frame declared both a writer and a transform.
    Conflict {
        /// Offending frame.
        frame: FrameId,
    },
    /// A frame's parent is not declared.
    UnknownParent {
        /// Offending frame.
        frame: FrameId,
        /// Missing parent frame.
        parent: FrameId,
    },
    /// A static chain formed a cycle.
    Cycle {
        /// Offending frame.
        frame: FrameId,
    },
}

impl fmt::Display for SpatialModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSpaceFile { path, message } => write!(f, "space `{path}`: {message}"),
            Self::DanglingFrame { frame } => {
                write!(f, "frame `{frame}` declares neither writer nor transform")
            }
            Self::Conflict { frame } => {
                write!(f, "frame `{frame}` declares both a writer and a transform")
            }
            Self::UnknownParent { frame, parent } => {
                write!(f, "frame `{frame}` references undeclared parent `{parent}`")
            }
            Self::Cycle { frame } => write!(f, "static chain at `{frame}` forms a cycle"),
        }
    }
}

impl std::error::Error for SpatialModelError {}

/// Derives a frame identity from a `spaces/**.drm` filesystem path
/// (`packages/robot/spaces/camera/front.drm` ⇒ `robot/camera/front`).
///
/// # Errors
/// [`SpatialModelError::BadSpaceFile`] when the path has no `spaces/` segment or
/// does not end in `.drm`.
/// REQ: REQ-SPATIAL-047.
pub fn frame_from_path(path: &str) -> Result<FrameId, SpatialModelError> {
    let bad = || SpatialModelError::BadSpaceFile {
        path: path.to_string(),
        message: "path must be `…/spaces/<frame>.drm`".to_string(),
    };
    let norm = path.replace('\\', "/");
    let segs: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    let idx = segs.iter().position(|s| *s == "spaces").ok_or_else(bad)?;
    let rest = &segs[idx + 1..];
    if rest.is_empty() {
        return Err(bad());
    }
    let last = rest[rest.len() - 1];
    let stem = last.strip_suffix(".drm").ok_or_else(bad)?;
    if stem.is_empty() {
        return Err(bad());
    }
    let mut out: Vec<&str> = rest[..rest.len() - 1].to_vec();
    out.push(stem);
    Ok(out.join("/"))
}

/// Parses a `spaces/**.drm` file body. Recognized keys: `parent` (string),
/// `translation` (`[x, y, z]`), `rotation` (`[x, y, z, w]`).
///
/// # Errors
/// [`SpatialModelError::BadSpaceFile`] for an unknown key or malformed value.
/// REQ: REQ-SPATIAL-047, REQ-SPATIAL-048.
pub fn parse_space_file(path: &str, src: &str) -> Result<SpaceFile, SpatialModelError> {
    let frame = frame_from_path(path)?;
    let mut parent = None;
    let mut translation = None;
    let mut rotation = None;
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| SpatialModelError::BadSpaceFile {
                path: path.to_string(),
                message: format!("expected `key = value`, got `{line}`"),
            })?;
        let key = key.trim();
        let value = value.trim();
        match key {
            "parent" => parent = Some(unquote(value).to_string()),
            "translation" => translation = Some(parse_floats::<3>(path, value)?),
            "rotation" => rotation = Some(parse_floats::<4>(path, value)?),
            other => {
                return Err(SpatialModelError::BadSpaceFile {
                    path: path.to_string(),
                    message: format!("unknown key `{other}`"),
                });
            }
        }
    }
    let transform = match (translation, rotation) {
        (Some(t), Some(r)) => Some(Transform3 {
            translation: t,
            rotation: r,
        }),
        (None, None) => None,
        _ => {
            return Err(SpatialModelError::BadSpaceFile {
                path: path.to_string(),
                message: "translation and rotation must be declared together".to_string(),
            });
        }
    };
    Ok(SpaceFile {
        frame,
        parent,
        transform,
    })
}

/// Strips one pair of surrounding double quotes.
/// REQ: REQ-SPATIAL-048.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// Parses a bracketed comma-separated `f64` list of length `N`.
/// REQ: REQ-SPATIAL-048.
fn parse_floats<const N: usize>(path: &str, value: &str) -> Result<[f64; N], SpatialModelError> {
    let inner = value
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| SpatialModelError::BadSpaceFile {
            path: path.to_string(),
            message: format!("expected `[..]`, got `{value}`"),
        })?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != N {
        return Err(SpatialModelError::BadSpaceFile {
            path: path.to_string(),
            message: format!("expected {N} values, got {}", parts.len()),
        });
    }
    let mut out = [0.0_f64; N];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse::<f64>()
            .map_err(|_| SpatialModelError::BadSpaceFile {
                path: path.to_string(),
                message: format!("`{p}` is not a number"),
            })?;
    }
    Ok(out)
}

/// Classifies parsed space files deterministically (`I-D6`): a declared writer
/// ⇒ dynamic; a transform and no writer ⇒ static; a parentless frame with
/// neither is a root; both ⇒ conflict; a non-root with neither ⇒ dangling.
///
/// # Errors
/// `Vec<SpatialModelError>` — every conflict, dangling frame, or unknown parent.
/// REQ: REQ-SPATIAL-031, REQ-SPATIAL-048.
pub fn classify(
    files: &[SpaceFile],
    writers: &BTreeMap<FrameId, String>,
) -> Result<SpatialModel, Vec<SpatialModelError>> {
    let mut model = SpatialModel::default();
    let mut errs = Vec::new();
    let mut seen: BTreeMap<&str, &SpaceFile> = BTreeMap::new();
    for f in files {
        seen.entry(f.frame.as_str()).or_insert(f);
    }
    for f in files {
        let writer = writers.get(&f.frame);
        let kind = match (writer, &f.transform, &f.parent) {
            (Some(_), Some(_), _) => {
                errs.push(SpatialModelError::Conflict {
                    frame: f.frame.clone(),
                });
                continue;
            }
            (Some(w), None, _) => EdgeKind::Dynamic { writer: w.clone() },
            (None, Some(t), _) => EdgeKind::Static { transform: *t },
            (None, None, None) => EdgeKind::Root,
            (None, None, Some(_)) => {
                errs.push(SpatialModelError::DanglingFrame {
                    frame: f.frame.clone(),
                });
                continue;
            }
        };
        if let Some(parent) = &f.parent
            && !seen.contains_key(parent.as_str())
        {
            errs.push(SpatialModelError::UnknownParent {
                frame: f.frame.clone(),
                parent: parent.clone(),
            });
            continue;
        }
        model.frames.insert(
            f.frame.clone(),
            FrameEntry {
                parent: f.parent.clone(),
                kind,
            },
        );
    }
    if errs.is_empty() {
        Ok(model)
    } else {
        Err(errs)
    }
}

/// One collapsed static constant: the transform from `anchor` to `frame`.
/// REQ: REQ-SPATIAL-049.
#[derive(Debug, Clone, PartialEq)]
pub struct FoldedConstant {
    /// Nearest dynamic/root ancestor.
    pub anchor: FrameId,
    /// Folded `anchor → frame` transform.
    pub transform: Transform3,
}

/// The result of folding all contiguous static chains (`I-S13`).
/// REQ: REQ-SPATIAL-049, REQ-SPATIAL-017.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CollapsePlan {
    /// Static frames folded to one constant, sorted by frame id (`I-D1`).
    pub constants: BTreeMap<FrameId, FoldedConstant>,
    /// Dynamic edges that remain runtime cache entries.
    pub dynamic_edges: BTreeSet<FrameId>,
}

/// Folds every contiguous static chain to a single constant from its nearest
/// dynamic/root anchor (`I-S13`).
///
/// # Errors
/// [`SpatialModelError::Cycle`] if the parent chain loops.
/// REQ: REQ-SPATIAL-049, REQ-SPATIAL-017.
pub fn collapse_static(model: &SpatialModel) -> Result<CollapsePlan, SpatialModelError> {
    let mut plan = CollapsePlan::default();
    for (frame, entry) in &model.frames {
        if let EdgeKind::Dynamic { .. } = entry.kind {
            plan.dynamic_edges.insert(frame.clone());
        }
    }
    let mut memo: BTreeMap<FrameId, FoldedConstant> = BTreeMap::new();
    for frame in model.frames.keys() {
        fold(model, frame, &mut memo, &mut Vec::new())?;
    }
    for (frame, entry) in &model.frames {
        if let EdgeKind::Static { .. } = entry.kind
            && let Some(c) = memo.get(frame)
        {
            plan.constants.insert(frame.clone(), c.clone());
        }
    }
    Ok(plan)
}

/// Recursively folds `frame`'s anchor→frame transform, memoizing results.
/// REQ: REQ-SPATIAL-049.
fn fold(
    model: &SpatialModel,
    frame: &str,
    memo: &mut BTreeMap<FrameId, FoldedConstant>,
    stack: &mut Vec<FrameId>,
) -> Result<FoldedConstant, SpatialModelError> {
    if let Some(c) = memo.get(frame) {
        return Ok(c.clone());
    }
    if stack.iter().any(|f| f == frame) {
        return Err(SpatialModelError::Cycle {
            frame: frame.to_string(),
        });
    }
    let Some(entry) = model.frames.get(frame) else {
        return Err(SpatialModelError::DanglingFrame {
            frame: frame.to_string(),
        });
    };
    let result = match &entry.kind {
        EdgeKind::Root | EdgeKind::Dynamic { .. } => FoldedConstant {
            anchor: frame.to_string(),
            transform: Transform3::IDENTITY,
        },
        EdgeKind::Static { transform } => match &entry.parent {
            Some(parent) => {
                stack.push(frame.to_string());
                let p = fold(model, parent, memo, stack)?;
                stack.pop();
                FoldedConstant {
                    anchor: p.anchor,
                    transform: p.transform.compose(*transform),
                }
            }
            None => FoldedConstant {
                anchor: frame.to_string(),
                transform: *transform,
            },
        },
    };
    memo.insert(frame.to_string(), result.clone());
    Ok(result)
}

/// A generated code artifact.
/// REQ: REQ-SPATIAL-017.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeArtifact {
    /// Artifact name.
    pub name: String,
    /// Target language.
    pub lang: String,
    /// Artifact body.
    pub body: String,
}

/// Emits collapsed constants and dynamic-edge names as a C artifact with a
/// `@generated` banner (MCU targets receive the constants at build; runtime
/// multiplies once, `I-S13`).
/// REQ: REQ-SPATIAL-017, REQ-SPATIAL-049.
#[must_use]
pub fn emit_constants(plan: &CollapsePlan) -> CodeArtifact {
    let mut body = String::from(
        "/* @generated by drm-workspace spatial collapse v0; do not edit */\n\
         #ifndef DRM_SPATIAL_CONSTANTS_H\n#define DRM_SPATIAL_CONSTANTS_H\n",
    );
    for (frame, c) in &plan.constants {
        let name = c_name(frame);
        let _ = write!(
            body,
            "/* anchor {} */\nstatic const double {name}_t[3] = {{{}, {}, {}}};\n\
             static const double {name}_q[4] = {{{}, {}, {}, {}}};\n",
            c.anchor,
            c.transform.translation[0],
            c.transform.translation[1],
            c.transform.translation[2],
            c.transform.rotation[0],
            c.transform.rotation[1],
            c.transform.rotation[2],
            c.transform.rotation[3],
        );
    }
    for edge in &plan.dynamic_edges {
        let _ = writeln!(body, "/* dynamic edge: {edge} */");
    }
    body.push_str("#endif\n");
    CodeArtifact {
        name: "spatial_constants.h".to_string(),
        lang: "c".to_string(),
        body,
    }
}

/// Sanitizes a frame id into a C identifier.
/// REQ: REQ-SPATIAL-017.
fn c_name(frame: &str) -> String {
    frame
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Transform3, classify, collapse_static, emit_constants, frame_from_path, parse_space_file,
    };
    use std::collections::BTreeMap;

    fn t(x: f64, y: f64, z: f64) -> Transform3 {
        Transform3 {
            translation: [x, y, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// REQ: REQ-SPATIAL-047 — frame identity derives from the filesystem path.
    #[test]
    fn frame_identity() {
        assert_eq!(
            frame_from_path("packages/robot/spaces/camera/front.drm").unwrap_or_default(),
            "camera/front"
        );
        assert_eq!(
            frame_from_path("packages/robot/spaces/robot/base.drm").unwrap_or_default(),
            "robot/base"
        );
        assert!(frame_from_path("packages/robot/topics/x.drm").is_err());
    }

    /// REQ: REQ-SPATIAL-048 — a static space file parses its nominal offset.
    #[test]
    fn parse_static_space() {
        let src = "parent = \"robot/base\"\ntranslation = [0.1, 0.0, 0.2]\nrotation = [0.0, 0.0, 0.0, 1.0]\n";
        let f = parse_space_file("packages/robot/spaces/camera/front.drm", src)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(f.parent.as_deref(), Some("robot/base"));
        assert_eq!(f.transform, Some(t(0.1, 0.0, 0.2)));
    }

    /// REQ: REQ-SPATIAL-049 — a contiguous static chain folds into one constant
    /// relative to its root anchor.
    #[test]
    fn collapse_chain() {
        let files = vec![
            parse_space_file("s/spaces/world.drm", "").unwrap_or_default(),
            parse_space_file(
                "s/spaces/base.drm",
                "parent = \"world\"\ntranslation = [1.0, 0.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
            )
            .unwrap_or_default(),
            parse_space_file(
                "s/spaces/rack.drm",
                "parent = \"base\"\ntranslation = [0.0, 2.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
            )
            .unwrap_or_default(),
            parse_space_file(
                "s/spaces/camera.drm",
                "parent = \"rack\"\ntranslation = [0.0, 0.0, 3.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
            )
            .unwrap_or_default(),
        ];
        let model = classify(&files, &BTreeMap::new()).unwrap_or_else(|e| unreachable!("{e:?}"));
        let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
        let camera = plan
            .constants
            .get("camera")
            .unwrap_or_else(|| unreachable!("camera constant"));
        assert_eq!(camera.anchor, "world");
        assert_eq!(camera.transform, t(1.0, 2.0, 3.0));
        assert!(plan.dynamic_edges.is_empty());
        let artifact = emit_constants(&plan);
        assert!(artifact.body.contains("@generated"));
        assert!(artifact.body.contains("camera_t"));
    }

    /// REQ: REQ-SPATIAL-031 — a dynamic edge breaks the chain and stays a
    /// runtime cache entry; both/neither declarations conflict.
    #[test]
    fn dynamic_breaks_chain() {
        let files = vec![
            parse_space_file("s/spaces/world.drm", "").unwrap_or_default(),
            parse_space_file(
                "s/spaces/odom.drm",
                "parent = \"world\"\n",
            )
            .unwrap_or_default(),
            parse_space_file(
                "s/spaces/base.drm",
                "parent = \"odom\"\ntranslation = [1.0, 0.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
            )
            .unwrap_or_default(),
        ];
        let mut writers = BTreeMap::new();
        writers.insert("odom".to_string(), "tracker".to_string());
        let model = classify(&files, &writers).unwrap_or_else(|e| unreachable!("{e:?}"));
        let plan = collapse_static(&model).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(plan.dynamic_edges.contains("odom"));
        let base = plan
            .constants
            .get("base")
            .unwrap_or_else(|| unreachable!("base constant"));
        assert_eq!(base.anchor, "odom");
        assert_eq!(base.transform, t(1.0, 0.0, 0.0));

        // Both a writer and a transform on one frame ⇒ conflict.
        let conflict = parse_space_file(
            "s/spaces/x.drm",
            "parent = \"world\"\ntranslation = [0.0, 0.0, 0.0]\nrotation = [0.0, 0.0, 0.0, 1.0]\n",
        )
        .unwrap_or_default();
        let mut w = BTreeMap::new();
        w.insert("x".to_string(), "node".to_string());
        assert!(classify(&[conflict], &w).is_err());
    }
}
