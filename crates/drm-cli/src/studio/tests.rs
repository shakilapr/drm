//! Studio host + protocol + journal tests (I-ST1..I-ST13 slices). Every test
//! cites the requirement it verifies. REQ: REQ-STUDIO-001, REQ-STUDIO-002,
//! REQ-STUDIO-003, REQ-STUDIO-005, REQ-STUDIO-007, REQ-STUDIO-008,
//! REQ-STUDIO-009, REQ-STUDIO-011, REQ-STUDIO-012, REQ-STUDIO-013,
//! REQ-STUDIO-014.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use drm_workspace::model::render_model;
use drm_workspace::verify::verify_lite;

use crate::args::Direction;
use crate::studio::host::StudioHost;
use crate::studio::journal::{ChangeSet, JournalStore, output_for};
use crate::studio::protocol::{
    Capability, Hello, MAX_CONTROL_BYTES, ModelCommand, NodeSpec, StudioError, StudioRequest,
    StudioResponse, parse_bounded, to_json_line,
};
use crate::studio::serve_stdio;
use crate::{assign, ws};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Creates a throwaway workspace with one topic and one empty node.
/// REQ: REQ-STUDIO-003.
fn temp_workspace(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("drm-studio-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("packages/control/topics"))
        .unwrap_or_else(|e| unreachable!("mkdir topics: {e}"));
    std::fs::write(root.join("packages/control/topics/target.drm"), "v: u32\n")
        .unwrap_or_else(|e| unreachable!("write target: {e}"));
    std::fs::write(root.join("packages/control/topics/pose.drm"), "x: f32\n")
        .unwrap_or_else(|e| unreachable!("write pose: {e}"));
    let node = root.join("packages/control/nodes/controller");
    std::fs::create_dir_all(&node).unwrap_or_else(|e| unreachable!("mkdir node: {e}"));
    std::fs::write(
        node.join("drm.toml"),
        "# controller node\nname = \"controller\"\n\npublishes = [\"control/target\"]\nconsumes = []\n",
    )
    .unwrap_or_else(|e| unreachable!("write node: {e}"));
    let producer = root.join("packages/control/nodes/producer");
    std::fs::create_dir_all(&producer).unwrap_or_else(|e| unreachable!("mkdir producer: {e}"));
    std::fs::write(
        producer.join("drm.toml"),
        "name = \"producer\"\npublishes = [\"control/pose\"]\nconsumes = []\n",
    )
    .unwrap_or_else(|e| unreachable!("write producer: {e}"));
    root
}

/// Builds + renders the canonical model of `root`. REQ: REQ-STUDIO-003.
fn render_at(root: &Path) -> String {
    let discovered = ws::discover(root).unwrap_or_else(|e| unreachable!("discover: {e}"));
    let model = ws::build_model(&discovered).unwrap_or_else(|e| unreachable!("model: {e:?}"));
    render_model(&model)
}

/// Serializes requests into a newline-joined stdio script. REQ: REQ-STUDIO-007.
fn script(requests: &[StudioRequest]) -> Vec<u8> {
    let mut out = String::new();
    for req in requests {
        out.push_str(&to_json_line(req).unwrap_or_else(|e| unreachable!("encode: {e}")));
        out.push('\n');
    }
    out.into_bytes()
}

/// Parses each output line back into a typed response. REQ: REQ-STUDIO-007.
fn parse_replies(bytes: &[u8]) -> Vec<StudioResponse> {
    let text = String::from_utf8(bytes.to_vec()).unwrap_or_else(|e| unreachable!("utf8: {e}"));
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| unreachable!("reply: {e}")))
        .collect()
}

