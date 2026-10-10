//! Transport-capability registry tests (work-plan Phase 29 seed):
//! unregistered kinds are explicit configuration errors, registered kinds
//! select their capability class, and the default table is internally
//! consistent.
//! REQ: REQ-ROUTE-034, REQ-ROUTE-009.

use drm_router::transport::Reliability;
use drm_router::transport::registry::{
    AdapterSelection, TransportCaps, TransportKind, TransportRegistry,
};
use drm_types::Status;

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// REQ: REQ-ROUTE-034 — an unregistered kind yields an explicit configuration
/// error (never a panic): the empty registry rejects every kind and the
/// default table leaves [`TransportKind::Disabled`] unmapped.
#[test]
fn registry_unknown_mapping_is_config_error() {
    let empty = TransportRegistry::new();
    for kind in TransportKind::ALL {
        assert_eq!(
            empty.lookup(kind),
            Err(Status::InvalidConfig),
            "empty registry kind {kind:?}"
        );
        assert_eq!(empty.select(kind), Err(Status::InvalidConfig));
        assert!(!empty.is_registered(kind));
    }

    let defaults = TransportRegistry::from_defaults();
    assert_eq!(
        defaults.lookup(TransportKind::Disabled),
        Err(Status::InvalidConfig),
        "disabled link is not a transport mapping"
    );
    assert_eq!(
        defaults.select(TransportKind::Disabled),
        Err(Status::InvalidConfig)
    );
    assert!(!defaults.is_registered(TransportKind::Disabled));
}

/// REQ: REQ-ROUTE-034, REQ-ROUTE-009 — every default kind selects a capability
/// class derived from the static table (adapter/caps returned).
#[test]
fn registry_known_mappings_select_adapter() {
    let registry = TransportRegistry::from_defaults();

    let udp = fixture(registry.select(TransportKind::Udp), "udp select");
    assert_eq!(
        udp,
        AdapterSelection {
            kind: TransportKind::Udp,
            caps: TransportCaps {
                reliable: false,
                max_frame: 1_400,
                datagram: true,
            },
        }
    );
    assert_eq!(udp.reliability(), Reliability::BestEffort);
    assert!(!udp.caps.reliable);
    assert!(!udp.caps.is_stream());

    let tcp = fixture(registry.select(TransportKind::Tcp), "tcp select");
    assert_eq!(tcp.kind, TransportKind::Tcp);
    assert!(tcp.caps.reliable);
    assert!(!tcp.caps.datagram);
    assert!(tcp.caps.is_stream());
    assert_eq!(tcp.reliability(), Reliability::Reliable);

    let serial = fixture(registry.select(TransportKind::Serial), "serial select");
    assert_eq!(serial.kind, TransportKind::Serial);
    assert!(!serial.caps.reliable);
    assert!(serial.caps.is_stream());
    assert_eq!(serial.reliability(), Reliability::BestEffort);

    // lookup agrees with select.
    for kind in [
        TransportKind::Udp,
        TransportKind::Tcp,
        TransportKind::Serial,
    ] {
        assert_eq!(
            fixture(registry.lookup(kind), "lookup"),
            registry_caps(&registry, kind)
        );
    }
}

fn registry_caps(registry: &TransportRegistry, kind: TransportKind) -> TransportCaps {
    fixture(registry.select(kind), "select for caps").caps
}

/// REQ: REQ-ROUTE-034 — the default capability table is internally consistent:
/// datagram bindings are best-effort, reliable bindings are streams, serial is
/// a best-effort stream (REQ-ROUTE-024), and every frame bound is non-zero.
#[test]
fn registry_capability_table_consistent() {
    let registry = TransportRegistry::from_defaults();
    for kind in [
        TransportKind::Udp,
        TransportKind::Tcp,
        TransportKind::Serial,
    ] {
        let caps = fixture(registry.lookup(kind), "lookup");
        assert!(caps.max_frame > 0, "{kind:?} frame bound");
        if caps.datagram {
            assert!(!caps.reliable, "{kind:?}: datagram is best-effort");
        }
        if caps.reliable {
            assert!(caps.is_stream(), "{kind:?}: reliable is a stream");
        }
    }
    // Serial is explicitly best-effort over a stream (REQ-ROUTE-024).
    let serial = fixture(registry.lookup(TransportKind::Serial), "serial lookup");
    assert!(!serial.reliable && serial.is_stream());
}

/// REQ: REQ-ROUTE-034 — registration rejects a zero frame bound
/// ([`Status::InvalidConfig`]) and a duplicate row
/// ([`Status::AlreadyExists`]) without replacing the original.
#[test]
fn registry_register_validates_rows() {
    let mut registry = TransportRegistry::new();
    let valid = TransportCaps {
        reliable: false,
        max_frame: 512,
        datagram: false,
    };
    assert_eq!(
        registry.register(
            TransportKind::Serial,
            TransportCaps {
                reliable: false,
                max_frame: 0,
                datagram: false,
            }
        ),
        Err(Status::InvalidConfig),
        "zero frame bound rejected"
    );
    assert_eq!(registry.register(TransportKind::Serial, valid), Ok(()));
    assert_eq!(
        registry.register(
            TransportKind::Serial,
            TransportCaps {
                reliable: true,
                max_frame: 999,
                datagram: false,
            }
        ),
        Err(Status::AlreadyExists),
        "duplicate rejected"
    );
    assert_eq!(registry.lookup(TransportKind::Serial), Ok(valid));
    assert!(registry.is_registered(TransportKind::Serial));
    assert!(!registry.is_registered(TransportKind::Udp));
}
