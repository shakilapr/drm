//! Framing tests (work-plan M02W2-E3-T1/T2; router-core.md I-RT18/I-RT3):
//! record round-trip, truncation rejection, datagram multiplex bound +
//! property sweep. REQ: REQ-ROUTE-038, REQ-ROUTE-032.

use drm_router::framing::datagram::{decode_multiplex, encode_multiplex, packed_len};
use drm_router::framing::record::{RECORD_PREFIX, decode_record, encode_record, record_len};
use drm_types::Status;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// REQ: REQ-ROUTE-038 — record round-trip incl. empty and large payloads.
#[test]
fn framing_record_roundtrip() {
    for payload in [&b"hello"[..], &b""[..], &[0u8; 4096][..], &[0xFF; 3][..]] {
        let total = record_len(payload.len());
        let mut dst = vec![0xAA_u8; total];
        let n = fixture(encode_record(&mut dst, payload), "encode");
        assert_eq!(n, total);
        // Prefix is the LE payload length.
        let plen = u32::from_le_bytes([dst[0], dst[1], dst[2], dst[3]]);
        assert_eq!(plen, u32::try_from(payload.len()).unwrap_or(u32::MAX));
        let (back, next) = fixture(decode_record(&dst, 0), "decode");
        assert_eq!(back, payload);
        assert_eq!(next, total);
    }
}

/// REQ: REQ-ROUTE-038 — truncation ⇒ error, never panic: short prefix,
/// short body, garbage huge length, and a bad start offset.
#[test]
fn framing_record_truncation() {
    let mut buf = vec![0_u8; 64];
    let n = fixture(encode_record(&mut buf, b"payload"), "encode");
    assert_eq!(n, RECORD_PREFIX + 7);
    // Every truncation of a valid frame must error.
    for cut in 0..n {
        let r = decode_record(&buf[..cut], 0);
        assert!(r.is_err(), "cut={cut}: truncated frame must reject");
    }
    // Declared length beyond u32 range is unreachable via encode; a hostile
    // u32::MAX prefix rejects without allocation.
    let mut hostile = 0xAA_u8.to_le_bytes().to_vec();
    hostile.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(decode_record(&hostile, 0), Err(Status::InvalidData));
    // Position beyond buffer ⇒ InvalidArg.
    assert_eq!(decode_record(&buf, buf.len() + 1), Err(Status::InvalidArg));
    // Destination shorter than the record ⇒ InvalidArg, buffer untouched.
    let mut short = [0xEE_u8; 4];
    assert_eq!(
        encode_record(&mut short, b"payload"),
        Err(Status::InvalidArg)
    );
    assert_eq!(short, [0xEE; 4]);
}

/// REQ: REQ-ROUTE-038 — stream decode: concatenated records decode in order
/// and the cursor advances exactly.
#[test]
fn framing_record_stream() {
    let payloads: Vec<&[u8]> = vec![b"one", b"", b"three33333"];
    let mut stream = Vec::new();
    for p in &payloads {
        let mut frame = vec![0_u8; record_len(p.len())];
        let n = fixture(encode_record(&mut frame, p), "encode");
        stream.extend_from_slice(&frame[..n]);
    }
    let mut pos = 0;
    let mut seen = Vec::new();
    while pos < stream.len() {
        let (p, next) = fixture(decode_record(&stream, pos), "decode");
        seen.push(p.to_vec());
        pos = next;
    }
    assert_eq!(
        seen,
        payloads.iter().map(|p| p.to_vec()).collect::<Vec<_>>()
    );
}

/// REQ: REQ-ROUTE-032 — multiplex: pack N small records, decode all; bound
/// never exceeded; oversize pack rejected before any write.
#[test]
fn framing_multiplex_bound() {
    let max_payload = 256;
    let payloads: Vec<&[u8]> = vec![b"a", b"bb", b"ccc", b""];
    let expect = packed_len(&payloads);
    assert!(expect <= max_payload);
    let mut dst = vec![0xEE_u8; max_payload];
    let n = fixture(encode_multiplex(&mut dst, &payloads, max_payload), "mux");
    assert_eq!(n, expect);
    let records = fixture(decode_multiplex(&dst[..n]), "demux");
    let back: Vec<&[u8]> = records.iter().map(|r| r.payload).collect();
    assert_eq!(back, payloads);

    // Oversize pack: rejected before any write (0xEE probe intact).
    let big: Vec<&[u8]> = vec![&[0u8; 200], &[0u8; 100]];
    dst.fill(0xEE);
    assert_eq!(
        encode_multiplex(&mut dst, &big, max_payload),
        Err(Status::InvalidArg)
    );
    assert!(
        dst.iter().all(|b| *b == 0xEE),
        "no bytes written on oversize"
    );

    // Exact fit at the bound is allowed.
    let exact_buf = vec![0u8; max_payload - RECORD_PREFIX];
    let exact: Vec<&[u8]> = vec![&exact_buf];
    assert_eq!(
        fixture(encode_multiplex(&mut dst, &exact, max_payload), "exact"),
        max_payload
    );
    // One byte over rejects.
    let over_buf = vec![0u8; max_payload - RECORD_PREFIX + 1];
    let over: Vec<&[u8]> = vec![&over_buf];
    assert_eq!(
        encode_multiplex(&mut dst, &over, max_payload),
        Err(Status::InvalidArg)
    );

    // Truncated multiplex body rejects.
    assert!(decode_multiplex(&dst[..n - 1]).is_err());
}
