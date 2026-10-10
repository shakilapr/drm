//! `drm` executable entry point — thin wrapper over [`drm_cli::run`]: parse,
//! dispatch, and map a [`drm_cli::CliError`] to a process exit code.
//! REQ: REQ-TOOL-038 (compaction is explicit-only; a missing acknowledgement is
//! a non-zero refusal), REQ-TOOL-080 (committed lockfile tooling).

#![forbid(unsafe_code)]

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = drm_cli::run(&raw) {
        eprintln!("drm: {err}");
        std::process::exit(err.exit_code());
    }
}
