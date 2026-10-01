//! Layout-calc v1 fixtures (work-plan M01W4-E2-T3; canon-codec §Functions
//! `layout_struct`). Every expected value is hand-computed from
//! docs/algorithms/canon-codec.md §Data Structures.
//! REQ: REQ-TOOL-042, REQ-TOOL-041, REQ-TOOL-040.

use drm_types::Status;
use drm_workspace::canon::{Region, layout, layout_struct};
use drm_workspace::parser::{Field, parse_bytes};

/// Fixture scaffolding: setup invariants only — never expect/unwrap (workspace
/// lint policy); `unreachable!` marks a fixture bug, not tested behavior.
fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated: {e:?}"),
    }
}

/// REQ: REQ-TOOL-042 — one-field schemas parsed from canonical text.
fn fields_of(schema: &str) -> Vec<Field> {
    fixture(parse_bytes(schema.as_bytes())).fields
}

/// REQ: REQ-TOOL-042 — `{f32,u64}`: f32@0, pad 4, u64@8, size 16, align 8.
#[test]
fn types_layout_f32_u64() {
    let sl = fixture(layout_struct(&fields_of("a: f32\nb: u64\n")));
    assert_eq!(sl.header_end, 16);
    assert_eq!(sl.sample_max, 16);
    assert_eq!(sl.struct_align, 8);
    assert_eq!(sl.members.len(), 2);
    assert_eq!(sl.members[0].name, "a");
    assert_eq!(sl.members[0].offset, 0);
    assert_eq!(sl.members[0].size, 4);
    assert_eq!(sl.members[0].align, 4);
    assert_eq!(sl.members[0].region, Region::Header);
    assert_eq!(sl.members[1].name, "b");
    assert_eq!(sl.members[1].offset, 8);
    assert_eq!(sl.members[1].size, 8);
    assert_eq!(sl.members[1].align, 8);
    assert_eq!(sl.members[1].region, Region::Header);
}

/// REQ: REQ-TOOL-042 — `{bool,u32}`: bool@0, pad 3, u32@4, size 8, align 4.
#[test]
fn types_layout_bool_u32() {
    let sl = fixture(layout_struct(&fields_of("a: bool\nb: u32\n")));
    assert_eq!(sl.header_end, 8);
    assert_eq!(sl.sample_max, 8);
    assert_eq!(sl.struct_align, 4);
    assert_eq!(sl.members[0].offset, 0);
    assert_eq!(sl.members[0].size, 1);
    assert_eq!(sl.members[1].offset, 4);
    assert_eq!(sl.members[1].size, 4);
}

/// REQ: REQ-TOOL-042 — `string[16]` alone: descriptor `len:u32`@0 (single
/// variable member ⇒ no offset field), `header_end` 4, body@4, size 20, align 4.
#[test]
fn types_layout_string16() {
    let sl = fixture(layout_struct(&fields_of("s: string[16]\n")));
    assert_eq!(sl.header_end, 4);
    assert_eq!(sl.sample_max, 20);
    assert_eq!(sl.struct_align, 4);
    assert_eq!(sl.members.len(), 2);
    assert_eq!(sl.members[0].region, Region::Descriptor);
    assert_eq!(sl.members[0].offset, 0);
    assert_eq!(sl.members[0].size, 4);
    assert_eq!(sl.members[1].region, Region::Body);
    assert_eq!(sl.members[1].offset, 4);
    assert_eq!(sl.members[1].size, 16);
}

/// REQ: REQ-TOOL-042, REQ-TOOL-040 — two-region packing with 2 variable
/// members: `{u16, string[8], u32[<=4]}` ⇒ u16@0, descriptors 2×8 @4..20,
/// `header_end` 20, string body@20 (max 8), u32 body@28 (align 4, max 16),
/// `sample_max` 44, `struct_align` 4; body offsets in declaration order.
#[test]
fn types_layout_two_region() {
    let sl = fixture(layout_struct(&fields_of(
        "t: u16\ns: string[8]\na: u32[<=4]\n",
    )));
    assert_eq!(sl.header_end, 20);
    assert_eq!(sl.sample_max, 44);
    assert_eq!(sl.struct_align, 4);
    assert_eq!(sl.members.len(), 5);
    assert_eq!(sl.members[0].name, "t");
    assert_eq!(sl.members[0].offset, 0);
    assert_eq!(sl.members[1].name, "s");
    assert_eq!(sl.members[1].region, Region::Descriptor);
    assert_eq!(sl.members[1].offset, 4);
    assert_eq!(sl.members[1].size, 8);
    assert_eq!(sl.members[2].name, "a");
    assert_eq!(sl.members[2].region, Region::Descriptor);
    assert_eq!(sl.members[2].offset, 12);
    assert_eq!(sl.members[2].size, 8);
    assert_eq!(sl.members[3].name, "s");
    assert_eq!(sl.members[3].region, Region::Body);
    assert_eq!(sl.members[3].offset, 20);
    assert_eq!(sl.members[3].size, 8);
    assert_eq!(sl.members[4].name, "a");
    assert_eq!(sl.members[4].region, Region::Body);
    assert_eq!(sl.members[4].offset, 28);
    assert_eq!(sl.members[4].size, 16);
}

/// REQ: REQ-TOOL-040 — `T[<=N]` body element-aligned: `{u8, u64[<=2]}` ⇒
/// `header_end` 8 (u8@0, single descriptor@4), body@8 (align 8), `sample_max` 24.
#[test]
fn types_layout_bounded_elem_aligned() {
    let sl = fixture(layout_struct(&fields_of("a: u8\nv: u64[<=2]\n")));
    assert_eq!(sl.header_end, 8);
    assert_eq!(sl.struct_align, 8);
    let body = &sl.members[2];
    assert_eq!(body.region, Region::Body);
    assert_eq!(body.offset, 8);
    assert_eq!(body.size, 16);
    assert_eq!(body.align, 8);
}

/// REQ: REQ-TOOL-042 — stable order: header fields, then descriptors, then
/// bodies, each group in declaration order.
#[test]
fn types_layout_stable_member_order() {
    let sl = fixture(layout_struct(&fields_of(
        "a: bytes[<=2]\nb: u8\nc: string[4]\nd: bool[2]\n",
    )));
    let seq: Vec<(String, Region)> = sl
        .members
        .iter()
        .map(|m| (m.name.clone(), m.region))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("b".into(), Region::Header),
            ("d".into(), Region::Header),
            ("a".into(), Region::Descriptor),
            ("c".into(), Region::Descriptor),
            ("a".into(), Region::Body),
            ("c".into(), Region::Body),
        ]
    );
}

/// REQ: REQ-TOOL-041, REQ-TOOL-001 — canonical sample size beyond the `u32::MAX`
/// MVP cap ⇒ `InvalidConfig` (checked arithmetic, never wrapped).
#[test]
fn types_layout_sample_cap_overflow() {
    let fs = fields_of("a: u64[4294967295]\n");
    assert_eq!(layout_struct(&fs).map(|_| ()), Err(Status::InvalidConfig));
    let fs2 = fields_of("a: u8[4294967295]\n");
    assert_eq!(fixture(layout(&fs2[0].ty)).size, 4_294_967_295);
}
