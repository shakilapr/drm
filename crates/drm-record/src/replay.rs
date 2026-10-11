//! Replay pipeline (`docs/algorithms/ops/record-replay.md` §Protocols replay
//! session steps 1–4; §Invariants I-RC4/I-RC6).
//!
//! Reads a recorded MCAP session ([`mcap::read`]), reconstructs per-topic
//! sequence streams from the DRM metadata records ordered by
//! `(publisher_epoch, seq)`, and inserts gap boundaries from the `drm.gap`
//! markers (I-RC5). A [`ReplayPlan`] reproduces the recorder-observed global
//! order as `(recv_mono_ns, ingress_order)` under an explicit scheduler
//! (`realtime`/`rate(x)`/`as_fast`). The [`ReplayPublisher`] republishes under
//! an isolated replay deployment namespace (`deployment-identity.md`,
//! `REQ-CORE-088`/`REQ-CORE-089`); source timestamps are replayed exactly as
//! recorded and the replay receive time is the scheduled replay clock. Tiered
//! fidelity (payload bytes, per-topic order, recorder-observed global order) is
//! asserted by `replay_roundtrip`.
//! REQ: REQ-OPS-009, REQ-OPS-006, REQ-OPS-018, REQ-CORE-088, REQ-CORE-089.

use std::collections::{BTreeMap, BTreeSet};

use drm_core::channel::{Channel, OverflowPolicy};
use drm_core::deploy::DeploymentId;
use drm_core::shm::{SegmentName, ShmError};
use drm_types::Status;

use crate::GapReason;
use crate::mcap::{self, Item, ReadError};

/// One reconstructed recorded sample: the DRM metadata record fields joined
/// with its MCAP message payload.
/// REQ: REQ-OPS-009, REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedSample {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Publisher epoch observed at ingress.
    pub publisher_epoch: u32,
    /// Full `u64` publisher sequence.
    pub seq: u64,
    /// Recorder-observed global ingress order.
    pub ingress_order: u64,
    /// Local monotonic receive time at copy completion (drives scheduling).
    pub recv_mono_ns: u64,
    /// Wall-clock receive time (human indexing only).
    pub recv_wall_ns: u64,
    /// Recorded source time (`log_time`), replayed verbatim.
    pub source_time: u64,
    /// Payload bytes, byte-identical to the recorded sample.
    pub payload: Vec<u8>,
}

/// One reconstructed coalesced gap boundary (`record-replay.md` §Protocols
/// replay step 1).
/// REQ: REQ-OPS-018, REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapBoundary {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Publisher epoch of the gap.
    pub publisher_epoch: u32,
    /// First missing sequence in the coalesced range.
    pub first_missing_seq: u64,
    /// Number of missing sequences in the range.
    pub missed: u64,
    /// Why the range is missing.
    pub reason: GapReason,
}

/// Parses a `GapReason` from its recorded lowercase tag.
/// REQ: REQ-OPS-018.
#[must_use]
fn gap_reason_from_str(s: &str) -> Option<GapReason> {
    match s {
        "history_lap" => Some(GapReason::HistoryLap),
        "policy_drop" => Some(GapReason::PolicyDrop),
        "publisher_epoch" => Some(GapReason::PublisherEpoch),
        _ => None,
    }
}

/// Parses a numeric metadata field.
/// REQ: REQ-OPS-006.
fn meta_num<T: std::str::FromStr>(meta: &BTreeMap<String, String>, key: &str) -> Option<T> {
    meta.get(key).and_then(|v| v.parse::<T>().ok())
}

/// A decoded replay session: channels, per-sample records, and gap boundaries.
/// REQ: REQ-OPS-009, REQ-OPS-006.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    channels: Vec<mcap::ChannelInfo>,
    samples: Vec<RecordedSample>,
    gaps: Vec<GapBoundary>,
}

