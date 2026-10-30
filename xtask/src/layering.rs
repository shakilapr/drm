//! Crate dependency-layer verification (codebase.md 2.1/2.2 R6) and registry.csv
//! structural lint (codebase.md 6.2). REQ: REQ-QAL-003, REQ-QAL-006.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Layers from codebase.md 2.1 (membership grows as crates land).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Types,
    Runtime,
    Toolchain,
    Ops,
    Bindings,
    Facade,
}

/// REQ: REQ-QAL-006 — layer membership mirrors codebase.md 2.1 exactly.
fn layer_of(crate_name: &str) -> Option<Layer> {
    match crate_name {
        "drm-types" | "drm-cdr" => Some(Layer::Types),
        "drm-core" | "drm-ffi" | "drm-router" | "drm-spatial" => Some(Layer::Runtime),
        "drm-workspace" => Some(Layer::Toolchain),
        "drm-record" | "drm-diag" | "drm-trace" | "drm-rpc" => Some(Layer::Ops),
        "drm" | "drm-python" => Some(Layer::Bindings),
        "drm-cli" | "drm-extension-host" | "xtask" => Some(Layer::Facade),
        _ => None,
    }
}

/// REQ: REQ-QAL-006 — dependency-layer allow matrix; `deps` = layers `from` may depend on.
fn allowed(from: Layer) -> &'static [Layer] {
    match from {
        Layer::Types => &[],
        Layer::Runtime | Layer::Ops | Layer::Bindings => &[Layer::Types, Layer::Runtime],
        Layer::Toolchain => &[Layer::Types],
        Layer::Facade => &[Layer::Types, Layer::Runtime, Layer::Toolchain, Layer::Ops],
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    #[serde(rename = "kind")]
    kind: Option<String>,
}

/// REQ: REQ-QAL-006 — `xtask check-layering`: cargo metadata graph must match
/// codebase.md 2.1 (types zero-dep; runtime→types+runtime; toolchain→types;
/// ops→types+runtime, never each other; facade→any lower layer).
pub fn check(_flags: &[String]) -> Result<()> {
    let meta = cargo_metadata()?;
    let member_names = [
        "drm-types",
        "drm-core",
        "drm-workspace",
        "drm-router",
        "drm-cli",
        "drm-ffi",
        "drm",
        "drm-python",
        "xtask",
    ];
    let mut violations: Vec<String> = Vec::new();
    for pkg in &meta.packages {
        let Some(from) = layer_of(&pkg.name) else {
            continue;
        };
        if from == Layer::Types {
            let normal: Vec<&str> = pkg
                .dependencies
                .iter()
                .filter(|d| d.kind.is_none() && member_names.contains(&d.name.as_str()))
                .map(|d| d.name.as_str())
                .collect();
            // zero deps means zero of everything, checked below via non-member externals.
            let externals: Vec<&str> = pkg
                .dependencies
                .iter()
                .filter(|d| d.kind.is_none())
                .map(|d| d.name.as_str())
                .collect();
            if !externals.is_empty() {
                violations.push(format!(
                    "drm-types must have zero dependencies, has {externals:?}"
                ));
            }
            let _ = normal;
            continue;
        }
        for dep in &pkg.dependencies {
            if dep.kind.is_some() {
                continue;
            }
            let Some(to) = layer_of(&dep.name) else {
                continue;
            };
            if !allowed(from).contains(&to) {
                violations.push(format!(
                    "{} ({from:?}) must not depend on {} ({to:?})",
                    pkg.name, dep.name
                ));
            }
        }
    }
    if violations.is_empty() {
        println!("layering: OK ({} workspace crates)", meta.packages.len());
        Ok(())
    } else {
        bail!("layering violations:\n{}", violations.join("\n"));
    }
}

/// REQ: REQ-QAL-003 — registry.csv structural lint: header schema, unique ids,
/// area enum, phase resolution form (`N` or `N, M ...`), status enum.
pub fn audit_registry() -> Result<()> {
    const AREAS: [&str; 8] = [
        "CORE", "TOOL", "ROUTE", "SPATIAL", "BIND", "STUDIO", "OPS", "QAL",
    ];
    const STATUSES: [&str; 4] = ["specified", "implemented", "tested", "verified"];
    let text = std::fs::read_to_string("docs/req/registry.csv").context("read registry.csv")?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().context("registry.csv is empty")?;
    if header != "id,area,statement,source_ref,phase,test,status" {
        bail!("registry.csv header mismatch: {header}");
    }
    let mut seen: Vec<String> = Vec::new();
    let mut findings: Vec<String> = Vec::new();
    let mut flagged = 0_usize;
    for (n, line) in lines.enumerate() {
        let row = split_csv(line);
        if row.len() != 7 {
            findings.push(format!(
                "row {}: expected 7 columns, got {}",
                n + 2,
                row.len()
            ));
            continue;
        }
        let [id, area, _statement, _source, phase, _test, status] = row.as_slice() else {
            unreachable!()
        };
        if !id.starts_with("REQ-") {
            findings.push(format!("row {}: id `{id}` malformed", n + 2));
        }
        if seen.contains(id) {
            findings.push(format!("row {}: duplicate id `{id}`", n + 2));
        }
        seen.push(id.clone());
        if !AREAS.contains(&area.as_str()) {
            findings.push(format!("row {}: unknown area `{area}`", n + 2));
        }
        // Multi-phase (`N, M ...`) and unresolved rows are legitimate Phase-1
        // extraction flags (docs/req/README.md); reported, never lint findings.
        if phase.split(',').count() > 1 || phase.trim().parse::<u32>().is_err() {
            flagged += 1;
        }
        if !STATUSES.contains(&status.as_str()) {
            findings.push(format!("row {}: unknown status `{status}`", n + 2));
        }
    }
    println!(
        "registry lint: {} rows, {} findings, {} flagged for phase resolution",
        seen.len(),
        findings.len(),
        flagged
    );
    if findings.is_empty() {
        Ok(())
    } else {
        bail!("registry findings:\n{}", findings.join("\n"));
    }
}

/// REQ: REQ-QAL-003 — quote-aware single-line CSV split (RFC 4180 subset: embedded
/// commas inside double-quoted fields, `""` escapes); never used for multiline fields.
pub(crate) fn split_csv(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => fields.push(std::mem::take(&mut field)),
            c => field.push(c),
        }
    }
    fields.push(field);
    fields
}

/// REQ: REQ-QAL-006 — cargo metadata via the installed cargo toolchain.
fn cargo_metadata() -> Result<Metadata> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .context("spawn cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    serde_json::from_slice(&out.stdout).context("parse cargo metadata")
}
