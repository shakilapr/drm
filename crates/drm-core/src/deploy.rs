//! Deployment identity guard, namespace scoping, and guarded cleanup
//! (`docs/algorithms/deployment-identity.md` §Functions `node_startup`,
//! `segment_open`, `clean`; `docs/algorithms/runtime/shm-backend.md` §Functions
//! `remove_namespace`).
//!
//! Three-identity model: the compiled contract fingerprint (this module), the
//! deployment instance id (this module), and publisher epochs
//! (`crate::channel::lifecycle`). Namespace scoping reuses
//! [`crate::shm::backend_segment_name`]; locks reuse [`crate::shm::LockPaths`]
//! and [`crate::shm::LockBackend`].
//! REQ: REQ-CORE-084, REQ-CORE-085, REQ-CORE-088, REQ-CORE-089, REQ-CORE-092,
//! REQ-CORE-080, REQ-CORE-069, REQ-CORE-070.

use crate::shm::{
    CleanReport, LockBackend, LockHandle, LockPaths, SegmentName, ShmBackend, ShmError,
    backend_segment_name,
};
use drm_types::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Compiled contract fingerprint: hash over the node's own interface inputs
/// (ABI version, topic schemas/layouts, declared interfaces), truncated to 128
/// bits. Width parity with `drm-workspace::ContractFingerprint`; kept local so
/// `drm-core` needs no toolchain dependency (codebase.md 2.2 R2).
/// REQ: REQ-CORE-085.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractFingerprint(
    /// 128-bit truncated digest.
    pub [u8; 16],
);

/// 128-bit random deployment instance id (runtime config, never compiled into
/// binaries). Encoded as a backend-safe token for namespace derivation.
/// REQ: REQ-CORE-084.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeploymentId(u128);

impl DeploymentId {
    /// Wraps a raw 128-bit id.
    /// REQ: REQ-CORE-084.
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    /// Backend-safe namespace token: 32 lowercase hex digits (fixed width, no
    /// `/`), derived solely from the id — a hand-edited `deployment.json`
    /// cannot redirect a node or `drm clean` at a foreign namespace (I-DP5).
    /// REQ: REQ-CORE-084, REQ-CORE-088.
    #[must_use]
    pub fn encoded(self) -> String {
        let raw = self.0;
        format!("{raw:032x}")
    }

    /// Deployment-scoped segment name `/drm.<encoded-id>.<slot>`: two
    /// deployments never share a name and the canonical topic never reaches the
    /// kernel namespace (I-DP2).
    /// REQ: REQ-CORE-088, REQ-CORE-089.
    ///
    /// # Errors
    /// [`ShmError`] when the generated name violates name conformance.
    pub fn segment_name(self, slot: u64) -> Result<SegmentName, ShmError> {
        backend_segment_name(&self.encoded(), slot)
    }
}

/// Startup fingerprint guard (I-DP1): the compiled fingerprint must equal the
/// deployment's expectation for this node. Pure comparison — no lock, no SHM
/// touch — so a refusal can never create or open a segment.
/// REQ: REQ-CORE-092, REQ-CORE-025.
///
/// # Errors
/// [`Status::NotFound`] when the deployment records no expectation for this
/// node; [`Status::WrongAbi`] on mismatch.
pub fn guard_fingerprint(
    compiled: ContractFingerprint,
    expected: Option<ContractFingerprint>,
) -> Result<(), Status> {
    match expected {
        Some(e) if e == compiled => Ok(()),
        Some(_) => Err(Status::WrongAbi),
        None => Err(Status::NotFound),
    }
}

/// Node startup sequence (deployment-identity.md §Protocols): the fingerprint
/// guard runs FIRST, then the shared deployment active hold is acquired for the
/// node's whole lifetime. A guard refusal returns before any lock or SHM object
/// is touched.
/// REQ: REQ-CORE-092, REQ-CORE-025, REQ-CORE-069.
///
/// # Errors
/// [`Status::NotFound`]/[`Status::WrongAbi`] from [`guard_fingerprint`];
/// [`Status::AlreadyExists`] when a `drm clean` is in flight.
pub fn node_startup<L: LockBackend>(
    locks: &L,
    paths: &LockPaths,
    compiled: ContractFingerprint,
    expected: Option<ContractFingerprint>,
) -> Result<LockHandle, Status> {
    guard_fingerprint(compiled, expected)?;
    locks.try_shared(&paths.active()).map_err(Status::from)
}