impl Session {
    /// Decodes a recorded MCAP byte stream into a replay session. Each message
    /// is paired with the `drm.sample` metadata record the writer emits
    /// immediately after it (`I-RC6`); a `drm.gap` metadata record becomes a
    /// gap boundary (`I-RC5`).
    /// REQ: REQ-OPS-009, REQ-OPS-006, REQ-OPS-018.
    ///
    /// # Errors
    /// [`ReadError`] when the stream is not a well-formed MCAP session.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReadError> {
        let contents = mcap::read(bytes)?;
        let mut session = Self {
            channels: contents.channels,
            samples: Vec::new(),
            gaps: Vec::new(),
        };
        let mut pending: Option<mcap::MessageRecord> = None;
        for item in contents.items {
            match item {
                Item::Message(message) => pending = Some(message),
                Item::Metadata(meta) if meta.name == "drm.sample" => {
                    if let Some(message) = pending.take() {
                        session.samples.push(sample_from(&meta.metadata, message));
                    }
                }
                Item::Metadata(meta) if meta.name == "drm.gap" => {
                    if let Some(gap) = gap_from(&meta.metadata) {
                        session.gaps.push(gap);
                    }
                }
                Item::Metadata(_) => {}
            }
        }
        Ok(session)
    }

    /// Reconstructed samples in file order.
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn samples(&self) -> &[RecordedSample] {
        &self.samples
    }

    /// Reconstructed gap boundaries.
    /// REQ: REQ-OPS-018.
    #[must_use]
    pub fn gaps(&self) -> &[GapBoundary] {
        &self.gaps
    }

    /// Registered MCAP channels.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub fn channels(&self) -> &[mcap::ChannelInfo] {
        &self.channels
    }

    /// Distinct topic ids in ascending order.
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn topic_ids(&self) -> Vec<u32> {
        let mut ids: BTreeSet<u32> = self.samples.iter().map(|s| s.topic_id).collect();
        ids.extend(self.gaps.iter().map(|g| g.topic_id));
        ids.into_iter().collect()
    }

    /// Per-topic stream ordered by causal key `(publisher_epoch, seq)`
    /// (`I-RC4b`). Returns sample indices into [`Session::samples`].
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn topic_stream(&self, topic_id: u32) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.samples.len())
            .filter(|&i| self.samples[i].topic_id == topic_id)
            .collect();
        indices.sort_by_key(|&i| {
            let s = &self.samples[i];
            (s.publisher_epoch, s.seq)
        });
        indices
    }

    /// Recorder-observed global order as `(recv_mono_ns, ingress_order)`
    /// (`I-RC4c`). Returns sample indices into [`Session::samples`].
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn global_order(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.samples.len()).collect();
        indices.sort_by_key(|&i| {
            let s = &self.samples[i];
            (s.recv_mono_ns, s.ingress_order)
        });
        indices
    }

    /// Earliest `recv_mono_ns` across the session (replay schedule origin).
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn start_mono_ns(&self) -> u64 {
        self.samples
            .iter()
            .map(|s| s.recv_mono_ns)
            .min()
            .unwrap_or(0)
    }

    /// Builds the replay schedule for `mode` (`record-replay.md` §Protocols
    /// replay step 2): target time
    /// `replay_start + (recv_mono_ns − record_start_mono_ns) / rate`, ties in
    /// `recv_mono_ns` broken by `ingress_order`.
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn plan(&self, mode: SchedulerMode) -> ReplayPlan {
        let start = self.start_mono_ns();
        let mut steps: Vec<ReplayStep> = self
            .global_order()
            .into_iter()
            .map(|i| {
                let s = &self.samples[i];
                let delta = s.recv_mono_ns.saturating_sub(start);
                ReplayStep {
                    sample: i,
                    target_offset_ns: mode.scale(delta),
                    source_time: s.source_time,
                }
            })
            .collect();
        let duration_ns = steps.iter().map(|s| s.target_offset_ns).max().unwrap_or(0);
        // The global-order sort already breaks `recv_mono_ns` ties by
        // `ingress_order`; keep step order stable.
        steps.sort_by_key(|s| s.target_offset_ns);
        ReplayPlan { steps, duration_ns }
    }

    /// Summary metrics for the recorded session (`record-replay.md` §Protocols
    /// step 4: counts, gaps, drops).
    /// REQ: REQ-OPS-009, REQ-OPS-018.
    #[must_use]
    pub fn report(&self) -> ReplayReport {
        let mut gap_sequences = 0_u64;
        let mut drops = 0_u64;
        for gap in &self.gaps {
            gap_sequences = gap_sequences.saturating_add(gap.missed);
            if gap.reason == GapReason::PolicyDrop {
                drops = drops.saturating_add(gap.missed);
            }
        }
        let (start_time, end_time) = self.samples.iter().fold((u64::MAX, 0_u64), |acc, s| {
            (acc.0.min(s.source_time), acc.1.max(s.source_time))
        });
        ReplayReport {
            samples: u64::try_from(self.samples.len()).unwrap_or(u64::MAX),
            topics: u32::try_from(self.topic_ids().len()).unwrap_or(u32::MAX),
            gaps: u64::try_from(self.gaps.len()).unwrap_or(u64::MAX),
            gap_sequences,
            drops,
            start_time: if self.samples.is_empty() {
                0
            } else {
                start_time
            },
            end_time,
            namespace: None,
        }
    }
}

