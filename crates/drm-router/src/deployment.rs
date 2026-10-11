//! Router deployment namespace/identity (work-plan M05W4-E3-T1/E3-T2, E4-T2):
//! router-side awareness of the deployment instance that scopes every
//! backend-safe name. Reuses the runtime identity (`drm_core::deploy`) and the
//! runtime name derivation (`drm_core::shm::backend_segment_name`) — one
//! identity, no second one, no I/O, no transport change.
//! REQ: REQ-CORE-084 (128-bit deployment id; never compiled),
//! REQ-CORE-088 (deployment-scoped backend-safe opaque names),
//! REQ-CORE-089 (new deployment ⇒ new namespace; never adopt/migrate).

use std::path::Path;

use drm_core::shm::{LockPaths, SegmentName, ShmError};

/// Deployment instance identity (128-bit runtime id from `deployment.json`,
/// never compiled into a binary), reused directly from the runtime so the
/// router derives the exact names the runtime publishes.
/// REQ: REQ-CORE-084.
pub use drm_core::deploy::DeploymentId;

/// Deployment-scoped namespace: the single source of every backend-safe key
/// the router derives for one deployment instance. Keys embed the deployment
/// id, so two deployments that use the same topic slot never share a name
/// (I-DP2 cross-attachment zero).
/// REQ: REQ-CORE-088, REQ-CORE-089.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentNamespace {
    id: DeploymentId,
}

impl DeploymentNamespace {
    /// Namespace scoped to `id`.
    /// REQ: REQ-CORE-088.
    #[must_use]
    pub const fn new(id: DeploymentId) -> Self {
        Self { id }
    }

    /// The deployment id this namespace scopes.
    /// REQ: REQ-CORE-088.
    #[must_use]
    pub const fn id(&self) -> DeploymentId {
        self.id
    }

    /// Deployment-scoped segment name `/drm.<encoded-id>.<slot>` (delegates to
    /// the runtime derivation, so router and runtime agree byte-for-byte).
    /// Two deployments with the same slot derive different names; the same
    /// deployment is stable.
    /// REQ: REQ-CORE-088.
    ///
    /// # Errors
    /// [`ShmError`] when the generated name violates name conformance.
    pub fn segment_name(&self, topic_slot: u64) -> Result<SegmentName, ShmError> {
        self.id.segment_name(topic_slot)
    }

    /// Deployment-scoped lock paths `<runtime_root>/<encoded-id>`: the
    /// environment root is the only configurable level, so an edited
    /// `deployment.json` cannot redirect the router at a foreign lock (I-DP5).
    /// REQ: REQ-CORE-088.
    #[must_use]
    pub fn lock_paths(&self, runtime_root: &Path) -> LockPaths {
        LockPaths::new(runtime_root.join(self.id.encoded()))
    }

    /// `true` iff `name` is a segment of this namespace (I-DP2): a name from a
    /// foreign deployment never matches, so a cross-deployment open resolves
    /// to a miss instead of attaching foreign memory.
    /// REQ: REQ-CORE-089.
    #[must_use]
    pub fn owns(&self, name: &SegmentName) -> bool {
        let prefix = format!("/drm.{}.", self.id.encoded());
        name.as_str()
            .strip_prefix(&prefix)
            .is_some_and(|slot| !slot.is_empty())
    }
}
