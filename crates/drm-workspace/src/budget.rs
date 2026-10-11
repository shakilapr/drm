//! Memory-budget verifier math
//! ([memory-budget.md](../../docs/algorithms/memory-budget.md)): deterministic
//! worst-case capacity accounting per compute unit, fully checked arithmetic
//! (I-MB6), and the budget verdict bands — hard errors for budget, lockable,
//! and pool-invariant violations (I-MB2, I-MB4), non-failing warnings for the
//! physical-RAM band and the drop-free shortfall (I-MB3, I-MB11), and
//! informational nominal estimates (REQ-TOOL-006). Never a `Result`: one run
//! reports every verdict (verify-engine I-V5); any error ⇒ build failure.
//!
//! `worst_case(compute) = Σ_ch (pool_slots × slot_bytes + control_block) +
//! router_buffers + spatial_index_memory` (I-MB1). Locked spans are
//! page-rounded per region, conservatively (I-MB10).
//!
//! REQ: REQ-TOOL-003, REQ-TOOL-004, REQ-TOOL-005, REQ-TOOL-006, REQ-TOOL-008,
//! REQ-TOOL-009, REQ-TOOL-010, REQ-CORE-042.

use std::collections::BTreeMap;
use std::fmt;

/// Configured memory ceilings for one compute target (REQ-TOOL-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudget {
    /// Configured DRM contribution ceiling (general targets).
    pub drm_budget_bytes: u64,
    /// Ceiling for `mlock`/`VirtualLock` (RT/precommit targets).
    pub lockable_budget_bytes: u64,
    /// Advisory physical-RAM reference for the warning band (I-MB3).
    pub physical_ram_bytes: u64,
    /// Target policy: precommit ⇒ router/spatial regions enter the locked sum.
    pub precommit: bool,
    /// Target-declared page granularity (4 KiB typical; ARM64 kernels 16/64 KiB).
    pub page_granularity: u32,
}

/// Per-active-channel fixed cost on a compute unit (I-MB1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCost {
    /// Stable channel/topic id for diagnostics.
    pub id: String,
    /// Payload-pool slots — validated ≥ R+1 (I-MB4).
    pub pool_slots: u64,
    /// Max sample size: fixed header + max tail capacity (canon-codec two-region).
    pub slot_bytes: u64,
    /// CCB + hazard array + ring — fixed overhead.
    pub control_block: u64,
    /// Drives the lockable sum (per channel, not per compute) (I-MB2).
    pub precommit: bool,
    /// History depth `R`; the pool floor is `R+1`.
    pub history_depth: u64,
    /// Declared borrowers + diagnostic-consumer range `H` (I-MB11).
    pub borrowers: u64,
    /// Declared nominal estimate for this channel (informational, REQ-TOOL-006).
    pub nominal_bytes: u64,
}

/// Every declared cost attached to one compute unit (REQ-TOOL-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeCost {
    /// Configured ceilings and policy for this compute.
    pub budget: MemoryBudget,
    /// Active channels/topics on this compute.
    pub channels: Vec<ChannelCost>,
    /// Router-owned lane buffers counted against this compute (I-MB8).
    pub router_buffers: u64,
    /// Fixed spatial-index memory for this compute (I-MB9).
    pub spatial_index_bytes: u64,
}

/// One deployment: compute name → cost. `BTreeMap` keeps iteration
/// deterministic (no filesystem/hash order leaks).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeploymentCost {
    /// Compute name → declared cost.
    pub computes: BTreeMap<String, ComputeCost>,
}

/// A gate failure: any error ⇒ build failure (I-MB2, I-MB4, I-MB6).
/// REQ: REQ-TOOL-003, REQ-TOOL-005.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// A capacity product/sum (including `R+1`) overflowed `u64` (I-MB6).
    CapacityOverflow {
        /// Owning compute.
        compute: String,
        /// Offending channel, or `None` for a compute-level sum (router/spatial).
        channel: Option<String>,
    },
    /// `pool_slots < R+1` — payload-pool I-P4 at verify (I-MB4).
    PoolInvariantViolation {
        /// Owning compute.
        compute: String,
        /// Offending channel.
        channel: String,
        /// Configured `pool_slots`.
        have: u64,
        /// Required floor `R+1`.
        need: u64,
    },
    /// `worst_case > drm_budget` (I-MB2).
    BudgetExceeded {
        /// Owning compute.
        compute: String,
        /// Computed worst case.
        worst: u64,
        /// Configured DRM budget.
        budget: u64,
    },
    /// `Σ locked spans > lockable_budget` (I-MB2).
    LockableExceeded {
        /// Owning compute.
        compute: String,
        /// Computed page-rounded locked sum.
        locked: u64,
        /// Configured lockable budget.
        lockable: u64,
    },
}

