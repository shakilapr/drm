//! From-scratch MCAP writer and reader for the record/replay pipeline
//! (`docs/algorithms/ops/record-replay.md` §Protocols record session steps 3–4
//! and replay session steps 1–2, §Invariants I-RC6).
//!
//! The writer frames standard MCAP records — header, schema, channel, message,
//! chunk, statistics, chunk index, summary offset, footer — so a third-party
//! MCAP reader parses the output, and embeds DRM metadata records carrying
//! `(publisher_epoch, seq: u64, ingress_order)` plus the receive-time pair and
//! coalesced `DrmGap` markers. Chunks are optionally zstd-compressed. The
//! writer never touches a DRM handle (I-RC1). The reader parses the same record
//! subset so the replay stage reconstructs per-topic streams and gap
//! boundaries from recorded files alone.
//! REQ: REQ-OPS-006, REQ-OPS-009.

use std::collections::BTreeMap;
use std::io::Write;

/// MCAP file magic (`\x89MCAP0\r\n`).
/// REQ: REQ-OPS-006.
pub const MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', 0x30, 0x0D, 0x0A];

const OP_HEADER: u8 = 0x01;
const OP_FOOTER: u8 = 0x02;
const OP_SCHEMA: u8 = 0x03;
const OP_CHANNEL: u8 = 0x04;
const OP_MESSAGE: u8 = 0x05;
const OP_CHUNK: u8 = 0x06;
const OP_CHUNK_INDEX: u8 = 0x08;
const OP_STATISTICS: u8 = 0x0B;
const OP_METADATA: u8 = 0x0C;
const OP_SUMMARY_OFFSET: u8 = 0x0E;
const OP_DATA_END: u8 = 0x0F;

/// Chunk compression (`record-replay.md` §Protocols: zstd optional).
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Uncompressed chunks (`compression = ""`).
    None,
    /// zstd-compressed chunks (`compression = "zstd"`).
    Zstd,
}

impl Compression {
    /// MCAP `compression` string.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Zstd => "zstd",
        }
    }

    /// Parses an MCAP `compression` string; unknown names are rejected.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "" => Some(Self::None),
            "zstd" => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// MCAP writer options.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone)]
pub struct Options {
    /// Target uncompressed chunk size in bytes (`RecorderConfig::chunk_target`).
    pub chunk_target: u64,
    /// Chunk compression.
    pub compression: Compression,
    /// MCAP header profile string.
    pub profile: String,
    /// MCAP header library string.
    pub library: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            chunk_target: 1 << 20,
            compression: Compression::None,
            profile: "drm".to_string(),
            library: format!("drm-record {}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// DRM per-sample metadata record content (`record-replay.md` §Invariants I-RC6).
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleMeta {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Publisher epoch observed at ingress.
    pub publisher_epoch: u32,
    /// Full `u64` publisher sequence.
    pub seq: u64,
    /// Recorder-observed global ingress order.
    pub ingress_order: u64,
    /// Local monotonic receive time at copy completion.
    pub recv_mono_ns: u64,
    /// Wall-clock receive time (human indexing only).
    pub recv_wall_ns: u64,
    /// MCAP message `log_time`/`publish_time` (source time when declared).
    pub log_time: u64,
}

/// One coalesced `DrmGap` marker (`record-replay.md` §Protocols step 3).
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapMeta {
    /// Runtime topic identity.
    pub topic_id: u32,
    /// Publisher epoch of the gap.
    pub publisher_epoch: u32,
    /// First missing sequence in the coalesced range.
    pub first_missing_seq: u64,
    /// Number of missing sequences in the range.
    pub missed: u64,
    /// Gap reason (`HistoryLap`/`PolicyDrop`/`PublisherEpoch`).
    pub reason: &'static str,
}

/// MCAP session summary returned by [`Writer::finish`].
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Recorded message count.
    pub message_count: u64,
    /// Chunk count.
    pub chunk_count: u32,
    /// Schema count.
    pub schema_count: u16,
    /// Channel count.
    pub channel_count: u32,
    /// DRM metadata record count (per-sample + gaps).
    pub metadata_count: u32,
    /// Total bytes written to the sink.
    pub bytes_written: u64,
    /// Earliest message log time.
    pub message_start_time: u64,
    /// Latest message log time.
    pub message_end_time: u64,
}

/// Disk-writer failure (`record-replay.md` §Edge Cases: `RECORDING_FAILED
/// reason: IO_ERROR`; no silent truncation).
/// REQ: REQ-OPS-006.
#[derive(Debug)]
pub enum WriteError {
    /// Sink write or compression failure (disk full, broken pipe).
    Io(std::io::Error),
    /// No channel is registered for the descriptor's topic.
    UnknownTopic(u32),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "mcap io error: {e}"),
            Self::UnknownTopic(t) => write!(f, "mcap unknown topic {t}"),
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::UnknownTopic(_) => None,
        }
    }
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// MCAP read/parse failure. Malformed input is rejected, never panicked.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Truncated or structurally invalid record stream.
    Malformed(&'static str),
    /// Chunk compression name is not one this reader implements.
    UnsupportedCompression,
    /// zstd decompression failed.
    Zstd(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "mcap malformed: {m}"),
            Self::UnsupportedCompression => write!(f, "mcap unsupported compression"),
            Self::Zstd(m) => write!(f, "mcap zstd: {m}"),
        }
    }
}

