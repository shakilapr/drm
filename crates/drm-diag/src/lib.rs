//! Out-of-band diagnostics aggregation (`docs/algorithms/ops/observability-plane.md`).
//!
//! Layer `ops` (codebase.md 2.1): depends only on `drm-types` + `drm-core`,
//! never on another ops crate, and never executes on the data path (`I-OB2`).
//! `#![forbid(unsafe_code)]` — aggregation reads the core's safe counter and
//! borrow surfaces; the lock-free primitives stay in `drm-core`.
//!
//! REQ: REQ-OPS-001 (observed vs expected rate, per-consumer deadline state,
//! link stats), REQ-OPS-002 (never on the data path), REQ-OPS-003 (read-only
//! introspection), REQ-OPS-016 (timing modes), REQ-CORE-093 (counter surface),
//! REQ-CORE-094 (timing variant).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use drm_core::channel::ccb::ChannelCounters;
use drm_core::channel::{Channel, ReadOutcome, ReaderState};
use drm_core::trace::TraceEvent;
use drm_types::Status;

/// Channel facts read out-of-band: cumulative counters plus the derived
/// observed rate and (on timing channels) the latest commit stamp. No deadline
/// fields — deadlines are consumer properties (`observability-plane.md` §Data
/// Structures `ChannelStats`).
/// REQ: REQ-OPS-001, REQ-CORE-093.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelStats {
    /// Newest committed sequence (`write_seq - 1`).
    pub last_sequence: u64,
    /// Samples dropped inside DRM (pool exhaustion).
    pub drop_count: u64,
    /// Sequences reported missed by consumer `Gap` outcomes.
    pub overrun_count: u64,
    /// Publisher epoch.
    pub publisher_epoch: u32,
    /// Pool-exhaustion event counter.
    pub pool_exhausted: u64,
    /// Sliding-window observed publication rate (`Hz`).
    pub observed_rate_hz: f64,
    /// Newest commit monotonic stamp; `None` without timing instrumentation.
    pub last_commit_mono_ns: Option<u64>,
}

/// A read-only view over a channel's diagnostic surface, decoupling the
/// aggregator from the concrete runtime type.
/// REQ: REQ-OPS-002, REQ-OPS-003.
pub trait ChannelView {
    /// Current publisher epoch (Acquire observation).
    /// REQ: REQ-OPS-003.
    fn epoch(&self) -> u32;

    /// Cumulative counter snapshot.
    /// REQ: REQ-CORE-093.
    fn counters(&self) -> ChannelCounters;

    /// Latest commit stamp on a timing channel; `None` otherwise.
    /// REQ: REQ-CORE-094.
    fn last_commit_mono_ns(&self) -> Option<u64> {
        None
    }
}

impl ChannelView for Channel {
    fn epoch(&self) -> u32 {
        Channel::epoch(self)
    }

    fn counters(&self) -> ChannelCounters {
        Channel::counters(self)
    }
}

/// Snapshot of a channel's counters with the epoch sandwich applied (`I-OB7`):
/// read `epoch → counters → epoch`; a change mid-read discards and retries.
/// Timing fields are absent until the timing variant lands (`REQ-CORE-094`).
/// REQ: REQ-OPS-001, REQ-OPS-002.
#[must_use]
pub fn diag_sample<V: ChannelView>(view: &V) -> ChannelStats {
    const RETRIES: u32 = 8;
    let mut counters = view.counters();
    for _ in 0..RETRIES {
        let e1 = view.epoch();
        counters = view.counters();
        if view.epoch() == e1 {
            break;
        }
    }
    ChannelStats {
        last_sequence: counters.last_sequence,
        drop_count: counters.drop_count,
        overrun_count: counters.overrun_count,
        publisher_epoch: counters.publisher_epoch,
        pool_exhausted: counters.pool_exhausted,
        observed_rate_hz: 0.0,
        last_commit_mono_ns: view.last_commit_mono_ns(),
    }
}

/// Per-consumer deadline state machine (`I-OB4`):
/// `WaitingFirst → Current ⇄ Late`.
/// REQ: REQ-OPS-001, REQ-OPS-016.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineState {
    /// No commit observed yet (never-published topic, or first poll).
    WaitingFirst,
    /// A commit exists and its age is below the declared deadline.
    Current,
    /// The latest commit's age meets or exceeds the declared deadline.
    Late,
}

