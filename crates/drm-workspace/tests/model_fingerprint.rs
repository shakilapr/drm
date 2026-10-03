//! Contract fingerprint tests (work-plan M02W3-E2-T3): pinned serialization,
//! determinism, and isolation — an unrelated schema edit cannot move an
//! unaffected node's fingerprint (I-D4). REQ: REQ-TOOL-023, REQ-TOOL-013.

use drm_types::{ABI_VERSION, blake3_128};
use drm_workspace::model::{
    ContractFingerprint, FINGERPRINT_FORMAT_VERSION, PackageInput, TreeInput, build,
    contract_fingerprint, layout_hash,
};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

fn topic_source(n: u32) -> String {
    format!("v: u32\nk: u{}\n", if n.is_multiple_of(2) { 8 } else { 16 })
}

type NodeSpec<'a> = (&'a str, &'a str, &'a [&'a str], &'a [&'a str]);

fn tree(nodes: &[NodeSpec<'_>], front: Option<&str>, telemetry: Option<&str>) -> TreeInput {
    let mut topics = vec![
        ("front".to_string(), topic_source(2).into_bytes()),
        ("telemetry".to_string(), topic_source(3).into_bytes()),
        ("cam".to_string(), topic_source(4).into_bytes()),
    ];
    if let Some(front) = front {
        topics[0] = ("front".to_string(), front.as_bytes().to_vec());
    }
    if let Some(telemetry) = telemetry {
        topics[1] = ("telemetry".to_string(), telemetry.as_bytes().to_vec());
    }
    TreeInput {
        packages: vec![PackageInput {
            name: "pkg".to_string(),
            topics,
        }],
        nodes: nodes
            .iter()
            .map(|(stem, name, pubs, cons)| {
                let list = |v: &[&str]| {
                    v.iter()
                        .map(|t| format!("\"{t}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let s = format!(
                    "name = \"{name}\"\npublishes = [{}]\nconsumes = [{}]\n",
                    list(pubs),
                    list(cons),
                );
                ((*stem).to_string(), s.into_bytes())
            })
            .collect(),
    }
}

/// REQ: REQ-TOOL-023 — serialization is pinned: the fingerprint equals
/// BLAKE3-128 over the exact documented byte sequence (domain tag, format
/// version, ABI version, count, per-interface direction/len/path/schema
/// hash/layout hash).
#[test]
fn fingerprint_serialization_is_pinned() {
    let m = fixture(
        build(&tree(
            &[("ctl", "ctl", &[], &["pkg/telemetry"])],
            None,
            None,
        )),
        "b",
    );
    let got = contract_fingerprint(&m.nodes["ctl"], &m);

    let mut expect = Vec::new();
    expect.extend_from_slice(b"DRM-CONTRACT\0");
    expect.extend_from_slice(&FINGERPRINT_FORMAT_VERSION.to_le_bytes());
    expect.extend_from_slice(&ABI_VERSION.to_le_bytes());
    expect.extend_from_slice(&1u32.to_le_bytes());
    expect.push(1); // direction: consume
    expect.extend_from_slice(&13u32.to_le_bytes());
    expect.extend_from_slice(b"pkg/telemetry");
    expect.extend_from_slice(&m.topics["pkg/telemetry"].schema_hash);
    expect.extend_from_slice(&layout_hash(&m.topics["pkg/telemetry"].layout));
    assert_eq!(got, ContractFingerprint(blake3_128(&expect)));
}

/// REQ: REQ-TOOL-023 — editing an unrelated topic leaves the fingerprint
/// unchanged; editing a topic the node uses changes it (I-D4).
#[test]
fn fingerprint_isolation() {
    let m1 = fixture(
        build(&tree(
            &[("motor", "motor", &[], &["pkg/telemetry"])],
            Some("v: u32\n"),
            None,
        )),
        "b1",
    );
    let m2 = fixture(
        build(&tree(
            &[("motor", "motor", &[], &["pkg/telemetry"])],
            Some("v: u32\nextra: u8\n"),
            None,
        )),
        "b2",
    );
    assert_eq!(
        contract_fingerprint(&m1.nodes["motor"], &m1),
        contract_fingerprint(&m2.nodes["motor"], &m2),
        "unrelated front edit must not move motor"
    );

    let m3 = fixture(
        build(&tree(
            &[("motor", "motor", &[], &["pkg/telemetry"])],
            None,
            Some("v: u32\nk: u16\n"),
        )),
        "b3",
    );
    let m4 = fixture(
        build(&tree(
            &[("motor", "motor", &[], &["pkg/telemetry"])],
            None,
            Some("v: u32\nk: u16\nmore: u8\n"),
        )),
        "b4",
    );
    assert_ne!(
        contract_fingerprint(&m3.nodes["motor"], &m3),
        contract_fingerprint(&m4.nodes["motor"], &m4),
        "a used topic edit must move the fingerprint"
    );
}

/// REQ: REQ-TOOL-023 — direction participates: publish-only differs from
/// consume-only over the same topic.
#[test]
fn fingerprint_covers_direction() {
    let m = fixture(
        build(&tree(
            &[
                ("pub", "pub", &["pkg/front"], &[]),
                ("con", "con", &[], &["pkg/front"]),
            ],
            None,
            None,
        )),
        "b",
    );
    assert_ne!(
        contract_fingerprint(&m.nodes["pub"], &m),
        contract_fingerprint(&m.nodes["con"], &m),
        "direction is an input"
    );
}

/// REQ: REQ-TOOL-023 — the layout hash equals BLAKE3-128 over the pinned
/// serialization and differs for differently-packed layouts.
#[test]
fn layout_hash_pinned_and_deterministic() {
    let m = fixture(
        build(&tree(&[("x", "x", &[], &["pkg/front"])], None, None)),
        "b",
    );
    let layout = &m.topics["pkg/front"].layout;

    let mut expect = Vec::new();
    expect.extend_from_slice(b"DRM-LAYOUT\0");
    expect.extend_from_slice(&layout.header_end.to_le_bytes());
    expect.extend_from_slice(&layout.sample_max.to_le_bytes());
    expect.extend_from_slice(&layout.struct_align.to_le_bytes());
    expect.extend_from_slice(
        &u32::try_from(layout.members.len())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    for mem in &layout.members {
        expect.push(match mem.region {
            drm_workspace::canon::Region::Header => 0u8,
            drm_workspace::canon::Region::Descriptor => 1,
            drm_workspace::canon::Region::Body => 2,
        });
        expect.extend_from_slice(&u32::try_from(mem.name.len()).unwrap_or(0).to_le_bytes());
        expect.extend_from_slice(mem.name.as_bytes());
        expect.extend_from_slice(&mem.offset.to_le_bytes());
        expect.extend_from_slice(&mem.size.to_le_bytes());
        expect.extend_from_slice(&mem.align.to_le_bytes());
    }
    assert_eq!(layout_hash(layout), blake3_128(&expect));

    // Same primitive set, different declaration order ⇒ different packing.
    let m2 = fixture(
        build(&tree(
            &[("x", "x", &[], &["pkg/front"])],
            Some("w: u32\nv: u32\n"),
            None,
        )),
        "b2",
    );
    assert_ne!(
        layout_hash(&m.topics["pkg/front"].layout),
        layout_hash(&m2.topics["pkg/front"].layout),
    );
}

/// REQ: REQ-TOOL-023 — repeated construction yields identical fingerprints.
#[test]
fn fingerprint_deterministic() {
    let t = tree(
        &[("ctl", "ctl", &["pkg/front"], &["pkg/telemetry"])],
        None,
        None,
    );
    let m1 = fixture(build(&t), "b1");
    let m2 = fixture(build(&t), "b2");
    assert_eq!(
        contract_fingerprint(&m1.nodes["ctl"], &m1),
        contract_fingerprint(&m2.nodes["ctl"], &m2)
    );
}

/// REQ: REQ-TOOL-023 — a node with no interfaces hashes just the header.
#[test]
fn fingerprint_empty_node() {
    let m = fixture(
        build(&tree(&[("empty", "empty", &[], &[])], None, None)),
        "b",
    );
    let mut expect = Vec::new();
    expect.extend_from_slice(b"DRM-CONTRACT\0");
    expect.extend_from_slice(&FINGERPRINT_FORMAT_VERSION.to_le_bytes());
    expect.extend_from_slice(&ABI_VERSION.to_le_bytes());
    expect.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        contract_fingerprint(&m.nodes["empty"], &m),
        ContractFingerprint(blake3_128(&expect))
    );
}
