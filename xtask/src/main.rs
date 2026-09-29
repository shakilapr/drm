//! Repository automation entry point (codebase.md 5.1). Commands are registered in
//! `xtask/README.md` and covered by ADR 0003; no command runs outside that registry.
//! REQ: REQ-QAL-003 (requirement with no implementation = 0), REQ-QAL-006 (every REQ
//! maps to implementation, evidence-classed test, passing result).

#![forbid(unsafe_code)]

mod golden;
mod layering;

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
            "usage: cargo xtask <fmt [--check] | lint | test | check-layering | audit-traceability>"
        );
    };
    let flags = &args[1..];
    match cmd.as_str() {
        "fmt" => fmt(flags),
        "lint" => lint(flags),
        "test" => test(flags),
        "check-layering" => layering::check(flags),
        "audit-traceability" => audit_traceability(flags),
        "golden-gen" => golden::generate(flags),
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
        ("mypy", vec!["."]),
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
fn test(flags: &[String]) -> Result<()> {
    if !flags.is_empty() {
        bail!("test takes no flags");
    }
    spawn("cargo", &["test", "--workspace"]).context("xtask test")?;
    Ok(())
}

/// REQ: REQ-QAL-003 — `xtask audit-traceability --registry`: registry.csv structural
/// lint (schema, unique ids, area enum, phase resolution form); `--citations` reports
/// REQ citation coverage over crate sources; no flags ⇒ both.
fn audit_traceability(flags: &[String]) -> Result<()> {
    let (mut registry, mut citations) = (false, false);
    for f in flags {
        match f.as_str() {
            "--registry" => registry = true,
            "--citations" => citations = true,
            other => bail!("unknown audit-traceability flag `{other}`"),
        }
    }
    if !registry && !citations {
        registry = true;
        citations = true;
    }
    if registry {
        layering::audit_registry()?;
    }
    if citations {
        audit_citations()?;
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
/// informational in W1 (enforcement tightens with xtask audit-traceability work).
fn audit_citations() -> Result<()> {
    let mut files = 0;
    let mut cited = 0;
    let mut uncited: Vec<String> = Vec::new();
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
                files += 1;
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                if text.contains("REQ:") {
                    cited += 1;
                } else {
                    uncited.push(path.display().to_string());
                }
            }
        }
    }
    println!("citations: {cited}/{files} .rs files carry REQ citations");
    if !uncited.is_empty() {
        println!("uncited: {}", uncited.join(", "));
    }
    Ok(())
}
