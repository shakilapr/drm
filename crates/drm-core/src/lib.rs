//! Runtime core (codebase.md 2.1 layer `runtime`): SHM backends, channels, rings,
//! hazard pools, epochs, precommit. MVP boundary: no networking, disk, or compiler
//! dependencies (codebase.md 2.2 R2). REQ: REQ-CORE-072, REQ-CORE-073, REQ-CORE-090.

pub mod alloc;
pub mod canon;
pub mod channel;
pub mod deploy;
pub mod hazard;
pub mod ring;
pub mod shm;
pub mod trace;

#[cfg(test)]
pub(crate) mod test_guard;