impl std::error::Error for ReadError {}

/// One registered MCAP channel (a topic).
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    /// Channel id.
    pub id: u16,
    /// Topic name.
    pub topic: String,
    /// Message encoding string.
    pub message_encoding: String,
}

/// One parsed MCAP message.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecord {
    /// Channel id this message belongs to.
    pub channel_id: u16,
    /// Advisory 32-bit sequence (I-RC6).
    pub sequence: u32,
    /// Log time.
    pub log_time: u64,
    /// Publish time.
    pub publish_time: u64,
    /// Payload bytes.
    pub data: Vec<u8>,
}

/// One parsed MCAP metadata record.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataRecord {
    /// Record name (`drm.sample`/`drm.gap`).
    pub name: String,
    /// Key/value fields.
    pub metadata: BTreeMap<String, String>,
}

/// A parsed record in data order: a message or a metadata marker. The writer
/// emits each message immediately followed by its `drm.sample` metadata, so a
/// reader pairs them positionally.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A recorded message.
    Message(MessageRecord),
    /// A metadata record.
    Metadata(MetadataRecord),
}

/// Everything a reader recovers from one MCAP session.
/// REQ: REQ-OPS-006.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    /// Registered channels in registration order.
    pub channels: Vec<ChannelInfo>,
    /// Data records in file order.
    pub items: Vec<Item>,
}

/// Widens a `usize` length to `u64` (64-bit hosts are lossless).
/// REQ: REQ-OPS-006.
fn len_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_u32(buf, u32::try_from(s.len()).unwrap_or(u32::MAX));
    buf.extend_from_slice(s.as_bytes());
}

fn put_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    put_u32(buf, u32::try_from(b.len()).unwrap_or(u32::MAX));
    buf.extend_from_slice(b);
}

fn put_str_map(buf: &mut Vec<u8>, map: &BTreeMap<String, String>) {
    let mut body = Vec::new();
    for (k, v) in map {
        put_str(&mut body, k);
        put_str(&mut body, v);
    }
    put_u32(buf, u32::try_from(body.len()).unwrap_or(u32::MAX));
    buf.extend_from_slice(&body);
}

fn put_u16_u64_map(buf: &mut Vec<u8>, map: &BTreeMap<u16, u64>) {
    put_u32(
        buf,
        u32::try_from(map.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(10),
    );
    for (k, v) in map {
        put_u16(buf, *k);
        put_u64(buf, *v);
    }
}

fn append_record(buf: &mut Vec<u8>, op: u8, body: &[u8]) {
    buf.push(op);
    put_u64(buf, len_u64(body.len()));
    buf.extend_from_slice(body);
}

fn record(op: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + body.len());
    append_record(&mut out, op, body);
    out
}

fn sample_map(meta: &SampleMeta) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("topic".to_string(), meta.topic_id.to_string()),
        (
            "publisher_epoch".to_string(),
            meta.publisher_epoch.to_string(),
        ),
        ("seq".to_string(), meta.seq.to_string()),
        ("ingress_order".to_string(), meta.ingress_order.to_string()),
        ("recv_mono_ns".to_string(), meta.recv_mono_ns.to_string()),
        ("recv_wall_ns".to_string(), meta.recv_wall_ns.to_string()),
    ])
}

