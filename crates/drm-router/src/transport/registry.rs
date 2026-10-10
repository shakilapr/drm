//! Transport-capability registry (router-core.md §Data Structures *capability
//! mapping*; work-plan Phase 29 seed). Maps a route's [`TransportKind`] to an
//! adapter selection derived from a static capability table
//! ([`TransportCaps`]); an unregistered kind is an explicit configuration
//! error, never a panic (I-RT11 media-pluggable adapters).
//! REQ: REQ-ROUTE-034, REQ-ROUTE-009.

use drm_types::Status;

use crate::transport::Reliability;

/// Number of distinct transport kinds the registry tables.
pub const KIND_COUNT: usize = 4;

/// Route transport binding kind (router-core.md `WireFrame` variants).
/// REQ: REQ-ROUTE-034.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// UDP datagrams: one sample per datagram, best-effort (I-RT3, I-RT11).
    Udp,
    /// TCP stream: length-framed records with ordered loss recovery (I-RT5).
    Tcp,
    /// Serial UART: best-effort COBS-framed stream (I-RT10, REQ-ROUTE-024).
    Serial,
    /// No transport bound — a route that must not egress (disabled link).
    Disabled,
}

impl TransportKind {
    /// All kinds in table order.
    pub const ALL: [Self; KIND_COUNT] = [Self::Udp, Self::Tcp, Self::Serial, Self::Disabled];

    /// Stable table index for this kind.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Udp => 0,
            Self::Tcp => 1,
            Self::Serial => 2,
            Self::Disabled => 3,
        }
    }
}

/// Static capability class of a transport binding — the authoritative row the
/// registry serves (router-core.md *capability mapping*; REQ-TOOL-061).
/// REQ: REQ-ROUTE-034.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportCaps {
    /// Ordered loss recovery is provided while the transport session is valid.
    pub reliable: bool,
    /// Maximum encoded frame the link accepts, in bytes (charge definition).
    pub max_frame: u32,
    /// One sample per datagram (`true`) versus a length-framed stream (`false`).
    pub datagram: bool,
}

impl TransportCaps {
    /// Capability class as the adapter surface reports it.
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn reliability(self) -> Reliability {
        if self.reliable {
            Reliability::Reliable
        } else {
            Reliability::BestEffort
        }
    }

    /// Whether this binding is a stream (length-framed records), not datagrams.
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn is_stream(self) -> bool {
        !self.datagram
    }
}

/// Registry row served for a registered kind: the kind plus its capability
/// class — the adapter *selection* the egress path consumes.
/// REQ: REQ-ROUTE-034.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterSelection {
    /// Bound transport kind.
    pub kind: TransportKind,
    /// Capability class of the binding.
    pub caps: TransportCaps,
}

impl AdapterSelection {
    /// Capability class as the adapter surface reports it.
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn reliability(self) -> Reliability {
        self.caps.reliability()
    }
}

/// Default capability row for a best-effort datagram link (UDP, I-RT3/I-RT11).
const UDP_CAPS: TransportCaps = TransportCaps {
    reliable: false,
    max_frame: 1_400,
    datagram: true,
};

/// Default capability row for a reliable ordered stream (TCP records, I-RT5).
const TCP_CAPS: TransportCaps = TransportCaps {
    reliable: true,
    max_frame: 65_535,
    datagram: false,
};

/// Default capability row for a best-effort serial stream (REQ-ROUTE-024).
const SERIAL_CAPS: TransportCaps = TransportCaps {
    reliable: false,
    max_frame: 256,
    datagram: false,
};

/// Static `kind → capability` table with explicit, non-panicking lookup.
/// A kind with no registered row is a configuration error surfaced by
/// [`TransportRegistry::lookup`]/[`TransportRegistry::select`], not a panic
/// (I-RT11 media-pluggable adapters). REQ: REQ-ROUTE-034.
#[derive(Debug, Clone)]
pub struct TransportRegistry {
    table: [Option<TransportCaps>; KIND_COUNT],
}

impl Default for TransportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportRegistry {
    /// Empty registry — every kind is unregistered until [`Self::register`].
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table: [None; KIND_COUNT],
        }
    }

    /// Registry preloaded with the v1 capability table: UDP (best-effort
    /// datagram), TCP (reliable stream), serial (best-effort stream).
    /// [`TransportKind::Disabled`] is intentionally unmapped.
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn from_defaults() -> Self {
        let mut registry = Self::new();
        // Constants are structurally valid, so registration cannot fail here.
        let _ = registry.register(TransportKind::Udp, UDP_CAPS);
        let _ = registry.register(TransportKind::Tcp, TCP_CAPS);
        let _ = registry.register(TransportKind::Serial, SERIAL_CAPS);
        registry
    }

    /// Registers a capability row for `kind`.
    /// REQ: REQ-ROUTE-034.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `caps.max_frame` is zero (a frame could
    /// never send); [`Status::AlreadyExists`] when `kind` already has a row.
    pub fn register(&mut self, kind: TransportKind, caps: TransportCaps) -> Result<(), Status> {
        if caps.max_frame == 0 {
            return Err(Status::InvalidConfig);
        }
        let slot = &mut self.table[kind.index()];
        if slot.is_some() {
            return Err(Status::AlreadyExists);
        }
        *slot = Some(caps);
        Ok(())
    }

    /// Capability class registered for `kind`.
    /// REQ: REQ-ROUTE-034.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `kind` has no registered row.
    pub fn lookup(&self, kind: TransportKind) -> Result<TransportCaps, Status> {
        self.table[kind.index()].ok_or(Status::InvalidConfig)
    }

    /// Adapter selection for `kind`: the kind plus its capability class.
    /// REQ: REQ-ROUTE-034.
    ///
    /// # Errors
    /// [`Status::InvalidConfig`] when `kind` has no registered row.
    pub fn select(&self, kind: TransportKind) -> Result<AdapterSelection, Status> {
        let caps = self.lookup(kind)?;
        Ok(AdapterSelection { kind, caps })
    }

    /// Whether `kind` has a registered capability row.
    /// REQ: REQ-ROUTE-034.
    #[must_use]
    pub fn is_registered(&self, kind: TransportKind) -> bool {
        self.table[kind.index()].is_some()
    }
}
