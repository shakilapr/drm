//! Bounds-checked CDR deserialization for ROS 2 / DDS-XTypes payloads.
//!
//! CDR (Common Data Representation) little-endian as emitted by ROS 2. The
//! encapsulation header (`00 01 00 00`) is consumed by [`CdrReader::new`]; every
//! subsequent alignment is relative to the first byte *after* that header, which
//! is the origin ROS 2 uses. Only `core` is used by the reader; allocation-backed
//! helpers live behind the `alloc` feature.
//!
//! REQ: REQ-BIND-028 (CDR bridge payload deserialization).

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// CDR decoding failure. Every fallible read returns this instead of panicking.
/// REQ: REQ-BIND-028.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdrError {
    /// The payload ended before the requested bytes were available.
    Truncated,
    /// Missing or unsupported encapsulation header (only `CDR_LE` is accepted).
    BadEncapsulation,
    /// A length or offset overflowed `usize`.
    LengthOverflow,
    /// A `bool` byte was neither `0` nor `1`.
    InvalidBool,
    /// A string was not valid UTF-8 or lacked its terminating NUL.
    InvalidString,
}

impl core::fmt::Display for CdrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::Truncated => "truncated CDR payload",
            Self::BadEncapsulation => "unsupported CDR encapsulation",
            Self::LengthOverflow => "CDR length overflow",
            Self::InvalidBool => "invalid CDR bool",
            Self::InvalidString => "invalid CDR string",
        };
        f.write_str(text)
    }
}

impl core::error::Error for CdrError {}

/// A bounds-checked cursor over a `CDR_LE` payload.
/// REQ: REQ-BIND-028.
#[derive(Debug, Clone)]
pub struct CdrReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CdrReader<'a> {
    /// Validates the 4-byte encapsulation header and starts reading after it.
    ///
    /// # Errors
    /// Returns [`CdrError::Truncated`] for fewer than four bytes and
    /// [`CdrError::BadEncapsulation`] for any header other than `CDR_LE`.
    /// REQ: REQ-BIND-028.
    pub fn new(payload: &'a [u8]) -> Result<Self, CdrError> {
        let (header, data) = payload
            .split_first_chunk::<4>()
            .ok_or(CdrError::Truncated)?;
        if *header != [0x00, 0x01, 0x00, 0x00] {
            return Err(CdrError::BadEncapsulation);
        }
        Ok(Self { data, pos: 0 })
    }

    /// Current byte offset from the post-encapsulation origin.
    /// REQ: REQ-BIND-028.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes left after the cursor.
    /// REQ: REQ-BIND-028.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Advances to the next `alignment` boundary measured from the payload
    /// origin, then returns `n` bytes. REQ: REQ-BIND-028.
    fn take(&mut self, alignment: usize, n: usize) -> Result<&'a [u8], CdrError> {
        let misalign = self.pos % alignment;
        let pad = if misalign == 0 {
            0
        } else {
            alignment - misalign
        };
        let start = self.pos.checked_add(pad).ok_or(CdrError::LengthOverflow)?;
        let end = start.checked_add(n).ok_or(CdrError::LengthOverflow)?;
        let bytes = self.data.get(start..end).ok_or(CdrError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn take_array<const N: usize>(&mut self, alignment: usize) -> Result<[u8; N], CdrError> {
        let bytes = self.take(alignment, N)?;
        <[u8; N]>::try_from(bytes).map_err(|_| CdrError::Truncated)
    }

    /// Reads one `uint8`.
    /// REQ: REQ-BIND-028.
    pub fn read_u8(&mut self) -> Result<u8, CdrError> {
        self.take(1, 1)?.first().copied().ok_or(CdrError::Truncated)
    }

    /// Reads one `int8`.
    /// REQ: REQ-BIND-028.
    pub fn read_i8(&mut self) -> Result<i8, CdrError> {
        Ok(self.read_u8()?.cast_signed())
    }

    /// Reads one `bool` (one byte, `0` or `1`).
    /// REQ: REQ-BIND-028.
    pub fn read_bool(&mut self) -> Result<bool, CdrError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CdrError::InvalidBool),
        }
    }

    /// Reads one `uint16` (2-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_u16(&mut self) -> Result<u16, CdrError> {
        Ok(u16::from_le_bytes(self.take_array(2)?))
    }

    /// Reads one `int16` (2-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_i16(&mut self) -> Result<i16, CdrError> {
        Ok(self.read_u16()?.cast_signed())
    }

    /// Reads one `uint32` (4-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_u32(&mut self) -> Result<u32, CdrError> {
        Ok(u32::from_le_bytes(self.take_array(4)?))
    }

    /// Reads one `int32` (4-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_i32(&mut self) -> Result<i32, CdrError> {
        Ok(self.read_u32()?.cast_signed())
    }

    /// Reads one `uint64` (8-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_u64(&mut self) -> Result<u64, CdrError> {
        Ok(u64::from_le_bytes(self.take_array(8)?))
    }

    /// Reads one `int64` (8-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_i64(&mut self) -> Result<i64, CdrError> {
        Ok(self.read_u64()?.cast_signed())
    }

    /// Reads one `float32` (4-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_f32(&mut self) -> Result<f32, CdrError> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    /// Reads one `float64` (8-byte aligned).
    /// REQ: REQ-BIND-028.
    pub fn read_f64(&mut self) -> Result<f64, CdrError> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    /// Reads a CDR string: `u32` length including the NUL, then UTF-8 bytes.
    ///
    /// # Errors
    /// Returns [`CdrError::InvalidString`] for a zero length, missing NUL, or
    /// invalid UTF-8, and [`CdrError::Truncated`] if the bytes are absent.
    /// REQ: REQ-BIND-028.
    pub fn read_string(&mut self) -> Result<&'a str, CdrError> {
        let len = usize::try_from(self.read_u32()?).map_err(|_| CdrError::LengthOverflow)?;
        if len == 0 {
            return Err(CdrError::InvalidString);
        }
        let bytes = self.take(1, len)?;
        let (nul, text) = bytes.split_last().ok_or(CdrError::InvalidString)?;
        if *nul != 0 {
            return Err(CdrError::InvalidString);
        }
        core::str::from_utf8(text).map_err(|_| CdrError::InvalidString)
    }

    /// Reads a sequence count (`u32`) without touching the elements.
    /// REQ: REQ-BIND-028.
    pub fn read_sequence_len(&mut self) -> Result<usize, CdrError> {
        let count = usize::try_from(self.read_u32()?).map_err(|_| CdrError::LengthOverflow)?;
        if count > self.remaining() {
            return Err(CdrError::Truncated);
        }
        Ok(count)
    }

    /// Reads a sequence of any [`CdrRead`] element type into a `Vec`.
    ///
    /// # Errors
    /// Propagates [`CdrError::Truncated`] for a short payload and
    /// [`CdrError::LengthOverflow`] if the count cannot be represented or
    /// reserved.
    /// REQ: REQ-BIND-028.
    #[cfg(feature = "alloc")]
    pub fn read_sequence<T: CdrRead>(&mut self) -> Result<Vec<T>, CdrError> {
        let count = self.read_sequence_len()?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| CdrError::LengthOverflow)?;
        for _ in 0..count {
            out.push(T::read(self)?);
        }
        Ok(out)
    }
}

