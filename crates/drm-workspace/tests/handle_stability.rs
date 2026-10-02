//! Handle assignment stability tests ([handle-system.md](../../../docs/algorithms/toolchain/handle-system.md)
//! Test Matrix; work-plan M02W2-E2-T1/T2). REQ: REQ-TOOL-036, REQ-TOOL-038,
//! REQ-TOOL-082.

use std::collections::{BTreeMap, BTreeSet};

use drm_workspace::model::handles::{AssignDelta, AssignmentHistory, HandleTable, ReserveReason};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn topics(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// REQ: REQ-TOOL-036 — I-H1: same workspace + same history ⇒ identical table,
/// ×100 runs (determinism ×100).
#[test]
fn handle_determinism() {
    let mut prior = AssignmentHistory::default();
    prior.assignments.insert("pkg/front".to_string(), 0);
    prior.assignments.insert("pkg/cam".to_string(), 1);
    let current = topics(&["pkg/front", "pkg/cam", "pkg/new"]);
    let (base, _) = fixture(HandleTable::from_history(&prior).assign(&current), "first");
    for _ in 0..100 {
        let (t, _) = fixture(HandleTable::from_history(&prior).assign(&current), "repeat");
        assert_eq!(t, base, "I-H1 byte-identical table");
    }
}

/// REQ: REQ-TOOL-036 — empty history + same workspace ⇒ identical initial
/// table (fresh determinism).
#[test]
fn handle_fresh_deterministic() {
    let current = topics(&["b", "a", "c"]);
    let (t1, d1) = fixture(HandleTable::default().assign(&current), "fresh1");
    let (t2, d2) = fixture(HandleTable::default().assign(&current), "fresh2");
    assert_eq!(t1, t2);
    assert_eq!(d1, d2);
    // Sorted allocation: a→0, b→1, c→2 (deterministic order).
    assert_eq!(t1.assignments["a"], 0);
    assert_eq!(t1.assignments["b"], 1);
    assert_eq!(t1.assignments["c"], 2);
    assert_eq!(t1.next_free, 3);
}

/// REQ: REQ-TOOL-036 — I-H2: adding an unrelated topic never renumbers the
/// existing assignments.
#[test]
fn handle_add_stability() {
    let before = topics(&["pkg/front", "pkg/cam"]);
    let (t0, _) = fixture(HandleTable::default().assign(&before), "before");
    let mut after_set = before.clone();
    after_set.insert("pkg/motor".to_string());
    let (t1, deltas) = fixture(t0.assign(&after_set), "after");
    assert_eq!(t1.assignments["pkg/front"], t0.assignments["pkg/front"]);
    assert_eq!(t1.assignments["pkg/cam"], t0.assignments["pkg/cam"]);
    assert_eq!(t1.assignments["pkg/motor"], 2, "fresh allocation above all");
    assert_eq!(deltas, vec![AssignDelta::Added("pkg/motor".to_string(), 2)]);
}

/// REQ: REQ-TOOL-036 — I-H3/I-H5: removing a topic tombstones its handle;
/// re-adding it (or any new topic) never reuses the number until compaction.
#[test]
fn handle_delete_readd() {
    let initial = topics(&["pkg/front", "pkg/cam", "pkg/motor"]);
    let (t0, _) = fixture(HandleTable::default().assign(&initial), "initial");
    // Sorted allocation: cam < front < motor.
    assert_eq!(t0.assignments["pkg/cam"], 0);
    assert_eq!(t0.assignments["pkg/front"], 1);
    assert_eq!(t0.assignments["pkg/motor"], 2);

    // Delete pkg/cam.
    let remaining = topics(&["pkg/front", "pkg/motor"]);
    let (t1, deltas) = fixture(t0.assign(&remaining), "delete");
    assert_eq!(deltas, vec![AssignDelta::Removed("pkg/cam".to_string(), 0)]);
    assert!(!t1.assignments.contains_key("pkg/cam"));
    assert_eq!(
        t1.reserved.get(&0),
        Some(&ReserveReason::Deleted),
        "tombstone"
    );

    // Re-add a different new topic: must take 3, not the tombstoned 0.
    let readd = topics(&["pkg/front", "pkg/motor", "pkg/lidar"]);
    let (t2, d2) = fixture(t1.assign(&readd), "readd");
    assert_eq!(t2.assignments["pkg/lidar"], 3, "no silent reuse (I-H5)");
    assert_eq!(d2, vec![AssignDelta::Added("pkg/lidar".to_string(), 3)]);
    // Re-adding the very same topic name also gets a fresh ID (history shows
    // delete+add ⇒ new number; identity is the path, the number is not).
    let back = topics(&["pkg/front", "pkg/motor", "pkg/lidar", "pkg/cam"]);
    let (t3, _) = fixture(t2.assign(&back), "back");
    assert_eq!(t3.assignments["pkg/cam"], 4, "tombstoned 0 stays reserved");
}

/// REQ: REQ-TOOL-036 — I-H6: exhaustion at the u32 boundary is a typed error,
/// never a wrapping increment. The final free slot (`u32::MAX`) is allocatable;
/// the table then saturates and further allocations fail.
#[test]
fn handle_exhaustion() {
    // Last free slot allocatable: prior top = MAX-1 ⇒ new topic takes MAX.
    let mut prior = AssignmentHistory::default();
    prior
        .assignments
        .insert("pkg/front".to_string(), u32::MAX - 1);
    let current = topics(&["pkg/front", "pkg/new"]);
    let (t, _) = fixture(
        HandleTable::from_history(&prior).assign(&current),
        "last slot",
    );
    assert_eq!(t.assignments["pkg/new"], u32::MAX);
    assert_eq!(t.next_free, u32::MAX, "table saturated");
    // MAX now taken ⇒ any further allocation errors (typed, no wrap).
    let more = topics(&["pkg/front", "pkg/new", "pkg/one_more"]);
    let r = t.assign(&more);
    assert!(r.is_err(), "no silent wrap after saturation");
    // MAX reserved ⇒ same typed error.
    let mut prior_r = AssignmentHistory::default();
    prior_r
        .assignments
        .insert("pkg/front".to_string(), u32::MAX - 1);
    prior_r
        .reserved
        .insert(u32::MAX, ReserveReason::MergeCollision);
    let r2 = HandleTable::from_history(&prior_r).assign(&topics(&["pkg/front", "pkg/x"]));
    assert!(r2.is_err(), "reserved MAX blocks allocation");
    // Full run up to exhaustion from a small table.
    let mut prior2 = AssignmentHistory::default();
    prior2
        .assignments
        .insert("pkg/front".to_string(), u32::MAX - 2);
    let (t2, _) = fixture(
        HandleTable::from_history(&prior2).assign(&topics(&["pkg/front", "pkg/new", "pkg/new2"])),
        "near-max",
    );
    assert_eq!(t2.assignments["pkg/new"], u32::MAX - 1);
    assert_eq!(t2.assignments["pkg/new2"], u32::MAX);
    assert_eq!(t2.next_free, u32::MAX, "table is full");
    let r3 = t2.assign(&topics(&[
        "pkg/front",
        "pkg/new",
        "pkg/new2",
        "pkg/one_more",
    ]));
    assert!(r3.is_err(), "no silent wrap");
}

/// REQ: REQ-TOOL-038 — compaction core: dense sorted re-pack, reservations
/// dropped, `next_free` == live count (the CLI adds the ack + identity gates).
#[test]
fn handle_compact_repack() {
    let mut prior = AssignmentHistory::default();
    prior.assignments.insert("a".to_string(), 5);
    prior.assignments.insert("b".to_string(), 9);
    prior.reserved.insert(7, ReserveReason::Deleted);
    let t = HandleTable::from_history(&prior);
    let c = t.compact_repack();
    assert_eq!(c.assignments["a"], 0);
    assert_eq!(c.assignments["b"], 1);
    assert!(
        c.reserved.is_empty(),
        "coordinated redeployment clears tombstones"
    );
    assert_eq!(c.next_free, 2);
}

/// REQ: REQ-TOOL-080 — lockfile content round-trips through the `AssignmentHistory`
/// struct (serde-JSON in the CLI layer; here the map semantics are the contract).
#[test]
fn handle_history_semantics() {
    let mut prior = AssignmentHistory::default();
    prior.assignments.insert("pkg/front".to_string(), 0);
    prior.reserved.insert(2, ReserveReason::MergeCollision);
    let t = HandleTable::from_history(&prior);
    assert_eq!(t.assignments, prior.assignments);
    assert_eq!(t.reserved, prior.reserved);
    assert_eq!(t.next_free, 3, "above every live/reserved ID");
    // Empty history: next_free 0.
    let t0 = HandleTable::from_history(&AssignmentHistory {
        assignments: BTreeMap::new(),
        reserved: BTreeMap::new(),
    });
    assert_eq!(t0.next_free, 0);
}
