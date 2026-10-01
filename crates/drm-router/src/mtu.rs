//! Datagram payload sizing: one sample per datagram, never IP-fragmented
//! (`docs/algorithms/router/router-core.md` REQ-ROUTE-032). UDP/IP headers are
//! subtracted from the link MTU; IPv4 floor 68 (RFC 791), IPv6 floor 1280
//! (RFC 8200); UDP payload cap 65507.
//! REQ: REQ-ROUTE-032, REQ-ROUTE-018 (oversize rejected at build time).

use drm_types::Status;

/// IP family of a datagram link.
/// REQ: REQ-ROUTE-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFamily {
    /// IPv4: 20-byte IP + 8-byte UDP header.
    V4,
    /// IPv6: 40-byte IP + 8-byte UDP header.
    V6,
}

impl IpFamily {
    /// Minimum link MTU accepted by the IP layer.
    /// REQ: REQ-ROUTE-032.
    #[must_use]
    pub fn min_mtu(self) -> u16 {
        match self {
            Self::V4 => 68,
            Self::V6 => 1280,
        }
    }

    /// IP + UDP header bytes on this family.
    /// REQ: REQ-ROUTE-032.
    #[must_use]
    pub fn header_bytes(self) -> u16 {
        match self {
            Self::V4 => 28,
            Self::V6 => 48,
        }
    }
}

/// UDP datagram payload upper bound (one datagram field).
const UDP_PAYLOAD_CAP: u16 = 65507;

/// Largest one-sample payload a datagram link of `link_mtu` carries.
/// REQ: REQ-ROUTE-032.
///
/// # Errors
/// [`Status::InvalidConfig`] when the link MTU is below the family floor or
/// the derived payload exceeds the UDP datagram cap.
pub fn max_datagram_payload(link_mtu: u16, family: IpFamily) -> Result<u16, Status> {
    if link_mtu < family.min_mtu() {
        return Err(Status::InvalidConfig);
    }
    let payload = link_mtu - family.header_bytes();
    if payload > UDP_PAYLOAD_CAP {
        return Err(Status::InvalidConfig);
    }
    Ok(payload)
}

/// Default datagram payload suggestion when the link MTU is unknown
/// (fits every IPv6-floor link).
/// REQ: REQ-ROUTE-032.
#[must_use]
pub fn auto_suggest() -> u16 {
    1400
}

/// Validates an operator-provided payload override against the UDP cap.
/// REQ: REQ-ROUTE-018.
///
/// # Errors
/// [`Status::InvalidConfig`] when the override exceeds the UDP datagram cap.
pub fn validate_override(payload: u16) -> Result<(), Status> {
    if payload > UDP_PAYLOAD_CAP {
        return Err(Status::InvalidConfig);
    }
    Ok(())
}

/// Validates an operator-provided link-MTU override against the family floor.
/// REQ: REQ-ROUTE-032, REQ-ROUTE-018.
///
/// # Errors
/// [`Status::InvalidConfig`] when the MTU is below the family minimum (an
/// override may raise, never lower, the floor).
pub fn validate_mtu_override(link_mtu: u16, family: IpFamily) -> Result<(), Status> {
    if link_mtu < family.min_mtu() {
        return Err(Status::InvalidConfig);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        IpFamily, auto_suggest, max_datagram_payload, validate_mtu_override, validate_override,
    };
    use drm_types::Status;

    /// REQ: REQ-ROUTE-032 — 1500 MTU yields 1472 (v4) / 1452 (v6).
    #[test]
    fn ethernet_mtu() {
        assert_eq!(max_datagram_payload(1500, IpFamily::V4), Ok(1472));
        assert_eq!(max_datagram_payload(1500, IpFamily::V6), Ok(1452));
    }

    /// REQ: REQ-ROUTE-032 — family floors: below ⇒ `InvalidConfig`, at ⇒ ok.
    #[test]
    fn floors() {
        assert_eq!(
            max_datagram_payload(67, IpFamily::V4),
            Err(Status::InvalidConfig)
        );
        assert_eq!(max_datagram_payload(68, IpFamily::V4), Ok(40));
        assert_eq!(
            max_datagram_payload(1279, IpFamily::V6),
            Err(Status::InvalidConfig)
        );
        assert_eq!(max_datagram_payload(1280, IpFamily::V6), Ok(1232));
    }

    /// REQ: REQ-ROUTE-032 — UDP cap boundary at maximum MTU.
    #[test]
    fn udp_cap_boundary() {
        assert_eq!(max_datagram_payload(65535, IpFamily::V4), Ok(65507));
        assert_eq!(max_datagram_payload(65535, IpFamily::V6), Ok(65487));
    }

    /// REQ: REQ-ROUTE-032 — auto suggestion constant.
    #[test]
    fn suggestion() {
        assert_eq!(auto_suggest(), 1400);
        assert!(max_datagram_payload(1500, IpFamily::V6).is_ok_and(|p| p >= 1400));
    }

    /// REQ: REQ-ROUTE-018 — override validation.
    #[test]
    fn override_bounds() {
        assert!(validate_override(65507).is_ok());
        assert_eq!(validate_override(65508), Err(Status::InvalidConfig));
    }

    /// REQ: REQ-ROUTE-032 — sub-Ethernet MTU edges: 1492 (`PPPoE`) and 1472.
    #[test]
    fn mtu_edges_1492_1472() {
        assert_eq!(max_datagram_payload(1492, IpFamily::V4), Ok(1464));
        assert_eq!(max_datagram_payload(1492, IpFamily::V6), Ok(1444));
        assert_eq!(max_datagram_payload(1472, IpFamily::V4), Ok(1444));
        assert_eq!(max_datagram_payload(1472, IpFamily::V6), Ok(1424));
    }

    /// REQ: REQ-ROUTE-032, REQ-ROUTE-018 — an MTU override below the family
    /// floor is an error, at the floor is accepted (both families).
    #[test]
    fn mtu_override_below_floor() {
        assert_eq!(
            validate_mtu_override(67, IpFamily::V4),
            Err(Status::InvalidConfig)
        );
        assert_eq!(validate_mtu_override(68, IpFamily::V4), Ok(()));
        assert_eq!(
            validate_mtu_override(1279, IpFamily::V6),
            Err(Status::InvalidConfig)
        );
        assert_eq!(validate_mtu_override(1280, IpFamily::V6), Ok(()));
    }
}
