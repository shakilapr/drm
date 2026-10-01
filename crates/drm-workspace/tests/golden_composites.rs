//! Golden composite runner (canon-codec.md I1/I9; work-plan M01W4-E4-T3):
//! six hand-authored two-region vectors. Every expectation — layout, counts,
//! offsets, values, `sample_len` — is re-declared here as an independent frozen
//! oracle and byte-diffed against the committed `.hex` fixtures; then each
//! fixture decodes back through the toolchain codec.
//! REQ: REQ-TOOL-042, REQ-TOOL-040, REQ-TOOL-001.

use drm_workspace::canon::value::{Value, decode, encode};
use drm_workspace::parser::parse_bytes;

/// Fixture bytes by file stem (committed `.hex`, lower-case, no separators).
fn hex_bytes(stem: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/golden/composites/{stem}.hex"));
    let hex =
        std::fs::read_to_string(&path).unwrap_or_else(|e| unreachable!("read {stem}.hex: {e:?}"));
    let mut out = Vec::new();
    let mut it = hex.trim().chars();
    while let (Some(hi), Some(lo)) = (it.next(), it.next()) {
        let byte = u8::from_str_radix(&hi.to_string(), 16)
            .and_then(|h| u8::from_str_radix(&lo.to_string(), 16).map(|l| (h << 4) | l))
            .unwrap_or_else(|e| unreachable!("hex nibble in {stem}: {e}"));
        out.push(byte);
    }
    out
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040 — C01: `{u8 a, u64 b}` full padding.
#[test]
fn golden_composites_001_padded() {
    let expected = vec![
        0x5A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
        0x11,
    ];
    let committed = hex_bytes("001_u8_u64_padded");
    assert_eq!(committed, expected, "C01 committed bytes");
    let fs = parse_bytes(b"a: u8\nb: u64\n")
        .unwrap_or_else(|d| unreachable!("C01 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 32];
    let n = encode(
        &fs,
        &[Value::U8(0x5A), Value::U64(0x1122_3344_5566_7788)],
        &mut dst,
    )
    .unwrap_or_else(|e| unreachable!("C01 encode: {e:?}"));
    assert_eq!(n, 16);
    assert_eq!(&dst[..16], &committed[..]);
    let (vals, _) = match decode(&fs, &committed) {
        Ok(x) => x,
        Err(e) => unreachable!("C01 decode: {e:?}"),
    };
    assert_eq!(vals[0], Value::U8(0x5A));
    assert_eq!(vals[1], Value::U64(0x1122_3344_5566_7788));
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040 — C02: two-region, 2 variable members,
/// counts (3, 2); bodies contiguous with 1 pad byte before the u32 body.
#[test]
fn golden_composites_002_two_region() {
    let committed = hex_bytes("002_two_region_str_bounded");
    assert_eq!(committed.len(), 32);
    // Hand-declared structural bytes (frozen oracle):
    assert_eq!(&committed[0..2], &[0x92, 0x10]); // t = 4242 LE
    assert_eq!(&committed[4..8], &[3, 0, 0, 0]); // s.count
    assert_eq!(&committed[8..12], &[20, 0, 0, 0]); // s.offset = 20
    assert_eq!(&committed[12..16], &[2, 0, 0, 0]); // v.count
    assert_eq!(&committed[16..20], &[24, 0, 0, 0]); // v.offset = 24
    assert_eq!(&committed[20..23], b"abc"); // s body
    assert_eq!(&committed[24..32], &[1, 0, 0, 0, 2, 0, 0, 0]); // v body
    let fs = parse_bytes(b"t: u16\ns: string[8]\nv: u32[<=4]\n")
        .unwrap_or_else(|d| unreachable!("C02 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 44];
    let n = encode(
        &fs,
        &[
            Value::U16(4242),
            Value::Str("abc".into()),
            Value::Bounded(vec![Value::U32(1), Value::U32(2)]),
        ],
        &mut dst,
    )
    .unwrap_or_else(|e| unreachable!("C02 encode: {e:?}"));
    assert_eq!(n, 32);
    assert_eq!(&dst[..32], &committed[..]);
}

/// REQ: REQ-TOOL-042 — C03: `string[16]` with 6 UTF-8 bytes (é = C3 A9);
/// byte length counts bytes, not characters (I10).
#[test]
fn golden_composites_003_utf8() {
    let committed = hex_bytes("003_string16_utf8");
    assert_eq!(
        committed,
        vec![6, 0, 0, 0, 0x68, 0xC3, 0xA9, 0x6C, 0x6C, 0x6F]
    );
    let fs = parse_bytes(b"s: string[16]\n")
        .unwrap_or_else(|d| unreachable!("C03 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 20];
    let n = encode(&fs, &[Value::Str("héllo".into())], &mut dst)
        .unwrap_or_else(|e| unreachable!("C03 encode: {e:?}"));
    assert_eq!(n, 10);
    assert_eq!(&dst[..10], &committed[..]);
    let (vals, _) = match decode(&fs, &committed) {
        Ok(x) => x,
        Err(e) => unreachable!("C03 decode: {e:?}"),
    };
    assert_eq!(vals[0], Value::Str("héllo".into()));
}

/// REQ: REQ-TOOL-040 — C04: `T[<=N]` of u64: single-variable descriptor has no
/// offset field; body element-aligned at 8 (absolute).
#[test]
fn golden_composites_004_elem_aligned() {
    let committed = hex_bytes("004_bounded_u64_aligned");
    assert_eq!(
        committed,
        vec![0x00, 0x00, 0xC0, 0x3F, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]
    );
    let fs = parse_bytes(b"x: f32\nv: u64[<=2]\n")
        .unwrap_or_else(|d| unreachable!("C04 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 24];
    let n = encode(
        &fs,
        &[Value::F32(1.5), Value::Bounded(vec![Value::U64(7)])],
        &mut dst,
    )
    .unwrap_or_else(|e| unreachable!("C04 encode: {e:?}"));
    assert_eq!(n, 16);
    assert_eq!(&dst[..16], &committed[..]);
}

/// REQ: REQ-TOOL-042 — C05: empty `bytes[<=4]` body: count 0, zero tail bytes;
/// `sample_len` == `header_end` == 8.
#[test]
fn golden_composites_005_empty_body() {
    let committed = hex_bytes("005_empty_bodied");
    assert_eq!(committed, vec![7, 1, 0, 0, 0, 0, 0, 0]);
    let fs = parse_bytes(b"a: u8\nf: bool[2]\nb: bytes[<=4]\n")
        .unwrap_or_else(|d| unreachable!("C05 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 16];
    let n = encode(
        &fs,
        &[
            Value::U8(7),
            Value::Fixed(vec![Value::Bool(true), Value::Bool(false)]),
            Value::Bytes(vec![]),
        ],
        &mut dst,
    )
    .unwrap_or_else(|e| unreachable!("C05 encode: {e:?}"));
    assert_eq!(n, 8);
    assert_eq!(&dst[..8], &committed[..]);
    let (vals, _) = match decode(&fs, &committed) {
        Ok(x) => x,
        Err(e) => unreachable!("C05 decode: {e:?}"),
    };
    assert_eq!(vals[0], Value::U8(7));
    assert_eq!(vals[2], Value::Bytes(vec![]));
}

/// REQ: REQ-TOOL-040 — C06: three variable members with interleaved fixed
/// fields (u32, u16): descriptor block 3×8 @8..32, bodies s@32, b@34, w@36;
/// declaration order preserved end-to-end.
#[test]
fn golden_composites_006_three_variable() {
    let committed = hex_bytes("006_three_variable_order");
    assert_eq!(committed.len(), 40);
    assert_eq!(&committed[0..4], &[1, 0, 0, 0]); // n = 1
    assert_eq!(&committed[4..6], &[5, 0]); // k = 5
    assert_eq!(&committed[8..12], &[2, 0, 0, 0]); // s.count
    assert_eq!(&committed[12..16], &[32, 0, 0, 0]); // s.offset
    assert_eq!(&committed[16..20], &[1, 0, 0, 0]); // b.count
    assert_eq!(&committed[20..24], &[34, 0, 0, 0]); // b.offset
    assert_eq!(&committed[24..28], &[1, 0, 0, 0]); // w.count
    assert_eq!(&committed[28..32], &[36, 0, 0, 0]); // w.offset
    assert_eq!(&committed[32..34], b"xy"); // s body
    assert_eq!(committed[34], 9); // b body
    assert_eq!(&committed[36..40], &[1, 0, 0, 0]); // w body
    let fs = parse_bytes(b"n: u32\ns: string[4]\nb: bytes[<=4]\nk: u16\nw: u32[<=2]\n")
        .unwrap_or_else(|d| unreachable!("C06 schema: {d:?}"))
        .fields;
    let mut dst = vec![0_u8; 48];
    let n = encode(
        &fs,
        &[
            Value::U32(1),
            Value::Str("xy".into()),
            Value::Bytes(vec![9]),
            Value::U16(5),
            Value::Bounded(vec![Value::U32(1)]),
        ],
        &mut dst,
    )
    .unwrap_or_else(|e| unreachable!("C06 encode: {e:?}"));
    assert_eq!(n, 40);
    assert_eq!(&dst[..40], &committed[..]);
    let (vals, _) = match decode(&fs, &committed) {
        Ok(x) => x,
        Err(e) => unreachable!("C06 decode: {e:?}"),
    };
    assert_eq!(vals[0], Value::U32(1));
    assert_eq!(vals[1], Value::Str("xy".into()));
    assert_eq!(vals[2], Value::Bytes(vec![9]));
    assert_eq!(vals[3], Value::U16(5));
    assert_eq!(vals[4], Value::Bounded(vec![Value::U32(1)]));
}

/// REQ: REQ-TOOL-042 — fixture count pinned: exactly 6 `.hex` vectors.
#[test]
fn golden_composites_count() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/golden/composites");
    let n = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| unreachable!("read composites dir: {e:?}"))
        .filter(|e| {
            let path = e
                .as_ref()
                .unwrap_or_else(|er| unreachable!("entry: {er:?}"))
                .path();
            path.extension().is_some_and(|x| x == "hex")
        })
        .count();
    assert_eq!(n, 6);
}
