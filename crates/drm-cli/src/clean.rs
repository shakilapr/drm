//! `drm clean` — guarded namespace teardown (`docs/algorithms/deployment-identity.md`
//! §Functions `clean`).
//!
//! The exclusive deployment active lock proves no node holds a shared hold:
//! success unlinks every deployed segment, a live hold leaves everything
//! untouched. The namespace derives from the deployment id in the generated
//! config — never from a configured path — so a hand-edited artifact cannot
//! redirect cleanup at a foreign namespace (I-DP5).
//! REQ: REQ-CORE-080.

use std::path::{Path, PathBuf};

use drm_core::deploy::{self as deployment, DeploymentConfig};
use drm_core::shm::{CleanReport, LockPaths, SegmentName};

#[cfg(unix)]
use drm_core::shm::{FlockBackend as Locks, PosixShm as Shm};
#[cfg(windows)]
use drm_core::shm::{WinLockBackend as Locks, WinShm as Shm};

use crate::error::CliError;

/// Runs `drm clean` against the deployment recorded in `root`.
///
/// # Errors
/// [`CliError::NotFound`] when no readable deployment config exists;
/// [`CliError::Io`] when the config is malformed or a lock/segment operation
/// fails.
pub fn run(root: &Path) -> Result<(), CliError> {
    let path = drm_workspace::deploy::deployment_path(root);
    let config = DeploymentConfig::load(&path).map_err(|e| {
        CliError::NotFound(format!(
            "no readable deployment config at {}: {e:?} (run `drm deploy`)",
            path.display()
        ))
    })?;
    let id = config
        .id()
        .map_err(|e| CliError::Io(format!("deployment id: {e:?}")))?;
    let namespace = runtime_root().join(id.encoded());
    if !namespace.is_dir() {
        println!(
            "clean: no namespace for deployment {} (nothing to clean)",
            config.deployment_id
        );
        return Ok(());
    }

    let mut names: Vec<SegmentName> = Vec::new();
    for slot in config.slots().values() {
        names.push(
            id.segment_name(u64::from(*slot))
                .map_err(|e| CliError::Io(format!("segment name: {e:?}")))?,
        );
    }
    names.sort();
    names.dedup();

    let paths = LockPaths::new(&namespace);
    match deployment::clean(&Shm, &Locks, &paths, &names) {
        Ok(CleanReport::Cleaned) => {
            println!(
                "clean: unlinked {} segment(s) in {}",
                names.len(),
                namespace.display()
            );
            Ok(())
        }
        Ok(CleanReport::ActiveUntouched) => {
            println!("clean: deployment is active; nothing removed");
            Ok(())
        }
        Err(e) => Err(CliError::Io(format!("clean: {e:?}"))),
    }
}

/// Backend runtime root (Linux `/run/drm`, Windows `%ProgramData%\drm`).
///
/// The root is the one environment-level setting in the identity model; it is
/// overridable with `DRM_RUNTIME_ROOT` for tests and non-default installations.
/// Everything below it derives solely from the deployment id.
fn runtime_root() -> PathBuf {
    if let Some(root) = std::env::var_os("DRM_RUNTIME_ROOT") {
        return PathBuf::from(root);
    }
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\drm")
    } else {
        PathBuf::from("/run/drm")
    }
}
