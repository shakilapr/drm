//! Watch-dispatch integration suite (work-plan M09W1-E2-T2;
//! build-orchestration.md §Test Matrix): reload-before-dispatch (I-BO5),
//! pending-dirty retention (I-BO6), rescan recovery (I-BO7), path-only
//! ownership (I-BO8), affected-node scoping (I-BO3), and the no-source-tracking
//! probe (I-BO2). REQ: REQ-TOOL-016, REQ-TOOL-066, REQ-TOOL-067, REQ-TOOL-014,
//! REQ-TOOL-015.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use drm_workspace::model::{ModelError, PackageInput, TreeInput, WorkspaceModel, build};
use drm_workspace::watch::{WatchControl, WatchEvent, WatchState, affected_nodes, apply_batch};

/// Builds a node manifest with the given interface. REQ: REQ-TOOL-047.
fn manifest(name: &str, publishes: &[&str], consumes: &[&str]) -> String {
    let list = |v: &[&str]| {
        v.iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = format!("name = \"{name}\"\n");
    let _ = writeln!(
        s,
        "publishes = [{}]\nconsumes = [{}]",
        list(publishes),
        list(consumes)
    );
    s
}

/// Builds a model over topics `pkg/t1..t3` with the given node interfaces.
/// REQ: REQ-TOOL-045.
fn model(nodes: &[(&str, &[&str], &[&str])]) -> WorkspaceModel {
    let topics = ["t1", "t2", "t3"]
        .iter()
        .map(|s| ((*s).to_string(), b"v: u32\n".to_vec()))
        .collect();
    let node_inputs = nodes
        .iter()
        .map(|(name, publishes, consumes)| {
            (
                (*name).to_string(),
                manifest(name, publishes, consumes).into_bytes(),
            )
        })
        .collect();
    let tree = TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics,
        }],
        nodes: node_inputs,
        topology: None,
    };
    build(&tree).unwrap_or_else(|e| unreachable!("model: {e:?}"))
}

/// Watch state with a single `controller` node rooted under `/ws`.
/// REQ: REQ-TOOL-016.
fn state(nodes: &[(&str, &[&str], &[&str])]) -> WatchState {
    let mut roots = BTreeMap::new();
    for (name, _, _) in nodes {
        roots.insert(
            (*name).to_string(),
            PathBuf::from(format!("/ws/packages/pkg/nodes/{name}")),
        );
    }
    WatchState {
        model: model(nodes),
        node_roots: roots,
        pending_source: BTreeSet::new(),
        pending_rescan: false,
    }
}

/// REQ: REQ-TOOL-016 — I-BO3: only nodes whose interface or used-topic schema
/// changed are affected; unrelated nodes are untouched.
#[test]
fn dev_affected_nodes() {
    let old: &[(&str, &[&str], &[&str])] = &[("a", &["pkg/t1"], &[]), ("b", &["pkg/t2"], &[])];
    let changed: &[(&str, &[&str], &[&str])] =
        &[("a", &["pkg/t1"], &["pkg/t2"]), ("b", &["pkg/t2"], &[])];
    let old = model(old);
    let changed = model(changed);
    assert_eq!(
        affected_nodes(&old, &changed),
        BTreeSet::from(["a".to_string()])
    );
}

/// REQ: REQ-TOOL-016 — I-BO5: a batch with a declaration and a source reloads
/// (model applied) before the source is mapped to its owning node.
#[test]
fn watch_reload_before_dispatch() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let batch = vec![
        PathBuf::from("/ws/packages/pkg/nodes/controller/drm.toml"),
        PathBuf::from("/ws/packages/pkg/nodes/controller/src/main.cpp"),
    ];
    let next = model(nodes);
    let mut reloads = 0_usize;
    let events = apply_batch(&mut st, &batch, WatchControl::Normal, &mut || {
        reloads += 1;
        Ok(next.clone())
    });
    assert_eq!(reloads, 1, "exactly one reload");
    assert!(
        matches!(events.first(), Some(WatchEvent::DrmDeclarationChanged(_))),
        "declaration dispatched first: {events:?}"
    );
    assert!(
        matches!(
            events.get(1),
            Some(WatchEvent::AppSourceChanged { node, .. }) if node == "controller"
        ),
        "source mapped after reload: {events:?}"
    );
}

/// REQ: REQ-TOOL-066 — I-BO6: source events in a failed-declaration batch are
/// retained and dispatched once after the next valid reload.
#[test]
fn watch_pending_dirty_dispatch() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let source = PathBuf::from("/ws/packages/pkg/nodes/controller/src/main.cpp");
    let decl = PathBuf::from("/ws/packages/pkg/nodes/controller/drm.toml");

    let failed = apply_batch(
        &mut st,
        &[source.clone(), decl.clone()],
        WatchControl::Normal,
        &mut || Err(Vec::<ModelError>::new()),
    );
    assert!(
        matches!(failed.as_slice(), [WatchEvent::DeclarationError(_)]),
        "only the declaration error surfaces: {failed:?}"
    );
    assert!(st.pending_source.contains(&source), "dirty source retained");
    assert!(
        !failed
            .iter()
            .any(|e| matches!(e, WatchEvent::AppSourceChanged { .. })),
        "no dispatch against a stale model"
    );

    let next = model(nodes);
    let recovered = apply_batch(&mut st, &[decl], WatchControl::Normal, &mut || {
        Ok(next.clone())
    });
    assert!(
        recovered
            .iter()
            .any(|e| matches!(e, WatchEvent::DrmDeclarationChanged(_)))
    );
    assert!(
        recovered.iter().any(
            |e| matches!(e, WatchEvent::AppSourceChanged { node, .. } if node == "controller")
        ),
        "pending dirty dispatched via the new model: {recovered:?}"
    );
    assert!(st.pending_source.is_empty(), "cleared after dispatch");
}

