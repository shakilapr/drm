//! Parser semantic round-trip proptest (schema-parser.md I4 + §Protocols):
//! for a generated AST-bounded [`TopicFile`], `parse(render(t)) == t`
//! (semantic equality) and `render` is idempotent. 10 000 accepted cases,
//! deterministic RNG (flake policy §4.2 R4).
//! REQ: REQ-TOOL-054, REQ-TOOL-055.

use drm_workspace::parser::{
    AnnotValue, Annotation, AttrKey, AttrValue, Attribute, EncodingSpec, Field, Prim, TopicFile,
    Type, parse_bytes, render_canonical,
};
use proptest::prelude::*;
use proptest::sample::select;

fn arb_prim() -> impl Strategy<Value = Prim> {
    select(vec![
        Prim::F32,
        Prim::F64,
        Prim::I8,
        Prim::I16,
        Prim::I32,
        Prim::I64,
        Prim::U8,
        Prim::U16,
        Prim::U32,
        Prim::U64,
        Prim::Bool,
    ])
}

fn arb_type() -> impl Strategy<Value = Type> {
    prop_oneof![
        4 => arb_prim().prop_map(Type::Prim),
        1 => (0..=4096u32).prop_map(|max| Type::String { max }),
        1 => (0..=4096u32).prop_map(|max| Type::Bytes { max }),
        1 => (arb_prim(), 0..=1024u32).prop_map(|(elem, n)| Type::FixedArray { elem, n }),
        1 => (arb_prim(), 0..=1024u32).prop_map(|(elem, max)| Type::Bounded { elem, max }),
        1 => (1..=256usize).prop_map(|n| Type::Enum {
            variants: (0..n).map(|i| format!("var_{i}")).collect(),
        }),
    ]
}

fn arb_number() -> impl Strategy<Value = f64> {
    any::<i32>().prop_map(|x| f64::from(x) / 4.0)
}

fn arb_annot_value() -> impl Strategy<Value = AnnotValue> {
    prop_oneof![
        3 => arb_number().prop_map(AnnotValue::Number),
        // Bare token: ident-start, no `,` `}` `:` or whitespace so lexing and
        // trimming reproduce the value verbatim.
        2 => "[a-z][a-z0-9/_.-]{0,8}".prop_map(AnnotValue::Token),
        2 => (arb_number(), arb_number()).prop_map(|(a, b)| AnnotValue::Range(a, b)),
        1 => "[^\"\n\r]{0,20}".prop_map(AnnotValue::Text),
    ]
}

fn arb_annotation() -> impl Strategy<Value = Annotation> {
    (
        select(vec!["unit", "gain", "range", "description", "u"]),
        arb_annot_value(),
    )
        .prop_map(|(key, value)| Annotation {
            key: key.to_string(),
            value,
            span: dummy_span(),
        })
}

fn dummy_span() -> drm_workspace::parser::diag::Span {
    drm_workspace::parser::diag::Span {
        line: 1,
        col: 1,
        offset: 0,
    }
}

fn arb_attrs() -> impl Strategy<Value = Vec<Attribute>> {
    (
        proptest::option::of(prop_oneof![
            Just(AttrValue::Latest),
            (1..=u32::MAX).prop_map(AttrValue::Depth),
        ]),
        proptest::option::of((1..=u32::MAX).prop_map(AttrValue::Size)),
        proptest::option::of(select(vec![AttrValue::Overwrite, AttrValue::Reject])),
        // Scale: positive, finite, two-decimal steps render canonically.
        proptest::option::of(
            (
                arb_prim(),
                (1..=10_000u32).prop_map(|x| f64::from(x) / 100.0),
            )
                .prop_map(|(ty, scale)| AttrValue::Encoding(EncodingSpec { ty, scale })),
        ),
    )
        .prop_map(|(history, pool, overflow, encoding)| {
            let mut attrs = Vec::new();
            for (key, value) in [
                (AttrKey::History, history),
                (AttrKey::PoolSize, pool),
                (AttrKey::Overflow, overflow),
                (AttrKey::Encoding, encoding),
            ] {
                if let Some(value) = value {
                    attrs.push(Attribute {
                        key,
                        value,
                        span: dummy_span(),
                    });
                }
            }
            attrs
        })
}

fn arb_topic_file() -> impl Strategy<Value = TopicFile> {
    (
        arb_attrs(),
        prop::collection::vec(
            (arb_type(), prop::collection::vec(arb_annotation(), 0..=4)),
            1..=8,
        ),
    )
        .prop_map(|(attrs, raw_fields)| TopicFile {
            attrs,
            fields: raw_fields
                .into_iter()
                .enumerate()
                .map(|(i, (ty, annotations))| Field {
                    name: format!("f{i}"),
                    ty,
                    annotations,
                    span: dummy_span(),
                })
                .collect(),
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// REQ: REQ-TOOL-054 — I4: `parse(render(t)) == t` and render idempotent.
    #[test]
    fn semantic_roundtrip(file in arb_topic_file()) {
        let s1 = render_canonical(&file);
        let t2 = parse_bytes(s1.as_bytes())
            .unwrap_or_else(|e| unreachable!("canonical render rejected: {e:?}"));
        prop_assert_eq!(&t2, &file);
        let s2 = render_canonical(&t2);
        prop_assert_eq!(s2, s1);
    }
}
