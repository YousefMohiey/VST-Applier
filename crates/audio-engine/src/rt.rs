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
    });
}
