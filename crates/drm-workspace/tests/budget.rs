//! Memory-budget verifier matrices (M06W4): five independently derived
//! fixture deployments asserted byte-exact (I-MB1), plus one negative per
//! verdict band — budget/lockable/pool/overflow hard errors (I-MB2, I-MB4,
//! I-MB6) and RAM/drop-free warnings (I-MB3, I-MB11). Nominal totals are
//! informational and never gate (REQ-TOOL-006).
//! REQ: REQ-TOOL-003, REQ-TOOL-005, REQ-TOOL-006, REQ-TOOL-008, REQ-TOOL-010,
//! REQ-CORE-042.

use drm_workspace::budget::{
    BudgetError, BudgetReport, BudgetWarning, ChannelCost, ComputeCost, DeploymentCost,
    MemoryBudget, channels_worst_case, compute_worst_case, verify_compute, verify_deployment,
};

/// A channel with zero `precommit`/nominal; override those fields per fixture.
fn channel(
    id: &str,
    pool_slots: u64,
    slot_bytes: u64,
    control_block: u64,
    r: u64,
    h: u64,
) -> ChannelCost {
    ChannelCost {
        id: id.to_string(),
        pool_slots,
        slot_bytes,
        control_block,
        precommit: false,
        history_depth: r,
        borrowers: h,
        nominal_bytes: 0,
    }
}

fn budget(drm: u64, lockable: u64, ram: u64, precommit: bool, page: u32) -> MemoryBudget {
    MemoryBudget {
        drm_budget_bytes: drm,
        lockable_budget_bytes: lockable,
        physical_ram_bytes: ram,
        precommit,
        page_granularity: page,
    }
}

fn compute(
    budget: MemoryBudget,
    channels: Vec<ChannelCost>,
    router_buffers: u64,
    spatial_index_bytes: u64,
) -> ComputeCost {
    ComputeCost {
        budget,
        channels,
        router_buffers,
        spatial_index_bytes,
    }
}

