//! Canonical renderer, verbatim from
//! `docs/algorithms/toolchain/schema-parser.md` §Functions (`render_canonical`):
//! attrs first in fixed key order (`History`, `PoolSize`, `Overflow`,
//! `Encoding`), then fields in declaration order with annotations in
//! declaration order; numbers rendered canonically from the stored `f64`;
//! quoted and bare annotation values re-emitted verbatim. Pure function of the
//! AST ⇒ render is idempotent and the semantic round-trip (I4) holds.
//! REQ: REQ-TOOL-054, REQ-TOOL-055.

use super::{AnnotValue, AttrKey, AttrValue, TopicFile, Type};

/// Renders the deterministic canonical text of a parsed topic file.
/// REQ: REQ-TOOL-054.
#[must_use]
pub fn render_canonical(f: &TopicFile) -> String {
    let mut s = String::new();
    for key in [
        AttrKey::History,
        AttrKey::PoolSize,
        AttrKey::Overflow,
        AttrKey::Encoding,
    ] {
        if let Some(a) = f.attrs.iter().find(|a| a.key == key) {
            render_attr(&mut s, key, &a.value);
            s.push('\n');
        }
    }
    for field in &f.fields {
        s.push_str(&field.name);
        s.push_str(": ");
        render_type(&mut s, &field.ty);
        if !field.annotations.is_empty() {
            s.push_str(" {");
            for (i, a) in field.annotations.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&a.key);
                s.push_str(": ");
                render_annot_value(&mut s, &a.value);
            }
            s.push('}');
        }
        s.push('\n');
    }
    s
}

fn render_attr(s: &mut String, key: AttrKey, value: &AttrValue) {
    let name = match key {
        AttrKey::History => "history",
        AttrKey::PoolSize => "pool_size",
        AttrKey::Overflow => "overflow",
        AttrKey::Encoding => "encoding",
    };
    s.push('@');
    s.push_str(name);
    s.push_str(": ");
    match value {
        AttrValue::Latest => s.push_str("latest"),
        AttrValue::Depth(n) | AttrValue::Size(n) => s.push_str(&n.to_string()),
        AttrValue::Overwrite => s.push_str("overwrite"),
        AttrValue::Reject => s.push_str("reject"),
        AttrValue::Encoding(spec) => {
            s.push_str("{type: ");
            s.push_str(spec.ty.as_str());
            s.push_str(", scale: ");
            s.push_str(&number(spec.scale));
            s.push('}');
        }
    }
}

fn render_type(s: &mut String, ty: &Type) {
    match ty {
        Type::Prim(p) => s.push_str(p.as_str()),
        Type::String { max } => {
            s.push_str("string[");
            s.push_str(&max.to_string());
            s.push(']');
        }
        Type::Bytes { max } => {
            s.push_str("bytes[<=");
            s.push_str(&max.to_string());
            s.push(']');
        }
        Type::FixedArray { elem, n } => {
            s.push_str(elem.as_str());
            s.push('[');
            s.push_str(&n.to_string());
            s.push(']');
        }
        Type::Bounded { elem, max } => {
            s.push_str(elem.as_str());
            s.push_str("[<=");
            s.push_str(&max.to_string());
            s.push(']');
        }
        Type::Enum { variants } => {
            s.push_str("enum { ");
            for (i, v) in variants.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(v);
            }
            s.push_str(" }");
        }
    }
}

fn render_annot_value(s: &mut String, v: &AnnotValue) {
    match v {
        AnnotValue::Number(n) => s.push_str(&number(*n)),
        AnnotValue::Token(t) => s.push_str(t),
        AnnotValue::Range(a, b) => {
            s.push('[');
            s.push_str(&number(*a));
            s.push_str(", ");
            s.push_str(&number(*b));
            s.push(']');
        }
        AnnotValue::Text(t) => {
            s.push('"');
            s.push_str(t);
            s.push('"');
        }
    }
}

/// Canonical number rendering: the shortest decimal that round-trips the
/// stored `f64` (`0.10` → `0.1`).
/// REQ: REQ-TOOL-054.
fn number(v: f64) -> String {
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::super::{AnnotValue, Type, parse_text};
    use super::render_canonical;

    /// REQ: REQ-TOOL-054 — attribute order and canonical number rendering.
    #[test]
    fn canonical_order_and_numbers() {
        let src = "@overflow: reject\n@history: latest\n@encoding: {type: i16, scale: 0.10}\nspeed: f32 {unit: m/s, gain: 0.10}\n";
        let f = match parse_text(src) {
            Ok(f) => f,
            Err(e) => unreachable!("parse: {e:?}"),
        };
        let out = render_canonical(&f);
        assert_eq!(
            out,
            "@history: latest\n@overflow: reject\n@encoding: {type: i16, scale: 0.1}\nspeed: f32 {unit: m/s, gain: 0.1}\n"
        );
        let f2 = match parse_text(&out) {
            Ok(f) => f,
            Err(e) => unreachable!("reparse: {e:?}"),
        };
        assert_eq!(f2, f);
        assert_eq!(render_canonical(&f2), out);
    }

    /// REQ: REQ-TOOL-055 — annotation value forms are preserved verbatim.
    #[test]
    fn annotation_forms_preserved() {
        let src = "v: f32[3] {unit: m/s, range: [-2.75, 2.75], description: \"display name\"}\n";
        let f = match parse_text(src) {
            Ok(f) => f,
            Err(e) => unreachable!("parse: {e:?}"),
        };
        let a = &f.fields[0].annotations;
        assert_eq!(a[0].value, AnnotValue::Token("m/s".into()));
        assert_eq!(a[1].value, AnnotValue::Range(-2.75, 2.75));
        assert_eq!(a[2].value, AnnotValue::Text("display name".into()));
        assert_eq!(render_canonical(&f), src);
    }

    /// REQ: REQ-TOOL-054 — enum, bounded, and string type renderings.
    #[test]
    fn type_renderings() {
        let src = "e: enum { idle, run }\nb: bytes[<=16]\ns: string[8]\nq: u16[<=8]\nr: u8[4]\n";
        let f = match parse_text(src) {
            Ok(f) => f,
            Err(e) => unreachable!("parse: {e:?}"),
        };
        assert_eq!(render_canonical(&f), src);
        let e = match &f.fields[0].ty {
            Type::Enum { variants } => variants.clone(),
            t => unreachable!("enum expected, got {t:?}"),
        };
        assert_eq!(e, vec!["idle".to_string(), "run".to_string()]);
    }
}
