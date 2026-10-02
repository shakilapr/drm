//! I6 layout crosscheck ([canon-codec.md §Protocols Layout fixture exchange];
//! work-plan M02W2-E1-T1): `drm-core` loads the `layout.json` fixtures emitted
//! by `drm-workspace`, re-derives the canonical packing independently, writes
//! via the runtime codec, and validates structurally — zero mismatches is the
//! permanent gate. REQ: REQ-TOOL-042, REQ-CORE-081, REQ-CORE-083.

use std::path::{Path, PathBuf};

use drm_core::canon::{self, CanonLayout, Descriptor};
use drm_types::Status;

/// One committed slot claim: `{name, region, offset, size, align}`.
type Slot = (String, String, u64, u64, u64);

/// One variable-member re-derivation input: `{name, max, elem_size, elem_align}`.
type Var = (String, u64, u64, u64);

/// Derived positions for one variable member: (descriptor offset, body
/// offset, bytes at the passed counts, descriptor stride).
type Derived = Vec<(u64, u64, u64, u64)>;

fn layout_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/layout")
}

fn u64_at(j: &serde_json::Value, key: &str, stem: &str) -> u64 {
    j.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| unreachable!("{stem}: {key} is a u64"))
}

fn slots_at(j: &serde_json::Value, stem: &str) -> Vec<Slot> {
    j.get("slots")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| unreachable!("{stem}: slots array"))
        .iter()
        .map(|s| {
            (
                str_at(s, "name", stem),
                str_at(s, "region", stem),
                u64_at(s, "offset", stem),
                u64_at(s, "size", stem),
                u64_at(s, "align", stem),
            )
        })
        .collect()
}

fn vars_at(j: &serde_json::Value, stem: &str) -> Vec<Var> {
    j.get("variables")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| unreachable!("{stem}: variables array"))
        .iter()
        .map(|v| {
            (
                str_at(v, "name", stem),
                u64_at(v, "max", stem),
                u64_at(v, "elem_size", stem),
                u64_at(v, "elem_align", stem),
            )
        })
        .collect()
}

fn str_at(j: &serde_json::Value, key: &str, stem: &str) -> String {
    j.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| unreachable!("{stem}: {key} is a string"))
        .to_string()
}

fn align_up(cursor: u64, align: u64) -> Option<u64> {
    debug_assert!(align > 0);
    let rem = cursor % align;
    if rem == 0 {
        Some(cursor)
    } else {
        cursor.checked_add(align - rem)
    }
}

/// Descriptor block stride: one `u32` count per variable member, plus one
/// `u32` offset each when the struct has > 1 variable member.
fn stride_of(n_vars: usize) -> u64 {
    if n_vars > 1 { 8 } else { 4 }
}

/// Re-derivation from counts (runtime-side, independent of the toolchain):
/// descriptor block sits directly below `header_end`; bodies element-aligned
/// contiguous from `header_end`; returned length is the derived `sample_len`.
fn derive_counts(header_end: u64, vars: &[Var], counts: &[u64]) -> Result<(Derived, u64), Status> {
    let stride = stride_of(vars.len());
    let block = u64::try_from(vars.len())
        .map_err(|_| Status::InvalidConfig)?
        .checked_mul(stride)
        .ok_or(Status::InvalidConfig)?;
    let desc_start = header_end.checked_sub(block).ok_or(Status::InvalidConfig)?;
    let mut cursor = header_end;
    let mut derived = Vec::new();
    for (i, (_, max, elem_size, elem_align)) in vars.iter().enumerate() {
        let count = counts.get(i).copied().ok_or(Status::InvalidData)?;
        if count > *max {
            return Err(Status::InvalidData);
        }
        let idx = u64::try_from(i).map_err(|_| Status::InvalidConfig)?;
        let desc_off = desc_start
            .checked_add(idx.checked_mul(stride).ok_or(Status::InvalidConfig)?)
            .ok_or(Status::InvalidConfig)?;
        let off = align_up(cursor, *elem_align).ok_or(Status::InvalidData)?;
        let bytes = count.checked_mul(*elem_size).ok_or(Status::InvalidData)?;
        cursor = off.checked_add(bytes).ok_or(Status::InvalidData)?;
        derived.push((desc_off, off, bytes, stride));
    }
    Ok((derived, cursor))
}

/// REQ: REQ-TOOL-042 — I6: every fixture agrees slot-for-slot with the
/// runtime re-derivation; the codec writes and structurally validates a
/// sample built purely from the JSON claims.
#[test]
fn crosscheck_layout_agreement() {
    let dir = layout_dir();
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| unreachable!("read layout dir: {e:?}"));
    let mut fixtures: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "json"))
        .collect();
    fixtures.sort();
    assert_eq!(fixtures.len(), 10, "fixture set present");

    for name in &fixtures {
        let stem = name.trim_end_matches(".json");
        let text = std::fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| unreachable!("read {name}: {e:?}"));
        let j: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| unreachable!("{name}: parse {e:?}"));
        let header_end = u64_at(&j, "header_end", stem);
        let sample_max = u64_at(&j, "sample_max", stem);
        let vars = vars_at(&j, stem);
        let slots = slots_at(&j, stem);

        check_slot_bounds(stem, &slots, header_end, sample_max);
        check_agreement(stem, header_end, sample_max, &vars, &slots);
        write_and_validate(stem, &j, header_end, sample_max, &vars);
    }
}

