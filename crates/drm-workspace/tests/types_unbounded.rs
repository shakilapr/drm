//! Unbounded-structure rejection gate (work-plan M01W4-E2-T2; canon-codec
//! REQ-TOOL-002): every data-plane structure without a structural bound fails
//! at parse with its closed-vocabulary reason. 100% asserted.
//! REQ: REQ-TOOL-002, REQ-TOOL-017.

use drm_workspace::parser::{Reason, parse_bytes};

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — bare `string` ⇒ `unbounded-string`.
#[test]
fn types_unbounded_bare_string() {
    let Err(d) = parse_bytes(b"s: string\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedString);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — bare `bytes` ⇒ `unbounded-bytes`.
#[test]
fn types_unbounded_bare_bytes() {
    let Err(d) = parse_bytes(b"b: bytes\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedBytes);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — sizeless array `T[]` ⇒ `unbounded-array`.
#[test]
fn types_unbounded_sizeless_array() {
    let Err(d) = parse_bytes(b"a: u8[]\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedArray);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — boundless `T[<=]` ⇒ `unbounded-array`.
#[test]
fn types_unbounded_bare_bounded() {
    let Err(d) = parse_bytes(b"a: u8[<=]\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedArray);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — `string[<=N]` is not a legal form ⇒
/// `unbounded-string`.
#[test]
fn types_unbounded_string_leq_form() {
    let Err(d) = parse_bytes(b"s: string[<=4]\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedString);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — `bytes[N]` is not a legal form ⇒
/// `unbounded-bytes`.
#[test]
fn types_unbounded_bytes_fixed_form() {
    let Err(d) = parse_bytes(b"b: bytes[4]\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::UnboundedBytes);
}

/// REQ: REQ-TOOL-002, REQ-TOOL-017 — empty enum ⇒ `enum-empty` (closed
/// vocabulary; no variant space = no structural bound).
#[test]
fn types_unbounded_empty_enum() {
    let Err(d) = parse_bytes(b"e: enum{}\n") else {
        unreachable!("must reject");
    };
    assert_eq!(d[0].reason, Reason::EnumEmpty);
}

/// REQ: REQ-TOOL-002 — every unbounded form in this gate is rejected: 7/7.
#[test]
fn types_unbounded_count_assert() {
    let forms: [&[u8]; 7] = [
        b"s: string\n",
        b"b: bytes\n",
        b"a: u8[]\n",
        b"a: u8[<=]\n",
        b"s: string[<=4]\n",
        b"b: bytes[4]\n",
        b"e: enum{}\n",
    ];
    let rejected = forms.iter().filter(|src| parse_bytes(src).is_err()).count();
    assert_eq!(rejected, forms.len());
}