fn gap_map(meta: &GapMeta) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("topic".to_string(), meta.topic_id.to_string()),
        (
            "publisher_epoch".to_string(),
            meta.publisher_epoch.to_string(),
        ),
        (
            "first_missing_seq".to_string(),
            meta.first_missing_seq.to_string(),
        ),
        ("missed".to_string(), meta.missed.to_string()),
        ("reason".to_string(), meta.reason.to_string()),
    ])
}

#[derive(Debug, Default, Clone)]
struct Stats {
    message_count: u64,
    schema_count: u16,
    channel_count: u32,
    metadata_count: u32,
    chunk_count: u32,
    message_start_time: u64,
    message_end_time: u64,
    channel_message_counts: BTreeMap<u16, u64>,
}

impl Stats {
    fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        put_u64(&mut body, self.message_count);
        put_u16(&mut body, self.schema_count);
        put_u32(&mut body, self.channel_count);
        put_u32(&mut body, 0); // attachment_count
        put_u32(&mut body, self.metadata_count);
        put_u32(&mut body, self.chunk_count);
        put_u64(&mut body, self.message_start_time);
        put_u64(&mut body, self.message_end_time);
        put_u16_u64_map(&mut body, &self.channel_message_counts);
        body
    }
}

#[derive(Debug, Clone)]
struct ChannelRow {
    topic_id: u32,
    id: u16,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ChunkIndex {
    start: u64,
    end: u64,
    chunk_start_offset: u64,
    chunk_length: u64,
    compression: &'static str,
    compressed: u64,
    uncompressed: u64,
}

impl ChunkIndex {
    fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        put_u64(&mut body, self.start);
        put_u64(&mut body, self.end);
        put_u64(&mut body, self.chunk_start_offset);
        put_u64(&mut body, self.chunk_length);
        put_u32(&mut body, 0); // message_index_offsets (empty map)
        put_u64(&mut body, 0); // message_index_length
        put_str(&mut body, self.compression);
        put_u64(&mut body, self.compressed);
        put_u64(&mut body, self.uncompressed);
        body
    }
}

/// Streaming MCAP writer over a byte sink. All record framing is produced here;
/// the writer observes only recorder-owned bytes and never a DRM handle (I-RC1).
/// REQ: REQ-OPS-006, REQ-OPS-009.
pub struct Writer<W: Write> {
    sink: W,
    offset: u64,
    opts: Options,
    chunk: Vec<u8>,
    chunk_bounds: Option<(u64, u64)>,
    schemas: Vec<Vec<u8>>,
    channels: Vec<ChannelRow>,
    chunk_indexes: Vec<ChunkIndex>,
    stats: Stats,
}

impl<W: Write> Writer<W> {
    /// Opens `sink` and writes the MCAP magic plus header record.
    /// REQ: REQ-OPS-006.
    ///
    /// # Errors
    /// [`WriteError::Io`] when the header cannot be written.
    pub fn new(mut sink: W, opts: Options) -> Result<Self, WriteError> {
        sink.write_all(&MAGIC)?;
        let mut body = Vec::new();
        put_str(&mut body, &opts.profile);
        put_str(&mut body, &opts.library);
        let rec = record(OP_HEADER, &body);
        sink.write_all(&rec)?;
        Ok(Self {
            sink,
            offset: len_u64(MAGIC.len()) + len_u64(rec.len()),
            opts,
            chunk: Vec::new(),
            chunk_bounds: None,
            schemas: Vec::new(),
            channels: Vec::new(),
            chunk_indexes: Vec::new(),
            stats: Stats::default(),
        })
    }

