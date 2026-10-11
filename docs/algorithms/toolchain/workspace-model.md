# Workspace Model Algorithms

Normative terminology follows [docs/standards.md](../../standards.md).
Scope: construction of the in-memory `WorkspaceModel` from manifests and `.drm` files — topics, nodes, packages, compute units, links, placement — plus normalized planning metadata (typed rates, contract metadata), contract validation, contract fingerprints, and deterministic reproducibility. Layering: model construction enforces **structural/referential integrity only**; architectural policy (single-writer, routing, reliability, budgets, compatibility) belongs to `drm verify` (verify-engine.md). Implements architecture.md §In-Memory Workspace Model, §Primary Domain Relationships, §Node Communication Contracts. Owner: E2 (crate `drm-workspace::model`). Phase 23.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-model-relationships` | §Primary Domain Relationships — publishes/consumes/placed_on/links | `REQ-TOOL-047` |
| `REQ-TOOL-model-determinism` | §In-Memory — parse whole model in ms; deterministic | `REQ-TOOL-045` |
| `REQ-TOOL-model-external-endpoints` | §External Endpoints — third-party devices are graph vertices | `REQ-TOOL-046` |
| `REQ-TOOL-model-timing-metadata` | publisher `expected_rate`, consumer `deadline` carried in the model | `REQ-TOOL-049` |
| `REQ-TOOL-model-timing-mode` | topic `timing = auto/exact/off` parsed into the model; `auto` resolved from consumer deadlines/jitter diagnostics | `REQ-OPS-016` |
| `REQ-TOOL-model-routing-overrides` | `[routing]` prefer/deny/force/export_limit carried in the model | `REQ-TOOL-048` |
| `REQ-TOOL-contract-single-source` | §Node Communication Contracts — manifest is single source of truth | `REQ-TOOL-013` |
| `REQ-TOOL-fingerprint-isolation` | §Node Interface Identity — unrelated schema edit ⇒ unaffected fingerprint unchanged | `REQ-TOOL-023` |
| `REQ-TOOL-no-build-cycles` | communication loops never create build cycles | `REQ-TOOL-050` |
| `REQ-TOOL-model-typed-rates` | dimensioned values parsed once into checked rational rates | `REQ-TOOL-068` |
| `REQ-TOOL-model-contract-meta` | consumer read mode, required flag, reliability requirement in model | `REQ-TOOL-069` |
| `REQ-TOOL-model-layering` | model = structural integrity; policy = verify | `REQ-TOOL-070` |
| `REQ-TOOL-model-runtime-hook` | model derives runtime descriptor expectations + pool sizing consumed at activation | `REQ-CORE-045`, `REQ-CORE-015` |

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
 indexes: BTreeMap<IndexName, IndexDecl>, // index.drm: anchor, cell_size, max_objects, max_cells_per_object, load_factor, max_query_cells
}
// BTree*/Sorted* everywhere ⇒ filesystem order can never leak into model output (I-D1).

pub struct NodeEntry {
 publishes: SortedSet<TopicName>, consumes: SortedSet<TopicName>,
 publish_meta: SortedMap<TopicName, PublishMeta>, // expected_rate (planning only)
 consume_meta: SortedMap<TopicName, ConsumeMeta>, // deadline/read mode/required/reliability (freshness + contract)
 spatial: SpatialContract, // [spatial] write = [frames], read = [frames] + per-read history requirements (spatial-core §Node Spatial Contracts)
 language: Language, build: BuildAdapter,
}

/// Dimensioned value parsed ONCE at model build (REQ-TOOL-068). Downstream
/// algorithms consume checked rationals, never source strings.
pub struct Rate { numerator: u64, denominator: u64 } // e.g. "100Hz" → 100/1, "2.5Hz" → 5/2; checked rational arithmetic
pub struct PublishMeta { expected_rate: Option<Rate>, rate_span: Option<Span> } // span kept for diagnostics only
pub struct SpatialContract {
 write: SortedSet<FrameId>, read: SortedSet<FrameId>,
 history: SortedMap<FrameId, HistoryRequirement>, // per read edge (REQ-SPATIAL-037)
}
pub struct HistoryRequirement { duration: Duration, max_samples: u32, min_sample_period: Duration }
// verify V-SPAT requires max_samples >= floor(duration / min_sample_period) + 2 — expected_rate never sizes it.
pub enum ReadMode { Latest, Sequential }
pub enum ReliabilityRequirement { BestEffort, Reliable }
pub struct ConsumeMeta {
 deadline: Option<Duration>,
 read_mode: ReadMode, // default from topic @history: absent ⇒ Latest
 required: bool, // required sequential consumer (overflow-policies.md registry sizing)
 reliability: ReliabilityRequirement, // route resolves against link capability (verify V-REL)
}
pub enum TimingMode { Auto, Exact, Off } // per topic; default Auto — TopicEntry.timing_override
// Build-time resolution (observability-plane.md): Auto ⇒ Exact iff any consumer of the topic
// declares `deadline` or jitter/inter-arrival diagnostics; otherwise Off. `expected_rate` alone
// never enables Exact. Resolved mode selects the CCB TIMING variant (channel-metadata.md
// REQ-CORE-094), covered by the layout hash. Conflict: explicit Off + a declared consumer
// deadline ⇒ build error.

pub struct ExternalEndpoint { link: LinkId, consume: SortedSet<TopicName>, produce: SortedSet<TopicName> }
pub struct RouteOverrides {
 prefer: SortedMap<TopicName, LinkId>,
 deny: SortedMap<TopicName, SortedSet<LinkId>>,
 force: SortedMap<TopicName, Vec<LinkId>>,
 export_limit: SortedMap<(TopicName, LinkId), Rate>, // per-egress shaping (architecture.md: boundary router export)
}

pub struct ContractFingerprint([u8; 16]); // BLAKE3 output truncated to 128 bits (see contract_fingerprint())
                                          // Width parity: schema_hash/layout_hash are [u8; 16] too
                                          // (channel-metadata.md); a freak fingerprint collision is
                                          // still caught by per-segment schema/layout validation at attach.
```

