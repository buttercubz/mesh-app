//! Media integration tests (WP-10C): real engines in group mode exchange SimCodec voice
//! over in-memory links (the secure module is replaced by a plaintext test glue), and a
//! single media module driven directly through the session → media seam (`rig.rs`).
//!
//! The binary installs a per-thread counting allocator so `alloc.rs` can prove that
//! `push_capture` / `pull_playout` do not allocate in steady state.

mod alloc;
mod check;
mod dsp;
mod floor;
mod latch;
mod latency;
mod profile;
mod prompts;
mod rig;
mod session;
mod voice;
mod world;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

/// Counts allocations of the current thread.
struct Counting;

// SAFETY: forwards to the system allocator; only a thread-local counter is added.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations made by this thread so far.
pub fn allocations() -> u64 {
    ALLOCS.with(Cell::get)
}
