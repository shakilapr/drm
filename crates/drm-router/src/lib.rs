//! Boundary router crate (codebase.md 2.1 layer `runtime`): framing, priority
//! lanes, traffic shaping. Transport adapters are the ONLY other OS-code home
//! besides `drm-core::shm` (codebase.md 3.1 R8).
//! REQ: REQ-ROUTE-001 (imported topics are never re-exported back over the ingress link),
//! REQ-ROUTE-032 (datagram payload sizing).

#![forbid(unsafe_code)]

pub mod config;
pub mod crc32c;
pub mod framing;
pub mod mtu;
pub mod routes;
pub mod shaping;
pub mod transport;
pub mod wire;

#[cfg(test)]
mod tests {
    /// REQ: REQ-ROUTE-032 — mtu module wires into the crate root.
    #[test]
    fn mtu_module_wired() {
        assert_eq!(super::mtu::auto_suggest(), 1400);
    }
}
