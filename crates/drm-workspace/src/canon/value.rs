//! Interpreter codec over parsed schema types (docs/algorithms/canon-codec.md):
//! value model, two-phase encode (validate → write, I3 bounds-before-write),
//! structural decode (offsets re-derived from counts, exact-packing agreement).
//! Toolchain mirror of the generated encoders; feeds the type-triple checklist
//! and the boundary matrix. REQ: REQ-TOOL-001, REQ-TOOL-042, REQ-TOOL-043.

use super::{Region, StructLayout, layout_struct, prim_layout, variable_parts};
use crate::parser::{Field, Prim, Type};
use drm_types::Status;

/// Schema value model (one variant per canonical type family).
/// REQ: REQ-TOOL-034.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `bool` — byte 0/1 on the wire.
    Bool(bool),
    /// `f32` — IEEE-754 bit pattern preserved.
    F32(f32),
    /// `f64` — IEEE-754 bit pattern preserved.
    F64(f64),
    /// `i8`.
    I8(i8),
    /// `i16`.
    I16(i16),
    /// `i32`.
    I32(i32),
    /// `i64`.
    I64(i64),
    /// `u8`.
    U8(u8),
    /// `u16`.
    U16(u16),
    /// `u32`.
    U32(u32),
    /// `u64`.
    U64(u64),
    /// `enum{…}` — tag = declaration index.
    Enum(u8),
    /// `T[N]` — exactly `N` elements.
    Fixed(Vec<Value>),
    /// `T[<=N]` — at most `max` elements.
    Bounded(Vec<Value>),
    /// `string[N]` — ≤ `max` UTF-8 bytes (I10).
    Str(String),
    /// `bytes[<=N]` — ≤ `max` raw bytes.
    Bytes(Vec<u8>),
}

/// Actual-sample packing plan: per-variable body offsets (pure function of the
/// preceding counts) plus the derived `sample_len`.
/// REQ: REQ-TOOL-042.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Derived sample length (`f(counts)`; never stored on the wire).
    pub sample_len: u64,
    /// Body offset per variable member, declaration order.
    pub body_offsets: Vec<u64>,
}

/// LE u32 read (toolchain mirror of `drm-core` `read_u32`). REQ: REQ-TOOL-043.
///
/// # Errors
/// [`Status::InvalidData`] when the 4-byte window leaves the buffer.
fn read_u32(buf: &[u8], off: u64) -> Result<u32, Status> {
    let off = usize::try_from(off).map_err(|_| Status::InvalidData)?;
    let end = off.checked_add(4).ok_or(Status::InvalidData)?;
    let s = buf.get(off..end).ok_or(Status::InvalidData)?;
    if s.len() != 4 {
        return Err(Status::InvalidData);
    }
    let mut b = [0_u8; 4];
    b.copy_from_slice(s);
    Ok(u32::from_le_bytes(b))
}

/// Bounds-checked LE write of a byte pattern (I3 before any byte moves).
/// REQ: REQ-TOOL-043, REQ-CORE-081.
fn write_bytes(dst: &mut [u8], off: u64, src: &[u8]) -> Result<(), Status> {
    let off = usize::try_from(off).map_err(|_| Status::InvalidArg)?;
    let end = off.checked_add(src.len()).ok_or(Status::InvalidArg)?;
    let slot = dst.get_mut(off..end).ok_or(Status::InvalidArg)?;
    slot.copy_from_slice(src);
    Ok(())
}

/// Bounds-checked LE read of a byte pattern. REQ: REQ-TOOL-043, REQ-CORE-083.
fn read_bytes(buf: &[u8], off: u64, n: u64) -> Result<&[u8], Status> {
    let off = usize::try_from(off).map_err(|_| Status::InvalidData)?;
    let n = usize::try_from(n).map_err(|_| Status::InvalidData)?;
    let end = off.checked_add(n).ok_or(Status::InvalidData)?;
    buf.get(off..end).ok_or(Status::InvalidData)
}