/// A non-failing diagnostic: printed, never a gate (I-MB3, I-MB11).
/// REQ: REQ-TOOL-008, REQ-CORE-042.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetWarning {
    /// `worst_case > physical RAM ∧ ≤ drm_budget` (I-MB3). Names the amount.
    OverPhysicalRam {
        /// Owning compute.
        compute: String,
        /// Computed worst case.
        worst: u64,
        /// Declared physical RAM.
        ram: u64,
    },
    /// `pool_slots < R + H + 1`: drop-free is unproven, not invalid (I-MB11).
    DropFreeShortfall {
        /// Owning compute.
        compute: String,
        /// Offending channel.
        channel: String,
        /// Configured `pool_slots`.
        have: u64,
        /// Required `R + H + 1`.
        need: u64,
    },
}

/// Informational per-compute totals; never gate a build (REQ-TOOL-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeEstimate {
    /// Owning compute.
    pub compute: String,
    /// Σ worst-case capacity (I-MB1).
    pub worst_case: u64,
    /// Σ page-rounded locked spans (I-MB2/I-MB10).
    pub locked: u64,
    /// Σ nominal estimates (informational).
    pub nominal: u64,
}

/// Full verifier report: never a `Result` — one run reports ALL verdicts
/// (verify-engine I-V5). `errors.is_empty()` ⇒ V-MEM passes (the only gate).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetReport {
    /// Gate failures; any entry ⇒ build failure.
    pub errors: Vec<BudgetError>,
    /// Non-failing diagnostics.
    pub warnings: Vec<BudgetWarning>,
    /// Per-compute informational totals.
    pub estimates: Vec<ComputeEstimate>,
}

impl BudgetReport {
    /// V-MEM gate: true iff no error was recorded.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityOverflow { compute, channel } => match channel {
                Some(id) => write!(
                    f,
                    "V-MEM: compute `{compute}` channel `{id}` capacity overflow (REQ-TOOL-003)"
                ),
                None => write!(
                    f,
                    "V-MEM: compute `{compute}` capacity overflow (REQ-TOOL-003)"
                ),
            },
            Self::PoolInvariantViolation {
                compute,
                channel,
                have,
                need,
            } => write!(
                f,
                "V-MEM: compute `{compute}` channel `{channel}` pool_slots {have} < R+1 {need} (I-MB4, REQ-TOOL-003)"
            ),
            Self::BudgetExceeded {
                compute,
                worst,
                budget,
            } => write!(
                f,
                "V-MEM: compute `{compute}` worst-case {worst} > drm budget {budget} (I-MB2, REQ-TOOL-005)"
            ),
            Self::LockableExceeded {
                compute,
                locked,
                lockable,
            } => write!(
                f,
                "V-MEM: compute `{compute}` locked {locked} > lockable budget {lockable} (I-MB2, REQ-TOOL-005)"
            ),
        }
    }
}

impl fmt::Display for BudgetWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OverPhysicalRam {
                compute,
                worst,
                ram,
            } => write!(
                f,
                "V-MEM: compute `{compute}` worst-case {worst} > physical RAM {ram} (I-MB3, REQ-TOOL-008)"
            ),
            Self::DropFreeShortfall {
                compute,
                channel,
                have,
                need,
            } => write!(
                f,
                "V-MEM: compute `{compute}` channel `{channel}` pool_slots {have} < R+H+1 {need}: drop-free unproven (I-MB11, REQ-CORE-042)"
            ),
        }
    }
}

/// Declared fixed capacity of one runtime spatial index per compute unit
/// ([spatial-index.md](../../docs/algorithms/spatial/spatial-index.md) `I-X9`,
/// memory-budget.md `I-MB9`). Every term is declared capacity; the index never
/// grows. REQ: REQ-SPATIAL-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpatialIndexCost {
    /// Open-addressing cell-hash slots `S = ⌈M/α_max⌉` (`I-X2`).
    pub cell_hash_slots: u64,
    /// Membership nodes `M = max_objects × max_cells_per_object` (`I-X2`).
    pub membership_pool: u64,
    /// Per-object record count at `max_objects`.
    pub object_records: u64,
    /// Nodes across every guarded compiled/calibration BVH bank.
    pub bvh_nodes: u64,
    /// Fixed per-declared-query scratch handles.
    pub query_scratch: u64,
}

