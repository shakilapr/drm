//! Runtime side of the generated deployment configuration (REQ-CORE-108): the
//! artifact `drm-workspace::deploy` emits is loaded and drives the startup guard
//! and the topic-slot table, so no caller hand-builds an expectation.
//! REQ: REQ-CORE-108, REQ-CORE-092, REQ-CORE-084, REQ-CORE-088.

use super::{CONFIG_FORMAT_VERSION, ContractFingerprint, DeploymentConfig, start_session};
use drm_types::Status;

/// Deployment id used by the fixtures (32 lowercase hex).
const ID: &str = "91ca8fa49e2b4f48a25c7ea98dcb84fe";
/// Node expectation digest used by the fixtures.
const PRODUCER: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
/// Schema digest used by the fixtures.
const SCHEMA: &str = "11223344556677889900112233445566";
/// Layout digest used by the fixtures.
const LAYOUT: &str = "99aabbccddeeff001122334455667788";

/// A configuration text in the exact shape the toolchain emits.
fn text(format_version: u64, abi_version: u32, overflow: &str) -> String {
    format!(
        r#"{{
  "format_version": {format_version},
  "abi_version": {abi_version},
  "deployment_id": "{ID}",
  "inputs_digest": "{SCHEMA}",
  "fingerprint_expectations": {{
    "producer": "{PRODUCER}"
  }},
  "topics": {{
    "counter/value": {{
      "slot": 5,
      "schema_hash": "{SCHEMA}",
      "layout_hash": "{LAYOUT}",
      "payload_capacity": 8,
      "slot_align": 8,
      "history_depth": 64,
      "pool_size": 128,
      "overflow": "{overflow}"
    }}
  }}
}}
"#
    )
}

/// A well-formed configuration text.
fn valid() -> String {
    text(CONFIG_FORMAT_VERSION, drm_types::ABI_VERSION, "overwrite")
}

/// REQ: REQ-CORE-108, REQ-CORE-084, REQ-CORE-088 — a loaded configuration yields
/// the deployment identity, the node expectation, and the topic-slot table the
/// runtime session needs.
#[test]
fn config_carries_runtime_identities() {
    let config = DeploymentConfig::parse(&valid()).unwrap_or_else(|e| unreachable!("parse: {e:?}"));

    let id = config.id().unwrap_or_else(|e| unreachable!("id: {e:?}"));
    assert_eq!(id.encoded(), ID, "id round-trips through hex");

    let expected = config
        .expectation("producer")
        .unwrap_or_else(|| unreachable!("producer expectation"));
    assert_eq!(expected.0[0], 0x0a, "digest bytes");

    assert!(
        config.expectation("consumer").is_none(),
        "a node the deployment does not know has no expectation"
    );

    let topic = config
        .topic("counter/value")
        .unwrap_or_else(|| unreachable!("topic"));
    assert_eq!(topic.slot, 5);
    assert_eq!(topic.payload_capacity, 8);
    assert_eq!(topic.history_depth, 64);
    assert_eq!(topic.pool_size, 128);
    assert_eq!(topic.overflow, "overwrite");
    assert_eq!(
        config.topic("counter/absent").map(|t| t.slot),
        None,
        "an undeployed topic is absent, never defaulted"
    );

    assert_eq!(
        config.slots(),
        std::collections::BTreeMap::from([("counter/value".to_string(), 5_u32)])
    );

    // The slot drives the same generated name family the runtime uses elsewhere.
    let name = id
        .segment_name(u64::from(5_u32))
        .unwrap_or_else(|e| unreachable!("name: {e:?}"));
    assert_eq!(name.as_str(), format!("/drm.{ID}.0000000000000005"));
}

/// REQ: REQ-CORE-108 — a configuration from another format version is refused
/// rather than interpreted loosely.
#[test]
fn unknown_format_version_refused() {
    for version in [0_u64, 1, CONFIG_FORMAT_VERSION + 1] {
        let text = text(version, drm_types::ABI_VERSION, "overwrite");
        assert_eq!(
            DeploymentConfig::parse(&text).err(),
            Some(Status::InvalidData),
            "format_version {version} must be refused"
        );
    }
}

