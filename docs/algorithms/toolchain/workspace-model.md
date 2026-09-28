# Workspace Model Algorithms

Scope: construction of the in-memory `WorkspaceModel` from manifests and `.drm` files — topics, nodes, packages, compute units, links, placement — plus contract validation, contract fingerprints, and deterministic reproducibility. Implements architecture.md §In-Memory Workspace Model, §Primary Domain Relationships, §Node Communication Contracts. Owner: E2 (crate `drm-workspace::model`). Phase 23.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-model-relationships` | §Primary Domain Relationships — publishes/consumes/placed_on/links | `REQ-TOOL-047` |
| `REQ-TOOL-model-determinism` | §In-Memory — parse whole model in ms; deterministic | `REQ-TOOL-045` |
| `REQ-TOOL-model-external-endpoints` | §External Endpoints — third-party devices are graph vertices | `REQ-TOOL-046` |
| `REQ-TOOL-model-timing-metadata` | publisher `expected_rate`, consumer `deadline` carried in the model | `REQ-TOOL-049` |
| `REQ-TOOL-model-routing-overrides` | `[routing]` prefer/deny/force carried in the model | `REQ-TOOL-048` |
| `REQ-TOOL-contract-single-source` | §Node Communication Contracts — manifest is single source of truth | `REQ-TOOL-013` |
| `REQ-TOOL-fingerprint-isolation` | §Node Interface Identity — unrelated schema edit ⇒ unaffected fingerprint unchanged | `REQ-TOOL-023` |
| `REQ-TOOL-no-build-cycles` | communication loops never create build cycles | `REQ-TOOL-050` |

## Data Structures

```rust
pub struct WorkspaceModel {
 topics: BTreeMap<TopicName, TopicEntry>, // canonical path → definition (schema hash, layout, attrs)
 nodes: BTreeMap<NodeName, NodeEntry>,
 packages: BTreeMap<PackageName, PackageMeta>,
 computes: BTreeMap<ComputeName, ComputeUnit>, // nodes placed here
 links: Vec<Link>, // undirected compute↔compute, transport, bandwidth
 placement: BTreeMap<NodeName, ComputeName>,
 imports: SortedMap<TopicName, PathBuf>, // schema source files (traceability)
 externals: BTreeMap<ExternalName, ExternalEndpoint>, // third-party devices as graph vertices
 routing: RouteOverrides, // [routing] prefer/deny/force
 spatial: SpatialModel, // frames, edges, indexes — parsed from spaces/ + [spatial] contracts (Phase 33)
}

pub struct SpatialModel {
 frames: BTreeMap<FrameId, FrameEntry>, // identity = filesystem path (spaces/**.drm)
 edges: BTreeMap<FrameId, EdgeKind>, // Dynamic{writer: NodeName} | Static{transform: Transform3}
 indexes: BTreeMap<IndexName, IndexDecl>, // index.drm: anchor, cell_size, max_objects
}
// BTree*/Sorted* everywhere ⇒ filesystem order can never leak into model output (I-D1).

pub struct NodeEntry {
 publishes: SortedSet<TopicName>, consumes: SortedSet<TopicName>,
 publish_meta: SortedMap<TopicName, PublishMeta>, // expected_rate (planning only)
 consume_meta: SortedMap<TopicName, ConsumeMeta>, // deadline (freshness)
 spatial: SpatialContract, // [spatial] write = [frames], read = [frames] (spatial-core §Node Spatial Contracts)
 language: Language, build: BuildAdapter,
}
pub struct PublishMeta { expected_rate: Option<String> }
pub struct ConsumeMeta { deadline: Option<String> }

pub struct ExternalEndpoint { link: LinkId, consume: SortedSet<TopicName>, produce: SortedSet<TopicName> }
pub struct RouteOverrides {
 prefer: SortedMap<TopicName, LinkId>,
 deny: SortedMap<TopicName, SortedSet<LinkId>>,
 force: SortedMap<TopicName, Vec<LinkId>>,
}

pub struct ContractFingerprint([u8; 16]); // BLAKE3 output truncated to 128 bits (see contract_fingerprint())
                                          // Width parity: schema_hash/layout_hash are [u8; 16] too
                                          // (channel-metadata.md); a freak fingerprint collision is
                                          // still caught by per-segment schema/layout validation at attach.
