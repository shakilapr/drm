//! CDR decode tests: known-good bytes, every primitive, truncation, alignment.
//! REQ: REQ-BIND-028.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use drm_cdr::{CdrError, CdrReader, LaserScanV1};

const ENC: [u8; 4] = [0x00, 0x01, 0x00, 0x00];

fn reader(rest: &[u8]) -> CdrReader<'static> {
    let mut buf = ENC.to_vec();
    buf.extend_from_slice(rest);
    CdrReader::new(Box::leak(buf.into_boxed_slice())).expect("valid header")
}

fn payload(rest: &[u8]) -> Vec<u8> {
    let mut buf = ENC.to_vec();
    buf.extend_from_slice(rest);
    buf
}

#[test]
fn hand_written_laser_scan() {
    #[rustfmt::skip]
    let bytes: [u8; 72] = [
        0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
        0x06, 0x00, 0x00, 0x00, 0x6C, 0x61, 0x73, 0x65, 0x72, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xC0, 0xBF, 0x00, 0x00, 0xC0, 0x3F, 0x00, 0x00, 0x00, 0x3F,
        0x00, 0x00, 0x00, 0x00, 0xCD, 0xCC, 0xCC, 0x3D, 0xCD, 0xCC, 0xCC, 0x3D,
        0x00, 0x00, 0x20, 0x41, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3F,
        0x00, 0x00, 0x00, 0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3F,
    ];
    let scan = LaserScanV1::decode(&bytes).expect("decode");
    assert_eq!(scan.header.stamp.sec, 1);
    assert_eq!(scan.header.stamp.nanosec, 2);
    assert_eq!(scan.header.frame_id, "laser");
    assert_eq!(scan.angle_min, -1.5);
    assert_eq!(scan.angle_max, 1.5);
    assert_eq!(scan.angle_increment, 0.5);
    assert_eq!(scan.time_increment, 0.0);
    assert_eq!(scan.scan_time, 0.1);
    assert_eq!(scan.range_min, 0.1);
    assert_eq!(scan.range_max, 10.0);
    assert_eq!(scan.ranges, vec![1.0, 2.0]);
    assert_eq!(scan.intensities, vec![0.5]);
}

#[test]
fn bad_encapsulation_rejected() {
    assert_eq!(CdrReader::new(&[]).unwrap_err(), CdrError::Truncated);
    assert_eq!(
        CdrReader::new(&[0x00, 0x00, 0x00, 0x00]).unwrap_err(),
        CdrError::BadEncapsulation
    );
    assert_eq!(
        CdrReader::new(&[0x00, 0x01, 0x00]).unwrap_err(),
        CdrError::Truncated
    );
}

#[test]
fn primitives_round_trip() {
    let mut r = reader(&[
        0xAB, // u8
        0xFF, // i8 (-1)
        0x01, // bool
        0x00, // pad for u16
        0x34, 0x12, // u16
        0xFE, 0xFF, // i16 (-2)
        0x78, 0x56, 0x34, 0x12, // u32
        0xFE, 0xFF, 0xFF, 0xFF, // i32 (-2)
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // u64
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // i64 (-1)
        0x00, 0x00, 0xC0, 0x3F, // f32 1.5
        0x00, 0x00, 0x00, 0x00, // pad for f64
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xC0, // f64 -2.5
        0x03, 0x00, 0x00, 0x00, b'h', b'i', 0x00, // string "hi"
        0x00, // pad for sequence count
        0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x40,
    ]);
    assert_eq!(r.read_u8().unwrap(), 0xAB);
    assert_eq!(r.read_i8().unwrap(), -1);
    assert!(r.read_bool().unwrap());
    assert_eq!(r.read_u16().unwrap(), 0x1234);
    assert_eq!(r.read_i16().unwrap(), -2);
    assert_eq!(r.read_u32().unwrap(), 0x1234_5678);
    assert_eq!(r.read_i32().unwrap(), -2);
    assert_eq!(r.read_u64().unwrap(), 0x0102_0304_0506_0708);
    assert_eq!(r.read_i64().unwrap(), -1);
    assert_eq!(r.read_f32().unwrap(), 1.5);
    assert_eq!(r.read_f64().unwrap(), -2.5);
    assert_eq!(r.read_string().unwrap(), "hi");
    assert_eq!(r.read_sequence::<f32>().unwrap(), vec![1.0, 2.0]);
    assert_eq!(r.remaining(), 0);
}

