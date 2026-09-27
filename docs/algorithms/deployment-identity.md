# Deployment Identity & Isolation Algorithms

Scope: the three-identity model — node contract fingerprints (compiled), deployment instance IDs (runtime config), publisher epochs (runtime state) — plus namespace scoping, startup compatibility guard, per-segment validation, and OS-lock-based cleanup. Implements architecture.md §Shared-Memory Lifetime: Three Identities and Deployment Isolation. Owner: E1 (runtime guards, `drm clean`) + E2 (fingerprint computation, deployment packaging). Phase 18. Depends on [workspace-model.md](toolchain/workspace-model.md) (fingerprints), [channel-metadata.md](runtime/channel-metadata.md) (validation), [shm-backend.md](runtime/shm-backend.md) (locks).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-fingerprint-inputs` | fingerprint = hash(ABI version, own topic schemas/layouts, declared interfaces) | `REQ-CORE-085` |
| `REQ-CORE-deploy-id-runtime-only` | 128-bit random ID in `deployment.json`; never compiled into binaries | `REQ-CORE-084` |
| `REQ-CORE-namespace-scoping` | deployment-scoped backend-safe opaque names | `REQ-CORE-088` |
| `REQ-CORE-startup-guard` | compiled fingerprint ≠ deployment fingerprint ⇒ refuse to start | `REQ-CORE-092` |
| `REQ-CORE-no-cross-deployment` | new deployment ⇒ new namespace; never adopt/migrate | `REQ-CORE-089` |
| `REQ-CORE-clean-active-guard` | `drm clean` unlinks only when the exclusive deployment active lock proves no nodes | `REQ-CORE-080` |

## Data Structures

```rust
pub struct DeploymentId(u128); // random per deployment generation
pub struct DeploymentConfig { // .drm/deploy/deployment.json (runtime config, not compiled)
 deployment_id: DeploymentId,
 fingerprint_expectations: BTreeMap<NodeName, ContractFingerprint>,
 routing: RoutePlans, // forwarding trees computed by toolchain (route-derivation.md)
 memory_budgets: BTreeMap<ComputeName, Budget>, // memory-budget.md
 lock_dir: PathBuf, // /run/drm/<deployment_id> (Linux); %ProgramData%\drm\<id> (Windows)
}
impl DeploymentConfig { fn lock_paths(&self) -> LockPaths { LockPaths(self.lock_dir.clone()) } }

pub enum AttachContext {
 Publisher { compiled: ContractFingerprint },
 Consumer { compiled: ContractFingerprint },
}

// The shared active-lock guard MUST be owned by the session for the node's whole lifetime;
// dropping it early would let `drm clean` acquire the exclusive lock while the node runs.
pub struct Session { deploy_id: DeploymentId, active_guard: LockHandle }
```

## Invariants

- **I-DP1 GuardRefusal**: binary whose compiled fingerprint ≠ `fingerprint_expectations[node]` refuses startup with explicit status. Test: `fingerprint_guard`.
- **I-DP2 CrossAttachZero**: two deployments with different IDs run simultaneously; cross-attachment impossible (namespace scoping) — attempts fail `NotFound`, never attach foreign memory. Test: `cross_deployment_isolation`.
- **I-DP3 IdNeverCompiled**: changing hardware placement/routing/ID in `deployment.json` requires no node recompile (fingerprint excludes deployment config). Test: `config_no_recompile` (evidence: fingerprint inputs — workspace-model §contract_fingerprint).
- **I-DP4 UnrelatedStable**: unrelated schema edit ⇒ unaffected node fingerprint unchanged (workspace-model I-D4 e2e through guard). Test: `fingerprint_isolation_e2e`.
- **I-DP5 CleanGuarded**: `drm clean` unlinks the namespace iff it acquires the exclusive deployment active lock (no shared holder = no live node); active deployments untouched. Test: `clean_active_refused`, `clean_inactive_ok`.

## Functions

```rust
// ---- Startup (both roles) ----
fn node_startup(ctx: &AttachContext, deploy: &DeploymentConfig, node: NodeName) -> Result<Session, Status> {
 let expected = deploy.fingerprint_expectations.get(node)
 .ok_or(NotFound)?;
 if ctx.compiled != *expected { return Err(WrongAbi); } // I-DP1: refuse immediately
 // Hold the shared active lock for the node's whole lifetime; the guard is stored in Session
 // (RAII: released only when the session drops, i.e. the node exits).
 let active_guard = lock_backend.lock_shared(&deploy.lock_paths().active())?;
 Ok(Session { deploy_id: deploy.deployment_id, active_guard })
}