```

Model construction is **total-reload**: every parse rebuilds the full model (no incremental caches — architecture §In-Memory Workspace Model).

## Invariants

- **I-D1 Deterministic**: same input tree (any read order) ⇒ byte-identical model render. Test: shuffled-fs-order ×100 seeds.
- **I-D2 ConflictRejection**: duplicate topic/node/package identity, unknown topic in publish/consume, dangling placement, unknown link endpoint ⇒ single `Err` with location. Test: `model_conflicts`.
- **I-D3 LoopsHarmless**: A publishes B-consumes, B publishes A-consumes ⇒ model builds fine (no build-cycle error). Test: `communication_loop`.
- **I-D4 FingerprintIsolation**: edit topic `camera/front` ⇒ `controller` fingerprint unchanged; edit a topic that `controller` itself uses ⇒ changed. Test: `fingerprint_isolation`.
- **I-D5 ReproducibleModel**: `render_model(parse(tree))` round-trips byte-identically (feeds property test).
- **I-D6 SpatialInference**: static/dynamic edge classification is deterministic — node declares `[spatial] write` ⇒ dynamic; `transform` given + no writer ⇒ static; both ⇒ conflict error; neither ⇒ dangling-frame error (spatial-core I-S10). Test: `spatial_inference`.
- **I-D7 SpatialSingleWriter**: two nodes writing one dynamic spatial edge ⇒ compiler conflict before deployment (spatial-core I-S11). Test: `spatial_single_writer`.
- **I-D8 ExternalVertices**: `[external.*]` endpoints with `link` + `consume`/`produce` are first-class graph vertices; unknown link or topic ⇒ error. Test: `model_external`.
- **I-D9 TimingOverrides**: publisher `expected_rate`, consumer `deadline`, and `[routing]` overrides are parsed into the model and preserved by render. Test: `model_timing_overrides`.

## Functions

```rust
fn build(tree: &FileTree) -> Result<WorkspaceModel, Vec<ModelError>> {
 let mut m = WorkspaceModel::default();
 for pkg in discover_packages_sorted(tree) { // sorted discovery (I-D1)
 parse_package_manifest(pkg)?; // duplicates ⇒ conflict error (I-D2)
 for topic_file in pkg.topics_sorted() {
 let t = schema_parser::parse(topic_file)?; // schema-parser.md
 let layout = canon::layout_struct(&t.fields); // canon-codec.md
 m.topics.insert(topic_name(pkg, file_stem), TopicEntry { schema_hash: hash(t), layout, .. });
 }
 }
 for node_manifest in discover_nodes_sorted(tree) {
 let n = parse_node_manifest(node_manifest)?; // publish/consume/language/build
 check_contracts(&mut m, &n)?; // unknown topic ⇒ error; single-writer precheck (verify-lite)
 m.nodes.insert(n.name, n);
 }
 parse_topology(tree)?.apply(&mut m)?; // computes, links, placement; dangling refs ⇒ error
 parse_spaces(tree)?.apply(&mut m.spatial)?; // spaces/**.drm frames; static/dynamic inference (I-D6);
 // [spatial] write/read contract wiring; single-writer spatial edge check (I-D7);
 // index.drm declarations (anchor, cell_size, max_objects) — capacity feeds memory budget (I-MB9)
 parse_externals(tree)?.apply(&mut m)?; // [external.*] vertices; unknown link/topic ⇒ error (I-D8)
 parse_routing(tree)?.apply(&mut m)?; // [routing] prefer/deny/force; unknown ⇒ error (I-D9)
 Ok(m)
}

fn contract_fingerprint(node: &NodeEntry, m: &WorkspaceModel) -> ContractFingerprint {
 // EXACTLY the inputs fixed by architecture §Node Interface Identity, over a PINNED canonical
 // serialization (interop-reproducible; the hash is defined over these bytes and nothing else):
 //
 //   b"DRM-CONTRACT\0"                domain tag
 //   u32 LE  FINGERPRINT_FORMAT_VERSION
 //   u32 LE  ABI_VERSION
 //   u32 LE  interface_count
 //   per interface, sorted by canonical topic path (map iteration order can never leak in):
 //     u8      direction (0 = publish, 1 = consume)
 //     u32 LE  topic_path_len; topic_path bytes (UTF-8)
 //     [16]u8  canonical layout hash (canon-codec.md)
 //
 // Hash: BLAKE3 over exactly these bytes, truncated to 16 bytes ([u8; 16]).
 // Deliberately EXCLUDED: other nodes, packages, topology, links, build config.
 // ⇒ unrelated schema edits cannot change unaffected nodes (I-D4).
}
```

## Protocols

*Model render → reparse round-trip (I-D5):*

1. `text = render_model(m)` (canonical TOML ordering: topics, nodes, computes, links).
2. `m2 = build(parse_tree(text))`.
3. Assert `render_model(m2) == text`.

*Consumer of fingerprints:* `deployment-identity.md` embeds `contract_fingerprint` into deployment startup guard and per-segment validation.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two packages define same topic path | conflict error listing both files |
| Node consumes undefined topic | error with manifest location |
| Node placed on unknown compute | error |
| Link references unknown compute | error |
| Empty workspace (no topics) | valid, empty model (tools still run) |
| Manifest TOML syntax error | schema-parser-style diagnostic with location |

## Test Matrix

| Case | Expected |
|---|---|
| 3-fixture workspace tree | model matches golden render |
| shuffled fs order ×100 | byte-identical model |
| conflict fixtures (dup topic, dup node, dangling placement, unknown link) | rejected with location |
| communication-loop fixture | builds, no cycle error |
| fingerprint isolation | I-D4 |
| fingerprint: shuffled manifest order / map iteration | identical bytes (canonical serialization, sorted inputs) |
| fingerprint: used schema modified / direction changed / ABI bumped / layout changed | different bytes |
| deployment.json `deployment_id` field | UUID/hex string, never a JSON number (128 bits > JSON-safe range) |
| external endpoint fixtures | I-D8 (unknown link/topic rejected) |
| timing/route override fixtures | I-D9 |
| round-trip ×5 000 | I-D5 |
| spatial inference matrix (write-only / transform-only / both / neither) | I-D6 |
| two-writers-one-edge fixture | I-D7 conflict |
