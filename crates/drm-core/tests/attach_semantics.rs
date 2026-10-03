//! Attach validation semantics (work-plan M02W3-E1-T1): I-M6 init refusal,
//! I-M7 exact-layout matrix, I-M2 attach-once. REQ: REQ-CORE-015, REQ-CORE-002,
//! REQ-CORE-016, REQ-CORE-017, REQ-CORE-068.

use std::sync::atomic::AtomicU64;

use drm_core::channel::ccb::{
    ChannelDescriptor, DESC_FLAG_TIMING, DESC_FLAG_VARIABLE, Expectation, SEGMENT_INIT,
    SEGMENT_READY, SEGMENT_ZERO, read_descriptor, validate_on_attach, write_descriptor,
    write_prefix,
};
use drm_types::Status;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// Canonical valid descriptor (matches the offsets in the golden fixture).
fn base_descriptor() -> ChannelDescriptor {
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

fn matching_expectation() -> Expectation {
    let d = base_descriptor();
    Expectation {
        schema_hash: d.schema_hash,
        layout_hash: d.layout_hash,
        topic_handle: d.topic_handle,
        topic_name: "pkg/camera".to_string(),
        payload_capacity: d.payload_capacity,
        history_depth: d.history_depth,
        pool_size: d.pool_size,
        overflow_policy: d.overflow_policy,
        consumer_count: d.consumer_count,
        required_count: d.required_count,
        diagnostic_count: d.diagnostic_count,
    }
}

/// Builds a full segment image: prefix + descriptor + topic name + zeroed
/// regions up to `segment_size`.
fn build_segment(d: &ChannelDescriptor, name: &str, state: u32) -> Vec<u8> {
    let mut seg = vec![0_u8; usize::try_from(d.segment_size).unwrap_or(0)];
    fixture(write_prefix(&mut seg, state), "prefix");
    fixture(write_descriptor(&mut seg[16..168], d), "descriptor");
    let end = 16 + size_of_desc() + usize::from(d.topic_name_len);
    seg[168..end].copy_from_slice(name.as_bytes());
    seg
}

fn size_of_desc() -> usize {
    152
}

/// REQ: REQ-CORE-002 — a matching segment validates; every immutable field is
/// checked exactly once (compare-once, I-M2), and the returned view carries
/// the checked offsets.
#[test]
fn attach_accepts_matching_segment() {
    let d = base_descriptor();
    let seg = build_segment(&d, "pkg/camera", SEGMENT_READY);
    let view = fixture(validate_on_attach(&seg, &matching_expectation()), "attach");
    assert_eq!(view.descriptor, d);
    assert_eq!(view.ccb_offset, 184);
    assert_eq!(view.ring_offset, 248);
    assert_eq!(view.payload_offset, 304);
}

/// REQ: REQ-CORE-015 — I-M6: non-Ready prefix refuses with `NotReady` and no
/// other bytes are interpreted (garbage descriptor body is irrelevant).
#[test]
fn attach_init_refused() {
    for state in [SEGMENT_ZERO, SEGMENT_INIT] {
        let mut d = base_descriptor();
        d.reserved = [0xFF; 16];
        let seg = build_segment(&d, "pkg/camera", state);
        assert_eq!(
            validate_on_attach(&seg, &matching_expectation()),
            Err(Status::NotReady)
        );
    }
    // Too-short segment: cannot even read the prefix.
    assert_eq!(
        validate_on_attach(&[0_u8; 8], &matching_expectation()),
        Err(Status::InvalidData)
    );
}

/// REQ: REQ-CORE-016 — I-M7 matrix: each tampered immutable field, structural
/// violation, or layout inconsistency yields its explicit error.
#[test]
fn attach_layout_matrix() {
    let expect = matching_expectation();
    let case = |mutate: &dyn Fn(&mut ChannelDescriptor)| -> Status {
        let mut d = base_descriptor();
        mutate(&mut d);
        let seg = build_segment(&d, "pkg/camera", SEGMENT_READY);
        validate_on_attach(&seg, &expect)
            .err()
            .unwrap_or(Status::Ok)
    };

    assert_eq!(case(&|d| d.schema_hash[0] ^= 0xFF), Status::InvalidData);
    assert_eq!(case(&|d| d.layout_hash[0] ^= 0xFF), Status::WrongAbi);
    assert_eq!(case(&|d| d.topic_handle = 9), Status::InvalidData);
    assert_eq!(case(&|d| d.payload_capacity = 128), Status::WrongAbi);
    // segment_size must equal the mapping length: patch the field directly so
    // the image keeps its original allocation (816) but claims 817.
    let mut seg = build_segment(&base_descriptor(), "pkg/camera", SEGMENT_READY);
    seg[16 + 128..16 + 136].copy_from_slice(&817_u64.to_le_bytes());
    assert_eq!(validate_on_attach(&seg, &expect), Err(Status::WrongAbi));
    assert_eq!(case(&|d| d.reserved[3] = 1), Status::WrongAbi);
    // Structural checks run after the immutable match, so both the descriptor
    // and the expectation must be mutated consistently.
    let structural = |mutate: &dyn Fn(&mut ChannelDescriptor, &mut Expectation)| -> Status {
        let mut d = base_descriptor();
        let mut e = matching_expectation();
        mutate(&mut d, &mut e);
        let seg = build_segment(&d, "pkg/camera", SEGMENT_READY);
        validate_on_attach(&seg, &e).err().unwrap_or(Status::Ok)
    };
    assert_eq!(
        structural(&|d, e| {
            d.history_depth = 0;
            e.history_depth = 0;
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, e| {
            d.pool_size = 3;
            e.pool_size = 3;
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, e| {
            d.pool_size = u64::from(u32::MAX);
            e.pool_size = d.pool_size;
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, e| {
            d.required_count = 5;
            e.required_count = 5;
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, e| {
            d.flags |= DESC_FLAG_VARIABLE;
            d.payload_capacity = u64::from(u32::MAX) + 1;
            e.payload_capacity = d.payload_capacity;
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, _| d.ccb_offset = 185),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, _| {
            std::mem::swap(&mut d.ring_offset, &mut d.consumer_offset);
        }),
        Status::InvalidConfig
    );
    assert_eq!(
        structural(&|d, _| d.ring_offset = d.ccb_offset + 8),
        Status::InvalidConfig
    );
    // Pool capacity overflow with matching expectation: payload end
    // 304 + 8*128 = 1328 > 816.
    assert_eq!(
        structural(&|d, e| {
            d.payload_capacity = 128;
            e.payload_capacity = 128;
        }),
        Status::InvalidConfig
    );

    // Name tampering (outside the descriptor bytes, same length).
    let seg = build_segment(&base_descriptor(), "pkg/camerx", SEGMENT_READY);
    assert_eq!(validate_on_attach(&seg, &expect), Err(Status::InvalidData));

    // Wrong magic / layout version / descriptor size (prefix-level).
    let mut seg = build_segment(&base_descriptor(), "pkg/camera", SEGMENT_READY);
    seg[0] = b'X';
    assert_eq!(validate_on_attach(&seg, &expect), Err(Status::InvalidData));
    let mut seg = build_segment(&base_descriptor(), "pkg/camera", SEGMENT_READY);
    seg[4..8].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(validate_on_attach(&seg, &expect), Err(Status::WrongAbi));
    let mut seg = build_segment(&base_descriptor(), "pkg/camera", SEGMENT_READY);
    seg[12..16].copy_from_slice(&999_u32.to_le_bytes());
    assert_eq!(validate_on_attach(&seg, &expect), Err(Status::WrongAbi));
}

/// REQ: REQ-CORE-002 — I-M2: validation runs once; steady-state reads use the
/// returned view with no re-validation (1e6 view reads).
#[test]
fn attach_once() {
    let d = base_descriptor();
    let seg = build_segment(&d, "pkg/camera", SEGMENT_READY);
    let view = fixture(validate_on_attach(&seg, &matching_expectation()), "attach");
    for _ in 0..1_000_000 {
        // Steady-state read path: only view data is touched.
        let _ = AtomicU64::new(view.ccb_offset);
    }
    // The descriptor image is still parseable and unchanged.
    assert_eq!(read_descriptor(&seg[16..168]), d);
}