// ---- Segment open (per topic, secondary check) ----
fn segment_open(sess: &Session, topic: &TopicPath, expect: &TopicExpectation) -> Result<ChannelView, Status> {
 // POSIX shm names permit only the leading slash. The descriptor retains the canonical topic.
 let name = backend_segment_name(sess.deploy_id, hash_topic(topic)); // /drm.<id>.<topic-hash>
 let m = shm.open_existing(&name, expect.size)?; // never create (consumers)
 channel_metadata::validate_on_attach(&m, expect) // exact descriptor check; returns validated view
}

// ---- drm clean ----
fn clean(deploy: &DeploymentConfig) -> Result<CleanReport, Status> {
 // Exclusive active lock succeeds ⇒ no node holds a shared hold ⇒ namespace is unowned (I-DP5).
 let _exclusive = match lock_backend.try_exclusive(&deploy.lock_paths().active()) {
 Ok(h) => h,
 Err(WouldBlock) => return Ok(CleanReport::ActiveUntouched), // shared hold exists ⇒ live nodes
 Err(e) => return Err(e),
 };
 for name in names_from_deployment_metadata(deploy) { shm_unlink(name)?; }
 Ok(CleanReport::Cleaned)
}
```

## Protocols

*Deployment generation (toolchain, `drm deploy`):*

1. `DeploymentId::random()` (CSPRNG).
2. Emit `deployment.json` with fingerprint expectations (from workspace-model contract fingerprints), routing tables, budgets.
3. Backend-safe opaque SHM names derive from the ID plus canonical-topic hash; the exact topic remains in the immutable descriptor. New deployment ⇒ fresh namespace (I-DP2); no live migration ever.

*Node startup sequence:* guard (fingerprint) → shared active hold → per-topic `segment_open` (secondary validation) → run.

*Cleanup decision tree:* exclusive deployment active lock ⇒ success ⇒ unlink each segment ⇒ `Cleaned`; failure (`WouldBlock` — a live node holds it shared) ⇒ `ActiveUntouched` (leave everything). One lock, one directory (`/run/drm/<deployment_id>/`). The exclusive handle is held until all unlinking completes, so no node can acquire the namespace mid-clean. Cleaners serialize against each other on the same lock; no separate cleanup lock exists.

*Direct result of the lock model:* nodes must retain the shared guard inside `Session`; a test asserts that a running session (which holds the guard) blocks `clean`, and that dropping the session unblocks it. Gate is a real multi-process `flock` integration test, not a mocked lock.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Outdated binary vs new deployment | refuse at startup (`WrongAbi`) — before any SHM touch |
| Two deployments same machine, same topic names | isolated by namespace; both run concurrently (I-DP2 test) |
| Deployment.json edited (routes/ID) mid-session | takes effect only on next deployment; running nodes unaffected |
| Cleanup racing node startup | exclusive vs shared hold on the same active lock serialize the decision; loser aborts cleanly |
| Stale segments from dead deployment | cleaned by `drm clean` (no holds) |
| Fingerprint mismatch on one node only | that node refuses; deployment continues for others (fault isolation) |

## Test Matrix

| Case | Expected |
|---|---|
| `fingerprint_guard` | I-DP1 |
| `cross_deployment_isolation` (two namespaces, same topics) | I-DP2 |
| `config_no_recompile` | I-DP3 |
| `fingerprint_isolation_e2e` | I-DP4 |
| `clean_active_refused` / `clean_inactive_ok` | I-DP5 |
| `clean_blocks_while_session_holds` (real multi-process `flock`) | guard retention: shared hold blocks exclusive clean |
| stale-binary × mixed-deployment matrix | per-node refusal |
