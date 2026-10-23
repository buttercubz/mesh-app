//! Clocks and thread priority: the only places mesh-ffi reads OS time or changes
//! scheduling (DESIGN §3.1 allows both in this crate).

use mesh_core::Instant;

/// Monotonic engine clock: microseconds since the runtime was created.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    base: std::time::Instant,
}

impl Clock {
    pub fn new() -> Self {
        Clock {
            base: std::time::Instant::now(),
        }
    }

    /// Engine time now.
    pub fn now(&self) -> Instant {
        let us = self.base.elapsed().as_micros();
        Instant(u64::try_from(us).unwrap_or(u64::MAX))
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// Wall clock in Unix ms (0 if the clock is before 1970).
pub fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Raises the calling thread to the engine priority (DESIGN §3.4): iOS/macOS
/// `QOS_CLASS_USER_INTERACTIVE`, Android `setpriority(PRIO_PROCESS, gettid(), -19)`.
/// Returns whether the OS accepted it (failure is logged by the caller, never fatal).
pub fn raise_engine_priority() -> bool {
    imp::raise()
}

#[cfg(target_vendor = "apple")]
mod imp {
    /// `QOS_CLASS_USER_INTERACTIVE` from `<sys/qos.h>`.
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;

    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }

    pub fn raise() -> bool {
        // SAFETY: plain libpthread call on the current thread.
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) == 0 }
    }
}

#[cfg(target_os = "android")]
mod imp {
    /// `PRIO_PROCESS` from `<sys/resource.h>`.
    const PRIO_PROCESS: i32 = 0;
    /// Audio-adjacent priority used by the engine thread.
    const ENGINE_NICE: i32 = -19;

    unsafe extern "C" {
        fn gettid() -> i32;
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }

    pub fn raise() -> bool {
        // SAFETY: bionic calls on the current thread id.
        unsafe {
            let tid = gettid();
            setpriority(PRIO_PROCESS, tid as u32, ENGINE_NICE) == 0
        }
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
mod imp {
    pub fn raise() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_wall_clock_is_recent() {
        let c = Clock::new();
        let a = c.now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = c.now();
        assert!(b.0 >= a.0 + 2_000);
        assert!(wall_ms() > 1_700_000_000_000);
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn qos_is_accepted_on_apple_hosts() {
        let ok = std::thread::spawn(raise_engine_priority).join().unwrap();
        assert!(ok);
    }
}
