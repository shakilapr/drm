//! Cursor delivery property harness: `next()` delivers every committed sequence
//! exactly once unless it was already aged out when the cursor reached it, in
//! which case exactly one `Gap` with the exact count is reported and the cursor
//! resumes at the oldest retained sequence (I-C4).
//! REQ: REQ-CORE-013, REQ-CORE-028, REQ-CORE-029.

use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use proptest::prelude::*;

/// Overwrite channel with `pool = depth + 1` (the minimal drop-free pool).
fn activate(depth: u64) -> Channel {
    Channel::activate(
        depth,
        depth + 1,
        16,
        8,
        OverflowPolicy::Overwrite,
        0,
        1,
        None,
    )
    .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// REQ: REQ-CORE-013, REQ-CORE-029 — against an independently derived model,
    /// `next()` never skips a sequence that is still retained (I-C4).
    #[test]
    fn next_no_skip(
        depth in 1_u64..=8,
        ops in prop::collection::vec(any::<bool>(), 1..=120),
    ) {
        let c = activate(depth);
        let mut st = c.reader();
        let mut buf = [0_u8; 16];
        let mut published: u64 = 0; // committed sequences == write_seq
        let mut last_read: Option<u64> = None; // model mirror of the cursor

        for publish in ops {
            if publish {
                if published >= 256 {
                    continue;
                }
                let value = u8::try_from(published).unwrap_or_else(|_| unreachable!("<256"));
                prop_assert_eq!(c.publish(&[value; 16]), Ok(published));
                published += 1;
                continue;
            }
            // Model: identical arithmetic to cursor-delivery.md §Functions `next`.
            let oldest = published.saturating_sub(depth);
            let wanted = match last_read {
                None => oldest,
                Some(last) => last + 1,
            };
            if wanted < oldest {
                let missed = oldest - wanted;
                prop_assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Gap { missed }));
                last_read = Some(oldest - 1);
            } else if wanted >= published {
                prop_assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Empty));
            } else {
                prop_assert_eq!(c.next(&mut st, 0, &mut buf), Ok(ReadOutcome::Sample));
                let value = u8::try_from(wanted).unwrap_or_else(|_| unreachable!("<256"));
                prop_assert_eq!(buf, [value; 16]);
                last_read = Some(wanted);
            }
        }
    }
}