Model construction is **total-reload**: every parse rebuilds the full model (no incremental caches — architecture §In-Memory Workspace Model).

## Invariants

- **I-D1 Deterministic**: same input tree (any read order) ⇒ byte-identical model render. Test: shuffled-fs-order ×100 seeds.
- **I-D2 ConflictRejection**: duplicate topic/node/package identity, unknown topic in publish/consume, dangling placement, unknown link endpoint ⇒ all failures collected with locations; duplicates retained in a staging map (`BTreeMap<Name, Vec<Definition>>`) until reported so every duplicate site is named. Test: `model_conflicts`, `model_all_duplicates_reported`.
- **I-D3 LoopsHarmless**: A publishes B-consumes, B publishes A-consumes ⇒ model builds fine (no build-cycle error). Test: `communication_loop`.
- **I-D4 FingerprintIsolation**: edit topic `camera/front` ⇒ `controller` fingerprint unchanged; edit a topic that `controller` itself uses ⇒ changed. Test: `fingerprint_isolation`.
- **I-D5 ReproducibleModel**: `render_model(parse(tree))` round-trips byte-identically (feeds property test). Test: `model_roundtrip` (×5 000).
- **I-D6 SpatialInference**: static/dynamic edge classification is deterministic — node declares `[spatial] write` ⇒ dynamic; `transform` given + no writer ⇒ static; both ⇒ conflict error; neither ⇒ dangling-frame error (spatial-core I-S10). Test: `spatial_inference`.
- **I-D7 SpatialSingleWriter**: two nodes writing one dynamic spatial edge ⇒ compiler conflict before deployment (spatial-core I-S11). Test: `spatial_single_writer`.
- **I-D8 ExternalVertices**: `[external.*]` endpoints with `link` + `consume`/`produce` are first-class graph vertices; unknown link or topic ⇒ error. Test: `model_external`.
- **I-D9 TimingOverrides**: publisher `expected_rate`, consumer `deadline`, and `[routing]` overrides (prefer/deny/force/export_limit) are parsed into the model, normalized to typed values, and preserved by render. Test: `model_timing_overrides`.
- **I-D10 TypedRates**: dimensioned rate/duration strings are parsed exactly once at model build into checked rational `Rate`s; downstream algorithms never re-parse strings; `rate_span` retained for diagnostics only. Invalid dimension ⇒ model error at build. Test: `model_typed_rates`, `model_bad_rate_unit`.
- **I-D11 ContractMetadata**: consumer `read_mode`, `required`, and `reliability` requirement are model data (REQ-TOOL-069); `reject`-topic required-sequential counts (overflow-policies.md) derive from `consume_meta`. Test: `model_contract_meta`.
- **I-D12 Layering**: model build enforces structural/referential integrity only (REQ-TOOL-070). Single-writer, route policy, reliability-vs-transport, budgets, and compatibility are exclusively `drm verify` checks (verify-engine.md). Test: `model_no_policy_checks` (two publishers ⇒ model builds; verify fails).
- **I-D13 TimingModeResolution**: topic `timing` parses (default `Auto`, REQ-OPS-016); `Auto` resolves deterministically from the consumer set; explicit `Off` + declared consumer deadline ⇒ single conflict error with location. Test: `model_timing_resolution`.

