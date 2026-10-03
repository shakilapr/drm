//! Route config parsing tests (work-plan M02W3-E3-T2): valid link/override
//! surface, rate matrix, and the explicit error taxonomy.
//! REQ: REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017, REQ-ROUTE-008.

use std::collections::BTreeSet;

use drm_router::config::{LinkConfig, Rate, RateError, RouteConfigError, parse_route_config};

fn fixture<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("fixture invariant violated ({what}): {e:?}"),
    }
}

/// REQ: REQ-ROUTE-002 — link `transport` + `bandwidth` parse to typed values.
#[test]
fn links_and_bandwidth_parse() {
    let text = r#"
[links."jetson <-> robot_pc"]
transport = "ethernet"
bandwidth = "1Gbps"

[links."jetson <-> mcu"]
transport = "serial"
bandwidth = "9600baud"
"#;
    let cfg = fixture(parse_route_config(text), "links config");
    assert_eq!(
        cfg.links.get("jetson <-> robot_pc"),
        Some(&LinkConfig {
            transport: String::from("ethernet"),
            bandwidth: Rate {
                numerator: 1_000_000_000,
                denominator: 1,
            },
        })
    );
    assert_eq!(
        cfg.links.get("jetson <-> mcu"),
        Some(&LinkConfig {
            transport: String::from("serial"),
            bandwidth: Rate {
                numerator: 9_600,
                denominator: 1,
            },
        })
    );
    assert!(cfg.routing.prefer.is_empty());
}

/// REQ: REQ-ROUTE-017, REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-008 —
/// `[routing]` overrides parse as carried data (conflict detection is
/// derivation-owned, I-R9: two `force` links survive parsing).
#[test]
fn routing_overrides_parse() {
    let text = r#"
[routing]
prefer."control/target" = "ethernet"
deny."camera/front" = ["wifi", "serial"]
force."diagnostics" = "ethernet"
force."motor/ctl" = ["ethernet", "wifi"]

[routing.export_limit."camera/front"]
ethernet = "10Hz"
wifi = "2.5Hz"
"#;
    let cfg = fixture(parse_route_config(text), "overrides config");
    assert_eq!(
        cfg.routing.prefer.get("control/target").map(String::as_str),
        Some("ethernet")
    );
    let expected_deny: BTreeSet<String> = ["wifi", "serial"].map(String::from).into();
    assert_eq!(cfg.routing.deny.get("camera/front"), Some(&expected_deny));
    assert_eq!(
        cfg.routing.force.get("diagnostics").map(Vec::as_slice),
        Some(&[String::from("ethernet")][..])
    );
    assert_eq!(
        cfg.routing.force.get("motor/ctl").map(Vec::as_slice),
        Some(&[String::from("ethernet"), String::from("wifi")][..])
    );
    let limits = cfg.routing.export_limit.get("camera/front");
    assert_eq!(
        limits.and_then(|m| m.get("ethernet")),
        Some(&Rate {
            numerator: 10,
            denominator: 1,
        })
    );
    assert_eq!(
        limits.and_then(|m| m.get("wifi")),
        Some(&Rate {
            numerator: 5,
            denominator: 2,
        })
    );
}

/// REQ: REQ-ROUTE-043 — an empty or section-less document parses to an empty
/// config; sections are optional at the structural layer.
#[test]
fn empty_config_parses_empty() {
    for text in ["", "\n", "[links]\n", "[routing]\n"] {
        let cfg = fixture(parse_route_config(text), "empty config");
        assert!(cfg.links.is_empty(), "text {text:?}");
        assert!(cfg.routing.prefer.is_empty());
        assert!(cfg.routing.deny.is_empty());
        assert!(cfg.routing.force.is_empty());
        assert!(cfg.routing.export_limit.is_empty());
    }
}

/// REQ: REQ-ROUTE-043 — malformed TOML text is a `Syntax` error.
#[test]
fn malformed_toml_is_syntax() {
    for text in [
        "[links",
        "transport = ",
        "[routing]\nprefer. = \"x\"",
        "= \"x\"",
    ] {
        assert_eq!(
            parse_route_config(text),
            Err(RouteConfigError::Syntax),
            "text {text:?}"
        );
    }
}

