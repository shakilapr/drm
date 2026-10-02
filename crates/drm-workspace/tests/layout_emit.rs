//! I6 layout-fixture emission ([canon-codec.md §Protocols](../../../docs/algorithms/canon-codec.md);
//! work-plan M02W2-E1-T1): renders `layout.json` per golden-composite schema
//! and keeps the committed artifacts byte-stable. Regenerate with
//! `DRM_LAYOUT_REGEN=1 cargo test -p drm-workspace --test layout_emit`
//! (`git diff --exit-code tests/fixtures/layout` must stay clean).
//! REQ: REQ-TOOL-042, REQ-CORE-081.

use drm_workspace::canon::{emit_layout_json, layout_struct};
use drm_workspace::parser::parse_bytes;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// The 10 golden-composite schemas (frozen oracle, canon-codec I9).
const COMPOSITES: [(&str, &str); 10] = [
    ("001_u8_u64_padded", "a: u8\nb: u64\n"),
    (
        "002_two_region_str_bounded",
        "t: u16\ns: string[8]\nv: u32[<=4]\n",
    ),
    ("003_string16_utf8", "s: string[16]\n"),
    ("004_bounded_u64_aligned", "x: f32\nv: u64[<=2]\n"),
    ("005_empty_bodied", "a: u8\nf: bool[2]\nb: bytes[<=4]\n"),
    (
        "006_three_variable_order",
        "n: u32\ns: string[4]\nb: bytes[<=4]\nk: u16\nw: u32[<=2]\n",
    ),
    ("007_enum_member", "e: enum{off,on}\nx: u32\n"),
    ("008_fixed_array_string", "f: u8[4]\ns: string[4]\n"),
    ("009_empty_bounded_u16", "a: u16\nv: u16[<=3]\n"),
    (
        "010_three_var_u64_gap",
        "t: u16\ns: string[8]\nv: u64[<=2]\nb: bytes[<=4]\n",
    ),
];

fn layout_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/layout")
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/golden/composites")
}

fn assert_json_shape(json: &str, stem: &str) {
    assert!(json.starts_with("{\n"), "{stem}: starts with object");
    assert!(json.ends_with("}\n"), "{stem}: single trailing newline");
    assert!(!json.contains(" \n"), "{stem}: no trailing spaces");
    assert!(!json.contains(",\n  ]"), "{stem}: no dangling comma");
    assert!(json.contains("\"banner\": \"@generated"), "{stem}: banner");
    for key in [
        "schema",
        "header_end",
        "sample_max",
        "struct_align",
        "slots",
        "variables",
    ] {
        assert!(
            json.contains(&format!("\"{key}\"")),
            "{stem}: {key} present"
        );
    }
}

/// REQ: REQ-TOOL-042 — render is deterministic (two passes byte-identical),
/// shaped, and matches the committed artifact (regen mode rewrites instead).
#[test]
fn layout_emit_stable() {
    let regen = std::env::var("DRM_LAYOUT_REGEN").is_ok();
    let dir = layout_dir();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("mkdir layout: {e}"));

    for (stem, schema) in COMPOSITES {
        let fields = fixture(parse_bytes(schema.as_bytes()), "{stem} parse").fields;
        let sl = fixture(layout_struct(&fields), "{stem} layout");
        let json = emit_layout_json(schema, &fields, &sl);
        assert_json_shape(&json, stem);
        // Determinism: a second render is byte-identical.
        let json2 = emit_layout_json(schema, &fields, &sl);
        assert_eq!(json, json2, "{stem}: render deterministic");
        let path = dir.join(format!("{stem}.json"));
        if regen {
            std::fs::write(&path, &json)
                .unwrap_or_else(|e| unreachable!("write {stem}.json: {e:?}"));
            continue;
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| unreachable!("read {stem}.json: {e:?}"));
        assert_eq!(
            json, committed,
            "{stem}: layout.json out of date — regenerate"
        );
    }
}

/// REQ: REQ-TOOL-042 — fixture completeness: every golden composite has a
/// committed layout.json (the exchange set never shrinks silently).
#[test]
fn layout_emit_complete() {
    let dir = fixture_dir();
    let mut found = 0;
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| unreachable!("read composites: {e:?}"));
    let mut stems: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .filter(|s| s.contains('_'))
        .collect();
    stems.sort();
    stems.dedup();
    let layout_dir = layout_dir();
    for stem in &stems {
        let path = layout_dir.join(format!("{stem}.json"));
        assert!(
            path.exists(),
            "missing layout.json for composite {stem} — regenerate"
        );
        found += 1;
    }
    assert_eq!(found, 10, "10 composite schemas covered");
}