/// Guarded namespace teardown (`drm clean`; shm-backend.md §Functions
/// `remove_namespace`): the exclusive deployment active lock proves no node
/// holds a shared hold. Success ⇒ every segment unlinked (`Cleaned`); a live
/// shared hold ⇒ nothing touched (`ActiveUntouched`). The exclusive handle is
/// held until all unlinking completes, so no node can acquire the namespace
/// mid-clean. Missing segments are tolerated (idempotent clean).
/// REQ: REQ-CORE-080, REQ-CORE-070, REQ-CORE-069.
///
/// # Errors
/// [`ShmError`] when the active lock cannot be probed or a segment unlink fails
/// for a reason other than absence.
pub fn clean<S: ShmBackend, L: LockBackend>(
    shm: &S,
    locks: &L,
    paths: &LockPaths,
    names: &[SegmentName],
) -> Result<CleanReport, ShmError> {
    let _exclusive = match locks.try_exclusive(&paths.active()) {
        Ok(h) => h,
        Err(ShmError::AlreadyExists) => return Ok(CleanReport::ActiveUntouched),
        Err(e) => return Err(e),
    };
    for name in names {
        match shm.unlink(name) {
            Ok(()) | Err(ShmError::NotFound) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(CleanReport::Cleaned)
}

// ---------------------------------------------------------------------------
// Generated deployment configuration (deployment-identity.md §Data Structures;
// REQ-CORE-108). The toolchain emits it; the runtime loads it so no caller ever
// hand-builds an expectation.
// ---------------------------------------------------------------------------

/// `format_version` of the emitted `deployment.json` this runtime understands. A
/// configuration from any other version is refused rather than interpreted
/// loosely. REQ: REQ-CORE-108.
pub const CONFIG_FORMAT_VERSION: u64 = 2;

/// One topic's connection data. The runtime validates it once, at attach.
/// REQ: REQ-CORE-108, REQ-CORE-088.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicConnection {
    /// Deployment-local topic slot; generated names derive from it.
    pub slot: u64,
    /// Canonical schema content hash (32 lowercase hex).
    pub schema_hash: String,
    /// Canonical layout hash (32 lowercase hex).
    pub layout_hash: String,
    /// Maximum payload bytes per sample.
    pub payload_capacity: u64,
    /// Segment alignment.
    pub slot_align: u64,
    /// Ring depth `R`.
    pub history_depth: u64,
    /// Payload pool slots.
    pub pool_size: u64,
    /// Overflow policy (`overwrite` | `reject`).
    pub overflow: String,
}

/// The generated deployment configuration: everything a process needs in order to
/// start and attach without hand-built expectations (REQ-CORE-108). This is
/// *runtime* configuration — never compiled into a node binary (I-DP3), which is
/// why the identity lives here and not in a build artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// Emitted shape version ([`CONFIG_FORMAT_VERSION`]).
    pub format_version: u64,
    /// Runtime ABI the artifact was generated against.
    pub abi_version: u32,
    /// Deployment instance id, 32 lowercase hex.
    pub deployment_id: String,
    /// Toolchain identity-reuse key (informational at runtime).
    pub inputs_digest: String,
    /// Node name → expected contract fingerprint (32 lowercase hex).
    pub fingerprint_expectations: BTreeMap<String, String>,
    /// Canonical topic path → connection data.
    pub topics: BTreeMap<String, TopicConnection>,
}