/// REQ: REQ-ROUTE-043 — a link missing `transport` or `bandwidth` names the
/// exact field path.
#[test]
fn missing_required_fields() {
    let no_transport = "[links.\"a\"]\nbandwidth = \"1Gbps\"\n";
    assert_eq!(
        parse_route_config(no_transport),
        Err(RouteConfigError::MissingField {
            field: String::from("links.\"a\".transport"),
        })
    );
    let no_bandwidth = "[links.\"a\"]\ntransport = \"ethernet\"\n";
    assert_eq!(
        parse_route_config(no_bandwidth),
        Err(RouteConfigError::MissingField {
            field: String::from("links.\"a\".bandwidth"),
        })
    );
}

/// REQ: REQ-ROUTE-043 — wrong value types and unparsable rates reject with
/// `BadValue` naming the field.
#[test]
fn bad_type_is_bad_value() {
    for (text, field) in [
        (
            "[links.\"a\"]\ntransport = \"ethernet\"\nbandwidth = 1500\n",
            "links.\"a\".bandwidth",
        ),
        (
            "[links.\"a\"]\ntransport = true\nbandwidth = \"1Gbps\"\n",
            "links.\"a\".transport",
        ),
        (
            "[routing]\nprefer.\"t\" = [\"ethernet\"]\n",
            "routing.prefer.\"t\"",
        ),
        ("[routing]\ndeny.\"t\" = [1]\n", "routing.deny.\"t\""),
        ("[routing]\nforce.\"t\" = 7\n", "routing.force.\"t\""),
        (
            "[routing.export_limit.\"t\"]\nethernet = 10\n",
            "routing.export_limit.\"t\".\"ethernet\"",
        ),
    ] {
        assert_eq!(
            parse_route_config(text),
            Err(RouteConfigError::BadValue {
                field: String::from(field),
            }),
            "text {text:?}"
        );
    }
}

/// REQ: REQ-ROUTE-043 — unknown keys at every level reject with their path.
#[test]
fn unknown_fields_rejected() {
    for (text, field) in [
        ("[topology]\ntransport = \"ethernet\"\n", "topology"),
        (
            "[links.\"a\"]\ntransport = \"ethernet\"\nbandwidth = \"1Gbps\"\ntransport2 = \"x\"\n",
            "links.\"a\".transport2",
        ),
        ("[routing]\nspeed.\"t\" = \"ethernet\"\n", "routing.speed"),
    ] {
        assert_eq!(
            parse_route_config(text),
            Err(RouteConfigError::UnknownField {
                field: String::from(field),
            }),
            "text {text:?}"
        );
    }
}

/// REQ: REQ-ROUTE-043 — empty names (transport, topic, link id) reject with
/// `BadValue` on the named field.
#[test]
fn empty_names_rejected() {
    for (text, field) in [
        (
            "[links.\"a\"]\ntransport = \"\"\nbandwidth = \"1Gbps\"\n",
            "links.\"a\".transport",
        ),
        (
            "[routing]\nprefer.\"\" = \"ethernet\"\n",
            "routing.prefer.\"\"",
        ),
        ("[routing]\ndeny.\"t\" = [\"\"]\n", "routing.deny.\"t\""),
        (
            "[routing.export_limit.\"t\"]\n\"\" = \"10Hz\"\n",
            "routing.export_limit.\"t\".\"\"",
        ),
    ] {
        assert_eq!(
            parse_route_config(text),
            Err(RouteConfigError::BadValue {
                field: String::from(field),
            }),
            "text {text:?}"
        );
    }
}

/// REQ: REQ-ROUTE-008 — dimensioned rate strings parse once into checked
/// rationals; malformed numbers and unknown units reject with the specific
/// [`RateError`] kind.
#[test]
fn rate_unit_matrix() {
    let ok = [
        ("10Hz", (10, 1)),
        ("2.5Hz", (5, 2)),
        ("1Gbps", (1_000_000_000, 1)),
        ("100Mbps", (100_000_000, 1)),
        ("9600baud", (9_600, 1)),
        ("512bps", (512, 1)),
        ("0Hz", (0, 1)),
        (".5Kbps", (500, 1)),
    ];
    for (text, (num, den)) in ok {
        assert_eq!(
            Rate::parse(text),
            Ok(Rate {
                numerator: num,
                denominator: den,
            }),
            "text {text:?}"
        );
    }
    let err = [
        ("", RateError::Empty),
        ("   ", RateError::Empty),
        ("Hz", RateError::Number),
        ("10.", RateError::Number),
        ("1.2.3bps", RateError::Number),
        ("5 furlongs", RateError::Unit),
        ("100", RateError::Unit),
        ("10hz", RateError::Unit),
        ("10 Mb", RateError::Unit),
    ];
    for (text, kind) in err {
        assert_eq!(Rate::parse(text), Err(kind), "text {text:?}");
    }
}
