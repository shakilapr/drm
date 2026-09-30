//! Platform SHM conformance suite (shm-backend.md §Test Matrix; work-plan
//! M01W3-E1-T1). Linux-only: POSIX shm + flock. Multiprocess scenarios spawn
//! this same test binary in child mode (`DRM_PLATFORM_CHILD` set).
//! REQ: REQ-CORE-072, REQ-CORE-098, REQ-CORE-070, REQ-CORE-073.
#![cfg(unix)]

pub(crate) mod child_util;

mod cleanup;
mod open_semantics;
mod race_startup;
mod same_channel;