/// Hand-derived reference set: `(name, compute, expected worst, locked, nominal)`.
/// Each total is derived by hand/independent script (memory-budget.md, I-MB1).
///
/// Fixture derivation (worst = Σ channel + router + spatial; locked = Σ page-
/// rounded spans of precommit channels + compute-level router/spatial when the
/// target is precommit):
///
/// 1. `nav` (doc worked example): fix 16×256+1024=5120, dbg 8×64+1024=1536,
///    router 4096 ⇒ worst 10752. locked: round(5120,4096)=8192 +
///    round(4096,4096)=4096 ⇒ 12288. nominal 1536+512=2048.
/// 2. `sensor`: c1 32×1024+2048=34816, c2 4×4096+512=16896, router 8192,
///    spatial 40960 ⇒ worst 100864. locked round(34816,4096)=36864 +
///    router 8192 + spatial 40960 ⇒ 86016. nominal 8192+4096=12288.
/// 3. `edge` (warning band): 10×1000+512=10512 ⇒ worst 10512 (> RAM 10000,
///    ≤ budget 1000000). no precommit ⇒ locked 0. nominal 4096.
/// 4. `arm64` (16 KiB pages): a 64×4096+8192=270336, b 3×16384+1024=50176 ⇒
///    worst 320512. locked round(270336,16384)=278528 + round(50176,16384)
///    =65536 ⇒ 344064. nominal 32768+16384=49152.
/// 5. `exact` (boundary): 1×4000+1000=5000 ⇒ worst 5000 == budget; locked
///    round(5000,4096)=8192 == lockable. nominal 2048.
fn reference_fixtures() -> Vec<(&'static str, ComputeCost, u64, u64, u64)> {
    // 1. nav — memory-budget.md worked example.
    let mut fix = channel("fix", 16, 256, 1024, 4, 2);
    fix.precommit = true;
    fix.nominal_bytes = 1536;
    let mut dbg = channel("dbg", 8, 64, 1024, 1, 0);
    dbg.nominal_bytes = 512;
    let nav = compute(
        budget(65536, 32768, 512 * 1024 * 1024, true, 4096),
        vec![fix, dbg],
        4096,
        0,
    );

    // 2. sensor — multi-channel + router + spatial index.
    let mut c1 = channel("c1", 32, 1024, 2048, 8, 0);
    c1.precommit = true;
    c1.nominal_bytes = 8192;
    let mut c2 = channel("c2", 4, 4096, 512, 2, 1);
    c2.nominal_bytes = 4096;
    let sensor = compute(
        budget(1_048_576, 524_288, 256 * 1024 * 1024, true, 4096),
        vec![c1, c2],
        8192,
        40960,
    );

    // 3. edge — worst > physical RAM but ≤ budget ⇒ warning only.
    let mut e1 = channel("e1", 10, 1000, 512, 1, 0);
    e1.nominal_bytes = 4096;
    let edge = compute(
        budget(1_000_000, 1_000_000, 10_000, false, 4096),
        vec![e1],
        0,
        0,
    );

    // 4. arm64 — 16 KiB page granularity, all precommit.
    let mut a = channel("a", 64, 4096, 8192, 16, 3);
    a.precommit = true;
    a.nominal_bytes = 32768;
    let mut b = channel("b", 3, 16384, 1024, 1, 0);
    b.precommit = true;
    b.nominal_bytes = 16384;
    let arm64 = compute(
        budget(16_777_216, 8_388_608, 64 * 1024 * 1024, true, 16384),
        vec![a, b],
        0,
        0,
    );

    // 5. exact — worst == budget and locked == lockable ⇒ both pass (strict >).
    let mut x = channel("x", 1, 4000, 1000, 0, 0);
    x.precommit = true;
    x.nominal_bytes = 2048;
    let exact = compute(budget(5000, 8192, 1_000_000, true, 4096), vec![x], 0, 0);

    vec![
        ("nav", nav, 10752, 12288, 2048),
        ("sensor", sensor, 100_864, 86016, 12288),
        ("edge", edge, 10512, 0, 4096),
        ("arm64", arm64, 320_512, 344_064, 49152),
        ("exact", exact, 5000, 8192, 2048),
    ]
}

/// Asserts one fixture's totals are byte-exact (I-MB1) and that the pure
/// channel-sum formula agrees with the report.
fn assert_reference(name: &str, worst: u64, locked: u64, nominal: u64) {
    let fixtures = reference_fixtures();
    let Some((_, cost, _, _, _)) = fixtures.iter().find(|(n, ..)| *n == name) else {
        unreachable!("fixture `{name}` missing");
    };
    let report = verify_compute(name, cost);
    assert!(
        report.passes(),
        "{name}: unexpected errors {:?}",
        report.errors
    );
    assert_eq!(report.estimates.len(), 1, "{name}: one estimate");
    let est = &report.estimates[0];
    assert_eq!(est.worst_case, worst, "{name}: worst-case");
    assert_eq!(est.locked, locked, "{name}: locked");
    assert_eq!(est.nominal, nominal, "{name}: nominal");
    assert!(
        est.nominal <= est.worst_case,
        "{name}: nominal ≤ worst by construction"
    );
    let direct = compute_worst_case(cost).unwrap_or_else(|| unreachable!("{name}: overflow"));
    assert_eq!(direct, worst, "{name}: compute_worst_case formula");
    let chans = channels_worst_case(&cost.channels).unwrap_or_else(|| unreachable!("{name}: sum"));
    assert_eq!(
        chans + cost.router_buffers + cost.spatial_index_bytes,
        worst
    );
}

/// I-MB1: all five independently derived fixtures match the verifier exactly.
#[test]
fn budget_reference_match() {
    for (name, _cost, worst, locked, nominal) in reference_fixtures() {
        assert_reference(name, worst, locked, nominal);
    }
}

/// I-MB1 reference #1 — the memory-budget.md worked example (`nav`).
#[test]
fn budget_reference_nav() {
    assert_reference("nav", 10752, 12288, 2048);
}

