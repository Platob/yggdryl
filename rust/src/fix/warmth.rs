//! What a parsing thread keeps warm, carried from one chunk's thread to the
//! next's.
//!
//! The caches a message read fills - the column plans, the replacement
//! plans, the bindings and the dictionary's mirror - are each thread's own,
//! and a door reading on several threads spawns them per chunk, so without
//! this every chunk would start cold and read the same forty rules again.
//! [`Warmth`] is the four taken out of a thread as it finishes its chunk
//! and put into the one spawned for the next, which is what
//! [`crate::parallel`] does with a [`Warm`] between chunks.

use crate::parallel::Warm;

/// One thread's caches, out of any thread.
#[derive(Default)]
pub(super) struct Warmth {
    plans: super::schema::ColumnPlans,
    replacements: super::latest::ReplacementPlans,
    bindings: super::latest::BoundTerms,
    mirror: super::memo::Mirror,
}

impl Warmth {
    /// The calling thread's caches and these exchanged.
    fn swap(&mut self) {
        super::schema::swap_column_plans(&mut self.plans);
        super::latest::swap_replacement_plans(&mut self.replacements);
        super::latest::swap_bound_terms(&mut self.bindings);
        super::memo::swap_mirror(&mut self.mirror);
    }
}

impl Warm for Warmth {
    fn take() -> Self {
        let mut taken = Self::default();
        taken.swap();
        taken
    }

    fn restore(mut self) {
        self.swap();
    }
}
