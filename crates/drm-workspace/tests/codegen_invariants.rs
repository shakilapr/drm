//! Codegen invalidation invariant suite (work-plan M08W4-E4-T1;
//! codegen-invalidation.md §Test Matrix): I-G4 atomicity (kill harness), I-G5
//! fingerprint pruning, I-G6 node-API invalidation, I-G7 atomic replacement,
//! I-G8 direction sensitivity, I-G9 handle-free bindings, and R6 regen-diff
//! hand-edit detection.
//! REQ: REQ-TOOL-024, REQ-TOOL-025, REQ-TOOL-027, REQ-TOOL-028, REQ-TOOL-030,
//! REQ-TOOL-065.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use drm_workspace::codegen::{
    FingerprintDb, FingerprintKey, GenKind, Lang, execute, existing_generated, generator_identity,
    node_fingerprint, node_fingerprint_with_generator, out_path, plan, plan_with_generator,
    regen_diff, topic_fingerprint_with_generator,
};
use drm_workspace::model::handles::{AssignmentHistory, HandleTable};
use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};

/// Unique scratch directory under the system temp dir.
fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("drm-gen-inv-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("scratch dir: {e}"));
    dir
}

/// One-package tree with the given `(stem, source)` topics.
fn tree(topics: &[(&str, &str)]) -> TreeInput {
    TreeInput {
        packages: vec![PackageInput {
            name: "control".to_string(),
            topics: topics
                .iter()
                .map(|(stem, src)| ((*stem).to_string(), src.as_bytes().to_vec()))
                .collect(),
        }],
        nodes: Vec::new(),
        topology: None,
    }
}

/// Node manifest text with the given publish/consume topic lists.
fn manifest(name: &str, publishes: &[&str], consumes: &[&str]) -> String {
    let list = |v: &[&str]| {
        v.iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "name = \"{name}\"\npublishes = [{}]\nconsumes = [{}]\n",
        list(publishes),
        list(consumes)
    )
}

/// Builds a model or marks a fixture bug.
fn model(t: &TreeInput) -> WorkspaceModel {
    build(t).unwrap_or_else(|e| unreachable!("fixture model must build: {e:?}"))
}

/// Executes a fresh plan into `root` and returns the updated database.
fn generate(m: &WorkspaceModel, root: &Path, db: &mut FingerprintDb) {
    execute(&plan(m, db, root), db, root).unwrap_or_else(|e| unreachable!("execute: {e:?}"));
}

/// Reads every generated file under `root/.drm/gen` into a sorted relative map.
fn generated_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let files = existing_generated(root).unwrap_or_else(|e| unreachable!("walk: {e:?}"));
    let mut out = BTreeMap::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map_or_else(|_| path.clone(), Path::to_path_buf);
        let bytes = fs::read(&path).unwrap_or_else(|e| unreachable!("read: {e}"));
        out.insert(rel, bytes);
    }
    out
}

/// A schema source with `n` `u64` fields (a large rendered body for the kill
/// harness to interrupt mid-write).
fn wide_source(n: usize) -> String {
    let mut src = String::new();
    for i in 0..n {
        let _ = writeln!(src, "f{i}: u64");
    }
    src
}

/// REQ: REQ-TOOL-028 — a node declaration change without any schema change
/// regenerates the node API unit only (I-G6).
#[test]
fn node_api_invalidation() {
    let mut before = tree(&[("target", "a: u8\n")]);
    before.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/target"], &[]).into_bytes(),
    ));
    let m1 = model(&before);
    let root = scratch("node-api");
    let mut db = FingerprintDb::default();
    generate(&m1, &root, &mut db);

    let mut after = tree(&[("target", "a: u8\n")]);
    after.nodes.push((
        "controller".to_string(),
        manifest("controller", &[], &["control/target"]).into_bytes(),
    ));
    let m2 = model(&after);
    assert_ne!(
        node_fingerprint(&m1.nodes["controller"], &m1, ""),
        node_fingerprint(&m2.nodes["controller"], &m2, ""),
        "declaration direction change moves the node fingerprint"
    );
    let next = plan(&m2, &db, &root);
    assert!(
        next.dirty_units
            .iter()
            .any(|u| u.kind == GenKind::NodeApi && u.key.identity == "controller"),
        "node API unit regenerated"
    );
    assert!(
        next.dirty_units.iter().all(|u| u.kind != GenKind::Topic),
        "unchanged schema does not dirty the topic unit"
    );
    let _ = fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-027 — swapping a topic between publish and consume changes the
