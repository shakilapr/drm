//! Type-triple checklist (encode/decode/layout per type family) and codec
//! hardening gates (work-plan M01W4-E2-T1; canon-codec I3/I5/I10).
//! REQ: REQ-TOOL-001, REQ-TOOL-042, REQ-TOOL-043.

use drm_types::Status;
use drm_workspace::canon::value::{Value, decode, encode};
use drm_workspace::parser::{Field, parse_bytes};

/// Fixture scaffolding: setup invariants only — never expect/unwrap (workspace
/// lint policy); `unreachable!` marks a fixture bug, not tested behavior.
fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated: {e:?}"),
    }
}

/// REQ: REQ-TOOL-034 — parse a one-field schema and return its single field.
fn field_of(ty: &str) -> Field {
    let schema = format!("v: {ty}\n");
    let mut fields = fixture(parse_bytes(schema.as_bytes())).fields;
    let Some(f) = fields.drain(..).next() else {
        unreachable!("one field");
    };
    f
}

/// REQ: REQ-TOOL-042 — round-trip: encode(value) → decode == value, and the
/// derived `sample_len` is returned by both sides.
fn roundtrip(ty: &str, v: Value) -> u64 {
    let f = field_of(ty);
    let fs = std::slice::from_ref(&f);
    let sl = fixture(drm_workspace::canon::layout_struct(fs));
    let cap = fixture(usize::try_from(sl.sample_max));
    let mut dst = vec![0_u8; cap];
    let args = [v];
    let Ok(n) = encode(fs, &args, &mut dst) else {
        unreachable!("encode accepts");
    };
    let (back, n2) = match decode(fs, &dst[..fixture(usize::try_from(n))]) {
        Ok(x) => x,
        Err(e) => unreachable!("decode accepts: {e:?}"),
    };
    assert_eq!(back[0], args[0]);
    assert_eq!(n, n2);
    n
}

/// u32→usize narrowing helper for fixture sample sizes (≤ `u32::MAX` by cap).
trait ExpectCap {
    fn expect_cap(self) -> usize;
}
impl ExpectCap for u64 {
    fn expect_cap(self) -> usize {
        match usize::try_from(self) {
            Ok(n) => n,
            Err(e) => unreachable!("sample ≤ `u32::MAX`: {e}"),
        }
    }
}

/// REQ: REQ-TOOL-042 — encode failure leaves every destination byte untouched
/// (I3 two-phase; 0xAA destination probe).
fn rejects_untouched(ty: &str, v: Value) {
    let f = field_of(ty);
    let mut dst = [0xAA_u8; 64];
    assert_eq!(
        encode(std::slice::from_ref(&f), &[v], &mut dst),
        Err(Status::InvalidArg)
    );
    assert!(dst.iter().all(|b| *b == 0xAA), "destination untouched");
}

/// REQ: REQ-TOOL-043 — integer prims: zero/min/max round-trip (range
/// enforcement lives at the schema/codegen literal boundary, not the codec).
#[test]
fn types_codec_int_prims() {
    assert_eq!(roundtrip("i8", Value::I8(i8::MIN)), 1);
    assert_eq!(roundtrip("i8", Value::I8(i8::MAX)), 1);
    assert_eq!(roundtrip("u8", Value::U8(0)), 1);
    assert_eq!(roundtrip("u8", Value::U8(u8::MAX)), 1);
    assert_eq!(roundtrip("i16", Value::I16(i16::MIN)), 2);
    assert_eq!(roundtrip("u16", Value::U16(u16::MAX)), 2);
    assert_eq!(roundtrip("i32", Value::I32(0)), 4);
    assert_eq!(roundtrip("i32", Value::I32(i32::MAX)), 4);
    assert_eq!(roundtrip("u32", Value::U32(u32::MAX)), 4);
    assert_eq!(roundtrip("i64", Value::I64(i64::MIN)), 8);
    assert_eq!(roundtrip("u64", Value::U64(u64::MAX)), 8);
}

/// REQ: REQ-TOOL-043 — floats: bit pattern preserved (zero/max/inf), layout
/// sizes 4/8.
#[test]
fn types_codec_floats() {
    assert_eq!(roundtrip("f32", Value::F32(0.0)), 4);
    assert_eq!(roundtrip("f32", Value::F32(f32::MAX)), 4);
    assert_eq!(roundtrip("f32", Value::F32(f32::INFINITY)), 4);
    assert_eq!(roundtrip("f64", Value::F64(f64::MIN)), 8);
    assert_eq!(roundtrip("f64", Value::F64(f64::MAX)), 8);
}

