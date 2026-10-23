//! The raw `extern "C"` ABI of DESIGN §11.2 (no wasm-bindgen):
//!
//! ```c
//! uint32_t mw_alloc(uint32_t len);  void mw_free(uint32_t ptr, uint32_t len);
//! uint32_t mw_world_new(uint32_t cfg_ptr, uint32_t cfg_len);          // JSON scenario {preset, nodes, loss, seed}
//! uint32_t mw_command(uint32_t world, uint32_t ptr, uint32_t len);    // returns ptr to [u32 len][JSON CommandResult]
//! void     mw_step(uint32_t world, uint64_t dt_us);                   // advance simulated time
//! uint32_t mw_poll_events(uint32_t world);                            // ptr to [u32 len][JSON UiEvent[]]
//! uint32_t mw_poll_lab(uint32_t world);                               // ptr to [u32 len][labrun/1 JSONL]
//! void     mw_world_free(uint32_t world);
//! ```
//!
//! Conventions (the JS loader in `@meshapp/engine-web` follows them):
//! - Input buffers are allocated by the page with `mw_alloc(len)`, filled, passed, and
//!   released with `mw_free(ptr, len)` (the callee never takes ownership).
//! - A world handle is a small non-zero integer; `mw_world_new` returns 0 for a
//!   scenario it refuses, and every call with an unknown handle is a no-op returning 0.
//! - Returned buffers are `[u32 little-endian byte length][UTF-8 bytes]`, owned by the
//!   world and valid until the next call on that world; the page copies them at once.
//!
//! The handle table and output framing are target-independent (and unit-tested on the
//! host); only the exported symbols are compiled for `wasm32`, where pointers are 32 bit.

use std::cell::RefCell;

use crate::config::ConvoyConfig;
use crate::convoy::SimConvoy;

/// One world and the output buffer of its last returning call.
struct Slot {
    convoy: SimConvoy,
    out: Vec<u8>,
}

/// The live worlds, indexed by `handle - 1`.
#[derive(Default)]
pub struct Registry {
    slots: Vec<Option<Box<Slot>>>,
}

/// Frames `body` as `[u32 LE len][bytes]` into `out` (replacing its contents).
pub fn frame_into(out: &mut Vec<u8>, body: &[u8]) {
    out.clear();
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&body[..len as usize]);
}

impl Registry {
    /// Creates a world from the scenario JSON; `0` when refused.
    pub fn world_new(&mut self, cfg_json: &str) -> u32 {
        let Ok(cfg) = ConvoyConfig::parse(cfg_json) else {
            return 0;
        };
        let slot = Box::new(Slot {
            convoy: SimConvoy::new(cfg),
            out: Vec::new(),
        });
        if let Some(i) = self.slots.iter().position(Option::is_none) {
            self.slots[i] = Some(slot);
            return i as u32 + 1;
        }
        self.slots.push(Some(slot));
        self.slots.len() as u32
    }

    fn slot(&mut self, h: u32) -> Option<&mut Slot> {
        let i = usize::try_from(h).ok()?.checked_sub(1)?;
        self.slots.get_mut(i)?.as_deref_mut()
    }

    /// The world behind handle `h`.
    pub fn convoy(&mut self, h: u32) -> Option<&mut SimConvoy> {
        self.slot(h).map(|s| &mut s.convoy)
    }

    /// Runs a command; returns the framed result (empty when the handle is unknown).
    pub fn command(&mut self, h: u32, json: &str) -> Option<&[u8]> {
        let s = self.slot(h)?;
        let r = s.convoy.command_json(json);
        frame_into(&mut s.out, r.as_bytes());
        Some(&s.out)
    }

    /// Advances world `h` by `dt_us`.
    pub fn step(&mut self, h: u32, dt_us: u64) {
        if let Some(s) = self.slot(h) {
            s.convoy.step(dt_us);
        }
    }

    /// The framed JSON array of pending UI events.
    pub fn poll_events(&mut self, h: u32) -> Option<&[u8]> {
        let s = self.slot(h)?;
        let evs = s.convoy.poll_events();
        let mut body = String::with_capacity(2 + evs.iter().map(|e| e.len() + 1).sum::<usize>());
        body.push('[');
        for (k, e) in evs.iter().enumerate() {
            if k > 0 {
                body.push(',');
            }
            body.push_str(e);
        }
        body.push(']');
        frame_into(&mut s.out, body.as_bytes());
        Some(&s.out)
    }