#[test]
fn string_errors() {
    // zero length
    assert_eq!(
        reader(&[0x00, 0x00, 0x00, 0x00]).read_string().unwrap_err(),
        CdrError::InvalidString
    );
    // missing NUL
    assert_eq!(
        reader(&[0x02, 0x00, 0x00, 0x00, b'a', b'b'])
            .read_string()
            .unwrap_err(),
        CdrError::InvalidString
    );
    // invalid UTF-8
    assert_eq!(
        reader(&[0x02, 0x00, 0x00, 0x00, 0xFF, 0x00])
            .read_string()
            .unwrap_err(),
        CdrError::InvalidString
    );
    // bad bool
    assert_eq!(
        reader(&[0x02]).read_bool().unwrap_err(),
        CdrError::InvalidBool
    );
}

#[test]
fn truncated_every_primitive() {
    type ReadFn = fn(&mut CdrReader<'_>) -> Result<(), CdrError>;
    let cases: [(&str, &[u8], ReadFn); 12] = [
        ("u8", &[0xAB], |r| r.read_u8().map(|_| ())),
        ("i8", &[0xAB], |r| r.read_i8().map(|_| ())),
        ("bool", &[0x01], |r| r.read_bool().map(|_| ())),
        ("u16", &[0x34, 0x12], |r| r.read_u16().map(|_| ())),
        ("i16", &[0xFE, 0xFF], |r| r.read_i16().map(|_| ())),
        ("u32", &[0x78, 0x56, 0x34, 0x12], |r| {
            r.read_u32().map(|_| ())
        }),
        ("i32", &[0x78, 0x56, 0x34, 0x12], |r| {
            r.read_i32().map(|_| ())
        }),
        (
            "u64",
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            |r| r.read_u64().map(|_| ()),
        ),
        (
            "i64",
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            |r| r.read_i64().map(|_| ()),
        ),
        ("f32", &[0x00, 0x00, 0xC0, 0x3F], |r| {
            r.read_f32().map(|_| ())
        }),
        (
            "f64",
            &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xC0],
            |r| r.read_f64().map(|_| ()),
        ),
        ("string", &[0x03, 0x00, 0x00, 0x00, b'h', b'i', 0x00], |r| {
            r.read_string().map(|_| ())
        }),
    ];
    for (name, good, read) in cases {
        let mut r = reader(good);
        assert!(read(&mut r).is_ok(), "{name} should read");
        let short = &good[..good.len() - 1];
        let mut r = reader(short);
        assert!(read(&mut r).is_err(), "{name} must fail when truncated");
    }

    // sequence count says two f32 but only one is present
    let seq = payload(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3F]);
    let mut r = CdrReader::new(&seq).expect("header");
    assert!(r.read_sequence::<f32>().is_err());
}

#[test]
fn f64_after_u8_aligns_from_post_header_origin() {
    // One u8 then an f64: 7 bytes of padding from the payload origin.
    let mut rest = vec![0x11];
    rest.extend_from_slice(&[0; 7]);
    rest.extend_from_slice(&3.25_f64.to_le_bytes());
    let mut r = reader(&rest);
    assert_eq!(r.read_u8().unwrap(), 0x11);
    assert_eq!(r.position(), 1);
    assert_eq!(r.read_f64().unwrap(), 3.25);
    assert_eq!(r.position(), 16);

    // Without the padding the reader must not silently misread.
    let mut bad = vec![0x11];
    bad.extend_from_slice(&3.25_f64.to_le_bytes());
    let mut r = reader(&bad);
    assert_eq!(r.read_u8().unwrap(), 0x11);
    assert!(r.read_f64().is_err());
}

#[test]
fn laser_scan_truncated_is_error() {
    let scan = payload(&[0x01, 0x00, 0x00, 0x00]);
    assert!(LaserScanV1::decode(&scan).is_err());
    assert!(LaserScanV1::decode(&[]).is_err());
}
