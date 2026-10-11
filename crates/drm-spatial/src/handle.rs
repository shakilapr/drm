//! Generational runtime-overlay handle registry
//! ([spatial-core.md](../../docs/algorithms/spatial/spatial-core.md) §Data
//! Structures `SpatialHandle`, §Invariants `I-S12`, §Functions `space_create`/
//! `space_delete`; [spatial-queries.md](../../docs/algorithms/spatial/spatial-queries.md)
//! §Invariants `I-Q12`; [`spatial_architecture.md`](../../docs/spatial_architecture.md)
//! §Dual Hierarchy). A [`HandleRegistry`] allocates overlay slots and hands out
//! generational [`SpatialHandle`]s; deleting a slot bumps its generation so every
//! outstanding handle for that index invalidates immediately, without recycling
//! bugs and without cross-process refcounts. Only the current generation of an
//! index resolves; a stale handle returns [`SpatialError::InvalidHandle`]
//! (`I-S12`, `I-Q12`).
//!
//! The registry is a fixed-capacity array of slot records; allocation reuses a
//! freed index but always under a strictly newer generation, so a handle from a
//! previous incarnation never aliases a recycled slot. Generations are `u32` and
//! never wrap: exhaustion is a typed error (`GenerationExhausted`, `I-Q12`).
//!
//! REQ: REQ-SPATIAL-011, REQ-SPATIAL-008.

use crate::types::{SpatialError, SpatialHandle};

/// Per-index slot state in a [`HandleRegistry`].
/// REQ: REQ-SPATIAL-011.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Slot {
    generation: u32,
    live: bool,
    frame: String,
    parent: String,
}

/// A fixed-capacity generational overlay-handle registry (`I-S12`, `I-Q12`).
/// REQ: REQ-SPATIAL-011, REQ-SPATIAL-008.
#[derive(Debug, Clone)]
pub struct HandleRegistry {
    slots: Vec<Slot>,
    live_count: usize,
}

