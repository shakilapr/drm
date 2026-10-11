//! Recorder disk stage (`docs/algorithms/ops/record-replay.md` §Functions
//! `disk_thread`, §Protocols record session steps 3–4).
//!
//! Drains the recorder's descriptor ring into an MCAP stream, observing
//! recorder-owned memory exclusively — the stage holds no DRM handle, so disk
//! work can never extend a borrow (`I-RC1`, `REQ-OPS-007`). Per-sample DRM
//! metadata records carry the full `(publisher_epoch, seq, ingress_order)`
//! (`I-RC6`); coalesced `DrmGap` markers are drained from the per-topic
//! ledgers at session stop (`I-RC5`). Slow disks simply drain fewer
//! descriptors per pump, letting the configured `on_full` policy act (`I-RC3`).
//! REQ: REQ-OPS-006, REQ-OPS-007, REQ-OPS-008, REQ-OPS-009.

use std::io::Write;

use crate::Recorder;
use crate::mcap::{self, Compression, GapMeta, SampleMeta, WriteError};

/// Disk-stage configuration.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Target uncompressed MCAP chunk size in bytes.
    pub chunk_target: u64,
    /// Chunk compression.
    pub compression: Compression,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            chunk_target: 1 << 20,
            compression: Compression::None,
        }
    }
}

/// Session summary written at stop (`record-replay.md` §Protocols step 4:
/// counts, gaps, drops, `gap_detail_lost`).
/// REQ: REQ-OPS-006, REQ-OPS-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSummary {
    /// Messages written.
    pub messages: u64,
    /// MCAP chunks written.
    pub chunks: u32,
    /// Coalesced `DrmGap` records written.
    pub gaps: u64,
    /// Missing sequences covered by those gap records.
    pub gap_sequences: u64,
    /// Sequences dropped by the `Drop` policy (`I-RC3`).
    pub dropped: u64,
    /// Whether a gap ledger overflowed (`gap_detail_lost`; `I-RC5`).
    pub gap_detail_lost: bool,
    /// Total bytes written to the sink.
    pub bytes: u64,
    /// Earliest message log time.
    pub start_time: u64,
    /// Latest message log time.
    pub end_time: u64,
    /// Whether chunks were compressed.
    pub compressed: bool,
}

/// The disk stage: an MCAP writer plus reusable scratch, owning no DRM handle.
/// REQ: REQ-OPS-007, REQ-OPS-009.
pub struct Stage<W: Write> {
    writer: mcap::Writer<W>,
    scratch: Vec<u8>,
    drained: u64,
    gaps_written: u64,
    gap_sequences: u64,
    dropped: u64,
    compressed: bool,
}

impl<W: Write> Stage<W> {
    /// Opens `sink` and registers every `(topic_id, topic_name)` as an MCAP
    /// channel; all channels precede any sample. No DRM handle is stored
    /// (`I-RC1`).
    /// REQ: REQ-OPS-006, REQ-OPS-007.
    ///
    /// # Errors
    /// [`WriteError::Io`] when the header or channel records cannot be written.
    pub fn new(sink: W, cfg: &Config, topics: &[(u32, &str)]) -> Result<Self, WriteError> {
        let opts = mcap::Options {
            chunk_target: cfg.chunk_target,
            compression: cfg.compression,
            ..mcap::Options::default()
        };
        let mut writer = mcap::Writer::new(sink, opts)?;
        for (topic_id, name) in topics {
            writer.add_channel(*topic_id, name, "application/octet-stream")?;
        }
        Ok(Self {
            writer,
            scratch: Vec::new(),
            drained: 0,
            gaps_written: 0,
            gap_sequences: 0,
            dropped: 0,
            compressed: cfg.compression != Compression::None,
        })
    }

    /// Drains up to `max` descriptors from `rec` into MCAP, copying each
    /// recorder-owned payload before freeing its slot. `max` bounds the work
    /// per pump so a slow disk drains slowly and the ingress policy acts
    /// (`I-RC3`). Recorder memory only (`I-RC1`).
    /// REQ: REQ-OPS-006, REQ-OPS-007.
    ///
    /// # Errors
    /// [`WriteError`] when the MCAP sink fails.
    pub fn pump(&mut self, rec: &mut Recorder, max: usize) -> Result<usize, WriteError> {
        let mut count = 0;
        while count < max {
            let Some(desc) = rec.pop_descriptor() else {
                break;
            };
            self.scratch.clear();
            if let Some(payload) = rec.slot_payload(&desc) {
                self.scratch.extend_from_slice(payload);
            }
            let meta = SampleMeta {
                topic_id: desc.topic_id,
                publisher_epoch: desc.publisher_epoch,
                seq: desc.seq,
                ingress_order: desc.ingress_order,
                recv_mono_ns: desc.recv_mono_ns,
                recv_wall_ns: desc.recv_wall_ns,
                log_time: desc.recv_mono_ns,
            };
            self.writer.write_sample(&meta, &self.scratch)?;
            rec.free_descriptor(&desc);
            self.drained = self.drained.saturating_add(1);
            count += 1;
        }
        Ok(count)
    }

    /// Drains every remaining descriptor, writes the coalesced gap markers,
    /// flushes the MCAP footer/summary, and returns the session summary plus
    /// the sink. No DRM handle is read (`I-RC1`).
    /// REQ: REQ-OPS-006, REQ-OPS-007, REQ-OPS-008, REQ-OPS-009.
    ///
    /// # Errors
    /// [`WriteError`] when the MCAP sink fails.
    pub fn finish(mut self, rec: &mut Recorder) -> Result<(SessionSummary, W), WriteError> {
        let _ = self.pump(rec, usize::MAX)?;
        for (topic_id, epoch, gap) in rec.gaps() {
            let missed = gap.last_seq.saturating_sub(gap.first_seq).saturating_add(1);
            let meta = GapMeta {
                topic_id,
                publisher_epoch: epoch,
                first_missing_seq: gap.first_seq,
                missed,
                reason: gap.reason.as_str(),
            };
            self.writer.write_gap(&meta)?;
            self.gaps_written = self.gaps_written.saturating_add(1);
            self.gap_sequences = self.gap_sequences.saturating_add(missed);
            if gap.reason == crate::GapReason::PolicyDrop {
                self.dropped = self.dropped.saturating_add(missed);
            }
        }
        let (mcap_summary, sink) = self.writer.finish()?;
        let summary = SessionSummary {
            messages: mcap_summary.message_count,
            chunks: mcap_summary.chunk_count,
            gaps: self.gaps_written,
            gap_sequences: self.gap_sequences,
            dropped: self.dropped,
            gap_detail_lost: rec.gap_detail_lost(),
            bytes: mcap_summary.bytes_written,
            start_time: mcap_summary.message_start_time,
            end_time: mcap_summary.message_end_time,
            compressed: self.compressed,
        };
        Ok((summary, sink))
    }

    /// Descriptors drained so far.
    /// REQ: REQ-OPS-007.
    #[must_use]
    pub fn drained(&self) -> u64 {
        self.drained
    }
}