/// Shape + bound validation of one value against one type.
/// REQ: REQ-TOOL-001, REQ-TOOL-002.
fn check_value(t: &Type, v: &Value) -> Result<(), Status> {
    match (t, v) {
        (Type::Prim(Prim::Bool), Value::Bool(_))
        | (Type::Prim(Prim::F32), Value::F32(_))
        | (Type::Prim(Prim::F64), Value::F64(_))
        | (Type::Prim(Prim::I8), Value::I8(_))
        | (Type::Prim(Prim::I16), Value::I16(_))
        | (Type::Prim(Prim::I32), Value::I32(_))
        | (Type::Prim(Prim::I64), Value::I64(_))
        | (Type::Prim(Prim::U8), Value::U8(_))
        | (Type::Prim(Prim::U16), Value::U16(_))
        | (Type::Prim(Prim::U32), Value::U32(_))
        | (Type::Prim(Prim::U64), Value::U64(_)) => Ok(()),
        (Type::Enum { variants }, Value::Enum(tag)) => {
            let n = u64::try_from(variants.len()).map_err(|_| Status::InvalidArg)?;
            if u64::from(*tag) < n {
                Ok(())
            } else {
                Err(Status::InvalidArg)
            }
        }
        (Type::FixedArray { elem, n }, Value::Fixed(elems)) => {
            let n = usize::try_from(*n).map_err(|_| Status::InvalidArg)?;
            if elems.len() != n {
                return Err(Status::InvalidArg);
            }
            elems
                .iter()
                .try_for_each(|e| check_value(&Type::Prim(*elem), e))
        }
        (Type::Bounded { elem, max }, Value::Bounded(elems)) => {
            let max = usize::try_from(*max).map_err(|_| Status::InvalidArg)?;
            if elems.len() > max {
                return Err(Status::InvalidArg);
            }
            elems
                .iter()
                .try_for_each(|e| check_value(&Type::Prim(*elem), e))
        }
        (Type::String { max }, Value::Str(s)) => {
            let max = usize::try_from(*max).map_err(|_| Status::InvalidArg)?;
            if s.len() <= max {
                Ok(())
            } else {
                Err(Status::InvalidArg)
            }
        }
        (Type::Bytes { max }, Value::Bytes(b)) => {
            let max = usize::try_from(*max).map_err(|_| Status::InvalidArg)?;
            if b.len() <= max {
                Ok(())
            } else {
                Err(Status::InvalidArg)
            }
        }
        _ => Err(Status::InvalidArg),
    }
}

/// Encode one primitive at `off` (bounds-checked before any write, I3).
/// REQ: REQ-TOOL-043, REQ-CORE-081.
fn write_prim(dst: &mut [u8], off: u64, p: Prim, v: &Value) -> Result<(), Status> {
    let n = prim_layout(p).size;
    let mut le = [0_u8; 8];
    match (p, v) {
        (Prim::Bool, Value::Bool(b)) => le[0] = u8::from(*b),
        (Prim::F32, Value::F32(x)) => le[..4].copy_from_slice(&x.to_le_bytes()),
        (Prim::F64, Value::F64(x)) => le.copy_from_slice(&x.to_le_bytes()),
        (Prim::I8, Value::I8(x)) => le[..1].copy_from_slice(&x.to_le_bytes()),
        (Prim::I16, Value::I16(x)) => le[..2].copy_from_slice(&x.to_le_bytes()),
        (Prim::I32, Value::I32(x)) => le[..4].copy_from_slice(&x.to_le_bytes()),
        (Prim::I64, Value::I64(x)) => le.copy_from_slice(&x.to_le_bytes()),
        (Prim::U8, Value::U8(x)) => le[0] = *x,
        (Prim::U16, Value::U16(x)) => le[..2].copy_from_slice(&x.to_le_bytes()),
        (Prim::U32, Value::U32(x)) => le[..4].copy_from_slice(&x.to_le_bytes()),
        (Prim::U64, Value::U64(x)) => le.copy_from_slice(&x.to_le_bytes()),
        _ => return Err(Status::InvalidArg),
    }
    write_bytes(
        dst,
        off,
        &le[..usize::try_from(n).map_err(|_| Status::InvalidArg)?],
    )
}

