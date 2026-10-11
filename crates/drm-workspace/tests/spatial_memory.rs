//! Spatial-index memory verification matrix (M21W2 E2):
//! [`spatial_index_memory_verified`] derives the fixed `index.drm` cost at
//! `max_objects` from first principles and asserts the verifier term matches
//! byte-for-byte for every folded bank (I-MB9/I-MB6), and
//! [`spatial_memory_budget_row`] asserts the per-compute budget row accounts the
//! spatial index as a distinct additive term, so an over-budget declaration is a
//! hard error, never a silent pass.
//!
//! REQ: REQ-SPATIAL-018, REQ-SPATIAL-052, REQ-TOOL-003, REQ-TOOL-006.

use drm_workspace::budget::{
    BudgetError, ChannelCost, ComputeCost, DeploymentCost, MemoryBudget, SpatialIndexCost,
    compute_worst_case, spatial_index_cost, verify_compute, verify_deployment,
};

/// The declared open-addressing load factor ceiling (`I-X2`): `α_max = 7/10`.
const ALPHA_MAX_NUM: u64 = 7;
const ALPHA_MAX_DEN: u64 = 10;

/// Fixed element sizes from `spatial_index_cost` (64-bit target), restated
/// independently so the test derives the total without trusting the body.
/// REQ: REQ-SPATIAL-018.
const CELL_SLOT_BYTES: u64 = 32;
const MEMBERSHIP_NODE_BYTES: u64 = 8;
const OBJECT_RECORD_BYTES: u64 = 256;
const BVH_NODE_BYTES: u64 = 48;
const QUERY_HANDLE_BYTES: u64 = 8;

/// A `max_objects`-sized index declaration turned into declared capacities:
/// cell-hash slots `S = ⌈M·α_max⁻¹⌉` with `M = max_objects × max_cells`, object
/// records at `max_objects`, the compiled BVH nodes, and fixed query scratch.
/// REQ: REQ-SPATIAL-018.
fn cost_at_max_objects(
    max_objects: u64,
    max_cells_per_object: u64,
    bvh_nodes: u64,
    queries: u64,
) -> SpatialIndexCost {
    let membership_pool = max_objects * max_cells_per_object;
    let slots = (membership_pool * ALPHA_MAX_DEN).div_ceil(ALPHA_MAX_NUM);
    SpatialIndexCost {
        cell_hash_slots: slots,
        membership_pool,
        object_records: max_objects,
        bvh_nodes,
        query_scratch: queries,
    }
}

/// The independently summed byte total for a declared cost.
/// REQ: REQ-SPATIAL-018.
fn expected_bytes(cost: &SpatialIndexCost) -> u64 {
    cost.cell_hash_slots * CELL_SLOT_BYTES
        + cost.membership_pool * MEMBERSHIP_NODE_BYTES
        + cost.object_records * OBJECT_RECORD_BYTES
        + cost.bvh_nodes * BVH_NODE_BYTES
        + cost.query_scratch * QUERY_HANDLE_BYTES
}

/// REQ: REQ-SPATIAL-018, REQ-TOOL-003 — `spatial_index_memory_verified`
/// (`memory-budget.md` §Test Matrix): an `index.drm` sized at `max_objects`
/// folds the cell-hash table, membership pool, object records, BVH banks, and
/// query scratch, each byte-exact (I-MB9/I-MB6).
#[test]
fn spatial_index_memory_verified() {
    let cost = cost_at_max_objects(4096, 27, 8191, 256);

    // The capacity derivation: α_max clamp, membership pool, record/BVH banks.
    assert_eq!(cost.membership_pool, 4096 * 27);
    assert_eq!(cost.object_records, 4096);
    assert_eq!(cost.bvh_nodes, 8191);
    // Load factor M/S ≤ α_max, by integer cross-multiplication (no floats).
    assert!(
        cost.membership_pool * ALPHA_MAX_DEN <= cost.cell_hash_slots * ALPHA_MAX_NUM,
        "load factor must stay ≤ α_max: {cost:?}"
    );
    assert!(
        cost.cell_hash_slots == (cost.membership_pool * ALPHA_MAX_DEN).div_ceil(ALPHA_MAX_NUM),
        "slots are the exact ceiling: {cost:?}"
    );

    // Every bank contributes; the verifier sum matches the independent total.
    let bytes = spatial_index_cost(&cost).unwrap_or_else(|| unreachable!("in-range capacities"));
    assert_eq!(bytes, expected_bytes(&cost));
    let mut without_bvh = cost;
    without_bvh.bvh_nodes = 0;
    assert!(
        bytes > spatial_index_cost(&without_bvh).unwrap_or_else(|| unreachable!("in-range")),
        "BVH banks must be folded (I-MB9)"
    );
    let mut without_scratch = cost;
    without_scratch.query_scratch = 0;
    assert!(
        bytes > spatial_index_cost(&without_scratch).unwrap_or_else(|| unreachable!("in-range")),
        "query scratch must be folded (I-MB9)"
    );

    // Overflow is reported, never wrapped (I-MB6).
    let huge = SpatialIndexCost {
        cell_hash_slots: u64::MAX,
        ..cost
    };
    assert_eq!(spatial_index_cost(&huge), None);
}