    /// Registers a topic as one MCAP schema + channel; must precede samples for
    /// that topic. Flushes the open chunk so the record lands in the data
    /// section before any message references it.
    /// REQ: REQ-OPS-006.
    ///
    /// # Errors
    /// [`WriteError::Io`] when the schema/channel records cannot be written.
    pub fn add_channel(
        &mut self,
        topic_id: u32,
        topic: &str,
        message_encoding: &str,
    ) -> Result<(), WriteError> {
        self.flush_chunk()?;
        let schema_id = u16::try_from(self.schemas.len() + 1).unwrap_or(u16::MAX);
        let channel_id = u16::try_from(self.channels.len() + 1).unwrap_or(u16::MAX);
        let mut schema_body = Vec::new();
        put_u16(&mut schema_body, schema_id);
        put_str(&mut schema_body, topic);
        put_str(&mut schema_body, "drm.raw");
        put_bytes(&mut schema_body, &[]);
        let mut channel_body = Vec::new();
        put_u16(&mut channel_body, channel_id);
        put_u16(&mut channel_body, schema_id);
        put_str(&mut channel_body, topic);
        put_str(&mut channel_body, message_encoding);
        put_str_map(&mut channel_body, &BTreeMap::new());
        let schema_rec = record(OP_SCHEMA, &schema_body);
        let channel_rec = record(OP_CHANNEL, &channel_body);
        self.write_raw(&schema_rec)?;
        self.write_raw(&channel_rec)?;
        self.schemas.push(schema_body);
        self.channels.push(ChannelRow {
            topic_id,
            id: channel_id,
            body: channel_body,
        });
        self.stats.schema_count = self.stats.schema_count.saturating_add(1);
        self.stats.channel_count = self.stats.channel_count.saturating_add(1);
        Ok(())
    }

    /// Channel id registered for `topic_id`.
    /// REQ: REQ-OPS-006.
    #[must_use]
    pub fn channel_for(&self, topic_id: u32) -> Option<u16> {
        self.channels
            .iter()
            .find(|c| c.topic_id == topic_id)
            .map(|c| c.id)
    }

    /// Appends one message plus its `drm.sample` metadata record to the open
    /// chunk, flushing when the target chunk size is reached.
    /// REQ: REQ-OPS-006, REQ-OPS-009.
    ///
    /// # Errors
    /// [`WriteError::UnknownTopic`] for an unregistered topic;
    /// [`WriteError::Io`] when a chunk flush fails.
    pub fn write_sample(&mut self, meta: &SampleMeta, payload: &[u8]) -> Result<(), WriteError> {
        let channel_id = self
            .channel_for(meta.topic_id)
            .ok_or(WriteError::UnknownTopic(meta.topic_id))?;
        let mut message = Vec::with_capacity(28 + payload.len());
        put_u16(&mut message, channel_id);
        put_u32(
            &mut message,
            u32::try_from(meta.seq & 0xFFFF_FFFF).unwrap_or(u32::MAX),
        );
        put_u64(&mut message, meta.log_time);
        put_u64(&mut message, meta.log_time);
        // MCAP `Message.data` is the record remainder, not length-prefixed.
        message.extend_from_slice(payload);
        append_record(&mut self.chunk, OP_MESSAGE, &message);
        let mut meta_body = Vec::new();
        put_str(&mut meta_body, "drm.sample");
        put_str_map(&mut meta_body, &sample_map(meta));
        append_record(&mut self.chunk, OP_METADATA, &meta_body);

        self.chunk_bounds = Some(match self.chunk_bounds {
            None => (meta.log_time, meta.log_time),
            Some((s, e)) => (s.min(meta.log_time), e.max(meta.log_time)),
        });
        if self.stats.message_count == 0 {
            self.stats.message_start_time = meta.log_time;
        } else {
            self.stats.message_start_time = self.stats.message_start_time.min(meta.log_time);
        }
        self.stats.message_end_time = self.stats.message_end_time.max(meta.log_time);
        self.stats.message_count = self.stats.message_count.saturating_add(1);
        self.stats.metadata_count = self.stats.metadata_count.saturating_add(1);
        let count = self
            .stats
            .channel_message_counts
            .entry(channel_id)
            .or_insert(0);
        *count = count.saturating_add(1);
        self.maybe_flush()
    }