/// Decode one primitive at `off`; bool strictly {0,1} (I5).
/// REQ: REQ-TOOL-043, REQ-CORE-083.
fn read_prim(buf: &[u8], off: u64, p: Prim) -> Result<Value, Status> {
    let n = prim_layout(p).size;
    let raw = read_bytes(buf, off, n)?;
    let le8 = |s: &[u8]| -> [u8; 8] {
        let mut a = [0_u8; 8];
        a[..s.len()].copy_from_slice(s);
        a
    };
    Ok(match p {
        Prim::Bool => match raw.first() {
            Some(0) => Value::Bool(false),
            Some(1) => Value::Bool(true),
            _ => return Err(Status::InvalidData),
        },
        Prim::F32 => {
            let mut b = [0_u8; 4];
            b.copy_from_slice(raw);
            Value::F32(f32::from_le_bytes(b))
        }
        Prim::F64 => Value::F64(f64::from_le_bytes(le8(raw))),
        Prim::I8 => Value::I8(i8::from_le_bytes([*raw
            .first()
            .ok_or(Status::InvalidData)?])),
        Prim::I16 => {
            let mut b = [0_u8; 2];
            b.copy_from_slice(raw);
            Value::I16(i16::from_le_bytes(b))
        }
        Prim::I32 => {
            let mut b = [0_u8; 4];
            b.copy_from_slice(raw);
            Value::I32(i32::from_le_bytes(b))
        }
        Prim::I64 => Value::I64(i64::from_le_bytes(le8(raw))),
        Prim::U8 => Value::U8(*raw.first().ok_or(Status::InvalidData)?),
        Prim::U16 => {
            let mut b = [0_u8; 2];
            b.copy_from_slice(raw);
            Value::U16(u16::from_le_bytes(b))
        }
        Prim::U32 => {
            let mut b = [0_u8; 4];
            b.copy_from_slice(raw);
            Value::U32(u32::from_le_bytes(b))
        }
        Prim::U64 => Value::U64(u64::from_le_bytes(le8(raw))),
    })
}

/// Actual-sample packing from declared counts: bodies element-aligned,
/// contiguous, declaration-ordered; `sample_len` derived. Errors are
/// structural (`InvalidData`): the encode path validates counts first.
/// REQ: REQ-TOOL-042, REQ-TOOL-040.
fn plan_from_counts(sl: &StructLayout, fields: &[Field], counts: &[u64]) -> Result<Plan, Status> {
    let mut cursor = sl.header_end;
    let mut body_offsets = Vec::new();
    let mut ci = 0;
    for f in fields {
        let Some((elem_size, elem_align, max)) = variable_parts(&f.ty) else {
            continue;
        };
        let count = counts.get(ci).copied().ok_or(Status::InvalidData)?;
        ci += 1;
        if count > max {
            return Err(Status::InvalidData);
        }
        let a = u64::from(elem_align);
        let rem = cursor % a;
        if rem != 0 {
            cursor = cursor.checked_add(a - rem).ok_or(Status::InvalidData)?;
        }
        body_offsets.push(cursor);
        let bytes = count.checked_mul(elem_size).ok_or(Status::InvalidData)?;
        cursor = cursor.checked_add(bytes).ok_or(Status::InvalidData)?;
    }
    Ok(Plan {
        sample_len: cursor,
        body_offsets,
    })
}

/// Counts of the variable members (declaration order).
/// REQ: REQ-TOOL-042.
fn counts_of(fields: &[Field], values: &[Value]) -> Result<Vec<u64>, Status> {
    let mut counts = Vec::new();
    for (f, v) in fields.iter().zip(values) {
        let c = match (&f.ty, v) {
            (Type::String { .. }, Value::Str(s)) => {
                u64::try_from(s.len()).map_err(|_| Status::InvalidArg)?
            }
            (Type::Bytes { .. }, Value::Bytes(b)) => {
                u64::try_from(b.len()).map_err(|_| Status::InvalidArg)?
            }
            (Type::Bounded { .. }, Value::Bounded(e)) => {
                u64::try_from(e.len()).map_err(|_| Status::InvalidArg)?
            }
            (Type::String { .. } | Type::Bytes { .. } | Type::Bounded { .. }, _) => {
                return Err(Status::InvalidArg);
            }
            _ => continue,
        };
        counts.push(c);
    }
    Ok(counts)
}

/// Phase A of the two-phase encode (I3): validate every value against its type
/// and derive the packing plan. No destination byte is touched.
/// REQ: REQ-TOOL-001, REQ-CORE-081, REQ-TOOL-051.
///
/// # Errors
/// [`Status::InvalidArg`] on arity mismatch, value/type mismatch, bound
/// violation (incl. UTF-8 byte length, I10), or unknown enum tag.
pub fn plan(fields: &[Field], values: &[Value]) -> Result<Plan, Status> {
    if fields.len() != values.len() {
        return Err(Status::InvalidArg);
    }
    for (f, v) in fields.iter().zip(values) {
        check_value(&f.ty, v)?;
    }
    let sl = layout_struct(fields)?;
    let counts = counts_of(fields, values)?;
    let p = plan_from_counts(&sl, fields, &counts)?;
    if p.sample_len > u64::from(u32::MAX) {
        return Err(Status::InvalidArg);
    }
    Ok(p)
}

