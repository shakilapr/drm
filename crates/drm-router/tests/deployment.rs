//! Router deployment namespace/isolation tests (work-plan M05W4-E3-T1/E3-T2,
//! E4-T2): the router derives backend-safe keys from the runtime deployment
//! identity, so two deployments that use the same topic slot never share a
//! namespace name (I-DP2). Deterministic — fixed ids, no randomness.
//! REQ: REQ-CORE-084, REQ-CORE-088, REQ-CORE-089.

use std::path::Path;

use drm_router::deployment::{DeploymentId, DeploymentNamespace};

/// Fixture: derive a segment name under `ns`, surfacing a suite bug loudly.
fn name_of(ns: DeploymentNamespace, slot: u64) -> drm_core::shm::SegmentName {
    ns.segment_name(slot)
        .unwrap_or_else(|e| unreachable!("segment name ({slot}): {e:?}"))
}

/// REQ: REQ-CORE-084, REQ-CORE-088 — the reused identity encodes to a fixed
/// 32-digit backend-safe token (no `/`, so it is namespace-safe).
#[test]
fn deployment_id_encoded_is_fixed_width() {
    for (value, want) in [
        (0_u128, "00000000000000000000000000000000"),
        (1, "00000000000000000000000000000001"),
        (0xDEAD_BEEF, "000000000000000000000000deadbeef"),
        (u128::MAX, "ffffffffffffffffffffffffffffffff"),
    ] {
        assert_eq!(DeploymentId::new(value).encoded(), want, "value {value:#x}");
    }
}

/// REQ: REQ-CORE-088 — the namespace's segment name and lock paths are the
/// documented backend-safe shapes, derived solely from the id and slot.
#[test]
fn deployment_namespace_key_shape() {
    let ns = DeploymentNamespace::new(DeploymentId::new(1));
    let name = name_of(ns, 7);
    assert_eq!(
        name.as_str(),
        "/drm.00000000000000000000000000000001.0000000000000007"
    );
    assert!(ns.owns(&name));
    assert!(!ns.owns(&name_of(DeploymentNamespace::new(DeploymentId::new(2)), 7)));

    let root = Path::new("/run/drm");
    let dir = root.join(ns.id().encoded());
    assert_eq!(ns.lock_paths(root).active(), dir.join("active.lock"));
}

/// REQ: REQ-CORE-089 — two deployments with the same topic slot derive
/// disjoint segment names, and each owns only its own (I-DP2).
#[test]
fn cross_deploy_same_topic_disjoint_keys() {
    let a = DeploymentNamespace::new(DeploymentId::new(0xA11CE));
    let b = DeploymentNamespace::new(DeploymentId::new(0xB0B));
    for slot in [0_u64, 1, 7, 42, u64::MAX] {
        let na = name_of(a, slot);
        let nb = name_of(b, slot);
        assert_ne!(na.as_str(), nb.as_str(), "slot {slot}");
        assert!(a.owns(&na) && !a.owns(&nb), "slot {slot}");
        assert!(b.owns(&nb) && !b.owns(&na), "slot {slot}");
    }
    let root = Path::new("/run/drm");
    assert_ne!(a.lock_paths(root).active(), b.lock_paths(root).active());
}

/// REQ: REQ-CORE-088 — derivation is a pure function of `(id, slot)`:
/// identical ids are stable, distinct ids are distinct, repeated calls agree.
#[test]
fn deployment_namespace_is_deterministic() {
    assert_eq!(
        DeploymentNamespace::new(DeploymentId::new(9)),
        DeploymentNamespace::new(DeploymentId::new(9))
    );
    assert_ne!(
        DeploymentNamespace::new(DeploymentId::new(9)),
        DeploymentNamespace::new(DeploymentId::new(10))
    );
    let ns = DeploymentNamespace::new(DeploymentId::new(9));
    assert_eq!(name_of(ns, 3).as_str(), name_of(ns, 3).as_str());
    assert_eq!(ns.id().encoded(), DeploymentId::new(9).encoded());
}
