//! Type-universe property harness (work-plan M02W1-E4-T2; canon-codec I1/I3/I8):
//! random type + random value → encode → decode → identity, plus encode
//! determinism and gap-zeroing over a dirty destination. 10 000 cases.
//! REQ: REQ-TOOL-001, REQ-TOOL-042, REQ-TOOL-043.

use drm_workspace::canon::layout_struct;
use drm_workspace::canon::value::{Value, decode, encode};
use drm_workspace::parser::parse_bytes;

use proptest::prelude::*;

/// Schema type text for one field, from the closed type universe (grammar:
/// depth ≤ 1).
fn arb_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("bool".to_string()),
        Just("i8".to_string()),
        Just("u8".to_string()),
        Just("i16".to_string()),
        Just("u16".to_string()),
        Just("i32".to_string()),
        Just("u32".to_string()),
        Just("f32".to_string()),
        Just("i64".to_string()),
        Just("u64".to_string()),
        Just("f64".to_string()),
        (
            0u32..=8,
            prop::option::of(Just("<=")),
            prop::sample::select(vec!["u8", "i32", "u64"])
        )
            .prop_map(|(n, bounded, elem)| match bounded {
                Some(_) => format!("{elem}[<={n}]"),
                None => format!("{elem}[{n}]"),
            }),
        Just("string[8]".to_string()),
        Just("bytes[<=8]".to_string()),
        Just("enum{off,on,unknown}".to_string()),
    ]
}

/// Random printable-ASCII string (valid UTF-8 by construction).
fn arb_ascii(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0x20_u8..0x7F, 0..=max)
        .prop_map(|b| String::from_utf8(b).unwrap_or_else(|e| unreachable!("ascii is utf-8: {e}")))
}

/// A random value inhabiting the given schema type.
fn arb_value(ty: &str) -> BoxedStrategy<Value> {
    match ty {
        "bool" => any::<bool>().prop_map(Value::Bool).boxed(),
        "i8" => any::<i8>().prop_map(Value::I8).boxed(),
        "u8" => any::<u8>().prop_map(Value::U8).boxed(),
        "i16" => any::<i16>().prop_map(Value::I16).boxed(),
        "u16" => any::<u16>().prop_map(Value::U16).boxed(),
        "i32" => any::<i32>().prop_map(Value::I32).boxed(),
        "u32" => any::<u32>().prop_map(Value::U32).boxed(),
        "f32" => any::<f32>()
            .prop_filter("no NaN (equality oracle)", |f| !f.is_nan())
            .prop_map(Value::F32)
            .boxed(),
        "i64" => any::<i64>().prop_map(Value::I64).boxed(),
        "u64" => any::<u64>().prop_map(Value::U64).boxed(),
        "f64" => any::<f64>()
            .prop_filter("no NaN (equality oracle)", |f| !f.is_nan())
            .prop_map(Value::F64)
            .boxed(),
        "string[8]" => arb_ascii(8).prop_map(Value::Str).boxed(),
        "bytes[<=8]" => prop::collection::vec(any::<u8>(), 0..=8)
            .prop_map(Value::Bytes)
            .boxed(),
        "enum{off,on,unknown}" => (0_u8..3).prop_map(Value::Enum).boxed(),
        other => {
            let Some(bracket) = other.find('[') else {
                unreachable!("universe type {other}");
            };
            let elem = &other[..bracket];
            let spec = &other[bracket + 1..other.len() - 1];
            let elem_val: BoxedStrategy<Value> = match elem {
                "u8" => any::<u8>().prop_map(Value::U8).boxed(),
                "i32" => any::<i32>().prop_map(Value::I32).boxed(),
                "u64" => any::<u64>().prop_map(Value::U64).boxed(),
                bad => unreachable!("universe elem {bad}"),
            };
            let Some(n_str) = spec.strip_prefix("<=") else {
                let n: usize = spec
                    .parse()
                    .unwrap_or_else(|e| unreachable!("universe N: {e}"));
                return prop::collection::vec(elem_val, n..=n)
                    .prop_map(Value::Fixed)
                    .boxed();
            };
            let n: usize = n_str
                .parse()
                .unwrap_or_else(|e| unreachable!("universe N: {e}"));
            prop::collection::vec(elem_val, 0..=n)
                .prop_map(Value::Bounded)
                .boxed()
        }
    }
}

/// Coupled (type, value) strategy: the value always inhabits the type.
fn type_and_value() -> BoxedStrategy<(String, Value)> {
    arb_type()
        .prop_flat_map(|ty| {
            let v = arb_value(&ty);
            (Just(ty.clone()), v)
        })
        .boxed()
}

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated: {e:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// REQ: REQ-TOOL-001, REQ-TOOL-042 — type universe: encode → decode identity,
    /// encode determinism independent of prior destination content, gap zeroing,
    /// and bytes ≥ `sample_len` never observed (I8).
    #[test]
    fn property_types_roundtrip((ty, v) in type_and_value()) {
        let schema = format!("v: {ty}\n");
        let fs = fixture(parse_bytes(schema.as_bytes())).fields;
        let sl = fixture(layout_struct(&fs));
        let cap = fixture(usize::try_from(sl.sample_max));
        let mut dst1 = vec![0xAA_u8; cap];
        let mut dst2 = vec![0x55_u8; cap];
        let n1 = fixture(encode(&fs, &[v.clone()], &mut dst1));
        let n2 = fixture(encode(&fs, &[v.clone()], &mut dst2));
        prop_assert_eq!(n1, n2);
        let n = fixture(usize::try_from(n1));
        prop_assert_eq!(&dst1[..n], &dst2[..n]);
        let (back, n3) = match decode(&fs, &dst1[..n]) {
            Ok(x) => x,
            Err(e) => unreachable!("decode accepts: {e:?}"),
        };
        prop_assert_eq!(n3, n1);
        prop_assert_eq!(&back[0], &v);
        prop_assert!(dst1[n..].iter().all(|b| *b == 0xAA), "tail unobserved (I8)");
    }
}
