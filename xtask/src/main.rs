//! Repository automation entry point (codebase.md 5.1). Commands are registered in
//! `xtask/README.md` and covered by ADR 0003; no command runs outside that registry.
//! REQ: REQ-QAL-003 (requirement with no implementation = 0), REQ-QAL-006 (every REQ
//! maps to implementation, evidence-classed test, passing result).

#![forbid(unsafe_code)]

mod cross_os;
mod fault;
mod gencheck;
mod golden;
mod layering;
mod soak;
mod studio_ts;
mod traceability;
mod two_host;
mod workplan;

use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

/// REQ: REQ-QAL-006 — single automation entry point; `cargo xtask <cmd>`.
fn main() -> std::process::ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// REQ: REQ-QAL-006 — command registry dispatch (codebase.md 5.1).
fn run(args: &[String]) -> Result<()> {
    let Some(cmd) = args.first() else {
        bail!(
            "usage: cargo xtask <fmt [--check] | lint | test [--stress|--two-host|--cross-os [--transport udp|tcp|serial | --full]] | check-layering | audit-traceability [--registry|--citations|--matrix] | soak (--duration <h>|--iterations <n>) [--topology <t>] [--ledger <path>] [--parallel <n>] [--memory-bound <mb>] [--report <path>] [--dashboard] | fault [--campaign <name>] [--seed <n>] [--steps <n>] [--list] [--dry-run] [--json <path>] | golden-gen | gen-check [--workspace <path>] | workplan [status|next|watch|mark-done <week>]>"
        );
    };
    let flags = &args[1..];
    match cmd.as_str() {
        "fmt" => fmt(flags),
        "lint" => lint(flags),
        "test" => test(flags),
        "check-layering" => layering::check(flags),
        "audit-traceability" => audit_traceability(flags),
        "soak" => soak::run(flags),
        "fault" => fault::run(flags),
        "golden-gen" => golden::generate(flags),
        "gen-check" => gencheck::run(flags),
        "workplan" => workplan::run(flags),
        other => bail!("unknown command `{other}`; see xtask/README.md"),
    }
}

/// REQ: REQ-QAL-006 — `xtask fmt [--check]`: rustfmt plus any installed secondary
/// formatter (clang-format, ruff, prettier, taplo, markdownlint); missing tools
/// are skipped so local gates stay runnable, CI installs them all.
fn fmt(flags: &[String]) -> Result<()> {
    let check = match flags {
        [] => false,
        [f] if f == "--check" => true,
        other => bail!("unknown fmt flags {other:?}; supported: --check"),
    };
    cargo_fmt(check)?;
    for (tool, check_args, write_args) in [
        ("ruff", vec!["format", "--check", "."], vec!["format", "."]),
        ("prettier", vec!["--check", "."], vec!["--write", "."]),
        ("taplo", vec!["format", "--check"], vec!["format"]),
        ("markdownlint", vec!["."], vec!["--fix", "."]),
    ] {
        if tool_found(tool) {
            let args = if check { check_args } else { write_args };
            spawn(tool, &args).with_context(|| format!("xtask fmt: {tool}"))?;
        } else {
            eprintln!("xtask fmt: skip {tool} (not installed)");
        }
    }
    clang_format(check)?;
    Ok(())
}

/// REQ: REQ-QAL-006 — `xtask lint`: clippy `-D warnings` plus installed secondary
/// linters (mypy, eslint, yamllint, markdownlint); missing tools skipped locally.
fn lint(flags: &[String]) -> Result<()> {
    if !flags.is_empty() {
        bail!("lint takes no flags");
    }
    spawn(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )
    .context("xtask lint: clippy")?;
    for (tool, args) in [
        ("mypy", vec!["bindings"]),
        ("eslint", vec!["."]),
        ("yamllint", vec![".github", "xtask"]),
        ("markdownlint", vec!["."]),
    ] {
        if tool_found(tool) {
            spawn(tool, &args).with_context(|| format!("xtask lint: {tool}"))?;
        } else {
            eprintln!("xtask lint: skip {tool} (not installed)");
        }
    }
    Ok(())
}