/// Per `(topic, consumer endpoint)` deadline tracking built from deployment
/// config (`observability-plane.md` §Data Structures `ConsumerStats`).
/// REQ: REQ-OPS-001.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerStats {
    /// Consumer endpoint identity.
    pub consumer_endpoint_id: u32,
    /// Topic identity.
    pub topic_id: u32,
    /// Declared deadline (nanoseconds); a consumer property.
    pub deadline_ns: u64,
    /// Last sequence seen by the consumer (diagnostic metadata).
    pub last_seen_seq: Option<u64>,
    /// Latest observed commit stamp (`None` until the first commit).
    pub last_commit_mono_ns: Option<u64>,
    /// Count of `Current → Late` transitions.
    pub miss_count: u64,
    /// Current deadline state.
    pub state: DeadlineState,
}

impl ConsumerStats {
    /// Builds tracking state for one consumer endpoint.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn new(consumer_endpoint_id: u32, topic_id: u32, deadline_ns: u64) -> Self {
        Self {
            consumer_endpoint_id,
            topic_id,
            deadline_ns,
            last_seen_seq: None,
            last_commit_mono_ns: None,
            miss_count: 0,
            state: DeadlineState::WaitingFirst,
        }
    }
}

/// Advances the deadline state machine one poll (`observability-plane.md`
/// §Functions `deadline_tick`). `miss_count` increments exactly once per
/// `Current → Late` transition; recovery `Late → Current` never counts.
/// Non-timing channels pass `None` and stay `WaitingFirst`.
/// REQ: REQ-OPS-001, REQ-OPS-016.
pub fn deadline_tick(c: &mut ConsumerStats, last_commit_mono_ns: Option<u64>, now_mono: u64) {
    c.last_commit_mono_ns = last_commit_mono_ns;
    match c.state {
        DeadlineState::WaitingFirst => {
            if last_commit_mono_ns.is_some() {
                c.state = DeadlineState::Current;
            }
        }
        DeadlineState::Current => {
            if let Some(t) = last_commit_mono_ns
                && now_mono.saturating_sub(t) >= c.deadline_ns
            {
                c.state = DeadlineState::Late;
                c.miss_count += 1;
            }
        }
        DeadlineState::Late => {
            if let Some(t) = last_commit_mono_ns
                && now_mono.saturating_sub(t) < c.deadline_ns
            {
                c.state = DeadlineState::Current;
            }
        }
    }
}

/// Sliding-window observed-rate estimator: `observed_rate = Δwrite_seq / Δtime`
/// over a bounded trailing window (`I-OB3`); no commit timestamps required.
/// REQ: REQ-OPS-001.
#[derive(Debug, Clone)]
pub struct RateWindow {
    samples: std::collections::VecDeque<(u64, u64)>,
    window_ns: u64,
}

impl RateWindow {
    /// Creates a window retaining samples within `window_ns` of the newest.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn new(window_ns: u64) -> Self {
        Self {
            samples: std::collections::VecDeque::new(),
            window_ns,
        }
    }

    /// Records `(now_mono, write_seq)` and returns the current observed rate.
    /// REQ: REQ-OPS-001.
    pub fn record(&mut self, now_mono: u64, write_seq: u64) -> f64 {
        if let Some(&(last_t, _)) = self.samples.back()
            && now_mono <= last_t
        {
            return self.observed_rate_hz();
        }
        self.samples.push_back((now_mono, write_seq));
        while let Some(&(t, _)) = self.samples.front() {
            if now_mono.saturating_sub(t) > self.window_ns {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        self.observed_rate_hz()
    }

    /// Observed rate over the retained window (`0.0` with fewer than two
    /// samples or a zero time delta).
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn observed_rate_hz(&self) -> f64 {
        let (Some(&(t0, s0)), Some(&(t1, s1))) = (self.samples.front(), self.samples.back()) else {
            return 0.0;
        };
        let dt = t1.saturating_sub(t0);
        if dt == 0 || s1 < s0 {
            return 0.0;
        }
        let delta = s1 - s0;
        #[allow(clippy::cast_precision_loss)]
        let rate = (delta as f64) / (dt as f64 / 1_000_000_000.0);
        rate
    }

    /// Number of retained samples.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples are retained.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Observed link statistics (router plane; reported, never used on the data
/// path). REQ: REQ-OPS-001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkStats {
    /// Nominal link capacity (bits per second).
    pub nominal_bps: u64,
    /// Observed throughput (bits per second).
    pub observed_bps: u64,
    /// Shaping token-bucket drops.
    pub bucket_drops: u64,
    /// Realtime-lane starvation events.
    pub lane_realtime_starve_events: u64,
}

/// One channel's aggregated diagnostics view: cumulative counters plus the
/// per-`(topic, consumer endpoint)` deadline states (`observability-plane.md`
/// §Data Structures).
/// REQ: REQ-OPS-001.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelAgg {
    /// Human topic/channel name.
    pub name: String,
    /// Channel facts read out-of-band.
    pub stats: ChannelStats,
    /// Per-consumer deadline tracking for this channel's topic.
    pub consumers: Vec<ConsumerStats>,
}

