//! Boundary matrix gate (work-plan M01W4-E4-T1; canon-codec Test Matrix, I3):
//! data-driven cells from `fixtures/boundary_matrix.toml`, encode+decode sides
//! per cell, and a completeness assertion — any missing (type, case) cell
//! fails, giving `TYPE_MATRIX_PCT` = 100.
//! REQ: REQ-TOOL-001, REQ-TOOL-002, REQ-CORE-081, REQ-CORE-083.

use drm_types::Status;
use drm_workspace::canon::value::{Value, decode, encode};
use drm_workspace::canon::{Region, layout_struct};
use drm_workspace::parser::{Field, Prim, Type, parse_bytes};

/// Completeness table: every (type, case) pair the matrix must contain.
const MATRIX: &[(&str, &[&str])] = &[
    ("u8", &["zero", "max"]),
    ("i8", &["zero", "min", "max"]),
    ("u16", &["zero", "max"]),
    ("i16", &["zero", "min", "max"]),
    ("u32", &["zero", "max"]),
    ("i32", &["zero", "min", "max"]),
    ("u64", &["zero", "max"]),
    ("i64", &["zero", "min", "max"]),
    ("f32", &["zero", "max", "inf"]),
    ("f64", &["zero", "max", "inf"]),
    ("bool", &["false", "true", "two"]),
    (
        "enum{off,on}",
        &["first", "last", "unknown", "unknown-decoded"],
    ),
    ("u8[4]", &["full", "over"]),
    (
        "u32[<=4]",
        &["empty", "full", "over", "overcount", "overcount-empty"],
    ),
    ("string[8]", &["empty", "full", "over", "badutf8"]),
    ("bytes[<=8]", &["empty", "full", "over", "overcount"]),
];

/// Fixture scaffolding: setup invariants only — never expect/unwrap (workspace
/// lint policy); `unreachable!` marks a fixture bug, not tested behavior.
fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated: {e:?}"),
    }
}

/// Option-flavored fixture helper.
fn fixture_some<T>(o: Option<T>) -> T {
    match o {
        Some(v) => v,
        None => unreachable!("fixture invariant violated"),
    }
}

/// One matrix cell as parsed from the TOML fixture.
#[derive(Debug)]
struct Cell {
    ty: String,
    case: String,
    value: String,
    patch: String,
    expect: String,
}

/// REQ: REQ-TOOL-001 — load the fixture and fail on ANY missing (type, case)
/// cell (completeness = `TYPE_MATRIX_PCT` 100).
fn load_cells() -> Vec<Cell> {
    let raw = include_str!("fixtures/boundary_matrix.toml");
    let table: toml::Table = fixture(toml::from_str(raw));
    let cells = fixture_some(table.get("cell").and_then(toml::Value::as_array));
    let parsed: Vec<Cell> = cells
        .iter()
        .map(|c| {
            let t = fixture_some(c.as_table());
            Cell {
                ty: fixture_some(t["type"].as_str()).to_owned(),
                case: fixture_some(t["case"].as_str()).to_owned(),
                value: fixture_some(t["value"].as_str()).to_owned(),
                patch: t
                    .get("patch")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                expect: fixture_some(t["expect"].as_str()).to_owned(),
            }
        })
        .collect();
    for (ty, cases) in MATRIX {
        for case in *cases {
            let hits = parsed
                .iter()
                .filter(|c| &c.ty == ty && &c.case == case)
                .count();
            assert_eq!(hits, 1, "missing cell ({ty}, {case})");
        }
    }
    let expected_total: usize = MATRIX.iter().map(|(_, cs)| cs.len()).sum();
    assert_eq!(parsed.len(), expected_total, "no unlisted cells allowed");
    parsed
}

/// REQ: REQ-TOOL-034 — single-field schema for a matrix type.
fn field_of(ty: &str) -> Field {
    let schema = format!("v: {ty}\n");
    let mut fields = fixture(parse_bytes(schema.as_bytes())).fields;
    fixture_some(fields.drain(..).next())
}

