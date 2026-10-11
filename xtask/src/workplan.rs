//! Work-plan progress tracker (`cargo xtask workplan`).
//!
//! Watches the 24-month work plan: it enumerates
//! `docs/work-plan/work-plan-mNN-wN.md`, reads the committed progress ledger
//! (`.workplan-progress.json`), and reports the completed weeks, the week
//! currently in progress, and what remains. A driver loop uses `next` /
//! `mark-done` to advance the plan week by week until it is complete.
//! REQ: REQ-QAL-006.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Committed progress ledger: the weeks already completed. Canonical order is
/// derived from the file set, not stored.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Progress {
    /// Completed week stems, e.g. `m03w4`.
    done: Vec<String>,
}

/// Repository root (the `xtask` crate lives at `<root>/xtask`).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Path of the committed progress ledger.
fn progress_path() -> PathBuf {
    root().join(".workplan-progress.json")
}

/// A work-plan week: `(month, week, stem, file)`.
type Week = (u32, u32, String, PathBuf);

/// Canonically ordered work-plan weeks found under `docs/work-plan/`.
/// REQ: REQ-QAL-006.
fn weeks() -> Result<Vec<Week>> {
    let dir = root().join("docs/work-plan");
    let mut out: Vec<Week> = Vec::new();
    let entries = fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name
            .strip_prefix("work-plan-")
            .and_then(|s| s.strip_suffix(".md"))
        else {
            continue;
        };
        let Some((month_tok, week_tok)) = stem.split_once("-w") else {
            continue;
        };
        let Some(month_digits) = month_tok.strip_prefix('m') else {
            continue;
        };
        let (Ok(month), Ok(week)) = (month_digits.parse::<u32>(), week_tok.parse::<u32>()) else {
            continue;
        };
        // Canonical stem (`m08w1`), matching `norm` so the ledger round-trips.
        let stem = format!("m{month:02}w{week}");
        out.push((month, week, stem, entry.path()));
    }
    if out.is_empty() {
        bail!("no work-plan files under {}", dir.display());
    }
    out.sort_by_key(|(m, w, _, _)| (*m, *w));
    Ok(out)
}

/// Load the progress ledger (empty when absent).
fn load() -> Result<Progress> {
    let path = progress_path();
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).with_context(|| format!("parse {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Progress::default()),
        Err(e) => Err(anyhow!("read {}: {e}", path.display())),
    }
}

/// Write the progress ledger (sorted, deduped, trailing newline).
fn save(progress: &Progress) -> Result<()> {
    let mut done = progress.done.clone();
    done.sort();
    done.dedup();
    let text = serde_json::to_string_pretty(&Progress { done })?;
    let path = progress_path();
    fs::write(&path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Normalize a week token to its stem (`m08w1`): accepts `m08w1`, `M08W1`,
/// `work-plan-m08w1`, `work-plan-m08-w1.md`.
fn norm(token: &str) -> String {
    let t = token.trim().trim_end_matches(".md");
    let t = t.strip_prefix("work-plan-").unwrap_or(t);
    t.to_lowercase().replace("-w", "w")
}

/// REQ: REQ-QAL-006 — `xtask workplan [status | next | watch [--interval <s>] |
/// mark-done <week>]`.
///
/// # Errors
/// Unknown flags, an unreadable work-plan directory, or a malformed ledger.
pub fn run(flags: &[String]) -> Result<()> {
    match flags {
        [] => status(),
        [f] if f == "status" => status(),
        [f] if f == "next" => next(),
        [f] if f == "watch" => watch(30),
        [f, w] if f == "mark-done" => mark_done(w),
        [f, g, n] if f == "watch" && g == "--interval" => {
            watch(n.parse().context("--interval must be seconds")?)
        }
        other => bail!("unknown workplan flags {other:?}; see xtask/README.md"),
    }
}

/// Print the plan status: totals, the week in progress, and the next few up.
fn status() -> Result<()> {
    let weeks = weeks()?;
    let done: BTreeSet<String> = load()?.done.iter().map(|s| norm(s)).collect();
    let total = weeks.len();
    let completed = weeks.iter().filter(|w| done.contains(&w.2)).count();
    let remaining = total - completed;
    println!("work plan: {completed}/{total} weeks complete, {remaining} remaining");
    match weeks.iter().find(|w| !done.contains(&w.2)) {
        Some((month, week, stem, path)) => {
            println!("in progress: {stem}  (month {month} week {week})");
            println!("             {}", path.display());
        }
        None => println!("in progress: - (plan complete)"),
    }
    if remaining > 0 {
        let upcoming = weeks
            .iter()
            .filter(|w| !done.contains(&w.2))
            .take(5)
            .map(|w| w.2.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("next up: {upcoming}");
    }
    Ok(())
}

/// Print the next incomplete week stem (empty when the plan is complete).
fn next() -> Result<()> {
    let weeks = weeks()?;
    let done: BTreeSet<String> = load()?.done.iter().map(|s| norm(s)).collect();
    if let Some((_, _, stem, _)) = weeks.into_iter().find(|w| !done.contains(&w.2)) {
        println!("{stem}");
    }
    Ok(())
}

/// Record `week` as complete.
fn mark_done(week: &str) -> Result<()> {
    let stem = norm(week);
    if !weeks()?.iter().any(|w| w.2 == stem) {
        bail!("unknown week `{week}` (expected e.g. m08w1)");
    }
    let mut progress = load()?;
    if !progress.done.iter().any(|d| norm(d) == stem) {
        progress.done.push(stem.clone());
    }
    save(&progress)?;
    println!("marked {stem} complete");
    Ok(())
}

/// Reprint the status every `interval` seconds (a live watch for the driver).
fn watch(interval: u64) -> Result<()> {
    loop {
        status()?;
        std::thread::sleep(Duration::from_secs(interval.max(1)));
    }
}