/// node fingerprint (I-G8: direction is identity, never a union of names).
#[test]
fn gen_direction_swap() {
    let mut publishes = tree(&[("a", "x: u8\n"), ("b", "y: u8\n")]);
    publishes.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/a"], &["control/b"]).into_bytes(),
    ));
    let m1 = model(&publishes);

    let mut consumes = tree(&[("a", "x: u8\n"), ("b", "y: u8\n")]);
    consumes.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/b"], &["control/a"]).into_bytes(),
    ));
    let m2 = model(&consumes);

    assert_ne!(
        node_fingerprint(&m1.nodes["controller"], &m1, ""),
        node_fingerprint(&m2.nodes["controller"], &m2, ""),
        "publish/consume swap changes the fingerprint"
    );
}

/// REQ: REQ-TOOL-027 — direction metamorphic relations: adding an unrelated
/// topic or editing another node leaves this node's fingerprint unchanged, while
/// flipping one of its capabilities changes it (I-G8).
#[test]
fn gen_direction_metamorphic() {
    let mut base = tree(&[("a", "x: u8\n"), ("b", "y: u8\n")]);
    base.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/a"], &["control/b"]).into_bytes(),
    ));
    base.nodes.push((
        "other".to_string(),
        manifest("other", &["control/b"], &[]).into_bytes(),
    ));
    let m_base = model(&base);
    let fp = node_fingerprint(&m_base.nodes["controller"], &m_base, "");

    // Add an unrelated topic: controller's fingerprint must not move.
    let mut extra = tree(&[("a", "x: u8\n"), ("b", "y: u8\n"), ("c", "z: u8\n")]);
    extra.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/a"], &["control/b"]).into_bytes(),
    ));
    extra.nodes.push((
        "other".to_string(),
        manifest("other", &["control/b"], &[]).into_bytes(),
    ));
    let m_extra = model(&extra);
    assert_eq!(
        fp,
        node_fingerprint(&m_extra.nodes["controller"], &m_extra, ""),
        "unrelated topic does not move the node fingerprint"
    );

    // Flip controller's capability: fingerprint changes.
    let mut flipped = tree(&[("a", "x: u8\n"), ("b", "y: u8\n")]);
    flipped.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/b"], &["control/a"]).into_bytes(),
    ));
    flipped.nodes.push((
        "other".to_string(),
        manifest("other", &["control/b"], &[]).into_bytes(),
    ));
    let m_flip = model(&flipped);
    assert_ne!(
        fp,
        node_fingerprint(&m_flip.nodes["controller"], &m_flip, ""),
        "flipped capability moves the node fingerprint"
    );

    // Edit another node's declarations: controller's fingerprint is unchanged.
    let mut other_edit = tree(&[("a", "x: u8\n"), ("b", "y: u8\n")]);
    other_edit.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/a"], &["control/b"]).into_bytes(),
    ));
    other_edit.nodes.push((
        "other".to_string(),
        manifest("other", &[], &["control/a"]).into_bytes(),
    ));
    let m_other = model(&other_edit);
    assert_eq!(
        fp,
        node_fingerprint(&m_other.nodes["controller"], &m_other, ""),
        "unrelated node edit does not move the node fingerprint"
    );
}

/// REQ: REQ-TOOL-025 — a removed declaration's fingerprint records are pruned
/// from the database (I-G5).
#[test]
fn gen_fingerprint_prune() {
    let root = scratch("prune");
    let mut db = FingerprintDb::default();
    let before = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
    generate(&model(&before), &root, &mut db);
    let state_key = FingerprintKey::new(GenKind::Topic, Lang::C, "control/state");
    assert!(db.get(&state_key).is_some(), "state recorded");

    let after = tree(&[("target", "a: u8\n")]);
    generate(&model(&after), &root, &mut db);
    assert!(
        db.get(&state_key).is_none(),
        "removed topic's fingerprint pruned"
    );
    let target_key = FingerprintKey::new(GenKind::Topic, Lang::C, "control/target");
    assert!(db.get(&target_key).is_some(), "retained topic kept");
    assert_eq!(db.len(), 4, "one retained topic × four languages");
    let _ = fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-065 — generated bindings embed canonical identities and
/// contracts, never runtime numeric handles (I-G9): a distinctive assigned
/// handle never appears in the rendered corpus, and handle assignment cannot
/// change the bytes.
#[test]
fn gen_bindings_handle_free() {
    let t = tree(&[("target", "a: u8\nb: u64\n")]);
    let m = model(&t);
    let root_a = scratch("handle-free-a");
    let mut db_a = FingerprintDb::default();
    generate(&m, &root_a, &mut db_a);

    let topics: BTreeSet<String> = m.topics.keys().cloned().collect();
    let history = AssignmentHistory {
        assignments: [("control/target".to_string(), 424_242_u32)]
            .into_iter()
            .collect(),
        reserved: BTreeMap::new(),
    };
    let (table, _) = HandleTable::from_history(&history)
        .assign(&topics)
        .unwrap_or_else(|e| unreachable!("handle assign: {e:?}"));
    assert_eq!(table.assignments["control/target"], 424_242, "handle set");

    let root_b = scratch("handle-free-b");
    let mut db_b = FingerprintDb::default();
    generate(&m, &root_b, &mut db_b);
    assert_eq!(
        generated_bytes(&root_a),
        generated_bytes(&root_b),
        "handle assignment does not change generated bytes"
    );
    for (path, bytes) in generated_bytes(&root_a) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("424242"),
            "{} embeds no runtime handle numeral",
            path.display()
        );
    }
    let _ = fs::remove_dir_all(&root_a);
    let _ = fs::remove_dir_all(&root_b);
}

