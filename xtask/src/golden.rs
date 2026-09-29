//! Golden vector generation (canon-codec.md §Protocols fixture exchange;
//! work-plan M01W3-E4-T3): writes the frozen primitive vector list as
//! `tests/fixtures/golden/primitives/NNN_<type>_<case>.{hex,json}` through the
//! drm-core canonical write path. Idempotent: regeneration reproduces
//! byte-identical files (`git diff --exit-code tests/fixtures/golden`).
//! REQ: REQ-QAL-006, REQ-TOOL-043, REQ-TOOL-044.

use anyhow::Result;
use drm_core::canon;
use serde_json::json;
use std::fmt::Write as _;
use std::path::PathBuf;

/// One frozen golden vector: canonical LE encoding plus JSON metadata sidecar.
struct GoldenVector {
    ty: &'static str,
    case: &'static str,
    encode: Box<dyn Fn() -> Result<Vec<u8>>>,
    json_value: serde_json::Value,
}

/// Encodes `N` little-endian bytes through a canon write accessor.
/// REQ: REQ-TOOL-043.
fn enc<const N: usize>(
    f: impl Fn(&mut [u8], usize) -> Result<(), drm_types::Status> + 'static,
) -> Box<dyn Fn() -> Result<Vec<u8>>> {
    Box::new(move || {
        let mut buf = vec![0u8; N];
        f(&mut buf, 0).map_err(|e| anyhow::anyhow!("canon write: {e:?}"))?;
        Ok(buf)
    })
}

/// Frozen vector list (work-plan M01W3-E4-T3): u8 {0,127,255}, i8 {0,−128,127},
/// u16 {0,32767,65535}, i16 {0,−32768,32767}, u32 {0,max}, i32 {0,min,max},
/// f32 {0.0,1.0,smallest-normal,max,−max}, bool {false,true}.
fn vectors() -> Vec<GoldenVector> {
    enum V {
        U8(u8),
        I8(i8),
        U16(u16),
        I16(i16),
        U32(u32),
        I32(i32),
        F32(f32),
        B(bool),
    }
    let table: &[(&str, &str, V)] = &[
        ("u8", "zero", V::U8(0)),
        ("u8", "max_signed", V::U8(127)),
        ("u8", "max", V::U8(255)),
        ("i8", "zero", V::I8(0)),
        ("i8", "min", V::I8(-128)),
        ("i8", "max", V::I8(127)),
        ("u16", "zero", V::U16(0)),
        ("u16", "max_signed", V::U16(32767)),
        ("u16", "max", V::U16(65535)),
        ("i16", "zero", V::I16(0)),
        ("i16", "min", V::I16(-32768)),
        ("i16", "max", V::I16(32767)),
        ("u32", "zero", V::U32(0)),
        ("u32", "max", V::U32(u32::MAX)),
        ("i32", "zero", V::I32(0)),
        ("i32", "min", V::I32(i32::MIN)),
        ("i32", "max", V::I32(i32::MAX)),
        ("f32", "zero", V::F32(0.0)),
        ("f32", "one", V::F32(1.0)),
        ("f32", "smallest_normal", V::F32(f32::MIN_POSITIVE)),
        ("f32", "max", V::F32(f32::MAX)),
        ("f32", "neg_max", V::F32(-f32::MAX)),
        ("bool", "false", V::B(false)),
        ("bool", "true", V::B(true)),
    ];
    table
        .iter()
        .map(|(ty, case, v)| {
            let (encode, json_value) = match *v {
                V::U8(x) => (enc::<1>(move |b, o| canon::write_u8(b, o, x)), json!(x)),
                V::I8(x) => (enc::<1>(move |b, o| canon::write_i8(b, o, x)), json!(x)),
                V::U16(x) => (enc::<2>(move |b, o| canon::write_u16(b, o, x)), json!(x)),
                V::I16(x) => (enc::<2>(move |b, o| canon::write_i16(b, o, x)), json!(x)),
                V::U32(x) => (
                    enc::<4>(move |b, o| canon::write_u32(b, o, x)),
                    json!(u64::from(x)),
                ),
                V::I32(x) => (
                    enc::<4>(move |b, o| canon::write_i32(b, o, x)),
                    json!(i64::from(x)),
                ),
                V::F32(x) => (enc::<4>(move |b, o| canon::write_f32(b, o, x)), json!(x)),
                V::B(x) => (enc::<1>(move |b, o| canon::write_bool(b, o, x)), json!(x)),
            };
            GoldenVector {
                ty,
                case,
                encode,
                json_value,
            }
        })
        .collect()
}

/// REQ: REQ-QAL-006 — `xtask golden-gen`: regenerate golden fixtures
/// (idempotent, verify with `git diff --exit-code tests/fixtures/golden`).
pub fn generate(flags: &[String]) -> Result<()> {
    if !flags.is_empty() {
        anyhow::bail!("golden-gen takes no flags");
    }
    let dir = PathBuf::from("tests/fixtures/golden/primitives");
    std::fs::create_dir_all(&dir)?;
    let mut index = Vec::new();
    for (i, vec) in vectors().iter().enumerate() {
        let name = format!("{:03}_{}_{}", i + 1, vec.ty, vec.case);
        let bytes = (vec.encode)()?;
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in &bytes {
            let _ = write!(hex, "{b:02x}");
        }
        index.push(json!({
            "file": name,
            "type": vec.ty,
            "case": vec.case,
            "value": vec.json_value,
            "len": bytes.len(),
        }));
        std::fs::write(dir.join(format!("{name}.hex")), &hex)?;
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_string_pretty(&vec.json_value)?,
        )?;
    }
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&json!({"vectors": index}))?,
    )?;
    println!("golden-gen: {} vectors in {}", index.len(), dir.display());
    Ok(())
}
