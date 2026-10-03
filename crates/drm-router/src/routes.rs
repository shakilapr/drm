//! Handle-keyed runtime route table (work-plan M02W3-E3-T1): the data path
//! is keyed only by the numeric topic handle — route names exist solely in
//! construction-time inputs, never in the table (architecture.md runtime
//! handle vs transport identifier; REQ-ROUTE-043 runtime forwarding tables
//! compute-only).
//! REQ: REQ-ROUTE-043, REQ-ROUTE-022.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use drm_types::Status;

/// One route entry as the data path consumes it: transport binding, optional
/// export limiter period, generated receive bound. Name-free by construction.
/// REQ: REQ-ROUTE-033, REQ-ROUTE-008, REQ-ROUTE-038.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteEntry {
    /// Transport-scoped wire channel id on the route's link — never the topic
    /// handle (`wire_id_not_handle`, I-RT7).
    /// REQ: REQ-ROUTE-033.
    pub wire_channel_id: u32,
    /// Export limiter period in nanoseconds (I-RT13/I-LS8); `None` = no
    /// limiter (sequential route).
    /// REQ: REQ-ROUTE-008.
    pub export_limit_period_ns: Option<u64>,
    /// Generated receive bound for this route (header + payload + optional
    /// e2e CRC), enforced before any payload buffering (I-RT18).
    /// REQ: REQ-ROUTE-038.
    pub max_record_bytes: u32,
}

/// Runtime route table keyed by numeric topic handle only — the sole lookup
/// path for route resolution (no string keys, no names on the data path).
/// REQ: REQ-ROUTE-043, REQ-ROUTE-022.
#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    routes: BTreeMap<u32, RouteEntry>,
}

impl RouteTable {
    /// New empty table.
    /// REQ: REQ-ROUTE-043.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `entry` under `topic_handle`; duplicate handles are rejected
    /// so a stale entry can never be silently replaced.
    /// REQ: REQ-ROUTE-043.
    ///
    /// # Errors
    /// [`Status::AlreadyExists`] when the handle is already registered.
    pub fn insert(&mut self, topic_handle: u32, entry: RouteEntry) -> Result<(), Status> {
        match self.routes.entry(topic_handle) {
            Entry::Vacant(slot) => {
                slot.insert(entry);
                Ok(())
            }
            Entry::Occupied(_) => Err(Status::AlreadyExists),
        }
    }

    /// Route for `topic_handle` — the only data-path key.
    /// REQ: REQ-ROUTE-022, REQ-ROUTE-043.
    #[must_use]
    pub fn route_for(&self, topic_handle: u32) -> Option<&RouteEntry> {
        self.routes.get(&topic_handle)
    }

    /// Registered route count.
    /// REQ: REQ-ROUTE-043.
    #[must_use]
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// `true` when no routes are registered.
    /// REQ: REQ-ROUTE-043.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}
