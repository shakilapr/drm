//! Descriptor byte-exact golden (I-M1) and offset asserts (I-M5)
//! (work-plan M02W3-E1-T1). REQ: REQ-CORE-015, REQ-CORE-014, REQ-CORE-016.

use std::mem::{offset_of, size_of};
use std::path::Path;

use drm_core::channel::ccb::{
    ChannelControlBlock, ChannelDescriptor, DESC_FLAG_TIMING, read_descriptor, write_descriptor,
};
use serde_json::Value;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// The golden descriptor value matching `tests/fixtures/golden/descriptor/`.
fn golden_descriptor() -> ChannelDescriptor {
    let mut schema_hash = [0_u8; 16];
    let mut layout_hash = [0_u8; 16];
    for i in 0..16 {
        schema_hash[i] = u8::try_from(i).unwrap_or(0);
        layout_hash[i] = u8::try_from(16 + i).unwrap_or(0);
    }
    ChannelDescriptor {
        flags: DESC_FLAG_TIMING,
        pad_header: 0,
        schema_hash,
        layout_hash,
        topic_handle: 7,
        topic_name_len: 10,
        overflow_policy: 1,
        pad0: 0,
        payload_capacity: 64,
        history_depth: 4,
        pool_size: 8,
        consumer_count: 3,
        required_count: 1,
        diagnostic_count: 1,
        pad_counts: 0,
        ccb_offset: 184,
        ring_offset: 248,
        consumer_offset: 264,
        slot_generation_offset: 288,
        payload_offset: 304,
        segment_size: 816,
        reserved: [0; 16],
    }
}

/// REQ: REQ-CORE-015 — descriptor bytes match the golden table field-for-field
/// including zero padding (I-M1); write/read roundtrip is identity.
#[test]
fn descriptor_golden() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/golden/descriptor/descriptor_golden.json");
    let raw = fixture(std::fs::read(&fixture_path), "fixture");
    let v: Value = fixture(serde_json::from_slice(&raw), "json");
    let d = golden_descriptor();
    let mut buf = vec![0xEE_u8; size_of::<ChannelDescriptor>()];
    fixture(write_descriptor(&mut buf, &d), "write");

    for f in v["fields"].as_array().unwrap_or(&Vec::new()) {
        let off =
            usize::try_from(fixture(f["offset"].as_u64().ok_or("offset"), "off")).unwrap_or(0);
        let size = usize::try_from(fixture(f["size"].as_u64().ok_or("size"), "size")).unwrap_or(0);
        let hex = fixture(f["hex"].as_str().ok_or("hex"), "hex");
        let got: Vec<String> = buf[off..off + size]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            got.join(""),
            hex,
            "field {} byte mismatch",
            f["name"].as_str().unwrap_or("?")
        );
    }
    // Padding sanity: every byte not covered by a named field is zero.
    let named: Vec<(usize, usize)> = v["fields"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|f| {
            (
                usize::try_from(fixture(f["offset"].as_u64().ok_or("o"), "o")).unwrap_or(0),
                usize::try_from(fixture(f["size"].as_u64().ok_or("s"), "s")).unwrap_or(0),
            )
        })
        .collect();
    for (i, b) in buf.iter().enumerate() {
        let covered = named.iter().any(|(o, s)| i >= *o && i < o + s);
        if !covered {
            assert_eq!(*b, 0, "padding byte {i} must be zero");
        }
    }
    // Read/write symmetry: parsing the golden image yields the same value.
    assert_eq!(read_descriptor(&buf), d);
}

/// REQ: REQ-CORE-014 — compile-time offsets are exactly the golden offsets
/// (I-M1/I-M5); every atomic CCB field is naturally aligned (I-M5).
#[test]
fn descriptor_offset_asserts() {
    let d = golden_descriptor();
    assert_eq!(size_of::<ChannelDescriptor>(), 152);
    assert_eq!(offset_of!(ChannelDescriptor, flags), 0);
    assert_eq!(offset_of!(ChannelDescriptor, pad_header), 4);
    assert_eq!(offset_of!(ChannelDescriptor, schema_hash), 8);
    assert_eq!(offset_of!(ChannelDescriptor, layout_hash), 24);
    assert_eq!(offset_of!(ChannelDescriptor, topic_handle), 40);
    assert_eq!(offset_of!(ChannelDescriptor, topic_name_len), 44);
    assert_eq!(offset_of!(ChannelDescriptor, overflow_policy), 46);
    assert_eq!(offset_of!(ChannelDescriptor, pad0), 47);
    assert_eq!(offset_of!(ChannelDescriptor, payload_capacity), 48);
    assert_eq!(offset_of!(ChannelDescriptor, history_depth), 56);
    assert_eq!(offset_of!(ChannelDescriptor, pool_size), 64);
    assert_eq!(offset_of!(ChannelDescriptor, consumer_count), 72);
    assert_eq!(offset_of!(ChannelDescriptor, required_count), 76);
    assert_eq!(offset_of!(ChannelDescriptor, diagnostic_count), 80);
    assert_eq!(offset_of!(ChannelDescriptor, pad_counts), 84);
    assert_eq!(offset_of!(ChannelDescriptor, ccb_offset), 88);
    assert_eq!(offset_of!(ChannelDescriptor, ring_offset), 96);
    assert_eq!(offset_of!(ChannelDescriptor, consumer_offset), 104);
    assert_eq!(offset_of!(ChannelDescriptor, slot_generation_offset), 112);
    assert_eq!(offset_of!(ChannelDescriptor, payload_offset), 120);
    assert_eq!(offset_of!(ChannelDescriptor, segment_size), 128);
    assert_eq!(offset_of!(ChannelDescriptor, reserved), 136);

    assert_eq!(offset_of!(ChannelControlBlock, publisher_epoch) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, first_valid_seq) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, write_seq) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, drop_count) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, overrun_count) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, pool_exhausted) % 8, 0);
    assert_eq!(offset_of!(ChannelControlBlock, reclaim_requested), 48);
    assert_eq!(offset_of!(ChannelControlBlock, publish_busy), 49);
    assert_eq!(size_of::<ChannelControlBlock>(), 56);
    let _ = &d;
}