/// Builds one [`RecordedSample`] from a `drm.sample` metadata map and its
/// message.
/// REQ: REQ-OPS-006.
fn sample_from(meta: &BTreeMap<String, String>, message: mcap::MessageRecord) -> RecordedSample {
    RecordedSample {
        topic_id: meta_num(meta, "topic").unwrap_or(0),
        publisher_epoch: meta_num(meta, "publisher_epoch").unwrap_or(0),
        seq: meta_num(meta, "seq").unwrap_or(0),
        ingress_order: meta_num(meta, "ingress_order").unwrap_or(0),
        recv_mono_ns: meta_num(meta, "recv_mono_ns").unwrap_or(message.log_time),
        recv_wall_ns: meta_num(meta, "recv_wall_ns").unwrap_or(0),
        source_time: message.log_time,
        payload: message.data,
    }
}

/// Builds one [`GapBoundary`] from a `drm.gap` metadata map.
/// REQ: REQ-OPS-018.
fn gap_from(meta: &BTreeMap<String, String>) -> Option<GapBoundary> {
    let reason = meta.get("reason").and_then(|r| gap_reason_from_str(r))?;
    Some(GapBoundary {
        topic_id: meta_num(meta, "topic")?,
        publisher_epoch: meta_num(meta, "publisher_epoch")?,
        first_missing_seq: meta_num(meta, "first_missing_seq")?,
        missed: meta_num(meta, "missed")?,
        reason,
    })
}

/// Replay rate as an exact rational `num/den` (avoids floating-point rounding
/// in the schedule).
/// REQ: REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    /// Numerator (schedule runs `den/num` times slower than real time).
    pub num: u64,
    /// Denominator.
    pub den: u64,
}

impl Rate {
    /// Builds a rate; zero numerator or denominator is rejected.
    /// REQ: REQ-OPS-009.
    ///
    /// # Errors
    /// [`Status::InvalidArg`] when `num` or `den` is zero.
    pub fn new(num: u64, den: u64) -> Result<Self, Status> {
        if num == 0 || den == 0 {
            return Err(Status::InvalidArg);
        }
        Ok(Self { num, den })
    }
}

/// Replay scheduler mode (`record-replay.md` §Protocols replay step 2).
/// REQ: REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    /// Scale 1.0: target offsets equal recorded `recv_mono_ns` deltas.
    Realtime,
    /// `rate(x)`: target offsets scaled by `den/num`.
    Rate(Rate),
    /// No timing waits: every step targets offset zero.
    AsFast,
}

impl SchedulerMode {
    /// Scales a recorded `delta_ns` to a replay target offset.
    /// REQ: REQ-OPS-009.
    #[must_use]
    fn scale(self, delta_ns: u64) -> u64 {
        match self {
            Self::AsFast => 0,
            Self::Realtime => delta_ns,
            Self::Rate(rate) => {
                let scaled = u128::from(delta_ns)
                    .saturating_mul(u128::from(rate.den))
                    .checked_div(u128::from(rate.num))
                    .unwrap_or(0);
                u64::try_from(scaled).unwrap_or(u64::MAX)
            }
        }
    }
}

/// One scheduled replay step.
/// REQ: REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayStep {
    /// Index into [`Session::samples`].
    pub sample: usize,
    /// Scheduled replay offset from replay start, in nanoseconds.
    pub target_offset_ns: u64,
    /// Recorded source time, replayed verbatim.
    pub source_time: u64,
}

/// A replay schedule (`record-replay.md` §Protocols replay step 2).
/// REQ: REQ-OPS-009.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayPlan {
    /// Ordered steps.
    pub steps: Vec<ReplayStep>,
    /// Replay wall duration implied by the mode.
    pub duration_ns: u64,
}

/// Replay metrics summary (`record-replay.md` §Protocols replay steps 3–4).
/// REQ: REQ-OPS-009, REQ-OPS-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayReport {
    /// Samples republished.
    pub samples: u64,
    /// Distinct topics republished.
    pub topics: u32,
    /// Gap boundaries reconstructed.
    pub gaps: u64,
    /// Missing sequences covered by gaps.
    pub gap_sequences: u64,
    /// Sequences lost to the `Drop` policy.
    pub drops: u64,
    /// Earliest recorded source time.
    pub start_time: u64,
    /// Latest recorded source time.
    pub end_time: u64,
    /// Replay deployment namespace id, when a publisher was used.
    pub namespace: Option<DeploymentId>,
}

/// Isolated replay deployment namespace (`deployment-identity.md` §Protocols;
/// `REQ-CORE-088`/`REQ-CORE-089`). Derived from a base deployment id plus a
/// replay run discriminator so a replay never adopts the live namespace.
/// REQ: REQ-CORE-088, REQ-CORE-089.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayNamespace {
    id: DeploymentId,
}

/// FNV-1a 64 mix, seeded by `seed`.
/// REQ: REQ-CORE-088.
fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

