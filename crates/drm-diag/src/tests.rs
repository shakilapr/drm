//! Diagnostics-plane suite (`docs/algorithms/ops/observability-plane.md`
//! §Invariants I-OB2/I-OB3/I-OB4/I-OB5/I-OB7, §Test Matrix `diag_no_overhead`,
//! `diag_observed_rate`, `diag_deadline_matrix`, `introspect_no_pin`,
//! `diag_epoch_sandwich`).
//! REQ: REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-014, REQ-OPS-016.

use std::cell::Cell;
use std::sync::atomic::Ordering;

use super::{
    Aggregator, ChannelStats, ChannelView, Condition, ConsumerStats, DeadlineState, DiagSignals,
    LinkStats, LogCatalog, LogLevel, Observer, RateWindow, deadline_tick, diag_sample,
};
use drm_core::channel::ccb::ChannelCounters;
use drm_core::channel::{Channel, OverflowPolicy, ReadOutcome};
use drm_core::hazard::{HAZARD_NONE, pack};
use drm_core::trace::TraceEvent;

fn ch() -> Channel {
    Channel::activate(2, 3, 16, 8, OverflowPolicy::Overwrite, 0, 3, None)
        .unwrap_or_else(|e| unreachable!("activate: {e:?}"))
}

/// REQ: REQ-OPS-002 — sampling is out-of-band: it reads the epoch sandwich and
/// leaves every counter untouched (no data-path involvement).
#[test]
fn diag_no_overhead() {
    let c = ch();
    assert!(c.publish(&[1_u8; 16]).is_ok());
    let before = c.counters();
    let stats = diag_sample(&c);
    let after = c.counters();
    assert_eq!(before, after, "sampling never mutates channel state");
    assert_eq!(stats.last_sequence, before.last_sequence);
    assert_eq!(stats.publisher_epoch, before.publisher_epoch);
    assert_eq!(stats.last_commit_mono_ns, None, "non-timing channel");
}

/// A view whose epoch flips between the two sandwich reads exactly once.
struct SandwichView {
    epoch_reads: Cell<u32>,
    counter_reads: Cell<u32>,
}

impl ChannelView for SandwichView {
    fn epoch(&self) -> u32 {
        let n = self.epoch_reads.get();
        self.epoch_reads.set(n + 1);
        if n == 0 { 5 } else { 6 }
    }

    fn counters(&self) -> ChannelCounters {
        let n = self.counter_reads.get();
        self.counter_reads.set(n + 1);
        ChannelCounters {
            write_seq: 100 + u64::from(n),
            last_sequence: 99 + u64::from(n),
            drop_count: 0,
            overrun_count: 0,
            pool_exhausted: 0,
            publisher_epoch: 6,
            reclaim_requested: false,
        }
    }
}

/// REQ: REQ-OPS-002 — an epoch change mid-snapshot discards the torn read and
/// retries; the returned sample is from a consistent sandwich (`I-OB7`).
#[test]
fn diag_epoch_sandwich() {
    let view = SandwichView {
        epoch_reads: Cell::new(0),
        counter_reads: Cell::new(0),
    };
    let stats = diag_sample(&view);
    assert_eq!(stats.last_sequence, 101, "stable snapshot used");
    assert_eq!(stats.publisher_epoch, 6);
    assert!(view.epoch_reads.get() >= 4, "at least one retry happened");
}

/// REQ: REQ-OPS-001 — `observed_rate = Δwrite_seq / Δtime` over a sliding
/// window matches injected known rates within tolerance (`I-OB3`); a burst
/// window reports its own high rate and decays to zero after the window.
#[test]
fn diag_observed_rate() {
    for declared_hz in [10_u64, 100, 1000] {
        let period_ns = 1_000_000_000 / declared_hz;
        let mut w = RateWindow::new(1_000_000_000);
        for i in 0..=declared_hz {
            let t = i * period_ns;
            w.record(t, i);
        }
        let observed = w.observed_rate_hz();
        #[allow(clippy::cast_precision_loss)]
        let expected = declared_hz as f64;
        assert!(
            (observed - expected).abs() / expected < 0.01,
            "declared={declared_hz} observed={observed}"
        );
    }

    let mut burst = RateWindow::new(100_000_000);
    for i in 0..100_u64 {
        burst.record(i * 100_000, i);
    }
    let hot = burst.observed_rate_hz();
    assert!(hot > 5_000.0, "burst rate {hot} should be high");
    burst.record(1_000_000_000, 100);
    assert!(
        burst.observed_rate_hz().abs() < f64::EPSILON,
        "window decayed to idle"
    );
}

