//! CRC32C known-answer tests (work-plan M02W3-E3-T4): official CRC-32C
//! (Castagnoli, reflected poly `0x82F63B78`, init and final XOR
//! `0xFFFFFFFF` — the existing `crc32()` convention).
//! REQ: REQ-ROUTE-033, REQ-ROUTE-027.

use drm_router::crc32c::{crc32c, crc32c_update};

/// Deterministic message set: empty, canonical, constant patterns, and
/// cycling byte patterns of mixed lengths.
fn messages() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"123456789".to_vec(),
        vec![0u8; 300],
        vec![0xFFu8; 300],
        (0u8..=255).cycle().take(1_000).collect(),
        (0u8..=255)
            .map(|b| b.wrapping_mul(7))
            .cycle()
            .take(513)
            .collect(),
    ]
}

/// REQ: REQ-ROUTE-033 — official known-answer vectors for one-shot CRC32C.
#[test]
fn kat_vectors() {
    assert_eq!(crc32c(b""), 0x0000_0000);
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
    assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    assert_eq!(crc32c(&(0u8..32).collect::<Vec<u8>>()), 0x46DD_794E);
    assert_eq!(crc32c(&(0u8..32).rev().collect::<Vec<u8>>()), 0x113F_DB5C);
}

/// REQ: REQ-ROUTE-033 — raw-state convention: `crc32(data)` equals
/// `!crc32_update(!0, data)` (init/final `0xFFFFFFFF`), verified for every
/// message. No discrepancy with the official convention.
#[test]
fn raw_state_convention() {
    for m in messages() {
        assert_eq!(
            crc32c(&m),
            !crc32c_update(0xFFFF_FFFF, &m),
            "len {}",
            m.len()
        );
    }
}

/// Cut-position sets (positions before the end) for a message length.
fn split_sets(len: usize) -> Vec<Vec<usize>> {
    let mut sets = vec![Vec::new()];
    if len > 0 {
        sets.push(vec![0]);
        sets.push(vec![len]);
        sets.push(vec![len / 2]);
        for step in [1usize, 3, 7, 64, 255] {
            let mut cuts = Vec::new();
            let mut pos = step;
            while pos < len {
                cuts.push(pos);
                pos += step;
            }
            if !cuts.is_empty() {
                sets.push(cuts);
            }
        }
    }
    sets
}

/// REQ: REQ-ROUTE-033, REQ-ROUTE-027 — streaming `crc32_update` chaining
/// over arbitrary deterministic split points equals the one-shot `crc32`
/// over the whole message.
#[test]
fn streaming_split_points() {
    for m in messages() {
        let whole = crc32c(&m);
        for cuts in split_sets(m.len()) {
            let mut bounds = cuts.clone();
            bounds.push(m.len());
            bounds.sort_unstable();
            bounds.dedup();
            let mut acc = 0xFFFF_FFFF_u32;
            let mut prev = 0usize;
            for &bound in &bounds {
                acc = crc32c_update(acc, &m[prev..bound]);
                prev = bound;
            }
            assert_eq!(acc ^ 0xFFFF_FFFF, whole, "cuts {cuts:?} len {}", m.len());
        }
    }
}