## Functions

```rust
fn build(tree: &FileTree) -> Result<WorkspaceModel, Vec<ModelError>> {
 let mut m = WorkspaceModel::default();
 let mut dup_staging = BTreeMap::new(); // I-D2: retain duplicates until all reported
 for pkg in discover_packages_sorted(tree) { // sorted discovery (I-D1)
 parse_package_manifest(pkg)?; // duplicates ⇒ conflict error (I-D2)
 for topic_file in pkg.topics_sorted() {
 let t = schema_parser::parse(topic_file)?; // schema-parser.md
 let layout = canon::layout_struct(&t.fields); // canon-codec.md
 m.topics.insert(topic_name(pkg, file_stem), TopicEntry { schema_hash: hash(t), layout, .. });
 }
 }
 let mut errs = Vec::new();
 for node_manifest in discover_nodes_sorted(tree) {
 let n = parse_node_manifest(node_manifest)?; // publish/consume/language/build/meta
 check_references(&mut m, &n, &mut errs)?; // unknown topic ⇒ error; NO policy checks here (I-D12)
 m.nodes.insert(n.name, n);
 }
 parse_topology(tree)?.apply(&mut m)?; // computes, links, placement; dangling refs ⇒ error
 parse_spaces(tree)?.apply(&mut m.spatial)?; // spaces/**.drm frames; static/dynamic inference (I-D6);
 // [spatial] write/read contract wiring; single-writer spatial edge check (I-D7);
 // history requirements (duration, max_samples, min_sample_period) per read edge;
 // index.drm declarations (anchor, cell_size, max_objects, max_cells_per_object, load_factor, max_query_cells)
 // — capacity feeds memory budget (I-MB9)
 parse_externals(tree)?.apply(&mut m)?; // [external.*] vertices; unknown link/topic ⇒ error (I-D8)
 parse_routing(tree)?.apply(&mut m)?; // [routing] prefer/deny/force/export_limit; unknown ⇒ error (I-D9)
 normalize_rates(&mut m)?; // I-D10: parse rate/duration strings once into typed Rate/Duration
 if !errs.is_empty() { return Err(collect_with_duplicates(errs, &mut dup_staging)); }
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
 //     [16]u8  schema content hash (schema_hash — architecture §Node Interface Identity requires
 //             schemas AND layouts; a schema edit with unchanged layout still changes the fingerprint)
 //     [16]u8  canonical layout hash (canon-codec.md)
 //
// Hash: BLAKE3 over exactly these bytes, truncated to 16 bytes ([u8; 16]).
// Deliberately EXCLUDED: other nodes, packages, topology, links, build config, runtime handles.
// ⇒ unrelated schema edits cannot change unaffected nodes (I-D4).
}
```