/// REQ: REQ-QAL-006 — `xtask test`: workspace unit + doc tests.
/// `--stress` runs the ignored `stress_*` suites (multiprocess, long-budget;
/// work-plan M02W1-E4-T4). REQ: REQ-QAL-006.
fn test(flags: &[String]) -> Result<()> {
    match flags
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => {
            spawn("cargo", &["test", "--workspace"]).context("xtask test")?;
        }
        ["--stress"] => {
            spawn(
                "cargo",
                &[
                    "test",
                    "--workspace",
                    "--",
                    "--ignored",
                    "stress",
                    "--nocapture",
                ],
            )
            .context("xtask test --stress")?;
        }
        ["--cross-os"] => {
            cross_os::run().context("xtask test --cross-os")?;
        }
        ["--cross-os", "--transport", transport] => {
            let transport = cross_os::InteropTransport::parse(transport)?;
            cross_os::run_interop(Some(transport)).context("xtask test --cross-os --transport")?;
        }
        ["--cross-os", "--full"] => {
            cross_os::run_full().context("xtask test --cross-os --full")?;
        }
        ["--two-host"] => {
            two_host::run().context("xtask test --two-host")?;
        }
        other => bail!(
            "test flags: {other:?} (expected none, --stress, --two-host, --cross-os [--transport udp|tcp|serial | --full])"
        ),
    }
    Ok(())
}

/// REQ: REQ-QAL-003, REQ-QAL-006 — `xtask audit-traceability --registry`:
/// registry.csv structural lint (schema, unique ids, area enum, phase resolution
/// form); `--citations` reports REQ citation coverage over crate sources;
/// `--matrix` builds the `I-QAL1` requirement → implementation → test matrix;
/// no flags ⇒ registry + citations.
fn audit_traceability(flags: &[String]) -> Result<()> {
    let (mut registry, mut citations, mut matrix) = (false, false, false);
    for f in flags {
        match f.as_str() {
            "--registry" => registry = true,
            "--citations" => citations = true,
            "--matrix" => matrix = true,
            other => bail!("unknown audit-traceability flag `{other}`"),
        }
    }
    if !registry && !citations && !matrix {
        registry = true;
        citations = true;
    }
    if registry {
        layering::audit_registry()?;
    }
    if citations {
        audit_citations()?;
    }
    if matrix {
        traceability::run()?;
    }
    Ok(())
}

/// REQ: REQ-QAL-006 — `cargo fmt --all` (optionally `--check`).
fn cargo_fmt(check: bool) -> Result<()> {
    let mut args = vec!["fmt", "--all"];
    if check {
        args.push("--");
        args.push("--check");
    }
    spawn("cargo", &args).context("xtask fmt: rustfmt")?;
    Ok(())
}

/// REQ: REQ-QAL-006 — clang-format over C/C++ sources under `bindings/`; skipped
/// when none exist or the tool is absent; `--dry-run --Werror` in check mode.
fn clang_format(check: bool) -> Result<()> {
    if !tool_found("clang-format") {
        eprintln!("xtask fmt: skip clang-format (not installed)");
        return Ok(());
    }
    let mut sources: Vec<String> = Vec::new();
    for path in walk("bindings") {
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("c" | "h" | "cpp" | "hpp")) {
            sources.push(path.display().to_string());
        }
    }
    if sources.is_empty() {
        return Ok(());
    }
    let mut args = if check {
        vec!["--dry-run", "--Werror"]
    } else {
        vec!["-i"]
    };
    args.extend(sources.iter().map(String::as_str));
    spawn("clang-format", &args).context("xtask fmt: clang-format")?;
    Ok(())
}

