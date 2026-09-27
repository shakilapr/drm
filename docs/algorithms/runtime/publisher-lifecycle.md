# Publisher Lifecycle & Epochs Algorithms

Scope: publisher authority acquisition via OS locks, restart semantics, `publisher_epoch` increment exactly-once, consumer epoch observation and state reset, freshness transitions. Implements architecture.md §Publisher Epoch, §Segment Creation and Writer Authority, §Data Freshness vs Stored Bytes, §Session-Scoped Persistence. Owner: E1 (crate `drm-core::channel::lifecycle`). Phase 17. Depends on [shm-backend.md](shm-backend.md) (locks, identity), [channel-metadata.md](channel-metadata.md) (epoch field, validation).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-publisher-lock-authority` | exactly one publisher per topic; OS lock; kernel release on death | — |
| `REQ-CORE-epoch-increment-once` | restart increments epoch exactly once | — |
| `REQ-CORE-epoch-consumer-reset` | consumers observe epoch change and reset reader state | — |
| `REQ-CORE-no-cross-epoch-confusion` | pre-restart sample never read as post-restart | — |
| `REQ-CORE-freshness-transitions` | STALE/OFFLINE per timing contracts; restart ⇒ NO_CURRENT_SAMPLE | — |

## Data Structures

```rust
// Writer lock path: /run/drm/<deployment_id>/<topic>.publisher.lock (shm-backend §Protocols).
// CCB carries publisher_epoch: AtomicU32 (channel-metadata.md).

pub enum SegmentState { Init = 0, Ready = 1 } // published in the descriptor before first use
pub enum StreamState { NoCurrentSample, Active, Stale, Offline }
// Consumer-local state machine; freshness thresholds come from topic timing contracts
// (expected_rate / deadline declarations), enforced observationally by drm-diag.
```

## Invariants

- **I-L1 SingleAuthority**: concurrent publisher opens ⇒ exactly one holds `.publisher.lock`; the loser refuses (no takeover). Test: `publisher_race` (two opens race).
- **I-L2 KernelRelease**: kill active publisher ⇒ lock released by kernel ⇒ restart re-acquires without heartbeat. Test: `publisher_kill_restart`.
- **I-L3 EpochOnce**: across crash+restart cycles, `publisher_epoch` increments exactly once per restart (never 0, never 2, monotonic). Test: `epoch_increment_once` (100 restart cycles).
- **I-L4 ConsumerReset**: consumer observing epoch' > epoch resets cursor; no pre-restart sequence is ever returned post-restart. Test: `epoch_consumer_reset`.
- **I-L5 NoCrossEpochReads**: samples from the previous run are never misattributed to the new run (sequence may continue numerically; epoch disambiguates). Test: `epoch_no_confusion`.
- **I-L6 FirstValidWatermark**: before publishing its first new sample, a restarting publisher advances `first_valid_seq = last_sequence + 1`, so the prior run's samples are not addressable even by a freshly reset cursor. Test: `epoch_valid_range`.
- **I-L7 RestartOrder**: restart invalidates prior-run state **first** (logical history clear → current sample invalid → epoch-local counters reset → `first_valid_seq`) and publishes the new epoch as the **final** Release operation; a consumer that observes the new epoch can never see prior-run state presented as current. Hazard words are never cleared during restart (outstanding borrows finish safely). Test: `epoch_restart_order` (crash injection at each step).
- **I-L8 SegmentReady**: a fresh segment is initialized (descriptor + CCB) and `READY` Release-stored before it is exposed; a crash before `READY` is recovered on open, never passed to attach validation. Test: `segment_init_ready` (kill between create and READY).

## Functions

```rust
// Publisher open (full lifecycle)
fn publisher_open(name: &SegmentName, expect: &Expectation) -> Result<Publisher, Status> {
 let lock = lock_backend.try_exclusive(name.publisher_lock())?; // I-L1: held-or-refuse

 let (mapping, fresh) = match shm.create_exclusive(name, expect.size) {
 Ok(m) => {
 // CREATE → INIT → READY → expose (I-L8). Nothing observes the segment before READY.
 init_descriptor(&m, expect); // magic, layout_version, schema_hash, capacities
 init_ccb(&m); // ring empty, hazards NONE, counters zero
 m.desc().segment_state.store(Ready, Release); // READY published last
 (m, true)
 }
 Err(AlreadyExists) => {
 let m = shm.open_existing(name, expect.size)?;
 if m.desc().segment_state.load(Acquire) != Ready {
 recover_init_or_recreate(name, &m)?; // crash before READY (I-L8)
 }
 (m, false)
 }
 };

 let desc = map_descriptor(&mapping);
 channel_metadata::validate_on_attach(desc, expect)?; // descriptor is READY here

 if !fresh && publisher_runs(desc) { // previous incarnation had run
 // Transactional restart: invalidate prior-run state FIRST, publish epoch LAST (I-L7).
 let last = desc.last_sequence(); // preserved (sequence continuity)
 desc.ring().logical_clear_history(); // old borrows stay hazard-protected
 desc.invalidate_current_sample(); // state ⇒ NO_CURRENT_SAMPLE
 reset_epoch_local_counters(desc); // counters reset; hazards untouched
 desc.ring().set_first_valid(last + 1); // I-L6: exclude prior-run sequences
 let next = desc.epoch.load(Relaxed).checked_add(1).ok_or(EpochExhausted)?;
 desc.epoch.store(next, Release); // I-L7: FINAL restart publication
 Some(next) // consumers observe (I-L4)
 } else {
 desc.epoch.store(1, Release); // first run
 desc.ring().set_first_valid(0);
 None
 }
 Ok(Publisher { lock, mapping, .. })
}

