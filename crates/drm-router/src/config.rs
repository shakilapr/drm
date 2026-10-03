//! Route configuration parsing (work-plan M02W3-E3-T2): `RouteConfig` from
//! TOML text — declared links (`transport`, `bandwidth`) and `[routing]`
//! overrides (`prefer`/`deny`/`force`/`export_limit`), carried as data to
//! route derivation (`docs/algorithms/toolchain/route-derivation.md`
//! `Overrides`). Structural parsing only: override conflicts
//! (`ConflictingRouteOverride`, I-R9) are derivation/verify-owned, never
//! resolved here. The TOML field surface follows `docs/architecture.md`
//! (§Hardware Topology, §Automatic Routing and Explicit Overrides,
//! §External Device Interoperability).
//! REQ: REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017, REQ-ROUTE-008,
//! REQ-ROUTE-002 (typed link capacity), REQ-ROUTE-043 (construction input to
//! compute-only runtime tables).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Checked rational rate (e.g. `10Hz` → 10/1, `2.5Hz` → 5/2,
/// `1Gbps` → `1_000_000_000/1`) parsed once at config load; downstream
/// algorithms consume the typed value, never source strings.
/// REQ: REQ-ROUTE-008, REQ-ROUTE-002.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    /// Numerator, reduced with the denominator.
    pub numerator: u64,
    /// Denominator, reduced with the numerator.
    pub denominator: u64,
}

impl Rate {
    /// Parses a dimensioned rate string: decimal number + unit. Units:
    /// `Hz` (frequency), `bps`/`Kbps`/`Mbps`/`Gbps` (decimal bit rates),
    /// `baud` (serial line rate, bits/s for capacity purposes).
    /// REQ: REQ-ROUTE-008, REQ-ROUTE-002.
    ///
    /// # Errors
    /// [`RateError`] when the input is empty, the number is missing,
    /// malformed, or does not fit `u64`/`u64` reduced, or the unit is unknown.
    pub fn parse(text: &str) -> Result<Self, RateError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(RateError::Empty);
        }
        let digits_len = text
            .as_bytes()
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .count();
        let (num_str, unit) = text.split_at(digits_len);
        let (numer0, denom0) = split_number(num_str)?;
        let multiplier: u128 = match unit {
            "Hz" | "bps" | "baud" => 1,
            "Kbps" => 1_000,
            "Mbps" => 1_000_000,
            "Gbps" => 1_000_000_000,
            _ => return Err(RateError::Unit),
        };
        let numer = numer0.checked_mul(multiplier).ok_or(RateError::Number)?;
        let gcd = gcd(numer, denom0);
        let numerator = u64::try_from(numer / gcd).map_err(|_| RateError::Number)?;
        let denominator = u64::try_from(denom0 / gcd).map_err(|_| RateError::Number)?;
        Ok(Self {
            numerator,
            denominator,
        })
    }
}

/// Rate parse failure kind (surfaced as [`RouteConfigError::BadValue`]).
/// REQ: REQ-ROUTE-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateError {
    /// Empty input.
    Empty,
    /// Missing, malformed, or overflowing number part.
    Number,
    /// Unknown or missing unit suffix.
    Unit,
}

impl fmt::Display for RateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty rate"),
            Self::Number => f.write_str("malformed rate number"),
            Self::Unit => f.write_str("unknown rate unit"),
        }
    }
}

/// Config parse failure with the offending field path (e.g.
/// `links."a <-> b".bandwidth`). router-core.md defines no config parse
/// function, so the crate-local taxonomy applies and renders via `Display`.
/// REQ: REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017, REQ-ROUTE-008.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteConfigError {
    /// TOML text failed to parse.
    Syntax,
    /// A required key is absent.
    MissingField {
        /// Field path of the missing key.
        field: String,
    },
    /// A key outside the declared surface.
    UnknownField {
        /// Field path of the unknown key.
        field: String,
    },
    /// A value of the wrong type, an empty name, or an unparsable rate.
    BadValue {
        /// Field path of the offending value.
        field: String,
    },
}

impl fmt::Display for RouteConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => f.write_str("malformed TOML"),
            Self::MissingField { field } => write!(f, "missing field {field}"),
            Self::UnknownField { field } => write!(f, "unknown field {field}"),
            Self::BadValue { field } => write!(f, "bad value for {field}"),
        }
    }
}

/// One declared link: transport medium and physical line capacity
/// (`[links."a <-> b"]`; architecture.md §Hardware Topology — `bandwidth`
/// is line capacity, never a payload budget).
/// REQ: REQ-ROUTE-002, REQ-ROUTE-043.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkConfig {
    /// Declared transport medium name (e.g. `ethernet`, `serial`);
    /// capability mapping is a later, verify-owned step.
    pub transport: String,
    /// Physical line capacity (`bandwidth = "1Gbps"`), typed [`Rate`].
    pub bandwidth: Rate,
}