    /// Appends one coalesced `drm.gap` metadata record to the open chunk.
    /// REQ: REQ-OPS-006.
    ///
    /// # Errors
    /// [`WriteError::Io`] when a chunk flush fails.
    pub fn write_gap(&mut self, meta: &GapMeta) -> Result<(), WriteError> {
        let mut body = Vec::new();
        put_str(&mut body, "drm.gap");
        put_str_map(&mut body, &gap_map(meta));
        append_record(&mut self.chunk, OP_METADATA, &body);
        self.stats.metadata_count = self.stats.metadata_count.saturating_add(1);
        self.maybe_flush()
    }

    fn maybe_flush(&mut self) -> Result<(), WriteError> {
        if len_u64(self.chunk.len()) >= self.opts.chunk_target {
            self.flush_chunk()?;
        }
        Ok(())
    }

    /// Writes the open chunk (optionally compressed) and records its index.
    /// REQ: REQ-OPS-006.
    ///
    /// # Errors
    /// [`WriteError::Io`] when compression or the sink write fails.
    pub fn flush_chunk(&mut self) -> Result<(), WriteError> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let uncompressed = std::mem::take(&mut self.chunk);
        let uncompressed_size = len_u64(uncompressed.len());
        let (name, data) = match self.opts.compression {
            Compression::None => (Compression::None, uncompressed),
            Compression::Zstd => {
                let compressed =
                    zstd::stream::encode_all(uncompressed.as_slice(), 0).map_err(WriteError::Io)?;
                (Compression::Zstd, compressed)
            }
        };
        let (start, end) = self.chunk_bounds.take().unwrap_or((0, 0));
        let chunk_start_offset = self.offset;
        let mut body = Vec::new();
        put_u64(&mut body, start);
        put_u64(&mut body, end);
        put_u64(&mut body, uncompressed_size);
        put_u32(&mut body, 0); // uncompressed_crc omitted (0 = not computed)
        put_str(&mut body, name.name());
        put_u64(&mut body, len_u64(data.len()));
        body.extend_from_slice(&data);
        let rec = record(OP_CHUNK, &body);
        self.write_raw(&rec)?;
        self.chunk_indexes.push(ChunkIndex {
            start,
            end,
            chunk_start_offset,
            chunk_length: len_u64(rec.len()),
            compression: name.name(),
            compressed: len_u64(data.len()),
            uncompressed: uncompressed_size,
        });
        self.stats.chunk_count = self.stats.chunk_count.saturating_add(1);
        Ok(())
    }

    /// Flushes the open chunk, writes `DataEnd`, the summary section, the
    /// footer, and the closing magic; returns the session summary and the sink.
    /// REQ: REQ-OPS-006, REQ-OPS-009.
    ///
    /// # Errors
    /// [`WriteError::Io`] when the sink write fails.
    pub fn finish(mut self) -> Result<(Summary, W), WriteError> {
        self.flush_chunk()?;
        let mut end = Vec::new();
        put_u32(&mut end, 0); // data_section_crc omitted (0 = not computed)
        let end_rec = record(OP_DATA_END, &end);
        self.write_raw(&end_rec)?;

        let summary_start = self.offset;
        let schema_recs: Vec<Vec<u8>> = self
            .schemas
            .iter()
            .map(|body| record(OP_SCHEMA, body))
            .collect();
        let channel_recs: Vec<Vec<u8>> = self
            .channels
            .iter()
            .map(|channel| record(OP_CHANNEL, &channel.body))
            .collect();
        let stats_rec = record(OP_STATISTICS, &self.stats.encode());
        let index_recs: Vec<Vec<u8>> = self
            .chunk_indexes
            .iter()
            .map(|index| record(OP_CHUNK_INDEX, &index.encode()))
            .collect();

        let schema_start = self.offset;
        for rec in &schema_recs {
            self.write_raw(rec)?;
        }
        let schema_len = self.offset.saturating_sub(schema_start);
        let channel_start = self.offset;
        for rec in &channel_recs {
            self.write_raw(rec)?;
        }
        let channel_len = self.offset.saturating_sub(channel_start);
        let stats_start = self.offset;
        self.write_raw(&stats_rec)?;
        let stats_len = self.offset.saturating_sub(stats_start);
        let index_start = self.offset;
        for rec in &index_recs {
            self.write_raw(rec)?;
        }
        let index_len = self.offset.saturating_sub(index_start);

        let summary_offset_start = self.offset;
        let groups = [
            (OP_SCHEMA, schema_start, schema_len),
            (OP_CHANNEL, channel_start, channel_len),
            (OP_STATISTICS, stats_start, stats_len),
            (OP_CHUNK_INDEX, index_start, index_len),
        ];
        for (op, start, len) in groups {
            let mut body = Vec::new();
            body.push(op);
            put_u64(&mut body, start);
            put_u64(&mut body, len);
            let rec = record(OP_SUMMARY_OFFSET, &body);
            self.write_raw(&rec)?;
        }

        let mut footer = Vec::new();
        put_u64(&mut footer, summary_start);
        put_u64(&mut footer, summary_offset_start);
        put_u32(&mut footer, 0); // summary_crc omitted (0 = not computed)
        let footer_rec = record(OP_FOOTER, &footer);
        self.write_raw(&footer_rec)?;
        self.sink.write_all(&MAGIC)?;
        self.offset = self.offset.saturating_add(len_u64(MAGIC.len()));

        let summary = Summary {
            message_count: self.stats.message_count,
            chunk_count: self.stats.chunk_count,
            schema_count: self.stats.schema_count,
            channel_count: self.stats.channel_count,
            metadata_count: self.stats.metadata_count,
            bytes_written: self.offset,
            message_start_time: self.stats.message_start_time,
            message_end_time: self.stats.message_end_time,
        };
        Ok((summary, self.sink))
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        self.sink.write_all(bytes)?;
        self.offset = self.offset.saturating_add(len_u64(bytes.len()));
        Ok(())
    }
}