/// A type decodable from a CDR stream; used by [`CdrReader::read_sequence`].
/// REQ: REQ-BIND-028.
pub trait CdrRead: Sized {
    /// Decodes one value, advancing the reader.
    /// REQ: REQ-BIND-028.
    fn read(reader: &mut CdrReader<'_>) -> Result<Self, CdrError>;
}

macro_rules! impl_cdr_read {
    ($($ty:ty => $method:ident),* $(,)?) => {
        $(
            impl CdrRead for $ty {
                fn read(reader: &mut CdrReader<'_>) -> Result<Self, CdrError> {
                    reader.$method()
                }
            }
        )*
    };
}

impl_cdr_read! {
    u8 => read_u8,
    i8 => read_i8,
    bool => read_bool,
    u16 => read_u16,
    i16 => read_i16,
    u32 => read_u32,
    i32 => read_i32,
    u64 => read_u64,
    i64 => read_i64,
    f32 => read_f32,
    f64 => read_f64,
}

/// `std_msgs/Header` stamp (`builtin_interfaces/Time`).
/// REQ: REQ-BIND-028.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Time {
    /// Whole seconds (ROS 2 uses a signed 32-bit field).
    pub sec: i32,
    /// Nanoseconds within the second.
    pub nanosec: u32,
}

/// `std_msgs/Header`.
/// REQ: REQ-BIND-028.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Acquisition timestamp.
    pub stamp: Time,
    /// Coordinate frame id.
    pub frame_id: String,
}

/// Decoded ROS 2 `sensor_msgs/msg/LaserScan`.
/// REQ: REQ-BIND-028.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub struct LaserScanV1 {
    /// Message header.
    pub header: Header,
    /// Start angle of the first range, radians.
    pub angle_min: f32,
    /// End angle of the last range, radians.
    pub angle_max: f32,
    /// Angular distance between measurements, radians.
    pub angle_increment: f32,
    /// Time between measurements, seconds.
    pub time_increment: f32,
    /// Time between scans, seconds.
    pub scan_time: f32,
    /// Minimum valid range, metres.
    pub range_min: f32,
    /// Maximum valid range, metres.
    pub range_max: f32,
    /// Range measurements, metres.
    pub ranges: Vec<f32>,
    /// Intensity per range measurement.
    pub intensities: Vec<f32>,
}

#[cfg(feature = "alloc")]
impl LaserScanV1 {
    /// Decodes a complete CDR `sensor_msgs/msg/LaserScan` payload.
    ///
    /// # Errors
    /// Returns any [`CdrError`] produced while reading the header, scalar
    /// fields, or the `ranges` / `intensities` sequences.
    /// REQ: REQ-BIND-028.
    pub fn decode(payload: &[u8]) -> Result<Self, CdrError> {
        let mut r = CdrReader::new(payload)?;
        let sec = r.read_i32()?;
        let nanosec = r.read_u32()?;
        let frame_id = String::from(r.read_string()?);
        let angle_min = r.read_f32()?;
        let angle_max = r.read_f32()?;
        let angle_increment = r.read_f32()?;
        let time_increment = r.read_f32()?;
        let scan_time = r.read_f32()?;
        let range_min = r.read_f32()?;
        let range_max = r.read_f32()?;
        let ranges = r.read_sequence::<f32>()?;
        let intensities = r.read_sequence::<f32>()?;
        Ok(Self {
            header: Header {
                stamp: Time { sec, nanosec },
                frame_id,
            },
            angle_min,
            angle_max,
            angle_increment,
            time_increment,
            scan_time,
            range_min,
            range_max,
            ranges,
            intensities,
        })
    }
}