/// `[routing]` overrides, carried as data to route derivation; the
/// selection pipeline (config-validity → deny → force → prefer → auto) and
/// conflict detection (I-R9) are derivation-owned.
/// REQ: REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017, REQ-ROUTE-008.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteOverrides {
    /// `prefer."topic" = "link"`: selects among viable paths.
    /// REQ: REQ-ROUTE-017.
    pub prefer: BTreeMap<String, String>,
    /// `deny."topic" = ["link", ...]`: prunes candidate links.
    /// REQ: REQ-ROUTE-015.
    pub deny: BTreeMap<String, BTreeSet<String>>,
    /// `force."topic" = "link" | ["link", ...]`: pins the route; ≥ 2 links
    /// or force∩deny is a derivation error, never resolved by precedence.
    /// REQ: REQ-ROUTE-016.
    pub force: BTreeMap<String, Vec<String>>,
    /// `export_limit."topic"."link" = "10Hz"`: per-egress shaping carried
    /// into plan edges (I-R14).
    /// REQ: REQ-ROUTE-008.
    pub export_limit: BTreeMap<String, BTreeMap<String, Rate>>,
}

/// Parsed route configuration: declared links plus `[routing]` overrides
/// (structural layer; policy is derivation/verify-owned).
/// REQ: REQ-ROUTE-043, REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017,
/// REQ-ROUTE-008.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteConfig {
    /// Declared links keyed by link id (`[links."a <-> b"]`).
    pub links: BTreeMap<String, LinkConfig>,
    /// `[routing]` overrides.
    pub routing: RouteOverrides,
}

/// Parses route configuration text into a typed [`RouteConfig`]. Unknown
/// keys, missing required keys, wrong types, empty names, and unparsable
/// rates reject with the offending field path.
/// REQ: REQ-ROUTE-043, REQ-ROUTE-015, REQ-ROUTE-016, REQ-ROUTE-017,
/// REQ-ROUTE-008.
///
/// # Errors
/// [`RouteConfigError`] on malformed TOML or any structural violation.
pub fn parse_route_config(text: &str) -> Result<RouteConfig, RouteConfigError> {
    let root: toml::Table = toml::from_str(text).map_err(|_| RouteConfigError::Syntax)?;
    for key in root.keys() {
        if !matches!(key.as_str(), "links" | "routing") {
            return Err(RouteConfigError::UnknownField { field: key.clone() });
        }
    }
    let links = parse_links(root.get("links"))?;
    let routing = parse_routing(root.get("routing"))?;
    Ok(RouteConfig { links, routing })
}

fn split_number(num_str: &str) -> Result<(u128, u128), RateError> {
    match num_str.split_once('.') {
        Some((int_part, frac_part)) => {
            if frac_part.is_empty()
                || !int_part.chars().all(|c| c.is_ascii_digit())
                || !frac_part.chars().all(|c| c.is_ascii_digit())
            {
                return Err(RateError::Number);
            }
            if frac_part.len() > 38 {
                return Err(RateError::Number);
            }
            let int_val = if int_part.is_empty() {
                0
            } else {
                int_part.parse::<u128>().map_err(|_| RateError::Number)?
            };
            let exp = u32::try_from(frac_part.len()).map_err(|_| RateError::Number)?;
            let frac_val = frac_part.parse::<u128>().map_err(|_| RateError::Number)?;
            let denom = u128::checked_pow(10, exp).ok_or(RateError::Number)?;
            let numer = int_val
                .checked_mul(denom)
                .and_then(|n| n.checked_add(frac_val))
                .ok_or(RateError::Number)?;
            Ok((numer, denom))
        }
        None => Ok((num_str.parse::<u128>().map_err(|_| RateError::Number)?, 1)),
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn as_table<'a>(value: &'a toml::Value, field: &str) -> Result<&'a toml::Table, RouteConfigError> {
    value.as_table().ok_or_else(|| RouteConfigError::BadValue {
        field: field.to_owned(),
    })
}

fn as_string(value: &toml::Value, field: &str) -> Result<String, RouteConfigError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| RouteConfigError::BadValue {
            field: field.to_owned(),
        })
}