/// REQ: REQ-TOOL-044 — bool encode 0/1; decode strictly {0,1} (byte 2 ⇒
/// `InvalidData`), never UB.
#[test]
fn types_codec_bool_strict() {
    assert_eq!(roundtrip("bool", Value::Bool(false)), 1);
    assert_eq!(roundtrip("bool", Value::Bool(true)), 1);
    let f = field_of("bool");
    let mut dst = [0_u8; 4];
    assert!(encode(std::slice::from_ref(&f), &[Value::Bool(true)], &mut dst).is_ok());
    dst[0] = 2;
    assert_eq!(
        decode(std::slice::from_ref(&f), &dst),
        Err(Status::InvalidData)
    );
    dst[0] = 255;
    assert_eq!(
        decode(std::slice::from_ref(&f), &dst),
        Err(Status::InvalidData)
    );
}

/// REQ: REQ-TOOL-034 — enum: first/last tags accept; unknown tag rejects on
/// both encode (plan) and decode sides.
#[test]
fn types_codec_enum_tags() {
    assert_eq!(roundtrip("enum{off,on}", Value::Enum(0)), 1);
    assert_eq!(roundtrip("enum{off,on}", Value::Enum(1)), 1);
    rejects_untouched("enum{off,on}", Value::Enum(2));
    rejects_untouched("enum{off,on}", Value::Enum(255));
    let f = field_of("enum{off,on}");
    let mut dst = [0_u8; 4];
    assert!(encode(std::slice::from_ref(&f), &[Value::Enum(1)], &mut dst).is_ok());
    dst[0] = 2;
    assert_eq!(
        decode(std::slice::from_ref(&f), &dst),
        Err(Status::InvalidData)
    );
}

/// REQ: REQ-TOOL-001 — fixed arrays: exact length required; over-length
/// rejected before any write.
#[test]
fn types_codec_fixed_array() {
    let full = Value::Fixed(vec![Value::U8(1), Value::U8(2), Value::U8(3), Value::U8(4)]);
    assert_eq!(roundtrip("u8[4]", full), 4);
    rejects_untouched(
        "u8[4]",
        Value::Fixed(vec![Value::U8(1), Value::U8(2), Value::U8(3)]),
    );
    rejects_untouched(
        "u8[4]",
        Value::Fixed(vec![
            Value::U8(1),
            Value::U8(2),
            Value::U8(3),
            Value::U8(4),
            Value::U8(5),
        ]),
    );
}

/// REQ: REQ-TOOL-001 — bounded arrays: empty/full accept; over rejects;
/// decode rejects a patched over-max count (`InvalidData` pre-read).
#[test]
fn types_codec_bounded_array() {
    assert_eq!(roundtrip("u32[<=4]", Value::Bounded(vec![])), 4);
    let full = Value::Bounded(vec![
        Value::U32(1),
        Value::U32(2),
        Value::U32(3),
        Value::U32(4),
    ]);
    assert_eq!(roundtrip("u32[<=4]", full), 20);
    rejects_untouched(
        "u32[<=4]",
        Value::Bounded(vec![
            Value::U32(1),
            Value::U32(2),
            Value::U32(3),
            Value::U32(4),
            Value::U32(5),
        ]),
    );
    // Decode side: count descriptor patched to max+1 ⇒ reject.
    let f = field_of("u32[<=4]");
    let mut dst = [0_u8; 24];
    let n = encode(
        std::slice::from_ref(&f),
        &[Value::Bounded(vec![Value::U32(1), Value::U32(2)])],
        &mut dst,
    );
    let Ok(n) = n else {
        unreachable!("count 2 fits");
    };
    dst[0] = 3;
    assert_eq!(
        decode(std::slice::from_ref(&f), &dst[..n.expect_cap()]),
        Err(Status::InvalidData)
    );
}

