//! I6 layout-fixture exchange ([canon-codec.md §Protocols](../../../docs/algorithms/canon-codec.md)):
//! deterministic JSON render of a two-region layout — the crosscheck artifact
//! `drm-core` consumes to validate its runtime codec against this toolchain
//! layout computation (zero mismatches, permanent gate).
//! REQ: REQ-TOOL-042, REQ-CORE-081.

use std::fmt::Write as _;

use crate::canon::{Region, StructLayout, variable_parts};
use crate::parser::Field;

/// Deterministic JSON render of the layout (`emit_layout_json`, canon-codec
/// §Functions): per-slot `{name, region, offset, size, align}` (body offsets
/// at declared max), plus `header_end`, `sample_max`, `struct_align`, and the
/// variable-member re-derivation inputs (`variables`: declared max, element
/// size, element alignment — everything a runtime needs to re-derive the
/// canonical packing from counts and compare slot-for-slot).
/// REQ: REQ-TOOL-042.
#[must_use]
pub fn emit_layout_json(schema: &str, fields: &[Field], sl: &StructLayout) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("{\n");
    out.push_str(
        "  \"banner\": \"@generated — regenerate: DRM_LAYOUT_REGEN=1 cargo test -p drm-workspace --test layout_emit\",\n",
    );
    let _ = writeln!(out, "  \"schema\": \"{}\",", json_escape(schema));
    let _ = writeln!(out, "  \"header_end\": {},", sl.header_end);
    let _ = writeln!(out, "  \"sample_max\": {},", sl.sample_max);
    let _ = writeln!(out, "  \"struct_align\": {},", sl.struct_align);

    out.push_str("  \"slots\": [\n");
    for (i, m) in sl.members.iter().enumerate() {
        let region = match m.region {
            Region::Header => "header",
            Region::Descriptor => "descriptor",
            Region::Body => "body",
        };
        let tail = if i + 1 < sl.members.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "    {{\"name\": \"{}\", \"region\": \"{region}\", \"offset\": {}, \"size\": {}, \"align\": {}}}{tail}",
            json_escape(&m.name),
            m.offset,
            m.size,
            m.align
        );
    }
    out.push_str("  ],\n");

    out.push_str("  \"variables\": [\n");
    let vars: Vec<(&str, u64, u64, u64)> = fields
        .iter()
        .filter_map(|f| {
            variable_parts(&f.ty).map(|(es, ea, max)| (f.name.as_str(), max, es, u64::from(ea)))
        })
        .collect();
    for (i, (name, max, elem_size, elem_align)) in vars.iter().enumerate() {
        let tail = if i + 1 < vars.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "    {{\"name\": \"{}\", \"max\": {max}, \"elem_size\": {elem_size}, \"elem_align\": {elem_align}}}{tail}",
            json_escape(name)
        );
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

/// Minimal JSON string escape (backslash, quote, control chars).
/// REQ: REQ-TOOL-042.
#[must_use]
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}
