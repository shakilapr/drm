# Publisher Lifecycle & Epochs Algorithms

Scope: publisher authority acquisition via OS locks, restart semantics, `publisher_epoch` increment exactly-once, consumer epoch observation and state reset, freshness transitions. Implements architecture.md §Publisher Epoch, §Segment Creation and Writer Authority, §Data Freshness vs Stored Bytes, §Session-Scoped Persistence. Owner: E1 (crate `drm-core::channel::lifecycle`). Phase 17. Depends on [shm-backend.md](shm-backend.md) (locks, identity), [channel-metadata.md](channel-metadata.md) (epoch field, validation).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-CORE-publisher-lock-authority` | exactly one publisher per topic; OS lock; kernel release on death | `REQ-CORE-054` |
| `REQ-CORE-epoch-increment-once` | restart increments epoch exactly once | `REQ-CORE-020` |
| `REQ-CORE-epoch-consumer-reset` | consumers observe epoch change and reset reader state | `REQ-CORE-018` |
| `REQ-CORE-no-cross-epoch-confusion` | pre-restart sample never read as post-restart | `REQ-CORE-038` |
| `REQ-CORE-freshness-transitions` | STALE/OFFLINE per timing contracts; restart ⇒ NO_CURRENT_SAMPLE | `REQ-CORE-027` |

## Data Structures

```rust
// Writer lock path: /run/drm/<deployment_id>/<topic>.publisher.lock (shm-backend §Protocols).
// CCB carries publisher_epoch: AtomicU32 (channel-metadata.md).

pub enum SegmentState { Init = 0, Ready = 1 } // published in the descriptor before first use
pub enum StreamState { NoCurrentSample, Active, Stale, Offline }
pub struct FreshnessTracker {
 epoch: u32,
 state: StreamState,
 last_arrival: Option<MonotonicInstant>,
 stale_after: Duration,
 offline_after: Duration,
}
```

## Invariants

- **I-L1 SingleAuthority**: concurrent publisher opens ⇒ exactly one holds `.publisher.lock`; the loser refuses (no takeover). Test: `publisher_race` (two opens race).
- **I-L2 KernelRelease**: kill active publisher ⇒ lock released by kernel ⇒ restart re-acquires without heartbeat. Test: `publisher_kill_restart`.
- **I-L3 EpochOnce**: each successful publisher-authority acquisition advances `publisher_epoch` once before returning an active publisher. A process killed after publication consumed that epoch; its successor advances again. Test: `epoch_increment_once` (100 restart cycles).
- **I-L4 ConsumerReset**: consumer observing epoch' > epoch resets cursor; no pre-restart sequence is ever returned post-restart. Test: `epoch_consumer_reset`.
- **I-L5 NoCrossEpochReads**: samples from the previous run are never misattributed to the new run (sequence may continue numerically; epoch disambiguates). Test: `epoch_no_confusion`.
- **I-L6 FirstValidWatermark**: before activation, a restarting publisher sets `first_valid_seq = write_seq` (the preserved next sequence), excluding every prior-run sample. Test: `epoch_valid_range`.
- **I-L7 RestartOrder**: restart advances the run floor and required acknowledgements to the preserved next sequence, then publishes the new epoch last. Consumers derive `NoCurrentSample` from that epoch/floor. Ring cells, hazards, and lifetime counters are not cleared. Test: `epoch_restart_order`.
- **I-L8 SegmentReady**: a fresh named segment remains unusable in `Init`; only complete descriptor + CCB initialization is published by a Release store to `Ready`. Test: `segment_init_ready`.
- **I-L9 FreshnessLocal**: freshness is consumer-local, uses monotonic arrival time, and obeys `stale_after < offline_after`; epoch change immediately yields `NoCurrentSample`. Test: `freshness_boundaries`.

## Functions

```rust
// Publisher open (full lifecycle)
fn publisher_open(name: &SegmentName, expect: &Expectation) -> Result<Publisher, Status> {
 let lock = lock_backend.try_exclusive(name.publisher_lock())?; // I-L1: held-or-refuse

 let mapping = match shm.create_exclusive(name, expect.size) {
 Ok(m) => {
 // The name is visible in Init; consumers return NotReady and retry without using descriptor bytes.
 m.prefix().segment_state.store(Init, Relaxed);
 init_descriptor(&m, expect); // magic, layout_version, schema_hash, capacities
 init_ccb(&m); // ring empty, hazards NONE, counters zero
 m.prefix().segment_state.store(Ready, Release);
 m
 }
 Err(AlreadyExists) => {
 let m = shm.open_existing(name, expect.size)?;
 if m.prefix().segment_state.load(Acquire) != Ready {
  drop(m);
  recover_init_segment_under_publisher_lock(name, expect)?
 } else { m }
 }
 };

 let ch = channel_metadata::validate_on_attach(&mapping, expect)?;
 ch.ring.repair_incomplete_banks()?; // publisher lock proves any odd bank's writer is dead
 let floor = ch.ccb.write_seq.load(Acquire); // next sequence; continuity preserved
 let next_epoch = ch.ccb.publisher_epoch.load(Relaxed)
  .checked_add(1).filter(|e| *e != 0).ok_or(EpochExhausted)?;
 ch.ccb.publish_busy.store(false, Release); // prior writer died; its uncommitted slot is unreachable
 ch.ccb.first_valid_seq.store(floor, Release);
 for req in ch.required_consumers() { req.ack_seq.fetch_max(floor, Release); }
 ch.ccb.publisher_epoch.store(next_epoch, Release); // final run publication
 Ok(Publisher { lock, mapping, epoch: next_epoch, .. })
}

