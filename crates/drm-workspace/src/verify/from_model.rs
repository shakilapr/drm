//! Build-time verifier configuration ([verify-engine.md](../../../docs/algorithms/toolchain/verify-engine.md)
//! §Functions `verify`): derives the verifier-side [`DeploymentConfig`] from a
//! [`WorkspaceModel`] plus the assigned handle table. This is **not** the
//! generated runtime `deployment.json` (`drm-core`); the two types are
//! unrelated declarations. REQ: REQ-TOOL-012, REQ-TOOL-057, REQ-TOOL-060.

use std::collections::{BTreeMap, BTreeSet};

use crate::budget::{ChannelCost, ComputeCost, MemoryBudget};
use crate::model::WorkspaceModel;
use crate::model::handles::HandleTable;
use crate::model::runtime::channel_expectations;

use super::engine::{DeploymentConfig, LinkCapability, Transport};

/// Derives the verifier configuration available at build time.
///
/// Sources and honest gaps:
/// - `handles`: the prior handle table the orchestrator already loaded (V-ID).
/// - `links`: `model.links` ids. The topology manifest carries no transport,
///   payload bound, or line rate, so every link defaults to `Udp` with no
///   declared `max_datagram_payload` and `capacity_bps = 0`. V-RT therefore
///   skips framing fit (`None` bound) and V-REL never fires (no topic declares
///   `reliable`); no per-link numbers are invented.
/// - `budgets`: channels are derived from placed nodes (publish/consume sets)
///   and the model's own pool/history expectations, so the intrinsic I-MB4
///   floor (`pool_slots >= R + 1`) is checkable without a declared ceiling.
///   No memory ceiling exists in the model, so `drm_budget_bytes`,
///   `lockable_budget_bytes`, and `physical_ram_bytes` are `u64::MAX`: only the
///   intrinsic overflow/pool checks can fail, never `BudgetExceeded`.
///   Unplaced nodes and topics attached to no node contribute no channel.
/// - `topics`, `spatial`, `rpc`, `recorders`: the workspace model has no source
///   for them, so they stay `Default`. V-CMP (consumer deadlines), V-SPAT,
///   V-RPC, and V-REC are consequently vacuous unless a future manifest
///   surface declares them.
///
/// REQ: REQ-TOOL-012, REQ-TOOL-057, REQ-TOOL-060.
#[must_use]
pub fn deployment_config(model: &WorkspaceModel, handles: &HandleTable) -> DeploymentConfig {
    let mut deploy = DeploymentConfig {
        handles: Some(handles.clone()),
        ..DeploymentConfig::default()
    };

    for link in &model.links {
        deploy.links.insert(
            link.id.clone(),
            LinkCapability {
                transport: Transport::Udp,
                max_datagram_payload: None,
                capacity_bps: 0,
            },
        );
    }

    let expectations = channel_expectations(model);
    let mut per_compute: BTreeMap<String, Vec<ChannelCost>> = BTreeMap::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (node, entry) in &model.nodes {
        let Some(compute) = model.placement.get(node) else {
            continue;
        };
        for topic in entry.publishes.iter().chain(entry.consumes.iter()) {
            if !seen.insert((compute.clone(), topic.clone())) {
                continue;
            }
            let Some(exp) = expectations.get(topic) else {
                continue;
            };
            per_compute
                .entry(compute.clone())
                .or_default()
                .push(ChannelCost {
                    id: topic.clone(),
                    pool_slots: exp.pool_size,
                    slot_bytes: exp.payload_capacity,
                    control_block: 0,
                    precommit: false,
                    history_depth: exp.history_depth,
                    borrowers: 0,
                    nominal_bytes: 0,
                });
        }
    }
    for (compute, channels) in per_compute {
        deploy.budgets.computes.insert(
            compute,
            ComputeCost {
                budget: MemoryBudget {
                    drm_budget_bytes: u64::MAX,
                    lockable_budget_bytes: u64::MAX,
                    physical_ram_bytes: u64::MAX,
                    precommit: false,
                    page_granularity: 4096,
                },
                channels,
                router_buffers: 0,
                spatial_index_bytes: 0,
            },
        );
    }

    deploy
}

#[cfg(test)]
mod tests {
    use crate::model::handles::HandleTable;
    use crate::model::{PackageInput, TreeInput, build};

    use super::deployment_config;

    /// REQ: REQ-TOOL-057 — placed nodes contribute channels with the model's
    /// own pool/history values; handles are carried through.
    #[test]
    fn derives_channels_and_handles() {
        let tree = TreeInput {
            packages: vec![PackageInput {
                name: "pkg".to_string(),
                topics: vec![(
                    "t".to_string(),
                    b"@history: 4\n@pool_size: 2\nv: u64\n".to_vec(),
                )],
            }],
            nodes: vec![(
                "n".to_string(),
                b"name = \"n\"\npublishes = [\"pkg/t\"]\n".to_vec(),
            )],
            topology: Some(b"[computes]\nc1 = true\n[placement]\nn = \"c1\"\n".to_vec()),
        };
        let model = build(&tree).unwrap_or_else(|e| unreachable!("{e:?}"));
        let deploy = deployment_config(&model, &HandleTable::default());
        let compute = deploy
            .budgets
            .computes
            .get("c1")
            .unwrap_or_else(|| unreachable!("compute"));
        assert_eq!(compute.channels.len(), 1);
        assert_eq!(compute.channels[0].pool_slots, 2);
        assert_eq!(compute.channels[0].history_depth, 4);
        assert!(deploy.handles.is_some());
    }
}