/// Bounds-checked little-endian cursor over an MCAP record stream. Every read
/// returns `None` on truncation so a malformed file never panics.
/// REQ: REQ-OPS-006.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn u16(&mut self) -> Option<u16> {
        let bytes: [u8; 2] = self.take(2)?.try_into().ok()?;
        Some(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Option<u64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u32()?).ok()?;
        self.take(len)
    }

    fn str(&mut self) -> Option<String> {
        let bytes = self.bytes()?;
        std::str::from_utf8(bytes).ok().map(str::to_string)
    }

    fn map(&mut self) -> Option<BTreeMap<String, String>> {
        let len = usize::try_from(self.u32()?).ok()?;
        let body = self.take(len)?;
        let mut inner = Cursor::new(body);
        let mut map = BTreeMap::new();
        while inner.pos < body.len() {
            let key = inner.str()?;
            let value = inner.str()?;
            map.insert(key, value);
        }
        Some(map)
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
}

/// Parses one message body.
/// REQ: REQ-OPS-006.
fn parse_message(body: &[u8]) -> Result<MessageRecord, ReadError> {
    let mut c = Cursor::new(body);
    let channel_id = c.u16().ok_or(ReadError::Malformed("message channel"))?;
    let sequence = c.u32().ok_or(ReadError::Malformed("message sequence"))?;
    let log_time = c.u64().ok_or(ReadError::Malformed("message log_time"))?;
    let publish_time = c
        .u64()
        .ok_or(ReadError::Malformed("message publish_time"))?;
    // MCAP `Message.data` is the record remainder, not length-prefixed.
    let data = c
        .take(c.remaining())
        .map(<[u8]>::to_vec)
        .ok_or(ReadError::Malformed("message data"))?;
    Ok(MessageRecord {
        channel_id,
        sequence,
        log_time,
        publish_time,
        data,
    })
}

/// Parses one metadata body.
/// REQ: REQ-OPS-006.
fn parse_metadata(body: &[u8]) -> Result<MetadataRecord, ReadError> {
    let mut c = Cursor::new(body);
    let name = c.str().ok_or(ReadError::Malformed("metadata name"))?;
    let metadata = c.map().ok_or(ReadError::Malformed("metadata map"))?;
    Ok(MetadataRecord { name, metadata })
}

/// Parses one channel body.
/// REQ: REQ-OPS-006.
fn parse_channel(body: &[u8]) -> Result<ChannelInfo, ReadError> {
    let mut c = Cursor::new(body);
    let id = c.u16().ok_or(ReadError::Malformed("channel id"))?;
    let _schema_id = c.u16().ok_or(ReadError::Malformed("channel schema"))?;
    let topic = c.str().ok_or(ReadError::Malformed("channel topic"))?;
    let message_encoding = c.str().ok_or(ReadError::Malformed("channel encoding"))?;
    let _metadata = c.map().ok_or(ReadError::Malformed("channel metadata"))?;
    Ok(ChannelInfo {
        id,
        topic,
        message_encoding,
    })
}