fn publisher_close(p: Publisher) { /* lock Drop ⇒ release; segment stays (session-scoped persistence) */ }

// Consumer side (cursor-delivery.md integrates)
fn observe_epoch(ch: &Channel, local: &mut u32) -> bool {
 let e = ch.ccb.publisher_epoch.load(Acquire);
 if e != *local { // I-L4
 *local = e;
 reset_cursor_state(); // cursor window floor = first_valid_seq (I-L6)
 return true;
 }
 false
}

fn observe_sample(f: &mut FreshnessTracker, epoch: u32, now: MonotonicInstant) {
 f.epoch = epoch;
 f.last_arrival = Some(now);
 f.state = Active;
}

fn freshness(f: &mut FreshnessTracker, epoch: u32, now: MonotonicInstant) -> StreamState {
 if epoch != f.epoch {
  f.epoch = epoch; f.last_arrival = None; f.state = NoCurrentSample;
  return f.state;
 }
 let Some(last) = f.last_arrival else { return NoCurrentSample; };
 let age = now.saturating_duration_since(last);
 f.state = if age >= f.offline_after { Offline }
  else if age >= f.stale_after { Stale } else { Active };
 f.state
}
```

*Sequence continuity policy:* `write_seq` is the next sequence and is never reset. Restart uses it directly as
the new `first_valid_seq`; no `last + 1` arithmetic is needed.

## Protocols

*Fresh segment (I-L8):*

1. `create_exclusive` publishes a named segment whose fixed prefix remains `Init`.
2. Initialize descriptor + CCB.
3. `segment_state.store(Ready, Release)` — the only point at which the segment is usable.
4. Consumers that observed `Init` unmap and retry under their bounded startup policy.

*Init crash recovery (I-L8):* only a process holding the publisher lock repairs `Init`. It unmaps,
unlinks the incomplete name, creates a new `Init` object, initializes it, and publishes `Ready`. POSIX
stale mappings remain unusable; Windows retries creation until observers have closed their `Init`
handles. Consumers never repair or retain an `Init` mapping.

*Restart cycle (I-L2/I-L3/I-L7, exercised 100×, crash-injected at each step):*

1. Publisher A holds lock, published N samples, then SIGKILL.
2. Kernel releases `.publisher.lock` (shm-backend I4).
3. Publisher B `publisher_open`: lock acquired; segment exists and is READY ⇒ `open_existing`; descriptor valid.
4. Read `floor = write_seq`; publish `first_valid_seq = floor` and advance required acknowledgement floors. Ring cells, hazards, and lifetime counters remain untouched.
5. Publish the checked new epoch **last** (I-L7). Sequence continues at `floor`.
6. Consumers observe epoch' ⇒ reset cursors to the new window floor; first read under the new epoch gets B's first sample, never A's stale payload as "new".

*Freshness transitions (consumer-local observation only):*

1. Verify `stale_after < offline_after` at attach.
2. A completed read records local monotonic arrival time and enters `Active`.
3. `age >= stale_after` enters `Stale`; `age >= offline_after` enters `Offline`.
4. Epoch change clears arrival time and enters `NoCurrentSample` until a complete new-epoch read.
5. Source timestamps never drive freshness; clock regression saturates age at zero.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Two publishers race open | loser: `Permission` (authority held); no partial init |
| Publisher killed mid-write (in-progress slot) | slot never committed to ring (commit is last step); consumers unaffected (borrowed samples finish safely) |
| Publisher killed during restart before epoch store | old epoch may observe no current sample; successor repeats idempotent floor publication |
| Publisher killed after epoch store | that authority acquisition consumed the epoch; successor advances again |
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
| `freshness_boundaries` (threshold−1, threshold, clock regression, epoch) | I-L9 |
| reboot-sim (fresh namespace) | epoch 1, clean segment |