/// REQ: REQ-TOOL-024 — atomic replacement overwrites an existing artifact
/// (I-G7: POSIX `rename`; Windows uses `ReplaceFile`/`MoveFileEx`) and leaves no
/// temporary sibling after a successful run.
#[test]
fn atomic_replace_overwrites_existing() {
    let root = scratch("replace");
    let mut db = FingerprintDb::default();
    generate(&model(&tree(&[("target", "a: u8\n")])), &root, &mut db);
    let path = out_path(&root, Lang::C, GenKind::Topic, "control/target");
    let first = fs::read(&path).unwrap_or_else(|e| unreachable!("read 1: {e}"));

    generate(
        &model(&tree(&[("target", "a: u8\nb: u64\n")])),
        &root,
        &mut db,
    );
    let second = fs::read(&path).unwrap_or_else(|e| unreachable!("read 2: {e}"));
    assert_ne!(first, second, "existing artifact replaced");
    assert!(!second.is_empty(), "replacement is complete");

    let parent = path.parent().unwrap_or_else(|| unreachable!("parent dir"));
    let leftovers: Vec<_> = fs::read_dir(parent)
        .unwrap_or_else(|e| unreachable!("read dir: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "no temp sibling survives: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// REQ: REQ-TOOL-024 — R6 hand-edit detection via regen-diff: a clean tree
/// reports no diff, a hand-edited artifact and a deleted artifact are both
/// reported.
#[test]
fn gen_check_detects_hand_edit() {
    let root = scratch("regen-diff");
    let m = model(&tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]));
    let mut db = FingerprintDb::default();
    generate(&m, &root, &mut db);

    let clean = regen_diff(&m, &root).unwrap_or_else(|e| unreachable!("diff: {e:?}"));
    assert!(clean.is_clean(), "fresh tree has no diff: {clean:?}");

    let edited = out_path(&root, Lang::C, GenKind::Topic, "control/target");
    let mut body = fs::read(&edited).unwrap_or_else(|e| unreachable!("read: {e}"));
    body.extend_from_slice(b"\n/* hand edited */\n");
    fs::write(&edited, &body).unwrap_or_else(|e| unreachable!("write: {e}"));

    let removed = out_path(&root, Lang::Rust, GenKind::Topic, "control/state");
    fs::remove_file(&removed).unwrap_or_else(|e| unreachable!("remove: {e}"));

    let diff = regen_diff(&m, &root).unwrap_or_else(|e| unreachable!("diff 2: {e:?}"));
    assert!(!diff.is_clean(), "hand-edit must be detected");
    assert!(
        diff.mismatched.iter().any(|p| p.ends_with("target.h")),
        "edited artifact reported: {diff:?}"
    );
    assert!(
        diff.missing.iter().any(|p| p.ends_with("target.rs")),
        "deleted artifact reported: {diff:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Child of the [`gen_atomicity`] kill harness: loops generation into the root
/// named by `DRM_GEN_ATOMICITY_ROOT` until killed. A no-op when the variable is
/// absent so an accidental `--ignored` run is harmless.
#[test]
#[ignore = "codegen kill harness child: spawned by gen_atomicity"]
fn gen_atomicity_child() {
    let Ok(root) = std::env::var("DRM_GEN_ATOMICITY_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let m = model(&tree(&[("target", &wide_source(96))]));
    let mut db = FingerprintDb::default();
    for _ in 0..1_000_000_u64 {
        execute(&plan(&m, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("child execute: {e:?}"));
    }
}

/// REQ: REQ-TOOL-024 — I-G4: killing generation mid-write ×20 leaves the prior
/// artifact intact; every committed artifact is a complete render, never a
/// truncated partial.
#[test]
fn gen_atomicity() {
    let exe = std::env::current_exe().unwrap_or_else(|e| unreachable!("current exe: {e}"));
    let root = scratch("atomicity");

    // Known-good small render, then a large render the child repeatedly writes.
    let small = model(&tree(&[("target", "a: u8\n")]));
    let mut db = FingerprintDb::default();
    generate(&small, &root, &mut db);
    let small_body = fs::read(out_path(&root, Lang::C, GenKind::Topic, "control/target"))
        .unwrap_or_else(|e| unreachable!("read small: {e}"));

    let big = model(&tree(&[("target", &wide_source(96))]));
    let big_body = {
        let scratch_root = scratch("atomicity-big");
        let mut big_db = FingerprintDb::default();
        generate(&big, &scratch_root, &mut big_db);
        let bytes = fs::read(out_path(
            &scratch_root,
            Lang::C,
            GenKind::Topic,
            "control/target",
        ))
        .unwrap_or_else(|e| unreachable!("read big: {e}"));
        let _ = fs::remove_dir_all(&scratch_root);
        bytes
    };

    for round in 0..20_u32 {
        let mut child = Command::new(&exe)
            .args(["--ignored", "--exact", "gen_atomicity_child", "--nocapture"])
            .env("DRM_GEN_ATOMICITY_ROOT", &root)
            .spawn()
            .unwrap_or_else(|e| unreachable!("spawn child: {e}"));
        std::thread::sleep(Duration::from_millis(5 + u64::from(round % 7) * 3));
        let _ = child.kill();
        let _ = child.wait();

        let target = out_path(&root, Lang::C, GenKind::Topic, "control/target");
        let got = fs::read(&target).unwrap_or_else(|e| unreachable!("read after kill: {e}"));
        assert!(
            got == small_body || got == big_body,
            "round {round}: artifact must be a complete render (got {} bytes)",
            got.len()
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// ADR [0008](../../docs/adr/0008-generator-identity-in-the-codegen-cache-key.md)
/// / REQ: REQ-TOOL-027 — the generator identity participates in every unit's cache
/// key: a changed identity (i.e. an edited renderer) marks the affected artifacts
/// dirty, while re-planning under the same identity stays cached.
#[test]
fn gen_generator_identity_invalidates_and_caches() {
    let mut t = tree(&[("target", "a: u8\nb: u64\n")]);
    t.nodes.push((
        "controller".to_string(),
        manifest("controller", &["control/target"], &[]).into_bytes(),
    ));
    let m = model(&t);
    let root = scratch("genid");

    // Two stand-ins for distinct renderer revisions.
    let mine = [0x11_u8; 16];
    let theirs = [0x22_u8; 16];

    // Build once, then re-plan under the same identity: fully cached.
    let mut db = FingerprintDb::default();
    execute(
        &plan_with_generator(&m, &db, &root, "", mine),
        &mut db,
        &root,
    )
    .unwrap_or_else(|e| unreachable!("execute: {e:?}"));
    let same = plan_with_generator(&m, &db, &root, "", mine);
    assert!(
        same.dirty_units.is_empty(),
        "an unchanged generator must stay cached, got {} dirty unit(s)",
        same.dirty_units.len()
    );

    // A different generator identity invalidates the affected units.
    let other = plan_with_generator(&m, &db, &root, "", theirs);
    assert!(
        other
            .dirty_units
            .iter()
            .any(|u| u.kind == GenKind::Topic && u.key.identity == "control/target"),
        "renderer change must dirty the topic unit"
    );
    assert!(
        other
            .dirty_units
            .iter()
            .any(|u| u.kind == GenKind::NodeApi && u.key.identity == "controller"),
        "renderer change must dirty the node API unit"
    );

    // Both fingerprint kinds really do fold the identity in.
    assert_ne!(
        topic_fingerprint_with_generator("control/target", &m.topics["control/target"], "", mine),
        topic_fingerprint_with_generator("control/target", &m.topics["control/target"], "", theirs),
        "topic fingerprint must depend on the generator identity"
    );
    assert_ne!(
        node_fingerprint_with_generator(&m.nodes["controller"], &m, "", mine),
        node_fingerprint_with_generator(&m.nodes["controller"], &m, "", theirs),
        "node fingerprint must depend on the generator identity"
    );

    // The shipped identity is the renderer's own digest, not a placeholder.
    assert_ne!(generator_identity(), [0_u8; 16]);

    let _ = fs::remove_dir_all(&root);
}