/// REQ: REQ-SPATIAL-018, REQ-TOOL-006 — `spatial_memory_budget_row`: the
/// per-compute budget row (its worst-case estimate) accounts the spatial index
/// as a distinct additive term, so a declaration within budget passes and an
/// over-budget one is a hard `BudgetExceeded` error (I-MB9/I-MB2).
#[test]
fn spatial_memory_budget_row() {
    let spatial = cost_at_max_objects(4096, 27, 8191, 256);
    let spatial_bytes =
        spatial_index_cost(&spatial).unwrap_or_else(|| unreachable!("in-range capacities"));
    assert!(spatial_bytes > 0);

    // A single channel plus the spatial index; spatial is ~7.4 MiB.
    let channel = ChannelCost {
        id: "pose".to_string(),
        pool_slots: 16,
        slot_bytes: 256,
        control_block: 1024,
        precommit: false,
        history_depth: 4,
        borrowers: 0,
        nominal_bytes: 1536,
    };
    let channel_bytes = 16 * 256 + 1024;
    let router = 4096;
    let expected_row = channel_bytes + router + spatial_bytes;

    let within = MemoryBudget {
        drm_budget_bytes: 64 * 1024 * 1024,
        lockable_budget_bytes: 0,
        physical_ram_bytes: 1024 * 1024 * 1024,
        precommit: false,
        page_granularity: 4096,
    };
    let cost = ComputeCost {
        budget: within,
        channels: vec![channel],
        router_buffers: router,
        spatial_index_bytes: spatial_bytes,
    };

    // The row sums channel + router + spatial, so the spatial term is present.
    assert_eq!(compute_worst_case(&cost), Some(expected_row));

    let report = verify_compute("nav", &cost);
    assert!(report.passes(), "in-budget row: {:?}", report.errors);
    assert_eq!(report.estimates.len(), 1, "one row per compute");
    let row = &report.estimates[0];
    assert_eq!(row.worst_case, expected_row);
    assert_eq!(row.nominal, 1536);
    assert!(row.nominal <= row.worst_case);

    // Drop the DRM ceiling below the spatial index alone ⇒ hard error naming it.
    let over = ComputeCost {
        budget: MemoryBudget {
            drm_budget_bytes: spatial_bytes - 1,
            ..within
        },
        ..cost
    };
    let report = verify_compute("nav", &over);
    assert!(!report.passes(), "over-budget spatial index must fail");
    assert!(
        report.errors.iter().any(|e| matches!(
            e,
            BudgetError::BudgetExceeded { compute, worst, budget }
                if compute == "nav" && *worst == expected_row && *budget == spatial_bytes - 1
        )),
        "expected BudgetExceeded naming the row: {:?}",
        report.errors
    );

    // The deployment walk surfaces the same failing row.
    let deploy = DeploymentCost {
        computes: [("nav".to_string(), over)].into_iter().collect(),
    };
    let report = verify_deployment(&deploy);
    assert!(!report.passes(), "deployment row must fail: {report:?}");
    assert_eq!(report.estimates.len(), 1);
    assert_eq!(report.estimates[0].worst_case, expected_row);
}