/// REQ: REQ-OPS-001, REQ-OPS-016 — the deadline state machine over equal-mean
/// burst vs even traffic, multi-deadline consumers, startup silence, publisher
/// death, and recovery between polls; one `miss_count` per `Current → Late`
/// (`I-OB4`).
#[test]
fn diag_deadline_matrix() {
    let deadline = 1_000_u64;

    // Startup silence: never published ⇒ WaitingFirst.
    let mut silent = ConsumerStats::new(1, 7, deadline);
    deadline_tick(&mut silent, None, 10_000);
    assert_eq!(silent.state, DeadlineState::WaitingFirst);
    assert_eq!(silent.miss_count, 0);

    // Even traffic at 2× deadline: stays Current.
    let mut even = ConsumerStats::new(1, 7, deadline);
    deadline_tick(&mut even, Some(0), 400);
    assert_eq!(even.state, DeadlineState::Current);
    deadline_tick(&mut even, Some(600), 900);
    assert_eq!(even.state, DeadlineState::Current);
    assert_eq!(even.miss_count, 0);

    // Burst then silence: age ≥ deadline ⇒ Late, counted once.
    let mut burst = ConsumerStats::new(2, 7, deadline);
    deadline_tick(&mut burst, Some(0), 500);
    assert_eq!(burst.state, DeadlineState::Current);
    deadline_tick(&mut burst, Some(0), 1_500);
    assert_eq!(burst.state, DeadlineState::Late);
    assert_eq!(burst.miss_count, 1);
    // Publisher death: age keeps growing; state stays Late, no new miss.
    deadline_tick(&mut burst, Some(0), 9_999);
    assert_eq!(burst.state, DeadlineState::Late);
    assert_eq!(burst.miss_count, 1, "no re-count while still Late");

    // Recover between polls: a fresh commit below deadline ⇒ Current.
    deadline_tick(&mut burst, Some(9_500), 9_600);
    assert_eq!(burst.state, DeadlineState::Current);
    assert_eq!(burst.miss_count, 1, "recovery never increments miss_count");

    // Multi-deadline consumers of one topic evaluated independently.
    let mut tight = ConsumerStats::new(3, 7, 500);
    let mut loose = ConsumerStats::new(4, 7, 5_000);
    deadline_tick(&mut tight, Some(0), 800);
    deadline_tick(&mut loose, Some(0), 800);
    deadline_tick(&mut tight, Some(0), 800);
    deadline_tick(&mut loose, Some(0), 800);
    assert_eq!(tight.state, DeadlineState::Late);
    assert_eq!(loose.state, DeadlineState::Current);
}

/// REQ: REQ-OPS-003 — read-only introspection: a continuous `copy_latest`
/// observer holds zero hazard slots (`I-OB5`) and the pool stays exhaustible
/// while it runs.
#[test]
fn introspect_no_pin() {
    let c = ch();
    for v in 1_u8..=3 {
        assert!(c.publish(&[v; 16]).is_ok());
    }
    let mut obs = Observer::attach(&c, 0, 16);
    for _ in 0..10_000 {
        let _ = obs.sample_latest(&c);
    }
    let rec0 = c.rec(0).unwrap_or_else(|| unreachable!("rec0"));
    assert_eq!(
        rec0.hazard.load(Ordering::SeqCst).1,
        HAZARD_NONE,
        "observer holds no hazard slot"
    );

    // Hold the sole free slot on another diagnostic record; the publisher must
    // still observe exhaustion, proving echo consumed no pool capacity.
    let free = free_slot(&c);
    let rec1 = c.rec(1).unwrap_or_else(|| unreachable!("rec1"));
    let (g, _) = rec1.hazard.load(Ordering::SeqCst);
    assert!(
        rec1.hazard
            .compare_exchange(pack(g, HAZARD_NONE), pack(g, free))
    );
    assert_eq!(
        c.publish(&[9_u8; 16]),
        Err(drm_types::Status::PoolExhausted)
    );
    assert!(
        rec1.hazard
            .compare_exchange(pack(g, free), pack(g, HAZARD_NONE))
    );
    assert!(c.publish(&[10_u8; 16]).is_ok(), "publishing resumes");
}

/// Finds the first pool slot not referenced by the retained ring window.
fn free_slot(c: &Channel) -> u32 {
    let ws = c.write_seq();
    let oldest = c.oldest_retained(ws);
    let mut used = Vec::new();
    for s in oldest..ws {
        if let Ok(e) = c.ring().lookup_sc(c.ccb(), s) {
            used.push(e.slot);
        }
    }
    let pool = u32::try_from(c.pool_size()).unwrap_or(0);
    (0..pool)
        .find(|s| !used.contains(s))
        .unwrap_or_else(|| unreachable!("no free slot"))
}

/// REQ: REQ-OPS-014, REQ-OPS-003 — the log catalog filters rendered trace
/// events by severity; uncatalogued ids are dropped.
#[test]
fn log_level_filter() {
    let event = |id: u16| TraceEvent {
        id,
        pad: 0,
        args: [0; 16],
        ts_ns: u64::from(id),
    };
    let events = vec![event(0), event(1), event(2)];
    let mut catalog = LogCatalog::new();
    catalog.set(0, LogLevel::Error);
    catalog.set(1, LogLevel::Info);
    catalog.set(2, LogLevel::Trace);

    let warnings = catalog.filter(&events, LogLevel::Warn);
    assert_eq!(warnings.len(), 1, "only Error is at/above Warn");
    assert_eq!(warnings[0].id, 0);
    let info = catalog.filter(&events, LogLevel::Info);
    assert_eq!(info.len(), 2);
    let all = catalog.filter(&events, LogLevel::Trace);
    assert_eq!(all.len(), 3);
    assert_eq!(catalog.level(9), None);
}

