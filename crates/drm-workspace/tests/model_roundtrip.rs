//! Model roundtrip (work-plan M02W3-E4-T2, I-D5): a generated tree builds a
//! model, the model reconstructs an equivalent tree, rebuilding yields the
//! identical model render — 5 000 deterministic cases; also asserts
//! shuffled-input-order determinism per case.
//! REQ: REQ-TOOL-045, REQ-TOOL-047.

use std::fmt::Write as _;

use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build, render_model};
use drm_workspace::parser::{Prim, render_canonical};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// Deterministic LCG (no external deps; flake policy 4.2 R4).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick_index(&mut self, n: usize) -> usize {
        usize::try_from(self.below(u64::try_from(n).unwrap_or(1))).unwrap_or(0)
    }
}

fn prim(i: u64) -> Prim {
    [
        Prim::U8,
        Prim::U16,
        Prim::U32,
        Prim::U64,
        Prim::I32,
        Prim::F32,
        Prim::Bool,
    ][(i % 7) as usize]
}

fn gen_tree(rng: &mut Rng) -> TreeInput {
    let pkgs = 1 + rng.below(3);
    let mut packages = Vec::new();
    let mut all_topics: Vec<String> = Vec::new();
    for p in 0..pkgs {
        let n_topics = 2 + rng.below(4);
        let mut topics = Vec::new();
        for t in 0..n_topics {
            let stem = format!("t{p}_{t}");
            let fields = 1 + rng.below(3);
            let mut src = String::new();
            for f in 0..fields {
                let ty = match rng.below(8) {
                    0 => format!("u{}", 8 << rng.below(4)),
                    1 => format!("i{}", 8 << rng.below(3)),
                    2 => "bool".to_string(),
                    3 => "f32".to_string(),
                    4 => "f64".to_string(),
                    5 => format!("string[{}]", 1 + rng.below(64)),
                    6 => format!("bytes[<={}]", 1 + rng.below(64)),
                    _ => format!("enum {{ a, b, c{f} }}"),
                };
                let _ = writeln!(src, "f{f}: {ty}");
            }
            let _ = prim(rng.next());
            topics.push((stem.clone(), src.into_bytes()));
            all_topics.push(format!("p{p}/{stem}"));
        }
        packages.push(PackageInput {
            name: format!("p{p}"),
            topics,
        });
    }
    let n_nodes = rng.below(5);
    let mut nodes = Vec::new();
    let mut node_names: Vec<String> = Vec::new();
    for n in 0..n_nodes {
        let name = format!("n{n}");
        node_names.push(name.clone());
        let mut publishes = Vec::new();
        let mut consumes = Vec::new();
        if !all_topics.is_empty() && rng.next().is_multiple_of(2) {
            let t = &all_topics[rng.pick_index(all_topics.len())];
            publishes.push(t.clone());
        }
        if !all_topics.is_empty() && rng.next().is_multiple_of(2) {
            let t = &all_topics[rng.pick_index(all_topics.len())];
            consumes.push(t.clone());
        }
        let list = |v: &[String]| {
            v.iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let manifest = format!(
            "name = \"{name}\"\npublishes = [{}]\nconsumes = [{}]\n",
            list(&publishes),
            list(&consumes),
        );
        nodes.push((name, manifest.into_bytes()));
    }
    let topology = if rng.next().is_multiple_of(4) {
        None
    } else {
        Some(gen_topology(rng, &node_names).into_bytes())
    };
    TreeInput {
        packages,
        nodes,
        topology,
    }
}

/// Generates a valid topology manifest: 1–3 computes, links over distinct
/// compute pairs, and placement of a random subset of the given nodes.
fn gen_topology(rng: &mut Rng, node_names: &[String]) -> String {
    let mut src = String::new();
    src.push_str("[computes]\n");
    let n_computes = rng.pick_index(3) + 1;
    let computes: Vec<String> = (0..n_computes).map(|c| format!("c{c}")).collect();
    for c in &computes {
        let _ = writeln!(src, "{c} = true");
    }
    let mut link_id = 0;
    for a in 0..n_computes {
        for b in (a + 1)..n_computes {
            if rng.next().is_multiple_of(2) {
                let _ = writeln!(
                    src,
                    "\n[[links]]\nid = \"l{link_id}\"\na = \"{}\"\nb = \"{}\"",
                    computes[a], computes[b]
                );
                link_id += 1;
            }
        }
    }
    let mut placed = false;
    for name in node_names {
        if rng.next().is_multiple_of(2) {
            if !placed {
                src.push_str("\n[placement]\n");
                placed = true;
            }
            let c = &computes[rng.pick_index(n_computes)];
            let _ = writeln!(src, "{name} = \"{c}\"");
        }
    }
    src
}

/// Reconstructs the topology manifest from the model (same closed surface as
/// the generator; `computes` → `true`, links in sorted order, placement).
fn reconstruct_topology(m: &WorkspaceModel) -> String {
    let mut src = String::new();
    src.push_str("[computes]\n");
    for name in m.computes.keys() {
        let _ = writeln!(src, "{name} = true");
    }
    for link in &m.links {
        let _ = writeln!(
            src,
            "\n[[links]]\nid = \"{}\"\na = \"{}\"\nb = \"{}\"",
            link.id, link.a, link.b
        );
    }
    if !m.placement.is_empty() {
        src.push_str("\n[placement]\n");
        for (node, compute) in &m.placement {
            let _ = writeln!(src, "{node} = \"{compute}\"");
        }
    }
    src
}

/// REQ: REQ-TOOL-045 — model → equivalent tree → identical model render and
/// identical model, across 5 000 generated trees and both input orders.
#[test]
fn model_roundtrip_5000() {
    let mut rng = Rng(0x243F_6A88_85A3_08D3);
    for case in 0..5_000u32 {
        let tree = gen_tree(&mut rng);
        let m1: WorkspaceModel = fixture(build(&tree), "m1");
        let rendered = render_model(&m1);

        // Reconstruct the tree from the model (canonical renders + manifests).
        let mut packages = Vec::new();
        for (pname, meta) in &m1.packages {
            let topics = meta
                .topics
                .iter()
                .map(|path| {
                    let stem = path.split('/').nth(1).unwrap_or(path.as_str());
                    let entry = &m1.topics[path];
                    (
                        stem.to_string(),
                        render_canonical(&entry.schema).into_bytes(),
                    )
                })
                .collect();
            packages.push(PackageInput {
                name: pname.clone(),
                topics,
            });
        }
        let nodes = m1
            .nodes
            .iter()
            .map(|(name, node)| {
                let list = |v: &std::collections::BTreeSet<String>| {
                    v.iter()
                        .map(|t| format!("\"{t}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                (
                    name.clone(),
                    format!(
                        "name = \"{name}\"\npublishes = [{}]\nconsumes = [{}]\n",
                        list(&node.publishes),
                        list(&node.consumes),
                    )
                    .into_bytes(),
                )
            })
            .collect();
        let topology = if m1.computes.is_empty() && m1.links.is_empty() && m1.placement.is_empty() {
            None
        } else {
            Some(reconstruct_topology(&m1).into_bytes())
        };
        let tree2 = TreeInput {
            packages,
            nodes,
            topology,
        };

        let m2 = fixture(build(&tree2), "m2");
        assert_eq!(
            render_model(&m2),
            rendered,
            "case {case}: roundtrip render mismatch"
        );
        assert_eq!(m1, m2, "case {case}: roundtrip model mismatch");

        // Shuffled input order ⇒ byte-identical render (I-D1).
        let mut reversed = tree.packages.clone();
        reversed.reverse();
        let mut rnodes = tree.nodes.clone();
        rnodes.reverse();
        let tree3 = TreeInput {
            packages: reversed,
            nodes: rnodes,
            topology: tree.topology.clone(),
        };
        let m3 = fixture(build(&tree3), "m3");
        assert_eq!(render_model(&m3), rendered, "case {case}: order leaked");
    }
}
