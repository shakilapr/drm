//! Fault-injection fixture (work-plan M03W2-E4-T3): a patterned sample
//! published and read through the live channel validates clean; a single
//! flipped byte is detected as `Invalid`; blended halves are `Torn` /
//! `MixedGeneration`. Permanent negative test.
//! REQ: REQ-CORE-082, REQ-QAL-006.

use drm_core::channel::overflow::OverflowPolicy;
use drm_core::channel::pattern::{Anomaly, FILL_START, validate_pattern, write_pattern};
use drm_core::channel::{Channel, ReadOutcome};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn channel() -> Channel {
    fixture(
        Channel::activate(2, 3, 128, 8, OverflowPolicy::Overwrite, 0, 1, None),
        "activate",
    )
}

/// REQ: REQ-CORE-082 — a pattern published and copied out byte-for-byte
/// validates; corruption in the copy is `Invalid` (exact class).
#[test]
fn pattern_corruption() {
    let ch = channel();
    let mut body = [0_u8; 128];
    write_pattern(&mut body, 0, 0, 1);
    assert_eq!(ch.publish(&body), Ok(0));

    let mut st = ch.reader();
    let mut out = [0_u8; 128];
    assert_eq!(ch.latest(&mut st, 0, &mut out), Ok(ReadOutcome::Sample));
    assert_eq!(validate_pattern(&out), Ok(()));

    out[FILL_START + 9] ^= 0x01;
    assert_eq!(validate_pattern(&out), Err(Anomaly::Invalid));
}

/// REQ: REQ-QAL-006 — a torn blend of two live publishes reports `Torn`; a
/// same-sequence generation blend reports `MixedGeneration`.
#[test]
fn pattern_torn_and_mixed() {
    let ch = channel();
    let mut a = [0_u8; 128];
    let mut b = [0_u8; 128];
    write_pattern(&mut a, 1, 0, 1);
    write_pattern(&mut b, 2, 0, 1);
    assert_eq!(ch.publish(&a), Ok(0));
    assert_eq!(ch.publish(&b), Ok(1));

    let mut torn = a;
    torn[64..].copy_from_slice(&b[64..]);
    assert_eq!(validate_pattern(&torn), Err(Anomaly::Torn));

    let mut c = [0_u8; 128];
    write_pattern(&mut c, 5, 0, 2);
    let mut mixed = a;
    write_pattern(&mut mixed, 5, 0, 1);
    let mut second = [0_u8; 128];
    write_pattern(&mut second, 5, 0, 2);
    mixed[64..].copy_from_slice(&second[64..]);
    let _ = c;
    assert_eq!(validate_pattern(&mixed), Err(Anomaly::MixedGeneration));
}