/// Fixed spatial-index memory for one compute: the declared slot, membership,
/// object-record, BVH-node, and query-scratch counts multiplied by their fixed
/// element sizes, summed with checked arithmetic (`I-MB6`/`I-MB9`). `None` on
/// overflow. REQ: REQ-SPATIAL-018, REQ-TOOL-003.
#[must_use]
pub fn spatial_index_cost(cost: &SpatialIndexCost) -> Option<u64> {
    // Fixed element sizes (64-bit target): cell slot, membership node,
    // object record, BVH node, candidate handle.
    const CELL_SLOT_BYTES: u64 = 32;
    const MEMBERSHIP_NODE_BYTES: u64 = 8;
    const OBJECT_RECORD_BYTES: u64 = 256;
    const BVH_NODE_BYTES: u64 = 48;
    const QUERY_HANDLE_BYTES: u64 = 8;
    cost.cell_hash_slots
        .checked_mul(CELL_SLOT_BYTES)?
        .checked_add(cost.membership_pool.checked_mul(MEMBERSHIP_NODE_BYTES)?)?
        .checked_add(cost.object_records.checked_mul(OBJECT_RECORD_BYTES)?)?
        .checked_add(cost.bvh_nodes.checked_mul(BVH_NODE_BYTES)?)?
        .checked_add(cost.query_scratch.checked_mul(QUERY_HANDLE_BYTES)?)
}

/// Σ over channels of `pool_slots × slot_bytes + control_block` — the fixed
/// capacity contribution of one compute's active channels/topics (I-MB1). Every
/// product and sum is checked (I-MB6); `None` ⇒ the sum overflowed `u64`.
/// REQ: REQ-TOOL-003, REQ-TOOL-010.
#[must_use]
pub fn channels_worst_case(channels: &[ChannelCost]) -> Option<u64> {
    let mut sum = 0u64;
    for ch in channels {
        let bytes = ch
            .pool_slots
            .checked_mul(ch.slot_bytes)?
            .checked_add(ch.control_block)?;
        sum = sum.checked_add(bytes)?;
    }
    Some(sum)
}

/// Full per-compute worst case (I-MB1): the channel sum plus router-owned lane
/// buffers (I-MB8) plus fixed spatial-index memory (I-MB9). Checked (I-MB6);
/// `None` ⇒ overflow.
/// REQ: REQ-TOOL-003, REQ-TOOL-009, REQ-TOOL-010.
#[must_use]
pub fn compute_worst_case(cost: &ComputeCost) -> Option<u64> {
    channels_worst_case(&cost.channels)?
        .checked_add(cost.router_buffers)?
        .checked_add(cost.spatial_index_bytes)
}

/// Runs the verifier over one compute unit, collecting every verdict
/// (I-MB2/I-MB3/I-MB4/I-MB6/I-MB11). Deterministic: inputs in declaration
/// order, no map iteration.
/// REQ: REQ-TOOL-003, REQ-TOOL-005, REQ-TOOL-006, REQ-TOOL-008.
#[must_use]
pub fn verify_compute(compute: &str, cost: &ComputeCost) -> BudgetReport {
    let mut report = BudgetReport::default();
    verify_compute_into(compute, cost, &mut report);
    report
}

/// Runs the verifier over every compute in a deployment. Computes are visited
/// in sorted-name order; errors, warnings, and estimates collect across the
/// whole deployment (verify-engine I-V5).
/// REQ: REQ-TOOL-004, REQ-TOOL-005.
#[must_use]
pub fn verify_deployment(deploy: &DeploymentCost) -> BudgetReport {
    let mut report = BudgetReport::default();
    for (compute, cost) in &deploy.computes {
        verify_compute_into(compute, cost, &mut report);
    }
    report
}