    /// The framed labrun/1 JSONL written since the last poll (`\n` after every line).
    pub fn poll_lab(&mut self, h: u32) -> Option<&[u8]> {
        let s = self.slot(h)?;
        let mut body = String::new();
        for l in s.convoy.poll_lab() {
            body.push_str(&l);
            body.push('\n');
        }
        frame_into(&mut s.out, body.as_bytes());
        Some(&s.out)
    }

    /// Frees world `h` (its handle may be reused).
    pub fn world_free(&mut self, h: u32) {
        if let Some(i) = usize::try_from(h).ok().and_then(|h| h.checked_sub(1))
            && let Some(s) = self.slots.get_mut(i)
        {
            *s = None;
        }
    }

    /// Number of live worlds.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Runs `f` on the process-wide registry (single-threaded on wasm32).
pub fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

#[cfg(target_arch = "wasm32")]
mod exports {
    //! The exported symbols. Pointers are linear-memory offsets (32 bit on wasm32).

    use super::with_registry;

    /// Reads `len` bytes at `ptr` as UTF-8 (lossy: invalid input becomes a refused
    /// command rather than a trap).
    ///
    /// # Safety
    /// `ptr..ptr+len` must be a live allocation from `mw_alloc`.
    unsafe fn input(ptr: u32, len: u32) -> String {
        if len == 0 {
            return String::new();
        }
        // SAFETY: the caller passes a buffer it allocated with `mw_alloc(len)` and filled.
        let bytes = unsafe { core::slice::from_raw_parts(ptr as usize as *const u8, len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn ptr_of(b: Option<&[u8]>) -> u32 {
        b.map_or(0, |b| b.as_ptr() as usize as u32)
    }

    /// Allocates `len` bytes for the page to fill; never returns 0.
    #[unsafe(no_mangle)]
    pub extern "C" fn mw_alloc(len: u32) -> u32 {
        let b: Box<[u8]> = vec![0u8; (len as usize).max(1)].into_boxed_slice();
        Box::into_raw(b) as *mut u8 as usize as u32
    }

    /// Releases a buffer from `mw_alloc(len)`.
    ///
    /// # Safety
    /// `ptr` must come from `mw_alloc(len)` with the same `len`, freed once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn mw_free(ptr: u32, len: u32) {
        if ptr == 0 {
            return;
        }
        let n = (len as usize).max(1);
        // SAFETY: `ptr` was produced by `mw_alloc` for a boxed slice of `n` bytes.
        drop(unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                ptr as usize as *mut u8,
                n,
            ))
        });
    }

    /// Creates a world from the scenario JSON; 0 when refused.
    ///
    /// # Safety
    /// `cfg_ptr..cfg_ptr+cfg_len` must be a live buffer from `mw_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn mw_world_new(cfg_ptr: u32, cfg_len: u32) -> u32 {
        // SAFETY: forwarded caller contract.
        let cfg = unsafe { input(cfg_ptr, cfg_len) };
        with_registry(|r| r.world_new(&cfg))
    }

    /// Runs a command; returns a pointer to `[u32 len][JSON result]` (0: unknown world).
    ///
    /// # Safety
    /// `ptr..ptr+len` must be a live buffer from `mw_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn mw_command(world: u32, ptr: u32, len: u32) -> u32 {
        // SAFETY: forwarded caller contract.
        let json = unsafe { input(ptr, len) };
        with_registry(|r| ptr_of(r.command(world, &json)))
    }

    /// Advances simulated time by `dt_us`.
    #[unsafe(no_mangle)]
    pub extern "C" fn mw_step(world: u32, dt_us: u64) {
        with_registry(|r| r.step(world, dt_us));
    }

    /// Pointer to `[u32 len][JSON UiEvent[]]` (0: unknown world).
    #[unsafe(no_mangle)]
    pub extern "C" fn mw_poll_events(world: u32) -> u32 {
        with_registry(|r| ptr_of(r.poll_events(world)))
    }

    /// Pointer to `[u32 len][labrun/1 JSONL]` (0: unknown world).
    #[unsafe(no_mangle)]
    pub extern "C" fn mw_poll_lab(world: u32) -> u32 {
        with_registry(|r| ptr_of(r.poll_lab(world)))
    }

    /// Frees a world.
    #[unsafe(no_mangle)]
    pub extern "C" fn mw_world_free(world: u32) {
        with_registry(|r| r.world_free(world));
    }
}