/// REQ: REQ-CORE-092 — a configuration generated against another ABI is refused:
/// the binary must not attach under an incompatible runtime contract.
#[test]
fn abi_mismatch_refused() {
    let text = text(
        CONFIG_FORMAT_VERSION,
        drm_types::ABI_VERSION.wrapping_add(1),
        "overwrite",
    );
    assert_eq!(DeploymentConfig::parse(&text).err(), Some(Status::WrongAbi));
}

/// REQ: REQ-CORE-108 — malformed digests, slots and overflow policies are refused
/// in every field, so a truncated or hand-edited artifact cannot half-load.
#[test]
fn malformed_fields_refused() {
    let cases = [
        valid().replace(ID, "not-hex"),
        valid().replace(ID, "91ca8fa4"),
        valid().replace(LAYOUT, &LAYOUT.to_uppercase()),
        valid().replace(LAYOUT, "xyz"),
        valid().replace("\"slot\": 5", "\"slot\": 4294967296"),
        valid().replace("\"overflow\": \"overwrite\"", "\"overflow\": \"grow\""),
        valid().replace(
            &format!("\"producer\": \"{PRODUCER}\""),
            "\"producer\": \"\"",
        ),
        valid().replace("\"inputs_digest\": \"", "\"inputs_digest\": \"!"),
    ];
    for (index, case) in cases.iter().enumerate() {
        assert_eq!(
            DeploymentConfig::parse(case).err(),
            Some(Status::InvalidData),
            "case {index} must be refused: {case}"
        );
    }
    // Control: the untouched fixture still loads.
    assert!(DeploymentConfig::parse(&valid()).is_ok());
}

/// REQ: REQ-CORE-108, REQ-CORE-092 — startup takes the expectation from the
/// loaded configuration: a matching fingerprint starts, a stale binary and an
/// unknown node are both refused before any lock is created.
#[cfg(unix)]
#[test]
fn start_session_uses_loaded_expectation() {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::shm::{FlockBackend, LockBackend, LockPaths};
    use crate::test_guard;

    static SEQ: AtomicU32 = AtomicU32::new(0);
    let _serial = test_guard::acquire();
    let locks = FlockBackend;
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir: PathBuf = std::env::temp_dir().join(format!("drm-config-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("lock dir: {e}"));
    let paths = LockPaths::new(&dir);

    let config = DeploymentConfig::parse(&valid()).unwrap_or_else(|e| unreachable!("parse: {e:?}"));
    let compiled = ContractFingerprint([
        0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8,
        0xf9,
    ]);

    // The compiled binary matches the deployment's expectation for this node.
    let hold = start_session(&locks, &paths, &config, "producer", compiled)
        .unwrap_or_else(|e| unreachable!("matching startup: {e:?}"));
    assert!(paths.active().exists(), "startup acquired the shared hold");
    locks.release(hold);

    // A stale binary (different contract) is refused, and refused BEFORE any lock.
    let stale = ContractFingerprint([0xff; 16]);
    let active_before = paths.active().exists();
    assert_eq!(
        start_session(&locks, &paths, &config, "producer", stale).err(),
        Some(Status::WrongAbi)
    );
    assert_eq!(
        paths.active().exists(),
        active_before,
        "a guard refusal touches no lock"
    );

    // A node the deployment does not know cannot start at all.
    assert_eq!(
        start_session(&locks, &paths, &config, "intruder", compiled).err(),
        Some(Status::NotFound)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// REQ: REQ-CORE-108 — a missing configuration is `NotFound`, never a defaulted
/// deployment: a node must not start without generated connection data.
#[test]
fn missing_config_is_not_found() {
    let path = std::env::temp_dir().join(format!("drm-absent-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    assert_eq!(DeploymentConfig::load(&path).err(), Some(Status::NotFound));
}