/// I-MB1 reference #2 — multi-channel + router + spatial (`sensor`).
#[test]
fn budget_reference_sensor() {
    assert_reference("sensor", 100_864, 86016, 12288);
}

/// I-MB1 reference #3 — warning band, exact totals still byte-exact (`edge`).
#[test]
fn budget_reference_edge() {
    assert_reference("edge", 10512, 0, 4096);
}

/// I-MB1 reference #4 — 16 KiB page rounding (`arm64`).
#[test]
fn budget_reference_arm64() {
    assert_reference("arm64", 320_512, 344_064, 49152);
}

/// I-MB1 reference #5 — boundary equality worst == budget, locked == lockable.
#[test]
fn budget_reference_exact() {
    assert_reference("exact", 5000, 8192, 2048);
}

/// I-MB1: the `edge` fixture's warning band is reported, never an error.
#[test]
fn budget_reference_warning_band_flagged() {
    let mut e1 = channel("e1", 10, 1000, 512, 1, 0);
    e1.nominal_bytes = 4096;
    let cost = compute(
        budget(1_000_000, 1_000_000, 10_000, false, 4096),
        vec![e1],
        0,
        0,
    );
    let report = verify_compute("edge", &cost);
    assert!(report.passes());
    assert_eq!(
        report.warnings,
        vec![BudgetWarning::OverPhysicalRam {
            compute: "edge".to_string(),
            worst: 10512,
            ram: 10_000,
        }]
    );
}

/// I-MB1/I-MB5: a multi-compute deployment reports every compute in sorted
/// order; totals and verdicts aggregate deterministically.
#[test]
fn budget_reference_deployment() {
    let fixtures = reference_fixtures();
    let mut computes = std::collections::BTreeMap::new();
    for (name, cost, _, _, _) in fixtures {
        if name == "nav" || name == "edge" {
            computes.insert(name.to_string(), cost);
        }
    }
    let report = verify_deployment(&DeploymentCost { computes });
    assert!(report.passes(), "no gate errors: {:?}", report.errors);
    let names: Vec<&str> = report
        .estimates
        .iter()
        .map(|e| e.compute.as_str())
        .collect();
    assert_eq!(names, vec!["edge", "nav"], "sorted compute order");
    let nav = report
        .estimates
        .iter()
        .find(|e| e.compute == "nav")
        .unwrap_or_else(|| unreachable!("nav estimate"));
    assert_eq!(
        (nav.worst_case, nav.locked, nav.nominal),
        (10752, 12288, 2048)
    );
    assert_eq!(report.warnings.len(), 1, "edge RAM warning");
    assert_eq!(
        verify_deployment(&DeploymentCost::default()),
        BudgetReport::default()
    );
}