/// Two-phase encode: [`plan`] first (destination untouched until it succeeds),
/// then descriptors, fixed fields, bodies — canonical packing. Returns the
/// derived `sample_len`. REQ: REQ-TOOL-042, REQ-CORE-081, REQ-TOOL-051.
///
/// # Errors
/// [`Status::InvalidArg`] from [`plan`], or when `dst` is shorter than the
/// derived `sample_len` (bounds-before-write).
pub fn encode(fields: &[Field], values: &[Value], dst: &mut [u8]) -> Result<u64, Status> {
    let p = plan(fields, values)?;
    if u64::try_from(dst.len()).map_err(|_| Status::InvalidArg)? < p.sample_len {
        return Err(Status::InvalidArg);
    }
    // Canonical determinism: every byte in [0, sample_len) is written —
    // alignment gaps between bodies (and header padding) are zeroed first,
    // then filled by the field writes below (I1/I8: gap bytes are exactly 0).
    let sample_len = usize::try_from(p.sample_len).map_err(|_| Status::InvalidArg)?;
    dst[..sample_len].fill(0);
    let sl = layout_struct(fields)?;
    let counts = counts_of(fields, values)?;
    let n_var = counts.len();

    // Descriptors: counts (+ offsets when > 1 variable member).
    for (k, m) in sl
        .members
        .iter()
        .filter(|m| m.region == Region::Descriptor)
        .enumerate()
    {
        let c = u32::try_from(counts[k]).map_err(|_| Status::InvalidArg)?;
        write_bytes(dst, m.offset, &c.to_le_bytes())?;
        if n_var > 1 {
            let o = u32::try_from(p.body_offsets[k]).map_err(|_| Status::InvalidArg)?;
            write_bytes(dst, m.offset + 4, &o.to_le_bytes())?;
        }
    }

    // Fixed fields + variable bodies.
    let mut headers = sl.members.iter().filter(|m| m.region == Region::Header);
    let mut bi = 0;
    for (f, v) in fields.iter().zip(values) {
        if variable_parts(&f.ty).is_some() {
            let off = p.body_offsets[bi];
            bi += 1;
            match v {
                Value::Str(s) => write_bytes(dst, off, s.as_bytes())?,
                Value::Bytes(b) => write_bytes(dst, off, b)?,
                Value::Bounded(elems) => {
                    let es = variable_parts(&f.ty).map_or(1, |(s, _, _)| s);
                    let elem = match &f.ty {
                        Type::Bounded { elem, .. } => *elem,
                        _ => Prim::U8,
                    };
                    for (j, e) in elems.iter().enumerate() {
                        let j = u64::try_from(j).map_err(|_| Status::InvalidArg)?;
                        write_prim(dst, off + j * es, elem, e)?;
                    }
                }
                _ => return Err(Status::InvalidArg),
            }
        } else {
            let Some(m) = headers.next() else {
                return Err(Status::InvalidArg);
            };
            write_header(dst, m.offset, &f.ty, v)?;
        }
    }
    Ok(p.sample_len)
}

/// Write one header field (prim / fixed array / enum). REQ: REQ-CORE-081.
fn write_header(dst: &mut [u8], off: u64, t: &Type, v: &Value) -> Result<(), Status> {
    match (t, v) {
        (Type::Prim(p), _) => write_prim(dst, off, *p, v),
        (Type::Enum { .. }, Value::Enum(tag)) => write_prim(dst, off, Prim::U8, &Value::U8(*tag)),
        (Type::FixedArray { elem, n }, Value::Fixed(elems)) => {
            let es = prim_layout(*elem).size;
            let n = u64::from(*n);
            for (j, e) in elems.iter().enumerate() {
                let j = u64::try_from(j).map_err(|_| Status::InvalidArg)?;
                if j >= n {
                    return Err(Status::InvalidArg);
                }
                write_prim(dst, off + j * es, *elem, e)?;
            }
            Ok(())
        }
        _ => Err(Status::InvalidArg),
    }
}

