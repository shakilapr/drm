//! Route table tests (work-plan M02W3-E3-T1): insert/lookup keyed only by
//! the numeric topic handle; no string keys on the data path.
//! REQ: REQ-ROUTE-043, REQ-ROUTE-022.

use drm_router::routes::{RouteEntry, RouteTable};
use drm_types::Status;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn entry(channel: u32) -> RouteEntry {
    RouteEntry {
        wire_channel_id: channel,
        export_limit_period_ns: None,
        max_record_bytes: 1_024,
    }
}

/// REQ: REQ-ROUTE-043 — handle-keyed insert/lookup: every registered handle
/// resolves to its exact entry; unknown handles return `None`; duplicates
/// are rejected without replacing the original.
#[test]
fn handle_keyed_insert_lookup() {
    let handles = [
        (0u32, 10u32),
        (7, 999),
        (0x0124, 0xFFFF_FFFF),
        (u32::MAX, 1),
    ];
    let mut table = RouteTable::new();
    assert!(table.is_empty());
    for (handle, channel) in handles {
        fixture(table.insert(handle, entry(channel)), "insert");
    }
    assert_eq!(table.len(), 4);
    for (handle, channel) in handles {
        assert_eq!(
            table.route_for(handle),
            Some(&entry(channel)),
            "handle {handle}"
        );
    }
    assert_eq!(table.route_for(8), None, "unknown handle");
    assert_eq!(
        table.insert(7, entry(123_456)),
        Err(Status::AlreadyExists),
        "duplicate handle rejected"
    );
    assert_eq!(table.route_for(7), Some(&entry(999)), "original preserved");
}

/// REQ: REQ-ROUTE-043 — the handle is the only lookup key: a route whose
/// wire channel id equals another handle's value never cross-resolves
/// (no string keys, no channel-id keys on the data path).
#[test]
fn handle_is_the_only_key() {
    let mut table = RouteTable::new();
    fixture(table.insert(1, entry(999)), "insert");
    fixture(table.insert(2, entry(1)), "insert");
    fixture(table.insert(0, entry(3)), "insert");
    assert_eq!(
        table.route_for(1),
        Some(&entry(999)),
        "handle 1, channel 999"
    );
    assert_eq!(table.route_for(2), Some(&entry(1)), "handle 2, channel 1");
    assert_eq!(table.route_for(0), Some(&entry(3)));
    assert_eq!(table.route_for(999), None, "channel id is not a key");
}