/// REQ: REQ-TOOL-067 — I-BO7: watch-queue loss emits `RescanRequired`; the next
/// batch reconstructs state and dispatches the conservative dirty set.
#[test]
fn watch_overflow_rescan() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let lost = apply_batch(&mut st, &[], WatchControl::Overflow, &mut || {
        unreachable!("overflow must not reload")
    });
    assert_eq!(lost, vec![WatchEvent::RescanRequired]);
    assert!(st.pending_rescan, "rescan flagged");

    let next = model(nodes);
    let mut reloads = 0_usize;
    let recovered = apply_batch(&mut st, &[], WatchControl::Normal, &mut || {
        reloads += 1;
        Ok(next.clone())
    });
    assert_eq!(reloads, 1, "recovery reload");
    assert!(!st.pending_rescan, "rescan cleared");
    assert!(
        recovered.iter().any(
            |e| matches!(e, WatchEvent::AppSourceChanged { node, .. } if node == "controller")
        ),
        "conservative source rescan dispatched: {recovered:?}"
    );
}

/// REQ: REQ-TOOL-014 — I-BO8: ownership is path-only; a non-UTF8-content source
/// (never read) still maps to its owning node.
#[test]
fn watch_binary_source() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let binary = PathBuf::from("/ws/packages/pkg/nodes/controller/src/main.cpp");
    let next = model(nodes);
    let events = apply_batch(&mut st, &[binary], WatchControl::Normal, &mut || {
        Ok(next.clone())
    });
    assert!(
        matches!(
            events.as_slice(),
            [WatchEvent::AppSourceChanged { node, .. }] if node == "controller"
        ),
        "path-only ownership: {events:?}"
    );
}

/// REQ: REQ-TOOL-015 — the no-source-tracking probe: one source change yields
/// exactly one owning-node dispatch (no DRM-driven dependency analysis).
#[test]
fn no_source_tracking_probe() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let source = PathBuf::from("/ws/packages/pkg/nodes/controller/src/lib.rs");
    let next = model(nodes);
    let events = apply_batch(&mut st, &[source], WatchControl::Normal, &mut || {
        Ok(next.clone())
    });
    let dispatches = events
        .iter()
        .filter(|e| matches!(e, WatchEvent::AppSourceChanged { .. }))
        .count();
    assert_eq!(dispatches, 1);
}

/// REQ: REQ-TOOL-014 — I-BO2: one application-source change invokes exactly one
/// owning-node adapter; DRM performs no dependency analysis (dev-suite alias of
/// the no-source-tracking probe).
#[test]
fn dev_no_source_tracking() {
    let nodes: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let mut st = state(nodes);
    let source = PathBuf::from("/ws/packages/pkg/nodes/controller/src/main.cpp");
    let next = model(nodes);
    let events = apply_batch(&mut st, &[source], WatchControl::Normal, &mut || {
        Ok(next.clone())
    });
    let dispatches = events
        .iter()
        .filter(|e| matches!(e, WatchEvent::AppSourceChanged { .. }))
        .count();
    assert_eq!(
        dispatches, 1,
        "exactly one adapter invocation per source change"
    );
}

/// REQ: REQ-TOOL-015 — I-BO4: 50 rapid declaration edits reload the model on
/// each event and leave the final state applied; no stale cache.
#[test]
fn dev_rapid_edits_final_state() {
    let initial: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let changed: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &["pkg/t2"])];
    let mut st = state(initial);
    let final_model = model(changed);
    let mut reloads = 0_usize;
    for i in 0..50 {
        let decl = PathBuf::from(format!("/ws/packages/pkg/topics/t{i}.drm"));
        let events = apply_batch(&mut st, &[decl], WatchControl::Normal, &mut || {
            reloads += 1;
            Ok(final_model.clone())
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WatchEvent::DrmDeclarationChanged(_))),
            "declaration dispatched at edit {i}"
        );
    }
    assert_eq!(reloads, 50, "one reload per declaration event");
    assert_eq!(st.model, final_model, "final state applied, no stale cache");
}

/// REQ: REQ-TOOL-015 — I-BO4: 50 declaration edits coalesced into one debounced
/// batch reload once and apply the final state.
#[test]
fn dev_rapid_edits_coalesced() {
    let initial: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &[])];
    let changed: &[(&str, &[&str], &[&str])] = &[("controller", &["pkg/t1"], &["pkg/t2"])];
    let mut st = state(initial);
    let final_model = model(changed);
    let batch: Vec<PathBuf> = (0..50)
        .map(|i| PathBuf::from(format!("/ws/packages/pkg/topics/t{i}.drm")))
        .collect();
    let mut reloads = 0_usize;
    let events = apply_batch(&mut st, &batch, WatchControl::Normal, &mut || {
        reloads += 1;
        Ok(final_model.clone())
    });
    assert_eq!(reloads, 1, "debounce coalesces to one reload (I-BO4)");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WatchEvent::DrmDeclarationChanged(_)))
    );
    assert_eq!(st.model, final_model);
}