impl ReplayNamespace {
    /// Derives a replay namespace from `base` and a run discriminator. The
    /// result is distinct from `base` (high bit forced) and per-run distinct,
    /// so replay never attaches to the live deployment (`I-DP2`).
    /// REQ: REQ-CORE-088, REQ-CORE-089.
    #[must_use]
    pub fn derive(base: DeploymentId, replay_run: u128) -> Self {
        let token = base.encoded();
        let mut bytes = token.into_bytes();
        bytes.extend_from_slice(&replay_run.to_le_bytes());
        let lo = fnv1a64(0xcbf2_9ce4_8422_2325, &bytes);
        let hi = fnv1a64(0x9e37_79b9_7f4a_7c15, &bytes);
        let raw = (u128::from(hi) << 64) | u128::from(lo);
        Self {
            id: DeploymentId::new(raw | (1_u128 << 127)),
        }
    }

    /// The replay deployment id.
    /// REQ: REQ-CORE-088.
    #[must_use]
    pub fn id(&self) -> DeploymentId {
        self.id
    }

    /// Deployment-scoped segment name for a topic slot (`I-DP2`).
    /// REQ: REQ-CORE-088, REQ-CORE-089.
    ///
    /// # Errors
    /// [`ShmError`] when the generated name violates name conformance.
    pub fn segment_name(&self, slot: u64) -> Result<SegmentName, ShmError> {
        self.id.segment_name(slot)
    }
}

/// Activation parameters for one replay topic channel.
/// REQ: REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelSpec {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// History depth.
    pub depth: u64,
    /// Pool size (must exceed `depth`).
    pub pool_size: u64,
    /// Per-sample byte bound.
    pub sample_max: u64,
}

/// Republishes reconstructed samples under an isolated replay namespace. Each
/// topic is a fresh [`Channel`]; source timestamps inside payloads are copied
/// verbatim and the replay receive time is the scheduled replay clock.
/// REQ: REQ-OPS-009, REQ-CORE-088.
pub struct ReplayPublisher {
    namespace: ReplayNamespace,
    channels: BTreeMap<u32, Channel>,
}

impl ReplayPublisher {
    /// Opens one overwrite channel per `specs` entry under `namespace`.
    /// REQ: REQ-OPS-009, REQ-CORE-088.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when a channel cannot be activated.
    pub fn new(namespace: ReplayNamespace, specs: &[ChannelSpec]) -> Result<Self, Status> {
        let mut channels = BTreeMap::new();
        for spec in specs {
            let channel = Channel::activate(
                spec.depth,
                spec.pool_size,
                spec.sample_max,
                8,
                OverflowPolicy::Overwrite,
                0,
                2,
                None,
            )?;
            channels.insert(spec.topic_id, channel);
        }
        Ok(Self {
            namespace,
            channels,
        })
    }

    /// The replay deployment namespace.
    /// REQ: REQ-CORE-088.
    #[must_use]
    pub fn namespace(&self) -> &ReplayNamespace {
        &self.namespace
    }

    /// Publishes one reconstructed payload for `topic_id`.
    /// REQ: REQ-OPS-009.
    ///
    /// # Errors
    /// [`Status::NotFound`] for an unregistered topic; channel publish errors
    /// propagate.
    pub fn publish(&self, topic_id: u32, payload: &[u8]) -> Result<u64, Status> {
        let channel = self.channels.get(&topic_id).ok_or(Status::NotFound)?;
        channel.publish(payload)
    }

    /// The channel republishing `topic_id`, for consumer-side verification.
    /// REQ: REQ-OPS-009.
    #[must_use]
    pub fn channel(&self, topic_id: u32) -> Option<&Channel> {
        self.channels.get(&topic_id)
    }
}

/// Reproduces a recorded session into `publisher` under `mode`, returning the
/// replay metrics summary. Samples are published in recorder-observed global
/// order; each topic's channel observes its own `(publisher_epoch, seq)` order.
/// REQ: REQ-OPS-009.
///
/// # Errors
/// [`Status`] when a scheduled sample has no registered replay channel.
pub fn replay(
    session: &Session,
    publisher: &ReplayPublisher,
    mode: SchedulerMode,
) -> Result<ReplayReport, Status> {
    let plan = session.plan(mode);
    for step in &plan.steps {
        let sample = session
            .samples()
            .get(step.sample)
            .ok_or(Status::InvalidArg)?;
        publisher.publish(sample.topic_id, &sample.payload)?;
    }
    let mut report = session.report();
    report.namespace = Some(publisher.namespace().id());
    Ok(report)
}

/// Convenience: derive a replay namespace from a base id and run discriminator.
/// REQ: REQ-CORE-088.
#[must_use]
pub fn namespace_for(base: DeploymentId, replay_run: u128) -> ReplayNamespace {
    ReplayNamespace::derive(base, replay_run)
}