/// REQ: REQ-TOOL-001, REQ-TOOL-042 — string/bytes: empty/full accept with
/// two-region `sample_len`; over rejects.
#[test]
fn types_codec_string_bytes_lengths() {
    assert_eq!(roundtrip("string[8]", Value::Str(String::new())), 4);
    assert_eq!(roundtrip("string[8]", Value::Str("12345678".into())), 12);
    rejects_untouched("string[8]", Value::Str("123456789".into()));
    assert_eq!(roundtrip("bytes[<=8]", Value::Bytes(vec![])), 4);
    assert_eq!(
        roundtrip("bytes[<=8]", Value::Bytes(vec![9, 8, 7, 6, 5, 4, 3, 2])),
        12
    );
    rejects_untouched("bytes[<=8]", Value::Bytes(vec![0; 9]));
}

/// REQ: REQ-TOOL-042, REQ-TOOL-001 — UTF-8 strictness (I10): byte length counts
/// UTF-8 bytes, not characters; malformed sequences rejected on decode.
#[test]
fn types_codec_string_utf8_matrix() {
    // 4 characters, 8 UTF-8 bytes (1+2+3+2) — fits string[8] exactly.
    assert_eq!(roundtrip("string[8]", Value::Str("aé€£".into())), 12);
    // 9 UTF-8 bytes in 5 characters — exceeds string[8].
    rejects_untouched("string[8]", Value::Str("aé€£日".into()));
    let f = field_of("string[8]");
    // Decode: valid descriptor over malformed bodies ⇒ InvalidData (I10).
    // Rust `String` values are always valid UTF-8, so encode-side malformed
    // rejection is structural; the byte matrix exercises decode.
    let malformed: [&[u8]; 6] = [
        &[0xC0, 0xAF],             // lone continuation
        &[0xE0, 0x80, 0x80],       // overlong 3-byte
        &[0xED, 0xA0, 0x80],       // surrogate
        &[0xF5, 0x80, 0x80, 0x80], // > U+10FFFF
        &[0xC3],                   // truncated multibyte
        &[0xF0, 0x9F],             // truncated 4-byte
    ];
    for bad in malformed {
        let mut dst = [0_u8; 12];
        let Ok(n) = encode(
            std::slice::from_ref(&f),
            &[Value::Str("abcd".into())],
            &mut dst,
        ) else {
            unreachable!("valid utf-8 baseline");
        };
        let mut patched = dst[..n.expect_cap()].to_vec();
        for (j, b) in bad.iter().enumerate() {
            patched[4 + j] = *b;
        }
        assert_eq!(
            decode(std::slice::from_ref(&f), &patched),
            Err(Status::InvalidData),
            "{bad:?}"
        );
    }
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040 — two-region integration: `{string[4],
/// bytes[<=4]}`: descriptors 2×8 @0..16, bodies declaration-ordered and
/// contiguous (a@16, b@16+len(a)); empty-everything `sample_len` = 16.
#[test]
fn types_codec_two_region_packing() {
    let fs = fixture(parse_bytes("a: string[4]\nb: bytes[<=4]\n".as_bytes())).fields;
    let mut dst = vec![0xAA_u8; 24];
    let n = encode(
        &fs,
        &[Value::Str("ab".into()), Value::Bytes(vec![9, 8])],
        &mut dst,
    );
    let Ok(n) = n else {
        unreachable!("encode ok");
    };
    assert_eq!(n, 20);
    // Descriptors: counts 2,2 at 0 and 8; stored offsets 16 and 18 at 4 and 12.
    assert_eq!(&dst[0..8], &[2, 0, 0, 0, 16, 0, 0, 0]);
    assert_eq!(&dst[8..16], &[2, 0, 0, 0, 18, 0, 0, 0]);
    assert_eq!(&dst[16..18], b"ab");
    assert_eq!(&dst[18..20], &[9, 8]);
    // Canonical determinism: the whole sample was written (0xAA probe gone),
    // every gap byte exactly 0; bytes at ≥ sample_len stay untouched.
    assert!(dst[..n.expect_cap()].iter().all(|b| *b != 0xAA));
    assert_eq!(dst[n.expect_cap()..], [0xAA_u8; 4]);
    let (vals, n2) = match decode(&fs, &dst[..n.expect_cap()]) {
        Ok((v, n2)) => (v, n2),
        Err(e) => unreachable!("decode ok: {e:?}"),
    };
    assert_eq!(vals[0], Value::Str("ab".into()));
    assert_eq!(vals[1], Value::Bytes(vec![9, 8]));
    assert_eq!(n, n2);
    // Empty-everything: sample_len == header_end.
    let n0 = encode(
        &fs,
        &[Value::Str(String::new()), Value::Bytes(vec![])],
        &mut dst,
    );
    let Ok(n0) = n0 else {
        unreachable!("empty ok");
    };
    assert_eq!(n0, 16);
    // Stored-offset tampering ⇒ reject (canonical packing is exact).
    let n = encode(
        &fs,
        &[Value::Str("ab".into()), Value::Bytes(vec![1, 2, 3])],
        &mut dst,
    );
    let Ok(n) = n else {
        unreachable!("encode ok");
    };
    let mut bad = dst[..n.expect_cap()].to_vec();
    bad[12] += 1;
    assert_eq!(decode(&fs, &bad), Err(Status::InvalidData));
}

/// REQ: REQ-CORE-081 — message-level two-phase: late-field over-bound fails
/// the whole sample and writes nothing (I3 `no_partial_write`, 0xAA probe).
#[test]
fn types_codec_no_partial_write() {
    let fs = fixture(parse_bytes("x: u32\ns: string[4]\n".as_bytes())).fields;
    let mut dst = [0xAA_u8; 32];
    assert_eq!(
        encode(&fs, &[Value::U32(7), Value::Str("abcde".into())], &mut dst),
        Err(Status::InvalidArg)
    );
    assert!(dst.iter().all(|b| *b == 0xAA));
}

/// REQ: REQ-CORE-083 — structural decode rejects: short buffer, trailing
/// bytes, all before any value read (I5).
#[test]
fn types_codec_decode_structure_rejects() {
    let f = field_of("string[8]");
    let mut dst = [0_u8; 12];
    let n = encode(
        std::slice::from_ref(&f),
        &[Value::Str("abc".into())],
        &mut dst,
    );
    let Ok(n) = n else {
        unreachable!("encode ok");
    };
    let good = &dst[..n.expect_cap()];
    assert_eq!(n, 7);
    assert_eq!(
        decode(std::slice::from_ref(&f), &good[..good.len() - 1]),
        Err(Status::InvalidData)
    );
    assert_eq!(
        decode(std::slice::from_ref(&f), &good[..2]),
        Err(Status::InvalidData)
    );
    // Trailing byte after the derived sample_len ⇒ reject (no trailing bytes).
    let mut longer = good.to_vec();
    longer.push(0);
    assert_eq!(
        decode(std::slice::from_ref(&f), &longer),
        Err(Status::InvalidData)
    );
}

/// REQ: REQ-TOOL-043, REQ-TOOL-042 — prim wire widths match the canonical
/// layout table (type-triple layout side).
#[test]
fn types_codec_prim_layout_sizes() {
    for (name, size, align) in [
        ("bool", 1, 1),
        ("i8", 1, 1),
        ("u8", 1, 1),
        ("i16", 2, 2),
        ("u16", 2, 2),
        ("i32", 4, 4),
        ("u32", 4, 4),
        ("f32", 4, 4),
        ("i64", 8, 8),
        ("u64", 8, 8),
        ("f64", 8, 8),
    ] {
        let f = field_of(name);
        let l = fixture(drm_workspace::canon::layout(&f.ty));
        assert_eq!((l.size, l.align), (size, align), "{name}");
    }
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040 - alignment gaps between variable bodies are
/// exactly zero on encode (canonical determinism, I1/I8).
#[test]
fn types_codec_gap_zeroing() {
    let fs = fixture(parse_bytes(
        "a: u32\ns: string[2]\nv: u64[<=1]\n".as_bytes(),
    ))
    .fields;
    let mut dst = vec![0xAA_u8; 48];
    let n = encode(
        &fs,
        &[
            Value::U32(9),
            Value::Str("z".into()),
            Value::Bounded(vec![Value::U64(5)]),
        ],
        &mut dst,
    );
    let Ok(n) = n else {
        unreachable!("encode ok");
    };
    assert_eq!(n, 32);
    // s body @20 (1 byte), gap @21..24 zeroed, v body @24 (element-aligned).
    assert_eq!(dst[20], b'z');
    assert_eq!(&dst[21..24], &[0, 0, 0]);
    assert_eq!(dst[24], 5);
    assert_eq!(&dst[32..], [0xAA_u8; 16]);
}