/// REQ: REQ-TOOL-034 — value literal → value model (matrix grammar: decimals,
/// `inf`, quoted strings, bracket lists).
fn parse_literal(ty: &Type, s: &str) -> Value {
    let int_lit = |x: &str| -> i128 { fixture(x.parse::<i128>()) };
    let uint_lit = |x: &str| -> u128 { fixture(x.parse::<u128>()) };
    match ty {
        Type::Prim(Prim::Bool) => match s {
            "false" => Value::Bool(false),
            "true" => Value::Bool(true),
            _ => unreachable!("matrix bool literal"),
        },
        Type::Prim(Prim::I8) => Value::I8(fixture(i8::try_from(int_lit(s)))),
        Type::Prim(Prim::I16) => Value::I16(fixture(i16::try_from(int_lit(s)))),
        Type::Prim(Prim::I32) => Value::I32(fixture(i32::try_from(int_lit(s)))),
        Type::Prim(Prim::I64) => Value::I64(fixture(i64::try_from(int_lit(s)))),
        Type::Prim(Prim::U8) => Value::U8(fixture(u8::try_from(uint_lit(s)))),
        Type::Prim(Prim::U16) => Value::U16(fixture(u16::try_from(uint_lit(s)))),
        Type::Prim(Prim::U32) => Value::U32(fixture(u32::try_from(uint_lit(s)))),
        Type::Prim(Prim::U64) => Value::U64(fixture(u64::try_from(uint_lit(s)))),
        Type::Prim(Prim::F32) => Value::F32(fixture(s.parse::<f32>())),
        Type::Prim(Prim::F64) => Value::F64(fixture(s.parse::<f64>())),
        Type::Enum { .. } => Value::Enum(fixture(s.parse::<u8>())),
        Type::FixedArray { elem, .. } | Type::Bounded { elem, .. } => {
            let inner = fixture_some(s.strip_prefix('[').and_then(|x| x.strip_suffix(']')));
            let elems: Vec<Value> = if inner.trim().is_empty() {
                Vec::new()
            } else {
                inner
                    .split(',')
                    .map(|e| parse_literal(&Type::Prim(*elem), e.trim()))
                    .collect()
            };
            match ty {
                Type::FixedArray { .. } => Value::Fixed(elems),
                _ => Value::Bounded(elems),
            }
        }
        Type::String { .. } => {
            let inner = fixture_some(s.strip_prefix('"').and_then(|x| x.strip_suffix('"')));
            Value::Str(inner.to_owned())
        }
        Type::Bytes { .. } => {
            let inner = fixture_some(s.strip_prefix('[').and_then(|x| x.strip_suffix(']')));
            let bytes: Vec<u8> = if inner.trim().is_empty() {
                Vec::new()
            } else {
                inner
                    .split(',')
                    .map(|e| fixture(e.trim().parse::<u8>()))
                    .collect()
            };
            Value::Bytes(bytes)
        }
    }
}

/// REQ: REQ-CORE-083 — apply a tamper to the canonical sample (decode-reject
/// rows): count+1, tag/bool wire byte 2, or malformed UTF-8 in the body.
fn patch_sample(patch: &str, fs: &[Field], sample: &mut [u8]) {
    let sl = fixture(layout_struct(fs));
    match patch {
        "count-max1" => {
            let desc = fixture_some(sl.members.iter().find(|m| m.region == Region::Descriptor));
            let off = fixture(usize::try_from(desc.offset));
            let c = u32::from_le_bytes([
                sample[off],
                sample[off + 1],
                sample[off + 2],
                sample[off + 3],
            ]);
            let c = fixture_some(c.checked_add(1));
            sample[off..off + 4].copy_from_slice(&c.to_le_bytes());
        }
        "bool-two" | "tag-unknown" => {
            let header = fixture_some(sl.members.iter().find(|m| m.region == Region::Header));
            let off = fixture(usize::try_from(header.offset));
            sample[off] = 2;
        }
        "utf8-bad" => {
            let body = fixture_some(sl.members.iter().find(|m| m.region == Region::Body));
            let off = fixture(usize::try_from(body.offset));
            sample[off] = 0xC0;
            sample[off + 1] = 0xAF;
        }
        "" => {}
        other => unreachable!("unknown patch {other}"),
    }
}

/// REQ: REQ-TOOL-001, REQ-CORE-081, REQ-CORE-083 — every cell: encode side,
/// decode side, and destination-untouched probes, exactly as declared.
#[test]
fn boundary_matrix_cells() {
    let cells = load_cells();
    for c in &cells {
        let f = field_of(&c.ty);
        let fs = std::slice::from_ref(&f);
        let sl = fixture(layout_struct(fs));
        let cap = fixture(usize::try_from(sl.sample_max));
        let mut dst = vec![0xAA_u8; cap];
        let v = parse_literal(&f.ty, &c.value);
        let ctx = format!("cell ({}/{})", c.ty, c.case);
        match c.expect.as_str() {
            "accept" => {
                let n = match encode(fs, std::slice::from_ref(&v), &mut dst) {
                    Ok(n) => n,
                    Err(e) => unreachable!("{ctx} encode must accept: {e:?}"),
                };
                let n = fixture(usize::try_from(n));
                let (back, _) = match decode(fs, &dst[..n]) {
                    Ok(x) => x,
                    Err(e) => unreachable!("{ctx} decode must accept: {e:?}"),
                };
                assert_eq!(back[0], v, "{ctx} roundtrip");
            }
            "reject" => {
                assert_eq!(encode(fs, &[v], &mut dst), Err(Status::InvalidArg), "{ctx}");
                assert!(
                    dst.iter().all(|b| *b == 0xAA),
                    "{ctx} destination untouched"
                );
            }
            "decode-reject" => {
                let n = match encode(fs, &[v], &mut dst) {
                    Ok(n) => n,
                    Err(e) => unreachable!("{ctx} baseline encode: {e:?}"),
                };
                let n = fixture(usize::try_from(n));
                patch_sample(&c.patch, fs, &mut dst[..n]);
                assert_eq!(
                    decode(fs, &dst[..n]),
                    Err(Status::InvalidData),
                    "{ctx} patched sample must reject"
                );
            }
            other => unreachable!("unknown expect {other}"),
        }
    }
}

/// REQ: REQ-TOOL-001 — `TYPE_MATRIX_PCT`: completeness assertion is part of
/// [`load_cells`]; this test pins the total cell count explicitly.
#[test]
fn boundary_matrix_completeness() {
    let cells = load_cells();
    assert_eq!(cells.len(), 48);
}