/// Claim sanity: header/descriptor slots within the header, body slots
/// element-aligned within the tail.
fn check_slot_bounds(stem: &str, slots: &[Slot], header_end: u64, sample_max: u64) {
    for (n, region, off, size, align) in slots {
        match region.as_str() {
            "header" | "descriptor" => {
                assert!(
                    off + size <= header_end,
                    "{stem}/{n}: header slot within header_end"
                );
            }
            "body" => {
                assert!(
                    *off >= header_end && off + size <= sample_max,
                    "{stem}/{n}: body slot within tail"
                );
            }
            other => unreachable!("{stem}: region {other}"),
        }
        assert!(*align > 0, "{stem}/{n}: positive align");
        if region == "body" {
            assert_eq!(off % align, 0, "{stem}/{n}: body element-aligned");
        }
    }
}

/// Slot-for-slot agreement with the independent full-count derivation.
fn check_agreement(stem: &str, header_end: u64, sample_max: u64, vars: &[Var], slots: &[Slot]) {
    let counts_full: Vec<u64> = vars.iter().map(|(_, max, ..)| *max).collect();
    let (derived, derived_max) = derive_counts(header_end, vars, &counts_full)
        .unwrap_or_else(|e| unreachable!("{stem}: derive {e:?}"));
    assert_eq!(
        derived_max, sample_max,
        "{stem}: sample_max agrees with full-count derivation"
    );
    let stride = stride_of(vars.len());
    let mut body_iter = slots.iter().filter(|(_, r, ..)| r == "body");
    let mut desc_iter = slots.iter().filter(|(_, r, ..)| r == "descriptor");
    for (i, (desc_off, off, size, _)) in derived.iter().enumerate() {
        let (_, _, d_off, d_size, _) = desc_iter
            .next()
            .unwrap_or_else(|| unreachable!("{stem}: descriptor slot {i}"));
        assert_eq!(d_off, desc_off, "{stem}: descriptor {i} offset agrees");
        assert_eq!(*d_size, stride, "{stem}: descriptor {i} slot width");
        let (_, _, b_off, b_size, _) = body_iter
            .next()
            .unwrap_or_else(|| unreachable!("{stem}: body slot {i}"));
        assert_eq!(b_off, off, "{stem}: body {i} offset agrees (at max)");
        assert_eq!(b_size, size, "{stem}: body {i} size agrees (at max)");
    }
    assert!(
        body_iter.next().is_none(),
        "{stem}: every body slot derived"
    );
    assert!(
        desc_iter.next().is_none(),
        "{stem}: every descriptor slot derived"
    );
}

/// Write an actual sample (half-max counts) with descriptor slots + body
/// pattern bytes, then validate through the runtime validator, plus the
/// full-max capacity agreement and negative length probes.
fn write_and_validate(
    stem: &str,
    j: &serde_json::Value,
    header_end: u64,
    sample_max: u64,
    vars: &[Var],
) {
    let counts: Vec<u64> = vars.iter().map(|(_, max, ..)| max / 2).collect();
    let (actual, actual_len) = derive_counts(header_end, vars, &counts)
        .unwrap_or_else(|e| unreachable!("{stem}: derive_counts {e:?}"));
    let buf_len = usize::try_from(sample_max).unwrap_or(usize::MAX);
    let mut buf = vec![0_u8; buf_len];
    for (i, (desc_off, off, bytes, stride)) in actual.iter().enumerate() {
        let d = usize::try_from(*desc_off).unwrap_or(usize::MAX);
        let stride_us = usize::try_from(*stride).unwrap_or(usize::MAX);
        let count = u32::try_from(counts[i]).unwrap_or(u32::MAX);
        buf[d..d + 4].copy_from_slice(&count.to_le_bytes());
        if stride_us == 8 {
            let off32 = u32::try_from(*off).unwrap_or(u32::MAX);
            buf[d + 4..d + 8].copy_from_slice(&off32.to_le_bytes());
        }
        let body = usize::try_from(*off).unwrap_or(usize::MAX);
        let end =
            usize::try_from(off.checked_add(*bytes).unwrap_or(u64::MAX)).unwrap_or(usize::MAX);
        let end = end.min(buf_len);
        let seed = u8::try_from(vars[i].0.len()).unwrap_or(7);
        for (k, slot) in buf[body..end].iter_mut().enumerate() {
            *slot = seed ^ u8::try_from((body + k) % 251).unwrap_or(0);
        }
    }
    let layout = CanonLayout {
        header_end,
        sample_max,
        struct_align: u32::try_from(u64_at(j, "struct_align", stem)).unwrap_or(1),
        descriptors: actual
            .iter()
            .zip(vars.iter())
            .zip(counts.iter())
            .map(|(((_, body_off, ..), (_, max, es, ea)), c)| Descriptor {
                count: *c,
                max: *max,
                offset: *body_off,
                header_end,
                elem_size: *es,
                elem_align: *ea,
            })
            .collect(),
    };
    assert_eq!(
        canon::validate_sample(&buf, &layout, actual_len),
        Ok(()),
        "{stem}: half-max sample validates"
    );
    // Negative probes against the validator's arithmetic (I5: value or error).
    if actual_len > header_end {
        assert_eq!(
            canon::validate_sample(&buf, &layout, actual_len - 1),
            Err(Status::InvalidData),
            "{stem}: short sample_len rejected (last body overruns)"
        );
    }
    let short = usize::try_from(actual_len.saturating_sub(1)).unwrap_or(usize::MAX);
    assert_eq!(
        canon::validate_sample(&buf[..short], &layout, actual_len),
        Err(Status::InvalidData),
        "{stem}: short buffer rejected"
    );
    assert_eq!(
        canon::validate_sample(&buf, &layout, sample_max + 1),
        Err(Status::InvalidData),
        "{stem}: over-capacity sample_len rejected"
    );
}