/// Parses one chunk body and appends its inner message/metadata records.
/// REQ: REQ-OPS-006.
fn parse_chunk(body: &[u8], out: &mut Contents) -> Result<(), ReadError> {
    let mut c = Cursor::new(body);
    let _start = c.u64().ok_or(ReadError::Malformed("chunk start"))?;
    let _end = c.u64().ok_or(ReadError::Malformed("chunk end"))?;
    let _uncompressed_size = c
        .u64()
        .ok_or(ReadError::Malformed("chunk uncompressed size"))?;
    let _crc = c.u32().ok_or(ReadError::Malformed("chunk crc"))?;
    let compression = c.str().ok_or(ReadError::Malformed("chunk compression"))?;
    // MCAP chunk `records` is a `uint64`-length-prefixed byte sequence.
    let records_len = usize::try_from(c.u64().ok_or(ReadError::Malformed("chunk records len"))?)
        .map_err(|_| ReadError::Malformed("chunk records len"))?;
    let compressed = c
        .take(records_len)
        .ok_or(ReadError::Malformed("chunk records"))?;
    let mode = Compression::from_name(&compression).ok_or(ReadError::UnsupportedCompression)?;
    let records = match mode {
        Compression::None => compressed.to_vec(),
        Compression::Zstd => {
            zstd::stream::decode_all(compressed).map_err(|e| ReadError::Zstd(e.to_string()))?
        }
    };
    parse_records(&records, out)
}

/// Parses a flat record sequence, appending messages and metadata. Schema,
/// channel, statistics, index, and footer records are ignored here.
/// REQ: REQ-OPS-006.
fn parse_records(bytes: &[u8], out: &mut Contents) -> Result<(), ReadError> {
    let mut c = Cursor::new(bytes);
    while c.remaining() >= 9 {
        let op = c.u8().ok_or(ReadError::Malformed("record op"))?;
        let len = usize::try_from(c.u64().ok_or(ReadError::Malformed("record len"))?)
            .map_err(|_| ReadError::Malformed("record len"))?;
        let body = c.take(len).ok_or(ReadError::Malformed("record body"))?;
        match op {
            OP_MESSAGE => out.items.push(Item::Message(parse_message(body)?)),
            OP_METADATA => out.items.push(Item::Metadata(parse_metadata(body)?)),
            _ => {}
        }
    }
    Ok(())
}

/// Reads one MCAP session from `bytes`, returning its channels and data
/// records. The closing magic and footer terminate parsing; malformed or
/// truncated input is rejected (`ReadError`), never panicked.
/// REQ: REQ-OPS-006.
///
/// # Errors
/// [`ReadError`] when the magic is absent, a record is truncated, or a chunk
/// uses an unsupported compression.
pub fn read(bytes: &[u8]) -> Result<Contents, ReadError> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(ReadError::Malformed("magic"));
    }
    let mut out = Contents::default();
    let mut c = Cursor::new(bytes.get(MAGIC.len()..).unwrap_or(&[]));
    while c.remaining() >= 9 {
        let op = c.u8().ok_or(ReadError::Malformed("record op"))?;
        let len = usize::try_from(c.u64().ok_or(ReadError::Malformed("record len"))?)
            .map_err(|_| ReadError::Malformed("record len"))?;
        let body = c.take(len).ok_or(ReadError::Malformed("record body"))?;
        match op {
            OP_CHANNEL => {
                let channel = parse_channel(body)?;
                if !out.channels.iter().any(|c| c.id == channel.id) {
                    out.channels.push(channel);
                }
            }
            OP_MESSAGE => out.items.push(Item::Message(parse_message(body)?)),
            OP_METADATA => out.items.push(Item::Metadata(parse_metadata(body)?)),
            OP_CHUNK => parse_chunk(body, &mut out)?,
            OP_FOOTER => break,
            _ => {}
        }
    }
    Ok(out)
}