/// Full out-of-band report: channel stats + consumer deadlines + link stats.
/// Read-only projection (`I-OB2`); rendering never touches a data path.
/// REQ: REQ-OPS-001.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagReport {
    /// Per-channel aggregated views.
    pub channels: Vec<ChannelAgg>,
    /// Per-link stats (router plane).
    pub links: Vec<LinkStats>,
}

impl DiagReport {
    /// Renders a terse human report for `drm echo`/`hz`/`log`-style output.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "channels: {}", self.channels.len());
        for c in &self.channels {
            let _ = writeln!(
                out,
                "  {}: seq={} drop={} overrun={} pool_exhausted={} epoch={} rate={:.3}Hz commit={}",
                c.name,
                c.stats.last_sequence,
                c.stats.drop_count,
                c.stats.overrun_count,
                c.stats.pool_exhausted,
                c.stats.publisher_epoch,
                c.stats.observed_rate_hz,
                c.stats
                    .last_commit_mono_ns
                    .map_or_else(|| "-".to_string(), |t| t.to_string()),
            );
            for consumer in &c.consumers {
                let _ = writeln!(
                    out,
                    "    consumer {} topic {}: deadline={}ns state={:?} misses={}",
                    consumer.consumer_endpoint_id,
                    consumer.topic_id,
                    consumer.deadline_ns,
                    consumer.state,
                    consumer.miss_count,
                );
            }
        }
        let _ = writeln!(out, "links: {}", self.links.len());
        for l in &self.links {
            let _ = writeln!(
                out,
                "  nominal_bps={} observed_bps={} bucket_drops={} lane_starve={}",
                l.nominal_bps, l.observed_bps, l.bucket_drops, l.lane_realtime_starve_events,
            );
        }
        out
    }
}

/// Out-of-band aggregation over channels and links (`I-OB2`): `sample` reads a
/// [`ChannelView`] through [`diag_sample`] and stores the snapshot; `report`
/// freezes a read-only [`DiagReport`]. Aggregation never runs on the data path.
/// REQ: REQ-OPS-001, REQ-OPS-002.
#[derive(Debug, Default)]
pub struct Aggregator {
    channels: Vec<ChannelAgg>,
    links: Vec<LinkStats>,
}

impl Aggregator {
    /// Empty aggregator.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples one channel (epoch sandwich) and records its stats plus the
    /// supplied per-consumer deadline states, replacing any prior snapshot for
    /// `name`. Returns the freshly sampled stats.
    /// REQ: REQ-OPS-001, REQ-OPS-002.
    pub fn sample<V: ChannelView>(
        &mut self,
        name: &str,
        consumers: Vec<ConsumerStats>,
        view: &V,
    ) -> ChannelStats {
        let stats = diag_sample(view);
        if let Some(existing) = self.channels.iter_mut().find(|c| c.name == name) {
            existing.stats = stats;
            existing.consumers = consumers;
        } else {
            self.channels.push(ChannelAgg {
                name: name.to_string(),
                stats,
                consumers,
            });
        }
        stats
    }

    /// Records one link's reported stats.
    /// REQ: REQ-OPS-001.
    pub fn add_link(&mut self, link: LinkStats) {
        self.links.push(link);
    }

    /// Freezes the current aggregation into a read-only report.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn report(&self) -> DiagReport {
        DiagReport {
            channels: self.channels.clone(),
            links: self.links.clone(),
        }
    }

    /// Number of sampled channels.
    /// REQ: REQ-OPS-001.
    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

/// Architecture-defined condition detected out-of-band by diag
/// (`observability-plane.md` §Invariants I-OB4, §Test Matrix
/// `diag_deadline_matrix`).
/// REQ: REQ-OPS-001, REQ-OPS-016.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Condition {
    /// A consumer's latest commit age met or exceeded its deadline.
    MissedDeadline,
    /// A history lap was reported by a consumer `Gap` outcome.
    Gap,
    /// The channel's payload pool reported exhaustion.
    PoolExhausted,
    /// The publisher epoch changed between polls (restart).
    EpochChange,
    /// A monitored link is down.
    LinkDown,
}