/// REQ: REQ-OPS-003 — `Observer` copies samples and reports them as owned
/// bytes; the copy is hazard-free after every call.
#[test]
fn observer_copies_owned_bytes() {
    let c = ch();
    assert!(c.publish(&[7_u8; 16]).is_ok());
    let mut obs = Observer::attach(&c, 0, 16);
    assert_eq!(obs.sample_latest(&c), Ok(ReadOutcome::Sample));
    assert_eq!(obs.sample(), &[7_u8; 16]);
}

/// REQ: REQ-OPS-001 — `ChannelStats` defaults are zeroed for a fresh channel.
#[test]
fn fresh_channel_stats() {
    let c = ch();
    let stats: ChannelStats = diag_sample(&c);
    assert_eq!(stats.last_sequence, 0);
    assert_eq!(stats.drop_count, 0);
    assert!(stats.observed_rate_hz.abs() < f64::EPSILON);
}

/// REQ: REQ-OPS-001, REQ-OPS-002 — the aggregator samples multiple channels
/// out-of-band and renders a complete read-only report with channel, consumer,
/// and link facts (`I-OB2`).
#[test]
fn diag_aggregate_report() {
    let c1 = ch();
    let c2 = ch();
    assert!(c1.publish(&[1_u8; 16]).is_ok());
    let mut agg = Aggregator::new();
    let consumers = vec![ConsumerStats::new(1, 7, 1_000)];
    let stats = agg.sample("camera/front", consumers, &c1);
    assert_eq!(stats.last_sequence, 0, "one publish ⇒ last_sequence 0");
    agg.sample("imu", Vec::new(), &c2);
    agg.add_link(LinkStats {
        nominal_bps: 1_000_000,
        observed_bps: 900_000,
        bucket_drops: 2,
        lane_realtime_starve_events: 0,
    });
    assert_eq!(agg.channel_count(), 2);

    let report = agg.report();
    assert_eq!(report.channels.len(), 2);
    assert_eq!(report.links.len(), 1);
    let text = report.render();
    assert!(text.contains("channels: 2"), "{text}");
    assert!(text.contains("camera/front"), "{text}");
    assert!(text.contains("consumer 1 topic 7"), "{text}");
    assert!(text.contains("links: 1"), "{text}");
    assert!(text.contains("bucket_drops=2"), "{text}");

    // Resampling replaces the stored snapshot (no duplicate channel rows).
    let _ = agg.sample("camera/front", Vec::new(), &c1);
    assert_eq!(agg.channel_count(), 2);
    assert_eq!(agg.report().channels[0].consumers.len(), 0);
}

/// REQ: REQ-OPS-001, REQ-OPS-016 — I-OB4: the diag condition matrix detects
/// every architecture-defined condition — missed deadline, gap, pool
/// exhaustion, epoch change, link down — individually and together.
#[test]
fn diag_condition_matrix() {
    let base = ChannelStats {
        last_sequence: 0,
        drop_count: 0,
        overrun_count: 0,
        publisher_epoch: 0,
        pool_exhausted: 0,
        observed_rate_hz: 0.0,
        last_commit_mono_ns: None,
    };
    let clean = DiagSignals {
        prev_epoch: 0,
        stats: base,
        deadline: DeadlineState::WaitingFirst,
        link_down: false,
    };
    assert!(clean.detect().is_empty(), "no condition on a quiet channel");

    let missed = DiagSignals {
        deadline: DeadlineState::Late,
        ..clean
    };
    assert_eq!(missed.detect(), vec![Condition::MissedDeadline]);

    let gap = DiagSignals {
        stats: ChannelStats {
            overrun_count: 3,
            ..base
        },
        ..clean
    };
    assert_eq!(gap.detect(), vec![Condition::Gap]);

    let pool = DiagSignals {
        stats: ChannelStats {
            pool_exhausted: 1,
            ..base
        },
        ..clean
    };
    assert_eq!(pool.detect(), vec![Condition::PoolExhausted]);

    let epoch = DiagSignals {
        stats: ChannelStats {
            publisher_epoch: 4,
            ..base
        },
        ..clean
    };
    assert_eq!(epoch.detect(), vec![Condition::EpochChange]);

    let link = DiagSignals {
        link_down: true,
        ..clean
    };
    assert_eq!(link.detect(), vec![Condition::LinkDown]);

    let all = DiagSignals {
        prev_epoch: 0,
        stats: ChannelStats {
            overrun_count: 1,
            pool_exhausted: 1,
            publisher_epoch: 1,
            ..base
        },
        deadline: DeadlineState::Late,
        link_down: true,
    };
    assert_eq!(
        all.detect(),
        vec![
            Condition::MissedDeadline,
            Condition::Gap,
            Condition::PoolExhausted,
            Condition::EpochChange,
            Condition::LinkDown,
        ]
    );
}