impl DeploymentConfig {
    /// Parses and validates generated configuration text.
    ///
    /// Validation is strict: an unknown `format_version`, an ABI this binary was
    /// not built for, or a malformed digest/slot/overflow value is refused. A
    /// partially understood configuration is never accepted.
    /// REQ: REQ-CORE-108, REQ-CORE-092.
    ///
    /// # Errors
    /// [`Status::InvalidData`] when the text is malformed or violates a shape
    /// invariant; [`Status::WrongAbi`] when `abi_version` differs from
    /// [`drm_types::ABI_VERSION`].
    pub fn parse(text: &str) -> Result<Self, Status> {
        let config: Self = serde_json::from_str(text).map_err(|_| Status::InvalidData)?;
        if config.format_version != CONFIG_FORMAT_VERSION {
            return Err(Status::InvalidData);
        }
        if config.abi_version != drm_types::ABI_VERSION {
            return Err(Status::WrongAbi);
        }
        if digest(&config.deployment_id).is_none() || digest(&config.inputs_digest).is_none() {
            return Err(Status::InvalidData);
        }
        for (node, hex) in &config.fingerprint_expectations {
            if node.is_empty() || digest(hex).is_none() {
                return Err(Status::InvalidData);
            }
        }
        for (topic, entry) in &config.topics {
            if topic.is_empty()
                || u32::try_from(entry.slot).is_err()
                || digest(&entry.schema_hash).is_none()
                || digest(&entry.layout_hash).is_none()
                || !matches!(entry.overflow.as_str(), "overwrite" | "reject")
            {
                return Err(Status::InvalidData);
            }
        }
        Ok(config)
    }

    /// Loads and validates the configuration at `path`.
    /// REQ: REQ-CORE-108.
    ///
    /// # Errors
    /// [`Status::NotFound`] when the file is absent (a node must not start
    /// without a deployment); otherwise as [`Self::parse`].
    pub fn load(path: &Path) -> Result<Self, Status> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Status::NotFound
            } else {
                Status::InvalidData
            }
        })?;
        Self::parse(&text)
    }

    /// The deployment instance id carried by the file.
    /// REQ: REQ-CORE-084, REQ-CORE-108.
    ///
    /// # Errors
    /// [`Status::InvalidData`] when the id is not 32 lowercase hex.
    pub fn id(&self) -> Result<DeploymentId, Status> {
        let bytes = digest(&self.deployment_id).ok_or(Status::InvalidData)?;
        Ok(DeploymentId::new(u128::from_be_bytes(bytes)))
    }

    /// This node's expected contract fingerprint, when the deployment records one
    /// (a node the deployment does not know has no expectation ⇒ `None` ⇒ the
    /// startup guard refuses with `NotFound`, I-DP1).
    /// REQ: REQ-CORE-108, REQ-CORE-092.
    #[must_use]
    pub fn expectation(&self, node: &str) -> Option<ContractFingerprint> {
        self.fingerprint_expectations
            .get(node)
            .and_then(|hex| digest(hex))
            .map(ContractFingerprint)
    }

    /// Connection data for `canonical`, if deployed.
    /// REQ: REQ-CORE-108.
    #[must_use]
    pub fn topic(&self, canonical: &str) -> Option<&TopicConnection> {
        self.topics.get(canonical)
    }

    /// The topic-slot table a runtime session carries (canonical topic → slot).
    /// REQ: REQ-CORE-108, REQ-CORE-088.
    #[must_use]
    pub fn slots(&self) -> BTreeMap<String, u32> {
        self.topics
            .iter()
            .filter_map(|(topic, entry)| {
                u32::try_from(entry.slot)
                    .ok()
                    .map(|slot| (topic.clone(), slot))
            })
            .collect()
    }
}

/// Node startup from a loaded configuration (deployment-identity.md §Functions
/// `node_startup`): the fingerprint guard runs first, then the shared active hold
/// is acquired for the node's whole lifetime. The expectation comes from the
/// generated configuration — never from the caller. REQ: REQ-CORE-108,
/// REQ-CORE-092, REQ-CORE-069.
///
/// # Errors
/// [`Status::NotFound`] when the deployment records no expectation for `node`;
/// [`Status::WrongAbi`] on mismatch; [`Status::AlreadyExists`] when a `drm clean`
/// is in flight.
pub fn start_session<L: LockBackend>(
    locks: &L,
    paths: &LockPaths,
    config: &DeploymentConfig,
    node: &str,
    compiled: ContractFingerprint,
) -> Result<LockHandle, Status> {
    node_startup(locks, paths, compiled, config.expectation(node))
}

/// Parses a 32-lowercase-hex digest.
fn digest(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0_u8; 16];
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        out[index] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod tests;
