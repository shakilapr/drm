//! Toolchain-side canonical layout computation (docs/algorithms/canon-codec.md
//! §Functions `layout`, `layout_struct`): fixed-width u64 arithmetic over the
//! parsed `Type` AST, two-region struct layout, hand-checkable fixtures.
//! Self-contained: drm-workspace never depends on drm-core (codebase.md 2.2 R3).
//! REQ: REQ-TOOL-042, REQ-TOOL-041.

use crate::parser::{Field, Prim, Type};
use drm_types::Status;

pub mod value;

/// Type-level layout in the fixed-width canonical domain (u64 size, u32 align).
/// REQ: REQ-TOOL-042.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeLayout {
    /// Canonical size in bytes (fixed members only; variable members report
    /// their descriptor size, bodies live in the tail).
    pub size: u64,
    /// Natural alignment in bytes.
    pub align: u32,
}

/// Region of a struct member in the two-region layout.
/// REQ: REQ-TOOL-042.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Region 1: fixed header field.
    Header,
    /// Region 1: per-variable descriptor (`count: u32`, plus `offset: u32`
    /// when the struct has > 1 variable member).
    Descriptor,
    /// Region 2: variable body (offset shown at declared max).
    Body,
}

/// One struct member as laid out. A schema field may occupy two slots
/// (descriptor + body) when it is a variable member.
/// REQ: REQ-TOOL-042.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberLayout {
    /// Field name (repeated for descriptor + body slots).
    pub name: String,
    /// Which region this slot lives in.
    pub region: Region,
    /// Absolute offset from sample start (body offsets at declared max).
    pub offset: u64,
    /// Slot size (body: size at declared max).
    pub size: u64,
    /// Slot alignment.
    pub align: u32,
}

/// Whole-struct two-region layout (canon-codec §Fixed Header + Bounded Tail).
/// REQ: REQ-TOOL-042.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructLayout {
    /// End of fixed header + descriptor block.
    pub header_end: u64,
    /// Maximum canonical sample size (header + Σ max-aligned bodies).
    pub sample_max: u64,
    /// Slot/struct alignment (max header align ∪ descriptor align ∪ body elem aligns).
    pub struct_align: u32,
    /// Members in declaration order (fixed: one slot; variable: descriptor + body).
    pub members: Vec<MemberLayout>,
}

/// Checked `align_up` in the u64 domain (I7). REQ: REQ-TOOL-041.
fn align_up(cursor: u64, align: u32) -> Result<u64, Status> {
    let a = u64::from(align);
    let rem = cursor % a;
    if rem == 0 {
        return Ok(cursor);
    }
    cursor.checked_add(a - rem).ok_or(Status::InvalidConfig)
}

/// Layout of a canonical primitive (canon-codec §Data Structures).
/// REQ: REQ-TOOL-044.
#[must_use]
pub const fn prim_layout(p: Prim) -> TypeLayout {
    let (size, align) = match p {
        Prim::Bool | Prim::I8 | Prim::U8 => (1, 1),
        Prim::I16 | Prim::U16 => (2, 2),
        Prim::I32 | Prim::U32 | Prim::F32 => (4, 4),
        Prim::I64 | Prim::U64 | Prim::F64 => (8, 8),
    };
    TypeLayout { size, align }
}

/// Layout of a schema `Type` as a struct member. Variable members (`string[N]`,
/// `bytes[<=N]`, `T[<=N]`) report their fixed-region descriptor: size 4, align 4;
/// the body lives in the tail and is re-derived by [`layout_struct`].
/// REQ: REQ-TOOL-042, REQ-TOOL-041, REQ-TOOL-001.
///
/// # Errors
/// [`Status::InvalidConfig`] when fixed-array size multiplication overflows u64
/// (checked arithmetic, I7; `N ≤ u32::MAX` and prim size ≤ 8 cannot overflow, so
/// this branch is defensive).
pub fn layout(t: &Type) -> Result<TypeLayout, Status> {
    Ok(match t {
        Type::Prim(p) => prim_layout(*p),
        Type::Enum { .. } => TypeLayout { size: 1, align: 1 },
        Type::String { .. } | Type::Bytes { .. } | Type::Bounded { .. } => {
            TypeLayout { size: 4, align: 4 }
        }
        Type::FixedArray { elem, n } => {
            let e = prim_layout(*elem);
            TypeLayout {
                size: e
                    .size
                    .checked_mul(u64::from(*n))
                    .ok_or(Status::InvalidConfig)?,
                align: e.align,
            }
        }
    })
}

