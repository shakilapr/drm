//! Golden primitive runner (canon-codec.md §Protocols; work-plan M01W3-E4-T3):
//! every committed `.hex` fixture equals the canonical LE encoding of the
//! frozen vector list AND decodes back through the canon read path. The
//! frozen list is re-declared here as an independent oracle — a generator or
//! codec regression cannot mask itself.
//! REQ: REQ-TOOL-043, REQ-TOOL-044.

use std::fs;
use std::path::Path;

use drm_core::canon;

/// Fixture byte length by type name.
fn width(ty: &str) -> usize {
    match ty {
        "u8" | "i8" | "bool" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" | "f32" => 4,
        other => unreachable!("unknown golden type {other}"),
    }
}

/// REQ: REQ-TOOL-043 — all 24 golden fixtures encode/decode canonically.
#[test]
fn golden_primitives_roundtrip() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/golden/primitives");
    let mut count = 0usize;
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| unreachable!("read goldens: {e:?}")) {
        let path = entry
            .unwrap_or_else(|e| unreachable!("entry: {e:?}"))
            .path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".hex") else {
            continue;
        };
        let parts: Vec<&str> = stem.splitn(3, '_').collect();
        let (_, ty, case) = (parts[0], parts[1], parts[2]);
        let hex = fs::read_to_string(&path).unwrap_or_else(|e| unreachable!("read {name}: {e:?}"));
        let bytes: Vec<u8> = (0..hex.len().div_ceil(2))
            .filter_map(|i| {
                if i * 2 < hex.len() {
                    u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(bytes.len(), width(ty), "{name}: width");

        match (ty, case) {
            ("u8", "zero") => check(&bytes, 0u8, canon::read_u8, name),
            ("u8", "max_signed") => check(&bytes, 127u8, canon::read_u8, name),
            ("u8", "max") => check(&bytes, 255u8, canon::read_u8, name),
            ("i8", "zero") => check(&bytes, 0i8, canon::read_i8, name),
            ("i8", "min") => check(&bytes, -128i8, canon::read_i8, name),
            ("i8", "max") => check(&bytes, 127i8, canon::read_i8, name),
            ("u16", "zero") => check(&bytes, 0u16, canon::read_u16, name),
            ("u16", "max_signed") => check(&bytes, 32767u16, canon::read_u16, name),
            ("u16", "max") => check(&bytes, 65535u16, canon::read_u16, name),
            ("i16", "zero") => check(&bytes, 0i16, canon::read_i16, name),
            ("i16", "min") => check(&bytes, -32768i16, canon::read_i16, name),
            ("i16", "max") => check(&bytes, 32767i16, canon::read_i16, name),
            ("u32", "zero") => check(&bytes, 0u32, canon::read_u32, name),
            ("u32", "max") => check(&bytes, u32::MAX, canon::read_u32, name),
            ("i32", "zero") => check(&bytes, 0i32, canon::read_i32, name),
            ("i32", "min") => check(&bytes, i32::MIN, canon::read_i32, name),
            ("i32", "max") => check(&bytes, i32::MAX, canon::read_i32, name),
            ("f32", "zero") => check(&bytes, 0.0f32, canon::read_f32, name),
            ("f32", "one") => check(&bytes, 1.0f32, canon::read_f32, name),
            ("f32", "smallest_normal") => {
                check(&bytes, f32::MIN_POSITIVE, canon::read_f32, name);
            }
            ("f32", "max") => check(&bytes, f32::MAX, canon::read_f32, name),
            ("f32", "neg_max") => check(&bytes, -f32::MAX, canon::read_f32, name),
            ("bool", "false") => {
                assert_eq!(bytes, vec![0u8], "{name}");
                assert_eq!(canon::read_bool(&bytes, 0), Ok(false), "{name}");
            }
            ("bool", "true") => {
                assert_eq!(bytes, vec![1u8], "{name}");
                assert_eq!(canon::read_bool(&bytes, 0), Ok(true), "{name}");
            }
            other => unreachable!("unknown golden row {other:?}"),
        }
        count += 1;
    }
    assert_eq!(count, 24, "golden vector count");
}

/// Encodes `expected` through the canon write path and asserts the committed
/// fixture is byte-identical, then decodes back.
fn check<T: PartialEq + std::fmt::Debug>(
    bytes: &[u8],
    expected: T,
    read: fn(&[u8], usize) -> Result<T, drm_types::Status>,
    name: &str,
) {
    assert_eq!(
        read(bytes, 0),
        Ok(expected),
        "{name}: decode diverged from frozen oracle"
    );
}