/// REQ: REQ-STUDIO-007, REQ-STUDIO-011 — a `Hello` handshake followed by
/// `OpenWorkspace` + `Snapshot` round-trips over stdio.
#[test]
fn studio_host_handshake_and_dispatch() {
    let root = temp_workspace("roundtrip");
    let mut host = StudioHost::new(root.clone(), true);
    let input = script(&[
        StudioRequest::Hello(Hello {
            protocol_version: 1,
            client_version: "1.7.0".to_string(),
        }),
        StudioRequest::OpenWorkspace {
            path: root.display().to_string(),
        },
        StudioRequest::Snapshot,
        StudioRequest::Verify,
    ]);
    let mut rx = Cursor::new(input);
    let mut tx: Vec<u8> = Vec::new();
    serve_stdio(&mut host, &mut rx, &mut tx).unwrap_or_else(|e| unreachable!("serve: {e}"));
    let replies = parse_replies(&tx);
    assert!(matches!(replies.first(), Some(StudioResponse::HelloAck(_))));
    assert!(
        replies
            .iter()
            .any(|r| matches!(r, StudioResponse::Snapshot(_))),
        "snapshot reply missing: {replies:?}"
    );
    assert!(
        replies
            .iter()
            .any(|r| matches!(r, StudioResponse::Verify(_))),
        "verify reply missing: {replies:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-007 — the first request must be `Hello`; anything else is
/// rejected with a handshake error.
#[test]
fn studio_host_handshake_required() {
    let root = temp_workspace("handshake");
    let mut host = StudioHost::new(root.clone(), true);
    let input = script(&[StudioRequest::Snapshot]);
    let mut rx = Cursor::new(input);
    let mut tx: Vec<u8> = Vec::new();
    let Err(err) = serve_stdio(&mut host, &mut rx, &mut tx) else {
        unreachable!("handshake must be mandatory")
    };
    assert!(matches!(err, StudioError::Handshake(_)), "got {err:?}");
    let replies = parse_replies(&tx);
    assert!(matches!(
        replies.first(),
        Some(StudioResponse::Error { .. })
    ));
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-001, REQ-STUDIO-012 — the capability model exposes `Trust`
/// and `ExtensionsWeb` only for a trusted workspace.
#[test]
fn studio_host_capability_model() {
    let trusted = crate::studio::protocol::capabilities(true);
    let untrusted = crate::studio::protocol::capabilities(false);
    assert!(trusted.contains(&Capability::Trust));
    assert!(trusted.contains(&Capability::ExtensionsWeb));
    assert!(!untrusted.contains(&Capability::Trust));
    assert!(!untrusted.contains(&Capability::ExtensionsWeb));
    assert!(untrusted.contains(&Capability::WorkspaceV2));
}

/// REQ: REQ-STUDIO-013 — an oversize control frame is rejected before parsing
/// (I-ST13).
#[test]
fn studio_host_frame_bounds() {
    let root = temp_workspace("bounds");
    let mut host = StudioHost::new(root.clone(), true);
    let mut input = script(&[StudioRequest::Hello(Hello {
        protocol_version: 1,
        client_version: "1.7.0".to_string(),
    })]);
    input.extend(std::iter::repeat_n(b'x', MAX_CONTROL_BYTES + 16));
    input.push(b'\n');
    let mut rx = Cursor::new(input);
    let mut tx: Vec<u8> = Vec::new();
    let Err(err) = serve_stdio(&mut host, &mut rx, &mut tx) else {
        unreachable!("oversize must fail")
    };
    assert!(matches!(err, StudioError::Bounds(_)), "got {err:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-003 — CLI `assign` and the Studio `NodeUpdate` command
/// produce identical model renders (single semantics, I-ST1 parity).
#[test]
fn studio_host_parity_cli_vs_studio() {
    let cli_root = temp_workspace("parity-cli");
    assign::run(&cli_root, "control/pose", Direction::Consume, "controller")
        .unwrap_or_else(|e| unreachable!("cli assign: {e}"));
    let cli_render = render_at(&cli_root);

    let studio_root = temp_workspace("parity-studio");
    let mut host = StudioHost::new(studio_root.clone(), true);
    host.dispatch(StudioRequest::OpenWorkspace {
        path: studio_root.display().to_string(),
    })
    .unwrap_or_else(|e| unreachable!("open: {e}"));
    let resp = host
        .dispatch(StudioRequest::Mutate {
            base_revision: host.revision(),
            command: ModelCommand::NodeUpdate(NodeSpec {
                name: "controller".to_string(),
                publishes: vec!["control/target".to_string()],
                consumes: vec!["control/pose".to_string()],
            }),
        })
        .unwrap_or_else(|e| unreachable!("mutate: {e}"));
    let StudioResponse::MutationResult { render, .. } = resp else {
        unreachable!("expected mutation result, got {resp:?}")
    };
    assert_eq!(
        cli_render, render,
        "CLI and Studio renders must match (I-ST1)"
    );
    let _ = std::fs::remove_dir_all(&cli_root);
    let _ = std::fs::remove_dir_all(&studio_root);
}

/// REQ: REQ-STUDIO-009 — a stale `base_revision` returns `Conflict{revision}`
/// and leaves the workspace unchanged (I-ST7).
#[test]
fn studio_revision_conflict() {
    let root = temp_workspace("revision");
    let mut host = StudioHost::new(root.clone(), true);
    host.dispatch(StudioRequest::OpenWorkspace {
        path: root.display().to_string(),
    })
    .unwrap_or_else(|e| unreachable!("open: {e}"));
    let first = host.revision();
    let cmd = || {
        ModelCommand::NodeUpdate(NodeSpec {
            name: "controller".to_string(),
            publishes: vec!["control/target".to_string()],
            consumes: vec!["control/pose".to_string()],
        })
    };
    let ok = host
        .dispatch(StudioRequest::Mutate {
            base_revision: first,
            command: cmd(),
        })
        .unwrap_or_else(|e| unreachable!("first mutate: {e}"));
    assert!(matches!(ok, StudioResponse::MutationResult { .. }));
    let current = host.revision();
    assert!(current > first);

    let stale = host
        .dispatch(StudioRequest::Mutate {
            base_revision: first,
            command: cmd(),
        })
        .unwrap_or_else(|e| unreachable!("stale mutate: {e}"));
    assert_eq!(stale, StudioResponse::Conflict { revision: current });
    assert_eq!(host.revision(), current, "conflict must not bump revision");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-012 — mutations on an untrusted workspace are refused
/// (I-ST11), while read-only snapshot stays available.
#[test]
fn studio_trust_gate() {
    let root = temp_workspace("trust");
    let mut host = StudioHost::new(root.clone(), false);
    assert!(!host.trusted());
    let open = host.dispatch(StudioRequest::OpenWorkspace {
        path: root.display().to_string(),
    });
    assert!(matches!(open, Ok(StudioResponse::Snapshot(_))));
    let refused = host.dispatch(StudioRequest::Mutate {
        base_revision: host.revision(),
        command: ModelCommand::NodeUpdate(NodeSpec {
            name: "controller".to_string(),
            publishes: vec!["control/target".to_string()],
            consumes: vec!["control/pose".to_string()],
        }),
    });
    assert!(matches!(refused, Err(StudioError::Untrusted(_))));
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-002 — undo is refused when an input changed externally, and
/// force reverts it (I-ST9).
#[test]
fn studio_undo_guard() {
    let root = temp_workspace("undo");
    let manifest = root.join("packages/control/nodes/controller/drm.toml");
    let prior = std::fs::read_to_string(&manifest).unwrap_or_else(|e| unreachable!("read: {e}"));
    let mut host = StudioHost::new(root.clone(), true);
    host.dispatch(StudioRequest::OpenWorkspace {
        path: root.display().to_string(),
    })
    .unwrap_or_else(|e| unreachable!("open: {e}"));
    let resp = host
        .dispatch(StudioRequest::Mutate {
            base_revision: host.revision(),
            command: ModelCommand::NodeUpdate(NodeSpec {
                name: "controller".to_string(),
                publishes: vec!["control/target".to_string()],
                consumes: vec!["control/pose".to_string()],
            }),
        })
        .unwrap_or_else(|e| unreachable!("mutate: {e}"));
    let StudioResponse::MutationResult { changeset, .. } = resp else {
        unreachable!("expected mutation")
    };
    let edited = std::fs::read_to_string(&manifest).unwrap_or_else(|e| unreachable!("read: {e}"));
    std::fs::write(&manifest, format!("{edited}\n# external edit\n"))
        .unwrap_or_else(|e| unreachable!("external: {e}"));

    let blocked = host
        .dispatch(StudioRequest::Undo {
            changeset: changeset.clone(),
            force: false,
        })
        .unwrap_or_else(|e| unreachable!("undo: {e}"));
    assert_eq!(
        blocked,
        StudioResponse::ConflictExternalChanged {
            changeset: changeset.clone()
        }
    );
    let forced = host
        .dispatch(StudioRequest::Undo {
            changeset,
            force: true,
        })
        .unwrap_or_else(|e| unreachable!("force undo: {e}"));
    assert!(matches!(forced, StudioResponse::MutationResult { .. }));
    let reverted = std::fs::read_to_string(&manifest).unwrap_or_else(|e| unreachable!("read: {e}"));
    assert_eq!(
        reverted, prior,
        "force undo must restore the prior content verbatim"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Writes a `Prepared` journal for `cs` into `store` (kill-point injection).
/// REQ: REQ-STUDIO-008.
fn write_prepared_journal(store: &JournalStore, cs: &ChangeSet) {
    std::fs::create_dir_all(store.dir()).unwrap_or_else(|e| unreachable!("dir: {e}"));
    std::fs::write(
        store.dir().join(format!("{}.json", cs.id)),
        serde_json::to_string_pretty(cs).unwrap_or_else(|e| unreachable!("json: {e}")),
    )
    .unwrap_or_else(|e| unreachable!("journal: {e}"));
}

/// REQ: REQ-STUDIO-008 — a `Prepared` journal with no rename rolls back:
/// temps removed, targets unchanged (kill between journal and rename).
#[test]
fn journal_recovery_prepared_rolls_back() {
    let root = temp_workspace("journal-prepared");
    let store = JournalStore::new(&root);
    let target = root.join("packages/control/nodes/controller/drm.toml");
    let prior = std::fs::read_to_string(&target).unwrap_or_else(|e| unreachable!("read: {e}"));
    let out = output_for(
        "t1",
        target.clone(),
        "name = \"controller\"\nchanged = true\n".to_string(),
    )
    .unwrap_or_else(|e| unreachable!("output: {e}"));
    let temp = out.temp.clone();
    std::fs::write(&temp, &out.content).unwrap_or_else(|e| unreachable!("temp: {e}"));
    let mut cs = ChangeSet::new(vec![out]);
    cs.id = "t1".to_string();
    write_prepared_journal(&store, &cs);

    let recovered = store
        .recover()
        .unwrap_or_else(|e| unreachable!("recover: {e}"));
    assert_eq!(recovered, 1);
    assert!(!temp.exists(), "rolled-back temp must be removed");
    let after = std::fs::read_to_string(&target).unwrap_or_else(|e| unreachable!("read: {e}"));
    assert_eq!(after, prior, "rollback must leave the target unchanged");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-008 — a partially renamed journal is finished on recovery,
/// and a second recovery is a no-op (idempotent, I-ST8/I-ST12).
#[test]
fn journal_recovery_partial_finishes() {
    let root = temp_workspace("journal-partial");
    let store = JournalStore::new(&root);
    let target_a = root.join("a.txt");
    let target_b = root.join("b.txt");
    let out_a = output_for("p1a", target_a.clone(), "alpha\n".to_string())
        .unwrap_or_else(|e| unreachable!("output a: {e}"));
    let out_b = output_for("p1b", target_b.clone(), "beta\n".to_string())
        .unwrap_or_else(|e| unreachable!("output b: {e}"));
    let temp_b = out_b.temp.clone();
    // Simulate a crash after the first rename and before the second.
    std::fs::write(&target_a, &out_a.content).unwrap_or_else(|e| unreachable!("a: {e}"));
    std::fs::write(&temp_b, &out_b.content).unwrap_or_else(|e| unreachable!("b temp: {e}"));
    let mut cs = ChangeSet::new(vec![out_a, out_b]);
    cs.id = "p1".to_string();
    write_prepared_journal(&store, &cs);

    let recovered = store
        .recover()
        .unwrap_or_else(|e| unreachable!("recover: {e}"));
    assert_eq!(recovered, 1);
    let a = std::fs::read_to_string(&target_a).unwrap_or_else(|e| unreachable!("read a: {e}"));
    let b = std::fs::read_to_string(&target_b).unwrap_or_else(|e| unreachable!("read b: {e}"));
    assert_eq!(a, "alpha\n");
    assert_eq!(b, "beta\n", "recovery must finish the pending rename");
    assert!(!temp_b.exists(), "temp consumed by the finishing rename");
    let again = store
        .recover()
        .unwrap_or_else(|e| unreachable!("recover 2: {e}"));
    assert_eq!(again, 0, "second recovery is a no-op");
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-008 — the six-step commit leaves the target updated and the
/// journal removed (happy path).
#[test]
fn journal_commit_clean() {
    let root = temp_workspace("journal-clean");
    let store = JournalStore::new(&root);
    let target = root.join("packages/control/nodes/controller/drm.toml");
    let out = output_for(
        "c1",
        target.clone(),
        "name = \"controller\"\nok = true\n".to_string(),
    )
    .unwrap_or_else(|e| unreachable!("output: {e}"));
    let mut cs = ChangeSet::new(vec![out]);
    store
        .commit(&mut cs)
        .unwrap_or_else(|e| unreachable!("commit: {e}"));
    let after = std::fs::read_to_string(&target).unwrap_or_else(|e| unreachable!("read: {e}"));
    assert!(after.contains("ok = true"));
    assert!(!store.dir().join(format!("{}.json", cs.id)).exists());
    assert!(
        store
            .recover()
            .unwrap_or_else(|e| unreachable!("recover: {e}"))
            == 0
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-005 — a format-preserving edit through the host keeps
/// comments and ordering (I-ST6).
#[test]
fn studio_host_format_preservation() {
    let root = temp_workspace("preserve");
    let mut host = StudioHost::new(root.clone(), true);
    host.dispatch(StudioRequest::OpenWorkspace {
        path: root.display().to_string(),
    })
    .unwrap_or_else(|e| unreachable!("open: {e}"));
    host.dispatch(StudioRequest::Mutate {
        base_revision: host.revision(),
        command: ModelCommand::NodeUpdate(NodeSpec {
            name: "controller".to_string(),
            publishes: vec!["control/target".to_string()],
            consumes: vec!["control/pose".to_string()],
        }),
    })
    .unwrap_or_else(|e| unreachable!("mutate: {e}"));
    let manifest = std::fs::read_to_string(root.join("packages/control/nodes/controller/drm.toml"))
        .unwrap_or_else(|e| unreachable!("read: {e}"));
    assert!(manifest.contains("# controller node"), "comment lost");
    assert!(manifest.contains("control/pose"));
    let _ = std::fs::remove_dir_all(&root);
}

/// REQ: REQ-STUDIO-013 — bounded parsing rejects an oversize line and decodes a
/// valid one.
#[test]
fn studio_parse_bounds() {
    let ok = parse_bounded("{\"method\":\"snapshot\"}");
    assert!(matches!(ok, Ok(StudioRequest::Snapshot)));
    let big = "x".repeat(MAX_CONTROL_BYTES + 1);
    assert!(matches!(parse_bounded(&big), Err(StudioError::Bounds(_))));
}

/// REQ: REQ-STUDIO-014 — verify findings surface as snapshot problems.
#[test]
fn studio_snapshot_reports_problems() {
    let root = temp_workspace("problems");
    // A declared topic consumed by a node but published by nobody is a
    // verify-lite finding.
    std::fs::write(root.join("packages/control/topics/orphan.drm"), "v: u8\n")
        .unwrap_or_else(|e| unreachable!("write orphan topic: {e}"));
    let node = root.join("packages/control/nodes/consumer");
    std::fs::create_dir_all(&node).unwrap_or_else(|e| unreachable!("mkdir: {e}"));
    std::fs::write(
        node.join("drm.toml"),
        "name = \"consumer\"\nconsumes = [\"control/orphan\"]\n",
    )
    .unwrap_or_else(|e| unreachable!("write: {e}"));
    let discovered = ws::discover(&root).unwrap_or_else(|e| unreachable!("discover: {e}"));
    let model = ws::build_model(&discovered).unwrap_or_else(|e| unreachable!("model: {e:?}"));
    let findings = verify_lite(&model);
    assert_eq!(findings.len(), 1, "expected a no-publisher finding");
    let mut host = StudioHost::new(root.clone(), true);
    let resp = host
        .dispatch(StudioRequest::OpenWorkspace {
            path: root.display().to_string(),
        })
        .unwrap_or_else(|e| unreachable!("open: {e}"));
    let StudioResponse::Snapshot(snap) = resp else {
        unreachable!("expected snapshot")
    };
    assert_eq!(snap.problems.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}
