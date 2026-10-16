//! Real-time thread priority (the fix for glitching under CPU load).
//!
//! WASAPI callback threads run at normal priority by default, so any heavy
//! background load can deschedule them past the audio deadline — the voice
//! stutters even though the DSP itself is cheap. The standard Windows fix
//! (same as pro-audio apps use) is MMCSS: register each audio thread for
//! the "Pro Audio" task, which boosts it into the real-time band for the
//! thread's lifetime. Falls back to `THREAD_PRIORITY_TIME_CRITICAL` when
//! MMCSS is unavailable.
//!
//! Call [`ensure_rt_priority`] at the top of every audio callback. It runs
//! once per thread (thread-local flag) and is a single relaxed load after.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadPriority,
    THREAD_PRIORITY_TIME_CRITICAL,
};

thread_local! {
    static BOOSTED: Cell<bool> = const { Cell::new(false) };
    // Kept alive for the thread's lifetime; the OS releases it on exit.
    static MMCSS_TASK: RefCell<Option<HANDLE>> = const { RefCell::new(None) };
}

/// Boost the calling thread for real-time audio. Idempotent per thread.
pub fn ensure_rt_priority() {
    BOOSTED.with(|done| {
        if done.get() {
            return;
        }
        done.set(true);
        unsafe {
            let mut index = 0u32;
            match AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut index) {
                Ok(task) => {
                    MMCSS_TASK.with(|slot| *slot.borrow_mut() = Some(task));
                }
                Err(_) => {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
                }
            }
        }
        // Flush-to-zero + denormals-are-zero (MXCSR bits 15|6). Audio
        // never needs denormals, and paying for them is catastrophic:
        // the denoiser's recurrent state decays into denormal floats
        // during quiet input and every denormal MAC costs ~100x on x86
        // (measured: 534 us -> 2859 us per chunk at 44.1 kHz feeding
        // silence vs noise — a full core of "spin" and drained rings).
        #[cfg(target_arch = "x86_64")]
        unsafe {
            use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
            _mm_setcsr(_mm_getcsr() | 0x8040);
        }
        #[cfg(target_arch = "x86")]
        unsafe {
            use std::arch::x86::{_mm_getcsr, _mm_setcsr};
            _mm_setcsr(_mm_getcsr() | 0x8040);
        }
    });
}