/// Tail-element geometry of a variable member: element size/alignment and the
/// declared max count. `string[N]`/`bytes[<=N]` bodies are byte arrays
/// (elem 1×1); `T[<=N]` bodies are prim arrays.
/// REQ: REQ-TOOL-042, REQ-TOOL-040.
#[must_use]
pub fn variable_parts(t: &Type) -> Option<(u64, u32, u64)> {
    match *t {
        Type::String { max } | Type::Bytes { max } => Some((1, 1, u64::from(max))),
        Type::Bounded { elem, max } => {
            let e = prim_layout(elem);
            Some((e.size, e.align, u64::from(max)))
        }
        _ => None,
    }
}

/// Two-region struct layout (canon-codec §Functions `layout_struct`):
/// declaration order, natural alignment, total padded; descriptor block after
/// fixed fields; bodies contiguous and element-aligned in the tail.
/// REQ: REQ-TOOL-042, REQ-TOOL-041, REQ-TOOL-040.
///
/// # Errors
/// [`Status::InvalidConfig`] on any overflow (I7) or canonical sample size
/// exceeding the `u32::MAX` MVP cap.
pub fn layout_struct(fields: &[Field]) -> Result<StructLayout, Status> {
    let mut cursor = 0_u64;
    let mut struct_align = 1_u32;
    let mut members = Vec::new();

    // Region 1: fixed fields in declaration order, natural alignment.
    for f in fields {
        if matches!(
            &f.ty,
            Type::String { .. } | Type::Bytes { .. } | Type::Bounded { .. }
        ) {
            continue;
        }
        let l = layout(&f.ty)?;
        let off = align_up(cursor, l.align)?;
        members.push(MemberLayout {
            name: f.name.clone(),
            region: Region::Header,
            offset: off,
            size: l.size,
            align: l.align,
        });
        struct_align = struct_align.max(l.align);
        cursor = off.checked_add(l.size).ok_or(Status::InvalidConfig)?;
    }

    // Region 1: descriptor block (u32 count per variable member; u32 offset
    // each when > 1 variable member).
    let variable: Vec<&Field> = fields
        .iter()
        .filter(|f| variable_parts(&f.ty).is_some())
        .collect();
    if !variable.is_empty() {
        struct_align = struct_align.max(4);
        cursor = align_up(cursor, 4)?;
        let per: u64 = if variable.len() > 1 { 8 } else { 4 };
        for f in &variable {
            members.push(MemberLayout {
                name: f.name.clone(),
                region: Region::Descriptor,
                offset: cursor,
                size: per,
                align: 4,
            });
            cursor = cursor.checked_add(per).ok_or(Status::InvalidConfig)?;
        }
    }
    let header_end = cursor;

    // Region 2: bodies in declaration order, element-aligned, contiguous.
    for f in &variable {
        let Some((elem_size, elem_align, max)) = variable_parts(&f.ty) else {
            continue;
        };
        let off = align_up(cursor, elem_align)?;
        let size = max.checked_mul(elem_size).ok_or(Status::InvalidConfig)?;
        members.push(MemberLayout {
            name: f.name.clone(),
            region: Region::Body,
            offset: off,
            size,
            align: elem_align,
        });
        struct_align = struct_align.max(elem_align);
        cursor = off.checked_add(size).ok_or(Status::InvalidConfig)?;
    }

    let sample_max = cursor;
    if sample_max > u64::from(u32::MAX) {
        return Err(Status::InvalidConfig);
    }
    Ok(StructLayout {
        header_end,
        sample_max,
        struct_align,
        members,
    })
}