/// I-MB2: worst-case > declared drm budget is a hard error, not a warning.
#[test]
fn budget_negative_budget_exceeded() {
    let ch = channel("over", 10, 100, 0, 0, 0); // 10×100 = 1000 bytes
    let cost = compute(
        budget(999, 1_000_000, 1_000_000, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("over", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::BudgetExceeded {
            compute: "over".to_string(),
            worst: 1000,
            budget: 999,
        }]
    );
    assert!(
        report.warnings.is_empty(),
        "band is an error, not a warning"
    );
    assert_eq!(report.estimates[0].worst_case, 1000);
}

/// I-MB2: Σ locked spans > lockable budget is a hard error (page-rounded).
#[test]
fn budget_negative_lockable_exceeded() {
    let mut ch = channel("lock", 1, 5000, 0, 0, 0); // 5000 B ⇒ one 8 KiB span
    ch.precommit = true;
    let cost = compute(
        budget(1_000_000, 8191, 1_000_000, true, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("lock", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::LockableExceeded {
            compute: "lock".to_string(),
            locked: 8192,
            lockable: 8191,
        }]
    );
    // worst 5000 ≤ budget ⇒ no budget error, and ≤ RAM ⇒ no RAM warning.
    assert!(report.warnings.is_empty());
}

/// I-MB4: `pool_slots` < R+1 is a hard error; the drop-free shortfall is a
/// separate, non-failing warning (I-MB11).
#[test]
fn budget_negative_pool_invariant() {
    let ch = channel("pool", 2, 100, 0, 4, 0); // R+1 = 5 > 2
    let cost = compute(
        budget(1_000_000, 1_000_000, 1_000_000, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("pool", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::PoolInvariantViolation {
            compute: "pool".to_string(),
            channel: "pool".to_string(),
            have: 2,
            need: 5,
        }]
    );
    assert_eq!(
        report.warnings,
        vec![BudgetWarning::DropFreeShortfall {
            compute: "pool".to_string(),
            channel: "pool".to_string(),
            have: 2,
            need: 5,
        }]
    );
}

/// I-MB3: worst-case > physical RAM but ≤ drm budget ⇒ strong warning, no error.
#[test]
fn budget_negative_warning_band() {
    let ch = channel("ram", 10, 1000, 0, 1, 0); // 10000 B
    let cost = compute(budget(500_000, 500_000, 9999, false, 4096), vec![ch], 0, 0);
    let report = verify_compute("ram", &cost);
    assert!(report.passes(), "warning band never fails the build");
    assert!(report.errors.is_empty());
    assert_eq!(
        report.warnings,
        vec![BudgetWarning::OverPhysicalRam {
            compute: "ram".to_string(),
            worst: 10_000,
            ram: 9999,
        }]
    );
}

/// I-MB11: `pool_slots` ≥ R+1 but < R+H+1 ⇒ drop-free shortfall warning, valid.
#[test]
fn budget_negative_dropfree_warning() {
    let ch = channel("df", 6, 100, 0, 4, 3); // R+1=5 ≤ 6; R+H+1=8 > 6
    let cost = compute(
        budget(1_000_000, 1_000_000, 1_000_000, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("df", &cost);
    assert!(report.passes(), "drop-free shortfall is not a gate");
    assert!(report.errors.is_empty());
    assert_eq!(
        report.warnings,
        vec![BudgetWarning::DropFreeShortfall {
            compute: "df".to_string(),
            channel: "df".to_string(),
            have: 6,
            need: 8,
        }]
    );
}

/// I-MB6: the pool-floor shortfall warning never fires when R+H+1 == `pool_slots`.
#[test]
fn budget_negative_dropfree_exact_boundary() {
    let ch = channel("dfx", 8, 100, 0, 4, 3); // R+H+1 = 8 == pool_slots
    let cost = compute(
        budget(1_000_000, 1_000_000, 1_000_000, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("dfx", &cost);
    assert!(report.passes() && report.warnings.is_empty(), "{report:?}");
}

/// I-MB6: a channel byte-product overflow is a collected `CapacityOverflow`,
/// naming the channel — never a wrapped small value or a panic.
#[test]
fn budget_negative_product_overflow() {
    let ch = channel("ovf", u64::MAX, 2, 0, 0, 0);
    let cost = compute(
        budget(u64::MAX, u64::MAX, u64::MAX, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("ovf", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::CapacityOverflow {
            compute: "ovf".to_string(),
            channel: Some("ovf".to_string()),
        }]
    );
}

/// I-MB4/I-MB6: `R+1` is itself checked arithmetic — `history_depth == u64::MAX`
/// overflows to `CapacityOverflow`, not a wrapped floor.
#[test]
fn budget_negative_r_plus_one_overflow() {
    let ch = channel("r", 1, 1, 0, u64::MAX, 0);
    let cost = compute(
        budget(u64::MAX, u64::MAX, u64::MAX, false, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("r", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::CapacityOverflow {
            compute: "r".to_string(),
            channel: Some("r".to_string()),
        }]
    );
}

/// I-MB6: an overflow summing across channels is collected, names the channel
/// that overflowed, and preserves the partial sum (no short-circuit).
#[test]
fn budget_negative_sum_overflow() {
    let half = 9_223_372_036_854_775_808u64; // 2^63
    let s1 = channel("s1", half, 1, 0, 0, 0); // 2^63 bytes
    let s2 = channel("s2", half, 1, 0, 0, 0); // 2^63 bytes ⇒ sum overflows
    let cost = compute(
        budget(u64::MAX, u64::MAX, u64::MAX, false, 4096),
        vec![s1, s2],
        0,
        0,
    );
    let report = verify_compute("sum", &cost);
    assert!(!report.passes());
    assert_eq!(
        report.errors,
        vec![BudgetError::CapacityOverflow {
            compute: "sum".to_string(),
            channel: Some("s2".to_string()),
        }]
    );
    assert_eq!(
        report.estimates[0].worst_case, half,
        "partial sum preserved"
    );
    assert!(channels_worst_case(&cost.channels).is_none());
}

/// I-MB6/I-MB10: page-rounding an almost-maximal span overflows to
/// `CapacityOverflow`, never a wrapped span.
#[test]
fn budget_negative_page_round_overflow() {
    let mut ch = channel("pg", 1, u64::MAX - 1, 0, 0, 0);
    ch.precommit = true; // round(u64::MAX-1, 4096) overflows
    let cost = compute(
        budget(u64::MAX, u64::MAX, u64::MAX, true, 4096),
        vec![ch],
        0,
        0,
    );
    let report = verify_compute("pg", &cost);
    assert!(!report.passes());
    assert!(
        report.errors.iter().any(|e| matches!(
            e,
            BudgetError::CapacityOverflow { channel: Some(id), .. } if id == "pg"
        )),
        "errors: {:?}",
        report.errors
    );
}

/// REQ-TOOL-006: nominal estimates are informational — an enormous nominal
/// never becomes an error, and it is reported verbatim.
#[test]
fn budget_negative_nominal_never_gates() {
    let mut ch = channel("nom", 1, 100, 0, 0, 0);
    ch.nominal_bytes = u64::MAX;
    let cost = compute(budget(1000, 1000, 1_000_000, false, 4096), vec![ch], 0, 0);
    let report = verify_compute("nom", &cost);
    assert!(report.passes(), "nominal is informational");
    assert_eq!(report.estimates[0].nominal, u64::MAX);
    assert_eq!(report.estimates[0].worst_case, 100);
}

/// I-MB2 boundary: worst == budget and locked == lockable are NOT errors
/// (the rules are strict `>`), exercised directly.
#[test]
fn budget_negative_equality_is_not_error() {
    let mut ch = channel("bnd", 1, 5000, 0, 0, 0); // 5000 B ⇒ 8192 locked span
    ch.precommit = true;
    let cost = compute(budget(5000, 8192, 1_000_000, true, 4096), vec![ch], 0, 0);
    let report = verify_compute("bnd", &cost);
    assert!(report.passes(), "equality passes: {:?}", report.errors);
    assert_eq!(report.estimates[0].worst_case, 5000);
    assert_eq!(report.estimates[0].locked, 8192);
}

/// I-MB2/I-MB8/I-MB9: on a precommit compute, router and spatial regions enter
/// the locked sum as page-rounded spans (worked example: 4096 router ⇒ +4096).
#[test]
fn budget_negative_precommit_regions_locked() {
    let cost = compute(
        budget(1_000_000, 5000, 1_000_000, true, 4096),
        Vec::new(),
        4096,
        3000,
    );
    let report = verify_compute("regions", &cost);
    // locked = round(4096) + round(3000) = 4096 + 4096 = 8192 > 5000.
    assert_eq!(
        report.errors,
        vec![BudgetError::LockableExceeded {
            compute: "regions".to_string(),
            locked: 8192,
            lockable: 5000,
        }]
    );
    assert_eq!(report.estimates[0].worst_case, 4096 + 3000);
    assert_eq!(report.estimates[0].locked, 8192);
}
