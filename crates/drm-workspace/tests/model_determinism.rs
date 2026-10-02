//! `WorkspaceModel` determinism + conflict-collection tests (workspace-model.md
//! I-D1, I-D2; work-plan M02W1-E2-T3). REQ: REQ-TOOL-045, REQ-TOOL-047.

use drm_workspace::canon;
use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build, render_model};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// Mixed-prims topic source with an attribute (exercises parser + layout).
fn topic_source(seq: u8) -> String {
    format!(
        "@history: {seq}\n\
         seq: u8\n\
         count: u16\n\
         window: u32[4]\n\
         gain: f32\n\
         phase: f64\n\
         armed: bool\n"
    )
}

/// A bounded-variety second topic (string tail + enum member).
fn topic_source_tail() -> String {
    "id: u32\n\
     label: string[16]\n\
     e: enum {off, on}\n"
        .to_string()
}

/// Reference tree: 3 packages, 2–3 topics each. REQ: REQ-TOOL-045.
fn fixture_tree() -> TreeInput {
    TreeInput {
        packages: vec![
            PackageInput {
                name: "alpha".to_string(),
                topics: vec![
                    ("front".to_string(), topic_source(100).into_bytes()),
                    ("telemetry".to_string(), topic_source_tail().into_bytes()),
                ],
            },
            PackageInput {
                name: "beta".to_string(),
                topics: vec![("back".to_string(), topic_source(50).into_bytes())],
            },
            PackageInput {
                name: "gamma".to_string(),
                topics: vec![
                    ("cam".to_string(), topic_source_tail().into_bytes()),
                    ("pose".to_string(), topic_source(200).into_bytes()),
                    ("cmd".to_string(), "opcode: u8\n".to_string().into_bytes()),
                ],
            },
        ],
    }
}

/// xorshift64* — dependency-free deterministic shuffle for the seed loop.
struct Xs {
    state: u64,
}

impl Xs {
    fn next(&mut self) -> u64 {
        let x = self.state;
        self.state = (x ^ (x << 13)) ^ (x >> 7);
        let y = self.state;
        self.state = (y ^ (y << 17)).wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.state
    }

    fn shuffle<T>(&mut self, xs: &mut [T]) {
        let n = xs.len();
        for i in (1..n).rev() {
            let bound = u64::try_from(i + 1).unwrap_or(u64::MAX);
            let j = usize::try_from(self.next() % bound).unwrap_or(usize::MAX);
            xs.swap(i, j);
        }
    }
}

/// REQ: REQ-TOOL-045 — reference build from the canonical-order tree.
#[test]
fn model_baseline() {
    let tree = fixture_tree();
    let m = fixture(build(&tree), "baseline build");
    assert_eq!(m.topics.len(), 6, "all topics parsed");
    assert_eq!(m.packages.len(), 3, "all packages discovered");
    let text = render_model(&m);
    assert!(text.starts_with("package alpha"), "sorted render: {text}");
    assert!(text.contains("topic alpha/front"), "topic line present");
    // Layout sanity: every topic got a two-region layout with slot 0 at 0.
    for (path, entry) in &m.topics {
        assert!(entry.layout.sample_max > 0, "{path}: nonzero sample size");
        let _ = canon::layout_struct(&entry.schema.fields);
    }
}

/// REQ: REQ-TOOL-045 — I-D1: 100 shuffled-fs-order seeds build a
/// byte-identical model render.
#[test]
fn model_determinism() {
    let tree = fixture_tree();
    let baseline = fixture(build(&tree), "baseline build");
    let text = render_model(&baseline);
    for seed in 0..100_u64 {
        let mut shuffled = fixture_tree();
        let mut rng = Xs {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        };
        rng.shuffle(&mut shuffled.packages);
        for pkg in &mut shuffled.packages {
            rng.shuffle(&mut pkg.topics);
        }
        let m = fixture(build(&shuffled), "shuffled build");
        assert_eq!(
            render_model(&m),
            text,
            "seed {seed}: render must be byte-identical"
        );
        assert_eq!(m, baseline, "seed {seed}: model must be equal");
    }
}

/// REQ: REQ-TOOL-045 — I-D2: a defect topic fails the build with the failing
/// path named; valid siblings are irrelevant (collect-then-report).
#[test]
fn model_conflicts() {
    let mut tree = fixture_tree();
    tree.packages.push(PackageInput {
        name: "delta".to_string(),
        topics: vec![
            ("broken".to_string(), b"a: u8\nmissing: string\n".to_vec()),
            ("fine".to_string(), b"ok: u8\n".to_vec()),
        ],
    });
    let errs = build(&tree);
    match errs {
        Err(list) => {
            assert_eq!(list.len(), 1, "one defect topic: {list:?}");
            match &list[0] {
                drm_workspace::model::ModelError::Parse { topic, .. } => {
                    assert_eq!(topic, "delta/broken");
                }
                other @ drm_workspace::model::ModelError::DuplicateTopic { .. } => {
                    unreachable!("expected Parse, got {other:?}")
                }
            }
        }
        Ok(_) => unreachable!("broken topic must fail the build"),
    }
}

/// REQ: REQ-TOOL-023 — schema hash: content-addressed, not order-addressed.
#[test]
fn model_schema_hash() {
    let tree = fixture_tree();
    let m: WorkspaceModel = fixture(build(&tree), "build");
    let front = &m.topics["alpha/front"];
    let telemetry = &m.topics["alpha/telemetry"];
    assert_ne!(front.schema_hash, telemetry.schema_hash, "content differs");
    // Same content in a different package ⇒ same hash (identity is content).
    let mut tree2 = fixture_tree();
    tree2.packages.push(PackageInput {
        name: "alpha_copy".to_string(),
        topics: vec![("front".to_string(), topic_source(100).into_bytes())],
    });
    let m2 = fixture(build(&tree2), "build2");
    let copy = &m2.topics["alpha_copy/front"];
    assert_eq!(
        front.schema_hash, copy.schema_hash,
        "same content, same hash"
    );
}