/// Read one header field (prim / fixed array / enum); bool/enum strict (I5).
/// REQ: REQ-TOOL-043, REQ-CORE-083.
fn read_header(buf: &[u8], off: u64, t: &Type) -> Result<Value, Status> {
    match t {
        Type::Prim(p) => read_prim(buf, off, *p),
        Type::Enum { variants } => {
            let n = u64::try_from(variants.len()).map_err(|_| Status::InvalidData)?;
            match read_prim(buf, off, Prim::U8)? {
                Value::U8(tag) if u64::from(tag) < n => Ok(Value::Enum(tag)),
                _ => Err(Status::InvalidData),
            }
        }
        Type::FixedArray { elem, n } => {
            let es = prim_layout(*elem).size;
            let mut elems = Vec::new();
            for j in 0..u64::from(*n) {
                elems.push(read_prim(buf, off + j * es, *elem)?);
            }
            Ok(Value::Fixed(elems))
        }
        _ => Err(Status::InvalidData),
    }
}

/// Structural decode: counts read from descriptors, offsets re-derived from the
/// canonical packing, exact agreement enforced (`buf.len() == derived
/// sample_len`; stored offsets == derived when > 1 variable member), UTF-8
/// strict, enum tags closed, bool strict. REQ: REQ-TOOL-042, REQ-TOOL-040,
/// REQ-CORE-083.
///
/// # Errors
/// [`Status::InvalidData`] on any structural violation (I5: value or error,
/// never UB).
pub fn decode(fields: &[Field], buf: &[u8]) -> Result<(Vec<Value>, u64), Status> {
    let sl = layout_struct(fields)?;
    let buf_len = u64::try_from(buf.len()).map_err(|_| Status::InvalidData)?;
    if buf_len < sl.header_end {
        return Err(Status::InvalidData);
    }

    // Counts from descriptors; declared maxima enforced (pre-read).
    let descriptor_slots: Vec<_> = sl
        .members
        .iter()
        .filter(|m| m.region == Region::Descriptor)
        .collect();
    let mut counts = Vec::new();
    for m in &descriptor_slots {
        let c = u64::from(read_u32(buf, m.offset)?);
        let idx = counts.len();
        let Some(f) = fields
            .iter()
            .filter(|f| variable_parts(&f.ty).is_some())
            .nth(idx)
        else {
            return Err(Status::InvalidData);
        };
        let Some((_, _, max)) = variable_parts(&f.ty) else {
            return Err(Status::InvalidData);
        };
        if c > max {
            return Err(Status::InvalidData);
        }
        counts.push(c);
    }

    // Packing derived from counts; exact agreement (no trailing bytes).
    let p = plan_from_counts(&sl, fields, &counts)?;
    if p.sample_len != buf_len {
        return Err(Status::InvalidData);
    }

    // Stored offsets must equal the canonical packing (> 1 variable member).
    if counts.len() > 1 {
        for (k, m) in descriptor_slots.iter().enumerate() {
            let stored = u64::from(read_u32(buf, m.offset + 4)?);
            if stored != p.body_offsets[k] {
                return Err(Status::InvalidData);
            }
        }
    }

    // Fixed fields + bodies → values.
    let mut headers = sl.members.iter().filter(|m| m.region == Region::Header);
    let mut bi = 0;
    let mut out = Vec::new();
    for f in fields {
        let Some((elem_size, _, _)) = variable_parts(&f.ty) else {
            let Some(m) = headers.next() else {
                return Err(Status::InvalidData);
            };
            out.push(read_header(buf, m.offset, &f.ty)?);
            continue;
        };
        let off = p.body_offsets[bi];
        let c = counts[bi];
        bi += 1;
        match &f.ty {
            Type::String { .. } => {
                let raw = read_bytes(buf, off, c)?;
                let s = std::str::from_utf8(raw).map_err(|_| Status::InvalidData)?;
                out.push(Value::Str(s.to_owned()));
            }
            Type::Bytes { .. } => out.push(Value::Bytes(read_bytes(buf, off, c)?.to_vec())),
            Type::Bounded { elem, .. } => {
                let mut elems = Vec::new();
                for j in 0..c {
                    elems.push(read_prim(buf, off + j * elem_size, *elem)?);
                }
                out.push(Value::Bounded(elems));
            }
            _ => return Err(Status::InvalidData),
        }
    }
    Ok((out, p.sample_len))
}