fn check_name(name: &str, field: &str) -> Result<(), RouteConfigError> {
    if name.is_empty() {
        return Err(RouteConfigError::BadValue {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn parse_rate(value: &toml::Value, field: &str) -> Result<Rate, RouteConfigError> {
    let text = as_string(value, field)?;
    Rate::parse(&text).map_err(|_| RouteConfigError::BadValue {
        field: field.to_owned(),
    })
}

fn parse_links(
    section: Option<&toml::Value>,
) -> Result<BTreeMap<String, LinkConfig>, RouteConfigError> {
    let Some(value) = section else {
        return Ok(BTreeMap::new());
    };
    let table = as_table(value, "links")?;
    let mut links = BTreeMap::new();
    for (name, entry) in table {
        let field = format!("links.\"{name}\"");
        check_name(name, &field)?;
        let entry_table = as_table(entry, &field)?;
        let mut transport = None;
        let mut bandwidth = None;
        for (key, item) in entry_table {
            let item_field = format!("{field}.{key}");
            match key.as_str() {
                "transport" => transport = Some(as_string(item, &item_field)?),
                "bandwidth" => bandwidth = Some(parse_rate(item, &item_field)?),
                _ => return Err(RouteConfigError::UnknownField { field: item_field }),
            }
        }
        let transport = transport.ok_or_else(|| RouteConfigError::MissingField {
            field: format!("{field}.transport"),
        })?;
        check_name(&transport, &format!("{field}.transport"))?;
        let bandwidth = bandwidth.ok_or_else(|| RouteConfigError::MissingField {
            field: format!("{field}.bandwidth"),
        })?;
        links.insert(
            name.clone(),
            LinkConfig {
                transport,
                bandwidth,
            },
        );
    }
    Ok(links)
}

fn parse_routing(section: Option<&toml::Value>) -> Result<RouteOverrides, RouteConfigError> {
    let Some(value) = section else {
        return Ok(RouteOverrides::default());
    };
    let table = as_table(value, "routing")?;
    let mut overrides = RouteOverrides::default();
    for (key, item) in table {
        match key.as_str() {
            "prefer" => overrides.prefer = parse_string_map(item, "routing.prefer")?,
            "deny" => overrides.deny = parse_string_sets(item, "routing.deny")?,
            "force" => overrides.force = parse_string_lists(item, "routing.force")?,
            "export_limit" => overrides.export_limit = parse_export_limits(item)?,
            other => {
                return Err(RouteConfigError::UnknownField {
                    field: format!("routing.{other}"),
                });
            }
        }
    }
    Ok(overrides)
}

fn parse_string_map(
    value: &toml::Value,
    section: &str,
) -> Result<BTreeMap<String, String>, RouteConfigError> {
    let table = as_table(value, section)?;
    let mut out = BTreeMap::new();
    for (topic, item) in table {
        let field = format!("{section}.\"{topic}\"");
        check_name(topic, &field)?;
        let link = as_string(item, &field)?;
        check_name(&link, &field)?;
        out.insert(topic.clone(), link);
    }
    Ok(out)
}

fn parse_string_sets(
    value: &toml::Value,
    section: &str,
) -> Result<BTreeMap<String, BTreeSet<String>>, RouteConfigError> {
    let table = as_table(value, section)?;
    let mut out = BTreeMap::new();
    for (topic, item) in table {
        let field = format!("{section}.\"{topic}\"");
        check_name(topic, &field)?;
        let array = item.as_array().ok_or_else(|| RouteConfigError::BadValue {
            field: field.clone(),
        })?;
        let mut set = BTreeSet::new();
        for element in array {
            let link = as_string(element, &field)?;
            check_name(&link, &field)?;
            set.insert(link);
        }
        out.insert(topic.clone(), set);
    }
    Ok(out)
}

fn parse_string_lists(
    value: &toml::Value,
    section: &str,
) -> Result<BTreeMap<String, Vec<String>>, RouteConfigError> {
    let table = as_table(value, section)?;
    let mut out = BTreeMap::new();
    for (topic, item) in table {
        let field = format!("{section}.\"{topic}\"");
        check_name(topic, &field)?;
        let mut links = Vec::new();
        match item {
            toml::Value::String(link) => {
                check_name(link, &field)?;
                links.push(link.clone());
            }
            toml::Value::Array(array) => {
                for element in array {
                    let link = as_string(element, &field)?;
                    check_name(&link, &field)?;
                    links.push(link);
                }
            }
            _ => return Err(RouteConfigError::BadValue { field }),
        }
        out.insert(topic.clone(), links);
    }
    Ok(out)
}

fn parse_export_limits(
    value: &toml::Value,
) -> Result<BTreeMap<String, BTreeMap<String, Rate>>, RouteConfigError> {
    let table = as_table(value, "routing.export_limit")?;
    let mut out: BTreeMap<String, BTreeMap<String, Rate>> = BTreeMap::new();
    for (topic, item) in table {
        let field = format!("routing.export_limit.\"{topic}\"");
        check_name(topic, &field)?;
        let links = as_table(item, &field)?;
        for (link, rate_value) in links {
            let link_field = format!("{field}.\"{link}\"");
            check_name(link, &link_field)?;
            let rate = parse_rate(rate_value, &link_field)?;
            out.entry(topic.clone())
                .or_default()
                .insert(link.clone(), rate);
        }
    }
    Ok(out)
}
