//! Test-binary allocation observation for the sequential ULBridge profile.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Allocation requests observed on the armed thread.
///
/// `requested_bytes` sums allocation sizes and each reallocation's requested
/// new size. It is not a live-byte or peak-memory measurement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counts {
    pub(crate) allocations: u64,
    pub(crate) reallocations: u64,
    pub(crate) requested_bytes: u64,
}

thread_local! {
    static ACTIVE: Cell<Option<Counts>> = const { Cell::new(None) };
}

/// The integration binary's normal system allocator, with thread-local counts
/// only while a profile section is armed.
pub(crate) struct CountingAllocator;

// SAFETY: allocation layouts and pointers pass unchanged to System. The
// const-initialized, non-dropping thread-local counter neither allocates nor
// calls back into this allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: System receives the caller's valid layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: System receives the caller's valid layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: this pointer and layout came from the same System allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_reallocation(new_size);
        // SAFETY: the original allocation and requested size pass unchanged.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[inline]
fn record_allocation(size: usize) {
    ACTIVE.with(|active| {
        let Some(mut counts) = active.get() else {
            return;
        };
        counts.allocations += 1;
        counts.requested_bytes = counts.requested_bytes.saturating_add(size as u64);
        active.set(Some(counts));
    });
}

#[inline]
fn record_reallocation(new_size: usize) {
    ACTIVE.with(|active| {
        let Some(mut counts) = active.get() else {
            return;
        };
        counts.reallocations += 1;
        counts.requested_bytes = counts.requested_bytes.saturating_add(new_size as u64);
        active.set(Some(counts));
    });
}

struct Scope;

impl Scope {
    fn arm() -> Self {
        ACTIVE.with(|active| {
            assert!(
                active.get().is_none(),
                "allocation profile sections must not nest"
            );
            active.set(Some(Counts::default()));
        });
        Self
    }

    fn finish(self) -> Counts {
        ACTIVE.with(|active| active.replace(None).expect("an armed allocation profile"))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // This also runs while the measured closure unwinds, so a panic cannot
        // leave allocation counting armed for the next test on this thread.
        ACTIVE.with(|active| active.set(None));
    }
}

/// Run one non-nested section and return its value with the thread-local
/// allocation requests it made. The scope resets during unwinding too.
pub(crate) fn measure<T>(body: impl FnOnce() -> T) -> (T, Counts) {
    let scope = Scope::arm();
    let value = body();
    let counts = scope.finish();
    (value, counts)
}
