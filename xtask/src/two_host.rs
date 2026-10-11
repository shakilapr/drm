//! Two-host router/transport matrix runner (work-plan M10W2-E4-T1; ADR 0003
//! extension command). Drives the whole-system conformance harness at
//! `tests/conformance` (excluded from the root workspace because it links the
//! runtime and toolchain layers together) and reports the topic-class matrix it
//! exercises.
//! REQ: REQ-QAL-008, REQ-ROUTE-022, REQ-ROUTE-031, REQ-ROUTE-032.

use anyhow::{Context, Result};

/// One topic class exchanged by the two-host simulation (router-core.md
/// §Protocols *Two-host sim plan*). The authoritative test lives in
/// `tests/conformance/two_host.rs`; this table is the runner's report.
/// REQ: REQ-ROUTE-029, REQ-ROUTE-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicClass {
    /// Stable class name.
    pub name: &'static str,
    /// Transport the class rides (`udp` datagram or `tcp` stream).
    pub transport: &'static str,
}

/// Every topic class the two-host sim must exchange.
/// REQ: REQ-ROUTE-029, REQ-ROUTE-032.
pub const TOPIC_CLASSES: &[TopicClass] = &[
    TopicClass {
        name: "structured",
        transport: "udp",
    },
    TopicClass {
        name: "bulk",
        transport: "tcp",
    },
    TopicClass {
        name: "event",
        transport: "udp",
    },
];

/// `cargo xtask test --two-host`: runs the two-host conformance harness
/// (`tests/conformance`) — two router instances as distinct hosts exchanging
/// every topic class over loopback, plus the route-derivation ↔ router-filtering
/// e2e.
/// REQ: REQ-QAL-008, REQ-ROUTE-022.
///
/// # Errors
/// [`anyhow::Error`] when the conformance harness build or tests fail.
pub fn run() -> Result<()> {
    crate::spawn(
        "cargo",
        &["test", "--manifest-path", "tests/conformance/Cargo.toml"],
    )
    .context("xtask test --two-host: conformance harness")?;
    let classes = TOPIC_CLASSES
        .iter()
        .map(|c| format!("{}/{}", c.name, c.transport))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "two-host sim: {} class(es) green ({classes})",
        TOPIC_CLASSES.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::TOPIC_CLASSES;

    /// REQ: REQ-ROUTE-032, REQ-ROUTE-029 — the matrix covers the structured,
    /// bulk, and event classes, with UDP for small classes and TCP for bulk.
    #[test]
    fn matrix_covers_topic_classes() {
        assert_eq!(TOPIC_CLASSES.len(), 3, "three topic classes");
        let names: Vec<&str> = TOPIC_CLASSES.iter().map(|c| c.name).collect();
        for want in ["structured", "bulk", "event"] {
            assert!(names.contains(&want), "missing class {want}");
        }
        let bulk = TOPIC_CLASSES
            .iter()
            .find(|c| c.name == "bulk")
            .unwrap_or_else(|| unreachable!("bulk class"));
        assert_eq!(bulk.transport, "tcp", "bulk rides TCP records");
        for class in TOPIC_CLASSES.iter().filter(|c| c.name != "bulk") {
            assert_eq!(class.transport, "udp", "small classes ride UDP");
        }
    }
}