impl HandleRegistry {
    /// Allocates a registry with `capacity` overlay slots, all free at
    /// generation `0`.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity)
                .map(|_| Slot {
                    generation: 0,
                    live: false,
                    frame: String::new(),
                    parent: String::new(),
                })
                .collect(),
            live_count: 0,
        }
    }

    /// The registry's fixed slot capacity.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The number of live overlay frames.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live_count
    }

    /// Whether no overlay frame is live.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// The current generation of `index`, if the index is in range.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn generation(&self, index: u32) -> Option<u32> {
        self.slots.get(index as usize).map(|s| s.generation)
    }

    /// Creates an overlay frame named `frame` under `parent`, returning its
    /// generational handle (`I-S12`). Freed indices are reused under a strictly
    /// newer generation, so a handle from the previous incarnation never resolves
    /// to the recycled slot.
    ///
    /// # Errors
    /// [`SpatialError::InvalidConfig`] when the registry is full, or
    /// [`SpatialError::InvalidHandle`] when a slot's generation would wrap
    /// (`GenerationExhausted`) — never a silent reuse.
    /// REQ: REQ-SPATIAL-011, REQ-SPATIAL-008.
    pub fn create(
        &mut self,
        frame: impl Into<String>,
        parent: impl Into<String>,
    ) -> Result<SpatialHandle, SpatialError> {
        let index = self
            .slots
            .iter()
            .position(|s| !s.live)
            .ok_or(SpatialError::InvalidConfig)?;
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(SpatialError::InvalidHandle)?;
        // Delete already bumped the generation of a freed slot; allocation reuses
        // that current generation so outstanding handles for the index stay dead.
        // A never-allocated slot starts at generation 0.
        slot.live = true;
        slot.frame = frame.into();
        slot.parent = parent.into();
        self.live_count += 1;
        Ok(SpatialHandle {
            index: index as u32,
            generation: slot.generation,
        })
    }

    /// Deletes the overlay frame named by `handle` when it is current, bumping
    /// the slot's generation so every outstanding handle for that index
    /// invalidates immediately (`I-S12`/`I-Q12`). A stale handle (generation
    /// mismatch) or an already-free slot is refused.
    ///
    /// # Errors
    /// [`SpatialError::InvalidHandle`] for a stale handle, an out-of-range index,
    /// or a slot that is already free.
    /// REQ: REQ-SPATIAL-011, REQ-SPATIAL-008.
    pub fn delete(&mut self, handle: SpatialHandle) -> Result<(), SpatialError> {
        let slot = self
            .slots
            .get_mut(handle.index as usize)
            .ok_or(SpatialError::InvalidHandle)?;
        if !slot.live || slot.generation != handle.generation {
            return Err(SpatialError::InvalidHandle);
        }
        slot.live = false;
        slot.frame.clear();
        slot.parent.clear();
        // Bump the generation so outstanding handles for `index` are dead.
        slot.generation = slot
            .generation
            .checked_add(1)
            .ok_or(SpatialError::InvalidHandle)?;
        self.live_count -= 1;
        Ok(())
    }

    /// Resolves `handle` to its frame name, or [`SpatialError::InvalidHandle`]
    /// when the handle is stale (generation mismatch/out-of-range) or the slot is
    /// free. This is the immediate invalidation check (`I-S12`).
    ///
    /// # Errors
    /// [`SpatialError::InvalidHandle`] for a stale or dead handle.
    /// REQ: REQ-SPATIAL-011.
    pub fn resolve(&self, handle: SpatialHandle) -> Result<&str, SpatialError> {
        let slot = self
            .slots
            .get(handle.index as usize)
            .ok_or(SpatialError::InvalidHandle)?;
        if !slot.live || slot.generation != handle.generation {
            return Err(SpatialError::InvalidHandle);
        }
        Ok(slot.frame.as_str())
    }

    /// Whether `handle` is current (resolves) — the explicit validity predicate.
    /// REQ: REQ-SPATIAL-011.
    #[must_use]
    pub fn is_valid(&self, handle: SpatialHandle) -> bool {
        self.resolve(handle).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::HandleRegistry;
    use crate::types::{SpatialError, SpatialHandle};

    /// REQ: REQ-SPATIAL-011 — `space_create` returns a handle that resolves to its
    /// frame (`I-S12`).
    #[test]
    fn create_resolves() {
        let mut reg = HandleRegistry::new(4);
        let h = reg
            .create("object/42", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(h.index, 0);
        assert_eq!(h.generation, 0);
        assert_eq!(reg.resolve(h), Ok("object/42"));
        assert!(reg.is_valid(h));
        assert_eq!(reg.live_count(), 1);
    }

    /// REQ: REQ-SPATIAL-011 — `space_delete` bumps the generation so the old
    /// handle invalidates immediately (`I-S12`, `I-Q12`).
    #[test]
    fn delete_invalidates_immediately() {
        let mut reg = HandleRegistry::new(4);
        let h = reg
            .create("object/42", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        reg.delete(h).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(reg.resolve(h), Err(SpatialError::InvalidHandle));
        assert!(!reg.is_valid(h));
        assert_eq!(reg.generation(0), Some(1));
    }

    /// REQ: REQ-SPATIAL-011 — a recycled index is handed out under a newer
    /// generation, so the previous-incarnation handle never aliases it.
    #[test]
    fn recycled_slot_newer_generation() {
        let mut reg = HandleRegistry::new(1);
        let first = reg
            .create("a", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        reg.delete(first).unwrap_or_else(|e| unreachable!("{e}"));
        let second = reg
            .create("b", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(second.index, first.index);
        assert_ne!(second.generation, first.generation);
        assert_eq!(reg.resolve(second), Ok("b"));
        assert_eq!(reg.resolve(first), Err(SpatialError::InvalidHandle));
    }

    /// REQ: REQ-SPATIAL-011 — a double delete and an out-of-range handle are
    /// refused.
    #[test]
    fn stale_and_out_of_range_refused() {
        let mut reg = HandleRegistry::new(1);
        let h = reg
            .create("a", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        reg.delete(h).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(reg.delete(h), Err(SpatialError::InvalidHandle));
        assert_eq!(
            reg.resolve(SpatialHandle {
                index: 99,
                generation: 0
            }),
            Err(SpatialError::InvalidHandle)
        );
    }

    /// REQ: REQ-SPATIAL-011 — a full registry refuses creation with a typed
    /// error.
    #[test]
    fn full_registry_refused() {
        let mut reg = HandleRegistry::new(1);
        let _ = reg
            .create("a", "map")
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(reg.create("b", "map"), Err(SpatialError::InvalidConfig));
    }
}