/// REQ: REQ-QAL-006 — recursive file walker used by the audits.
fn walk(root: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// REQ: REQ-QAL-006 — PATH probe so secondary tools degrade gracefully.
fn tool_found(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// REQ: REQ-QAL-006 — inherited-stdio spawn; nonzero exit ⇒ error.
fn spawn(tool: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(tool)
        .args(args)
        .status()
        .with_context(|| format!("spawn {tool}"))?;
    if !status.success() {
        return Err(anyhow!("{tool} exited with {status}"));
    }
    Ok(())
}

/// REQ: REQ-QAL-003 — citation coverage report over `crates/**/*.rs` and `xtask/**/*.rs`;
/// REQ: REQ-QAL-003 — citation scan over `.rs` files under the given roots:
/// a file without any `REQ:` marker is listed as uncited. Blocking per
/// work-plan M02W1-E4-T3 (CI fails on an uncited file).
fn citation_offenders(roots: &[std::path::PathBuf]) -> Result<Vec<String>> {
    let mut offenders: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                if !text.contains("REQ:") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    offenders.sort();
    Ok(offenders)
}

/// REQ: REQ-QAL-003 — `xtask audit-traceability --citations`: file-level REQ
/// citation coverage over crate sources; any uncited file fails the audit.
fn audit_citations() -> Result<()> {
    let offenders = citation_offenders(&[
        std::path::PathBuf::from("crates"),
        std::path::PathBuf::from("xtask"),
    ])?;
    let total = count_rs_files();
    println!(
        "citations: {}/{total} .rs files carry REQ citations",
        total - offenders.len()
    );
    if !offenders.is_empty() {
        bail!(
            "uncited source files (REQ marker required, work-plan M02W1-E4-T3): {}",
            offenders.join(", ")
        );
    }
    Ok(())
}

/// Total `.rs` file count under the audit roots (for the coverage report).
fn count_rs_files() -> usize {
    let mut count = 0;
    let mut stack = vec![
        std::path::PathBuf::from("crates"),
        std::path::PathBuf::from("xtask"),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                count += 1;
            }
        }
    }
    count
}

/// REQ: REQ-QAL-003 — self-test: the citation scan catches an uncited file and
/// clears a cited one (CI red→green fixture, M02W1-E4-T3).
#[cfg(test)]
mod citation_tests {
    use super::citation_offenders;

    #[test]
    fn detects_uncited_and_clears_cited() {
        let dir =
            std::env::temp_dir().join(format!("drm-citation-selftest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("tmpdir: {e}"));
        let cited = dir.join("cited.rs");
        let uncited = dir.join("uncited.rs");
        std::fs::write(&cited, "//! REQ: REQ-QAL-003\nfn main() {}\n")
            .unwrap_or_else(|e| unreachable!("write cited: {e}"));
        std::fs::write(&uncited, "//! no marker here\nfn main() {}\n")
            .unwrap_or_else(|e| unreachable!("write uncited: {e}"));
        let offenders = citation_offenders(std::slice::from_ref(&dir))
            .unwrap_or_else(|e| unreachable!("scan: {e}"));
        assert_eq!(offenders.len(), 1, "uncited file must be listed");
        assert!(offenders[0].ends_with("uncited.rs"));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| unreachable!("cleanup: {e}"));
    }
}

/// REQ: REQ-SPATIAL-036, REQ-SPATIAL-004, REQ-OPS-008, REQ-OPS-009, REQ-OPS-014 —
/// M14W3 design-confirmation lock: the normative algorithm docs must keep the
/// confirmed spatial quantities, clock domains, recorder `on_full` policies, and
/// trace dictionary versioning. A design edit that drops one turns this red
/// (regression permanence, AGENTS #14).
#[cfg(test)]
mod design_lock {
    use std::path::Path;

    /// Reads a repository doc relative to the workspace root.
    fn repo_doc(rel: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."));
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| unreachable!("read {rel}: {e}"))
    }

    #[test]
    fn spatial_types_confirmed() {
        let doc = repo_doc("docs/algorithms/spatial/spatial-core.md");
        for ty in [
            "Point3",
            "Direction3",
            "Vector3",
            "Pose3",
            "Transform3",
            "Twist3",
        ] {
            assert!(doc.contains(ty), "spatial-core.md lost typed quantity {ty}");
        }
        for item in ["Stamped", "SpatialId"] {
            assert!(doc.contains(item), "spatial-core.md lost binding {item}");
        }
    }

    #[test]
    fn clock_domains_confirmed() {
        let doc = repo_doc("docs/algorithms/spatial/spatial-core.md");
        for domain in ["HostMonotonic", "Device", "Global", "SimReplay"] {
            assert!(
                doc.contains(domain),
                "spatial-core.md lost clock domain {domain}"
            );
        }
        for item in [
            "ClockIdentity",
            "ClockMapping",
            "rate_ratio",
            "map_timestamp",
        ] {
            assert!(doc.contains(item), "spatial-core.md lost clock item {item}");
        }
    }