/// Per-compute core: channel loop, router/spatial regions, verdict bands.
fn verify_compute_into(compute: &str, cost: &ComputeCost, report: &mut BudgetReport) {
    let budget = &cost.budget;
    let (mut worst, mut locked, mut nominal) = (0u64, 0u64, 0u64);

    for ch in &cost.channels {
        let Some(ch_bytes) = check_channel(compute, ch, report) else {
            continue; // CapacityOverflow already recorded; keep checking others.
        };
        add_checked(&mut worst, ch_bytes, compute, Some(&ch.id), report);
        if ch.precommit {
            // I-MB2/I-MB10: page-rounded span, conservative.
            match page_round_up(ch_bytes, budget.page_granularity) {
                Some(span) => add_checked(&mut locked, span, compute, Some(&ch.id), report),
                None => record_overflow(report, compute, Some(&ch.id)),
            }
        }
        add_checked(
            &mut nominal,
            ch.nominal_bytes,
            compute,
            Some(&ch.id),
            report,
        );
    }

    // I-MB8: router-owned lane buffers on this compute.
    add_checked(&mut worst, cost.router_buffers, compute, None, report);
    if budget.precommit {
        // I-MB2/I-MB10: router lanes lock too on precommit computes.
        match page_round_up(cost.router_buffers, budget.page_granularity) {
            Some(span) => add_checked(&mut locked, span, compute, None, report),
            None => record_overflow(report, compute, None),
        }
    }
    // I-MB9: spatial cell-hash + membership + records + BVH + scratch — fixed.
    add_checked(&mut worst, cost.spatial_index_bytes, compute, None, report);
    if budget.precommit {
        // I-MB2/I-MB10: spatial banks/scratch lock too.
        match page_round_up(cost.spatial_index_bytes, budget.page_granularity) {
            Some(span) => add_checked(&mut locked, span, compute, None, report),
            None => record_overflow(report, compute, None),
        }
    }

    // I-MB2: worst-case > drm budget ⇒ hard error.
    if worst > budget.drm_budget_bytes {
        report.errors.push(BudgetError::BudgetExceeded {
            compute: compute.to_string(),
            worst,
            budget: budget.drm_budget_bytes,
        });
    } else if worst > budget.physical_ram_bytes {
        // I-MB3: > RAM but ≤ budget ⇒ strong warning, never an error.
        report.warnings.push(BudgetWarning::OverPhysicalRam {
            compute: compute.to_string(),
            worst,
            ram: budget.physical_ram_bytes,
        });
    }
    // I-MB2: Σ locked spans > lockable budget ⇒ hard error.
    if locked > budget.lockable_budget_bytes {
        report.errors.push(BudgetError::LockableExceeded {
            compute: compute.to_string(),
            locked,
            lockable: budget.lockable_budget_bytes,
        });
    }
    // Informational only (REQ-TOOL-006).
    report.estimates.push(ComputeEstimate {
        compute: compute.to_string(),
        worst_case: worst,
        locked,
        nominal,
    });
}

/// Per-channel checks (I-MB4 pool floor, I-MB11 drop-free warning) plus the
/// checked channel byte cost. `None` ⇒ a `CapacityOverflow` was recorded and
/// this channel contributes nothing further (the compute keeps checking).
/// REQ: REQ-TOOL-003, REQ-CORE-042.
fn check_channel(compute: &str, ch: &ChannelCost, report: &mut BudgetReport) -> Option<u64> {
    // I-MB4/I-MB6: R+1 itself is checked arithmetic — never a raw `+ 1`.
    let Some(min_pool) = ch.history_depth.checked_add(1) else {
        record_overflow(report, compute, Some(&ch.id));
        return None;
    };
    if ch.pool_slots < min_pool {
        report.errors.push(BudgetError::PoolInvariantViolation {
            compute: compute.to_string(),
            channel: ch.id.clone(),
            have: ch.pool_slots,
            need: min_pool,
        });
    }
    // I-MB11: drop-free is a warning, not a gate (R+1 is the hard floor).
    let dropfree_pool = ch
        .history_depth
        .checked_add(ch.borrowers)
        .and_then(|x| x.checked_add(1));
    let Some(dropfree_pool) = dropfree_pool else {
        record_overflow(report, compute, Some(&ch.id));
        return None;
    };
    if ch.pool_slots < dropfree_pool {
        report.warnings.push(BudgetWarning::DropFreeShortfall {
            compute: compute.to_string(),
            channel: ch.id.clone(),
            have: ch.pool_slots,
            need: dropfree_pool,
        });
    }
    // I-MB1/I-MB6: pool_slots × slot_bytes + control_block, checked.
    let bytes = ch
        .pool_slots
        .checked_mul(ch.slot_bytes)
        .and_then(|b| b.checked_add(ch.control_block));
    if bytes.is_none() {
        record_overflow(report, compute, Some(&ch.id));
    }
    bytes
}

/// Rounds `bytes` up to the next multiple of `page` (I-MB10). Zero granularity
/// is treated as identity (never a division by zero); overflow of the rounded
/// value returns `None` so the caller records `CapacityOverflow`.
fn page_round_up(bytes: u64, page: u32) -> Option<u64> {
    if page == 0 {
        return Some(bytes);
    }
    let p = u64::from(page);
    let rem = bytes % p;
    if rem == 0 {
        Some(bytes)
    } else {
        bytes.checked_add(p - rem)
    }
}

/// `acc = acc.checked_add(add)`, recording a `CapacityOverflow` on overflow
/// instead of wrapping (I-MB6).
fn add_checked(
    acc: &mut u64,
    add: u64,
    compute: &str,
    channel: Option<&str>,
    report: &mut BudgetReport,
) {
    match acc.checked_add(add) {
        Some(v) => *acc = v,
        None => record_overflow(report, compute, channel),
    }
}

/// Records a `CapacityOverflow` naming the compute (and channel when known).
fn record_overflow(report: &mut BudgetReport, compute: &str, channel: Option<&str>) {
    report.errors.push(BudgetError::CapacityOverflow {
        compute: compute.to_string(),
        channel: channel.map(str::to_string),
    });
}