fn publisher_close(p: Publisher) { /* lock Drop ⇒ release; segment stays (session-scoped persistence) */ }

// Consumer side (cursor-delivery.md integrates)
fn observe_epoch(ch: &Channel, local: &mut u32) -> bool {
 let e = ch.desc.epoch.load(Acquire);
 if e != *local { // I-L4
 *local = e;
 reset_cursor_state(); // cursor window floor = first_valid_seq (I-L6)
 return true;
 }
 false
}
```

*Sequence continuity policy:* the restarted publisher continues sequence numbering from the shared CCB (`last_sequence` preserved) — monotonicity (Phase 9 invariant) holds across restarts; epoch disambiguates runs (I-L5). Alternative zero-restart was rejected: consumers rely on monotonic sequences for Gap accounting.

## Protocols

*Fresh segment (I-L8):*

1. `create_exclusive` ⇒ segment mapped, `segment_state = Init`.
2. Initialize descriptor + CCB.
3. `segment_state.store(Ready, Release)` — the only point at which the segment is usable.
4. Same process proceeds directly (it just initialized the descriptor).

*Init crash recovery (I-L8):* a later publisher opens an existing segment, sees `segment_state != Ready` (crash between create and READY), and re-initializes descriptor + CCB before setting READY.

*Restart cycle (I-L2/I-L3/I-L7, exercised 100×, crash-injected at each step):*

1. Publisher A holds lock, published N samples, then SIGKILL.
2. Kernel releases `.publisher.lock` (shm-backend I4).
3. Publisher B `publisher_open`: lock acquired; segment exists and is READY ⇒ `open_existing`; descriptor valid.
4. Invalidate prior-run state first: `ring.logical_clear_history()`, `invalidate_current_sample()`, reset epoch-local counters, `set_first_valid(N+1)` (I-L6). Hazard words are untouched.
5. Publish the new epoch **last**: `epoch.store(next, Release)` (I-L7). Sequence continues at N.
6. Consumers observe epoch' ⇒ reset cursors to the new window floor; first read under the new epoch gets B's first sample, never A's stale payload as "new".

*Freshness transitions (observation-only):*

1. Consumer tracks time-since-last-sample against topic timing contract.
2. Transitions: `Active → Stale` (soft threshold) `→ Offline` (hard); restart ⇒ `NoCurrentSample` until first complete sample under new epoch (architecture rule).

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two publishers race open | loser: `Permission` (authority held); no partial init |
| Publisher killed mid-write (in-progress slot) | slot never committed to ring (commit is last step); consumers unaffected (borrowed samples finish safely) |
| Restart without prior run | epoch initialized to 1 |
| Consumer attaches during restart race | either pre-restart descriptor (epoch e) or post (e+1); attach validation passes both (fingerprint identical); cursor resets on first observation |
| Machine reboot | segments recreated fresh (session-scoped persistence); epoch starts at 1 in new session |
| `drm clean` while publisher active | active.lock shared hold ⇒ cleanup refuses (deployment-identity.md) |

## Test Matrix

| Case | Expected |
|---|---|
| `publisher_race` | I-L1 |
| `publisher_kill_restart` (×100) | I-L2 + I-L3 |
| `epoch_consumer_reset` | I-L4 |
| `epoch_no_confusion` | I-L5 |
| `epoch_valid_range` (restart watermark) | I-L6 |
| `epoch_restart_order` (crash after each init/reset/epoch/commit step) | I-L7 |
| `segment_init_ready` (kill between create and READY) | I-L8 |
| freshness fixtures | transitions per contracts |
| reboot-sim (fresh namespace) | epoch 1, clean segment |