## Runtime Hooks

`runtime_plan(model)` derives the runtime activation expectations the runtime
consumes; the runtime never parses `.drm` schemas itself, so a topic edit and the
channel configuration cannot drift (architecture.md §Channel Metadata):

```rust
pub struct TopicRuntime {
 schema_hash: [u8; 16], layout_hash: [u8; 16], // descriptor expectations (REQ-CORE-015)
 payload_capacity: u64, slot_align: u64, // StructLayout::sample_max / struct_align
 history_depth: u64, // @history: latest ⇒ 1
 pool_size: u64, // explicit @pool_size or the R + 1 activation floor (REQ-CORE-045)
 overflow: RuntimeOverflow, // @overflow: reject | overwrite (default)
}
pub fn runtime_plan(model: &WorkspaceModel) -> RuntimePlan // BTreeMap<topic, TopicRuntime>
```

Sizing here is **structural**: the hook emits declared values and the `R + 1`
floor. The policy check `pool_size >= R + H + 1` and drop-free validation stay in
`drm verify` (I-D12). `Channel::activate` rejects a pool below the `R + 1` floor.

## Protocols

*Model render → reparse round-trip (I-D5):*

1. `text = render_model(m)` (canonical TOML ordering: topics, nodes, computes, links).
2. `m2 = build(parse_tree(text))`.
3. Assert `render_model(m2) == text`.

*Runtime plan consumption:* `runtime_plan(model)` → per-topic `Channel::activate`
parameters (depth, pool size, sample max, policy) and attach-time descriptor
expectations (`schema_hash`/`layout_hash`).

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
| Rate string not parseable (`"100"`, `"5 furlongs"`) | model error at build with `rate_span` (I-D10) |
| Two nodes publish one topic | model builds; `drm verify` V-SW fails (I-D12) |

## Test Matrix

| Case | Expected |
|---|---|
| 3-fixture workspace tree | model matches golden render |
| shuffled fs order ×100 | byte-identical model |
| conflict fixtures (dup topic, dup node, dangling placement, unknown link) | rejected with location |
| two publishers, same topic | model builds; verify V-SW catches (I-D12) |
| typed-rate fixtures (`100Hz`, `2.5Hz`, invalid unit) | I-D10 parse once / build error |
| contract-meta fixtures (`read_mode`, `required`, `reliable`) | I-D11 carried in model |
| communication-loop fixture | builds, no cycle error |
| fingerprint isolation | I-D4 |
| fingerprint: shuffled manifest order / map iteration | identical bytes (canonical serialization, sorted inputs) |
| fingerprint: used schema modified / direction changed / ABI bumped / layout changed | different bytes |
| deployment.json `deployment_id` field | UUID/hex string, never a JSON number (128 bits > JSON-safe range) |
| external endpoint fixtures | I-D8 (unknown link/topic rejected) |
| timing/route override fixtures | I-D9 |
| timing-mode resolution (auto-with-deadline / auto-without / explicit exact / off+deadline conflict) | I-D13 |
| round-trip ×5 000 | I-D5 |
| spatial inference matrix (write-only / transform-only / both / neither) | I-D6 |
| two-writers-one-edge fixture | I-D7 conflict |
| `model_conflicts` / `model_all_duplicates_reported` | I-D2 |
| `communication_loop` | I-D3: builds, no cycle error |
| `fingerprint_isolation` | I-D4 |
| `model_roundtrip` (×5 000) | I-D5 |
| `spatial_inference` | I-D6 |
| `spatial_single_writer` | I-D7 |
| `model_external` | I-D8 |
| `model_timing_overrides` | I-D9 |
| `model_typed_rates` / `model_bad_rate_unit` | I-D10 |
| `model_contract_meta` | I-D11 |
| `model_no_policy_checks` (two publishers ⇒ model builds; verify fails) | I-D12 |
| `model_timing_resolution` | I-D13 |