    #[test]
    fn record_policies_confirmed() {
        let doc = repo_doc("docs/algorithms/ops/record-replay.md");
        for policy in ["Drop", "Stop", "Backpressure"] {
            assert!(
                doc.contains(policy),
                "record-replay.md lost on_full policy {policy}"
            );
        }
        for item in ["GapLedger", "ingress_loop", "chunk_target"] {
            assert!(
                doc.contains(item),
                "record-replay.md lost record item {item}"
            );
        }
    }

    #[test]
    fn trace_dictionary_confirmed() {
        let doc = repo_doc("docs/algorithms/ops/observability-plane.md");
        for item in ["TraceRingSet", "TraceLane", "TraceEvent", "dict_version"] {
            assert!(
                doc.contains(item),
                "observability-plane.md lost trace item {item}"
            );
        }
        for item in ["diag_sample", "deadline_tick"] {
            assert!(
                doc.contains(item),
                "observability-plane.md lost diag item {item}"
            );
        }
    }

    /// REQ: REQ-OPS-012, REQ-OPS-013, REQ-OPS-020, REQ-OPS-021, REQ-OPS-023,
    /// REQ-OPS-025 — M14W4 RPC design seed: the algorithm doc must keep the
    /// correlated-topic layout, cid bookkeeping, cancel, and deadline semantics.
    #[test]
    fn rpc_design_confirmed() {
        let doc = repo_doc("docs/algorithms/ops/rpc-actions.md");
        for item in [
            "CorrelationId",
            "client_id",
            "session_nonce",
            "server_session",
            "CancelAccepted",
            "CancelRejected",
            "remaining_budget_ns",
            "SequenceExhausted",
        ] {
            assert!(
                doc.contains(item),
                "rpc-actions.md lost cid/deadline item {item}"
            );
        }
        for topic in [
            "request/<client_id>",
            "cancel/<client_id>",
            "event/<client_id>",
        ] {
            assert!(doc.contains(topic), "rpc-actions.md lost topic {topic}");
        }
        assert!(
            doc.contains("Design Confirmation (M14W4)"),
            "rpc-actions.md lost the M14W4 design confirmation"
        );
    }

    /// REQ: REQ-STUDIO-001, REQ-STUDIO-006, REQ-STUDIO-007, REQ-STUDIO-011,
    /// REQ-STUDIO-013 — M14W4 Studio prep survey: the protocol doc must keep the
    /// stdio host shape, generated-protocol rule, handshake, and plane split.
    #[test]
    fn studio_design_confirmed() {
        let doc = repo_doc("docs/algorithms/studio/studio-protocol.md");
        for item in [
            "serve_stdio",
            "Hello",
            "HelloAck",
            "Capability",
            "base_revision",
            "ChangeSet",
            "parse_bounded",
        ] {
            assert!(
                doc.contains(item),
                "studio-protocol.md lost protocol item {item}"
            );
        }
        for rule in [
            "studio-protocol",
            "gen-check",
            "engine calls only",
            "binary frames",
        ] {
            assert!(doc.contains(rule), "studio-protocol.md lost rule {rule}");
        }
        assert!(
            doc.contains("Design Confirmation (M14W4)"),
            "studio-protocol.md lost the M14W4 design confirmation"
        );
    }

    /// REQ: REQ-OPS-004, REQ-OPS-015, REQ-OPS-019, REQ-ROUTE-025,
    /// REQ-ROUTE-040, REQ-CORE-087, REQ-STUDIO-011 — M14W4 tooling design lock:
    /// the consolidated lock record must name every Phase 32 sub-design and its
    /// owner signature.
    #[test]
    fn tooling_design_lock_confirmed() {
        let doc = repo_doc("docs/spec/tooling-design-lock.md");
        for sub in [
            "echo",
            "hz",
            "diag",
            "record",
            "trace",
            "serial/MCU",
            "RPC/actions",
            "Studio bridge",
        ] {
            assert!(
                doc.contains(sub),
                "tooling-design-lock.md lost sub-design {sub}"
            );
        }
        for owner in ["E1", "E2", "E3", "E4"] {
            assert!(
                doc.contains(owner),
                "tooling-design-lock.md lost owner {owner}"
            );
        }
    }
}
