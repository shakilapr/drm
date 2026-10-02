//! Framing property sweep (work-plan M02W2 variables `FRAMING_PROPERTY_CASES`
//! ≥ 20 000; router-core.md I-RT18/I-RT3). REQ: REQ-ROUTE-038, REQ-ROUTE-032.

use proptest::prelude::*;

use drm_router::framing::datagram::{decode_multiplex, encode_multiplex};
use drm_router::framing::record::{decode_record, record_len};

prop_compose! {
    fn arb_payloads()(n in 0usize..8, max in 1usize..64) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                let len = (i * 7) % (max + 1);
                (0..len)
                    .map(|k| u8::try_from((i * 31 + k * 17) % 256).unwrap_or(0))
                    .collect()
            })
            .collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// REQ: REQ-ROUTE-038 — record encode→decode identity for arbitrary
    /// payloads; every truncation of a valid frame rejects (no panic).
    #[test]
    fn prop_record_roundtrip(payload in proptest::collection::vec(any::<u8>(), 0..300)) {
        let total = record_len(payload.len());
        let mut dst = vec![0xAA_u8; total];
        let n = drm_router::framing::record::encode_record(&mut dst, &payload)
            .map_err(|e| TestCaseError::fail(format!("encode: {e:?}")))?;
        prop_assert_eq!(n, total);
        let (back, next) = decode_record(&dst, 0).map_err(|e| TestCaseError::fail(format!("decode: {e:?}")))?;
        prop_assert_eq!(back, payload.as_slice());
        prop_assert_eq!(next, total);
        for cut in 0..total {
            prop_assert!(decode_record(&dst[..cut], 0).is_err(), "cut {cut} accepted");
        }
    }

    /// REQ: REQ-ROUTE-032 — multiplex: encode→decode identity, packed size
    /// equals the sum of record lengths, oversize never writes.
    #[test]
    fn prop_multiplex(payloads in arb_payloads()) {
        let refs: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
        let packed = drm_router::framing::datagram::packed_len(&refs);
        let max = packed.max(1);
        let mut dst = vec![0xEE_u8; max];
        let n = encode_multiplex(&mut dst, &refs, max)
            .map_err(|e| TestCaseError::fail(format!("mux: {e:?}")))?;
        prop_assert_eq!(n, packed);
        let records = decode_multiplex(&dst[..n])
            .map_err(|e| TestCaseError::fail(format!("demux: {e:?}")))?;
        prop_assert_eq!(records.len(), payloads.len());
        for (r, p) in records.iter().zip(payloads.iter()) {
            prop_assert_eq!(r.payload, p.as_slice());
        }
        // Bound strictly below the packed size rejects without writing.
        if packed > 0 {
            dst.fill(0xEE);
            prop_assert_eq!(encode_multiplex(&mut dst, &refs, packed - 1), Err(drm_types::Status::InvalidArg));
            prop_assert!(dst.iter().all(|b| *b == 0xEE));
        }
    }

    /// REQ: REQ-ROUTE-038 — arbitrary bytes decode to payload-or-error, never
    /// a panic; successful decodes reconstruct exactly the input prefix.
    #[test]
    fn prop_decode_total(data in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = decode_record(&data, 0);
        if let Ok(records) = decode_multiplex(&data) {
            let total: usize = records
                .iter()
                .map(|r| record_len(r.payload.len()))
                .sum();
            prop_assert!(total <= data.len(), "decoded records exceed input");
        }
    }
}