/// Injected signals for one diag poll, mapped to [`Condition`]s by
/// [`DiagSignals::detect`]. This is the detection surface the deadline matrix
/// exercises (`I-OB4`).
/// REQ: REQ-OPS-001, REQ-OPS-016.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiagSignals {
    /// Publisher epoch observed at the previous poll.
    pub prev_epoch: u32,
    /// Current channel stats snapshot.
    pub stats: ChannelStats,
    /// Current per-consumer deadline state.
    pub deadline: DeadlineState,
    /// Whether the monitored link is down.
    pub link_down: bool,
}

impl DiagSignals {
    /// Detects every active condition, in the fixed order
    /// `MissedDeadline, Gap, PoolExhausted, EpochChange, LinkDown`. `expected_rate`
    /// alone never enables timing; detection is observational (`I-OB4`).
    /// REQ: REQ-OPS-001, REQ-OPS-016.
    #[must_use]
    pub fn detect(&self) -> Vec<Condition> {
        let mut out = Vec::new();
        if self.deadline == DeadlineState::Late {
            out.push(Condition::MissedDeadline);
        }
        if self.stats.overrun_count > 0 {
            out.push(Condition::Gap);
        }
        if self.stats.pool_exhausted > 0 {
            out.push(Condition::PoolExhausted);
        }
        if self.stats.publisher_epoch != self.prev_epoch {
            out.push(Condition::EpochChange);
        }
        if self.link_down {
            out.push(Condition::LinkDown);
        }
        out
    }
}

/// Non-publishing introspection observer (`I-OB5`): owns a reader cursor and a
/// private copy buffer; every read is a momentary borrow copied out and
/// released immediately. Holds zero hazard slots between calls.
/// REQ: REQ-OPS-003.
pub struct Observer {
    state: ReaderState,
    buf: Vec<u8>,
    len: usize,
    rec_index: usize,
}

impl Observer {
    /// Builds an observer over a diagnostic consumer record with a private
    /// `sample_max`-byte buffer.
    /// REQ: REQ-OPS-003.
    #[must_use]
    pub fn attach(ch: &Channel, rec_index: usize, sample_max: usize) -> Self {
        Self {
            state: ch.reader(),
            buf: vec![0_u8; sample_max],
            len: sample_max,
            rec_index,
        }
    }

    /// Momentary `copy_latest`: borrow → copy → release, all within the call.
    /// REQ: REQ-OPS-003.
    ///
    /// # Errors
    /// As [`Channel::latest`].
    pub fn sample_latest(&mut self, ch: &Channel) -> Result<ReadOutcome, Status> {
        ch.latest(&mut self.state, self.rec_index, &mut self.buf)
    }

    /// The bytes copied by the most recent successful read (the declared
    /// payload bound for a fixed-size channel).
    /// REQ: REQ-OPS-003.
    #[must_use]
    pub fn sample(&self) -> &[u8] {
        &self.buf[..self.len.min(self.buf.len())]
    }
}

/// Severity for the application log view (`drm log`). REQ: REQ-OPS-003.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Error severity.
    Error,
    /// Warning severity.
    Warn,
    /// Informational.
    Info,
    /// Debug detail.
    Debug,
    /// Verbose trace.
    Trace,
}

/// Maps numeric event ids to log levels so `drm log` can filter the trace ring
/// (`observability-plane.md` §Protocols compiled trace pipeline).
/// REQ: REQ-OPS-014, REQ-OPS-003.
#[derive(Debug, Clone, Default)]
pub struct LogCatalog {
    levels: BTreeMap<u16, LogLevel>,
}

impl LogCatalog {
    /// Empty catalog.
    /// REQ: REQ-OPS-003.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Associates an event id with a level (last write wins).
    /// REQ: REQ-OPS-003.
    pub fn set(&mut self, id: u16, level: LogLevel) {
        self.levels.insert(id, level);
    }

    /// Level for an event id, if catalogued.
    /// REQ: REQ-OPS-003.
    #[must_use]
    pub fn level(&self, id: u16) -> Option<LogLevel> {
        self.levels.get(&id).copied()
    }

    /// Filters events to those at or above `min`; uncatalogued ids are dropped.
    /// REQ: REQ-OPS-003.
    #[must_use]
    pub fn filter<'a>(&self, events: &'a [TraceEvent], min: LogLevel) -> Vec<&'a TraceEvent> {
        events
            .iter()
            .filter(|e| self.level(e.id).is_some_and(|l| l <= min))
            .collect()
    }
}

#[cfg(test)]
mod tests;
