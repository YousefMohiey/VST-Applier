//! Own WASAPI capture (IAudioClient3 + IAudioCaptureClient), 100% Rust
//! and fully ours — the last foreign piece of the audio path.
//!
//! Why this exists (the incident): the cpal 0.15.3 capture thread could
//! wedge inside a single COM call to the audio service — counters froze,
//! the thread busy-spun at 100% of a core in the RPC layer for minutes,
//! capture delivery collapsed, the ring starved (dropout floods), the
//! starving thread hogged the VM so the whole app looked hung, and
//! cpal's `Stream::drop` waits on that thread forever — so stop/restart
//! (and the self-heal watchdog) froze with it. Don't ask cpal to run
//! streams; ask it to list devices (that part never bit us) and run the
//! audio with our own loop and our own teardown rules:
//!
//!  - every error path returns to the event wait — this loop cannot spin;
//!  - teardown is bounded: signal stop, wait 1.5 s, then TerminateThread
//!    as the last resort (the only case where leaking a few COM refs
//!    beats freezing the UI forever);
//!  - a `status` line records the ugly path so incidents are visible.
//!
//! Shared-mode capture always delivers the mix format (f32). If the
//! device's mix format isn't f32 / doesn't match the requested rate and
//! channels, `try_start` refuses and the caller falls back to its cpal
//! input stream — with the wedging callbacks OWNED by us, the ugly path
//! is now the fallback, not the default.

#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows::core::Interface;
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    eCapture, IAudioCaptureClient, IAudioClient3, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenThread, SetEvent, TerminateThread, WaitForSingleObject,
    THREAD_TERMINATE,
};

use crate::engine::Shared;
use crate::lowlat::{device_display_name, inspect_format, name_matches};

/// Message the engine's status line carries when the ugly path fires.
pub const WEDGED_MSG: &str = "capture teardown timed out — wedged thread terminated";

pub struct LowLatencyCapture {
    client: IAudioClient3,
    event: HANDLE,
    done: HANDLE,
    stop: Arc<AtomicBool>,
    tid: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<()>>,
    period: u32,
    shared: Arc<Shared>,
}

impl LowLatencyCapture {
    /// Period (frames) the stream was opened with — for diagnostics.
    pub fn period_frames(&self) -> u32 {
        self.period
    }
}

impl std::fmt::Debug for LowLatencyCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LowLatencyCapture")
    }
}

impl Drop for LowLatencyCapture {
    fn drop(&mut self) {
        // Bounded teardown. Clean exit is the normal case; the timeout
        // path means a COM call to the audio service never returned.
        self.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = SetEvent(self.event);
        }
        let clean = unsafe { WaitForSingleObject(self.done, 1_500) == WAIT_OBJECT_0 };
        if let Some(t) = self.thread.take() {
            if clean {
                let _ = t.join();
            } else {
                // Last resort: kill the wedged thread outright instead of
                // freezing stop/restart/UI behind it forever. Its COM
                // refs leak (bounded, one stream's worth) and the audio
                // session dies with the process; the stream is rebuilt by
                // the caller right after us.
                let tid = self.tid.load(Ordering::Relaxed);
                unsafe {
                    if let Ok(h) = OpenThread(THREAD_TERMINATE, false, tid) {
                        let _ = TerminateThread(h, 0);
                        let _ = CloseHandle(h);
                    }
                }
                if let Ok(mut st) = self.shared.status.lock() {
                    if st.as_str() == "running" {
                        *st = WEDGED_MSG.to_string();
                    }
                }
                eprintln!("capture teardown timed out — wedged thread terminated (tid {tid})");
                // No client.Stop() in this path: it is a COM call into
                // the same wedged connection and could block the caller.
            }
        }
        if clean {
            unsafe {
                let _ = self.client.Stop();
            }
        }
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.done);
        }
    }
}

/// Open `device_name` as a shared-mode capture stream (event-driven) and
/// run `callback(data_interleaved_f32, channels)` from an owned
/// real-time thread. Returns the callback back on failure so the caller
/// can reuse it for the cpal fallback.
pub(crate) fn try_start<F>(
    device_name: &str,
    in_rate: u32,
    in_channels: usize,
    shared: Arc<Shared>,
    callback: F,
) -> Result<LowLatencyCapture, (F, String)>
where
    F: FnMut(&[f32], usize) + Send + 'static,
{
    unsafe {
        try_start_inner(device_name, in_rate, in_channels, shared, callback)
    }
}

unsafe fn try_start_inner<F>(
    device_name: &str,
    in_rate: u32,
    in_channels: usize,
    shared: Arc<Shared>,
    mut callback: F,
) -> Result<LowLatencyCapture, (F, String)>
where
    F: FnMut(&[f32], usize) + Send + 'static,
{
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    macro_rules! bail {
        ($cb:ident, $($arg:tt)*) => { return Err(($cb, format!($($arg)*))) };
    }

    let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(
        &MMDeviceEnumerator,
        None,
        CLSCTX_ALL,
    ) else {
        bail!(callback, "device enumerator unavailable");
    };
    let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else {
        bail!(callback, "capture endpoints unavailable");
    };

    // Find the endpoint by display name; require an f32 mix format at
    // the expected rate and channel count (otherwise the cpal fallback
    // handles the conversion).
    let mut chosen: Option<IAudioClient3> = None;
    for i in 0..collection.GetCount().unwrap_or(0) {
        let Ok(dev) = collection.Item(i) else { continue };
        let display = device_display_name(&dev);
        if !name_matches(device_name, &display) {
            continue;
        }
        let Ok(client3) = dev.Activate::<IAudioClient3>(CLSCTX_ALL, None) else {
            continue;
        };
        let Ok(mix) = client3.GetMixFormat() else { continue };
        let (sr, ch, is_float) = inspect_format(mix);
        CoTaskMemFree(Some(mix as *const _ as *const core::ffi::c_void));
        if sr != in_rate || ch != in_channels as u16 || !is_float {
            continue;
        }
        chosen = Some(client3);
        break;
    }
    let Some(client) = chosen else {
        bail!(
            callback,
            "no f32 capture endpoint named '{device_name}' at {in_rate} Hz x{in_channels}"
        );
    };

    let mix = match client.GetMixFormat() {
        Ok(m) => m,
        Err(e) => bail!(callback, "mix format: {e}"),
    };
    // Event callback + the default engine period (capture stays at the
    // device's normal 10 ms packet cadence; the preload absorbs it).
    let init = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        0,
        0,
        mix,
        None,
    );
    CoTaskMemFree(Some(mix as *const _ as *const core::ffi::c_void));
    if let Err(e) = init {
        bail!(callback, "Initialize rejected: {e}");
    }
    let buffer_frames = match client.GetBufferSize() {
        Ok(n) => n,
        Err(_) => bail!(callback, "buffer size unavailable"),
    };
    let Ok(capture) = client.GetService::<IAudioCaptureClient>() else {
        bail!(callback, "capture service unavailable");
    };

    let event = match CreateEventW(None, false, false, None) {
        Ok(e) => e,
        Err(_) => bail!(callback, "event unavailable"),
    };
    if client.SetEventHandle(event).is_err() {
        let _ = CloseHandle(event);
        bail!(callback, "SetEventHandle failed");
    }
    let done = match CreateEventW(None, false, false, None) {
        Ok(e) => e,
        Err(_) => {
            let _ = CloseHandle(event);
            bail!(callback, "done event unavailable");
        }
    };

    let stop = Arc::new(AtomicBool::new(false));
    let tid = Arc::new(AtomicU32::new(0));
    // The callback rides in a cell so a spawn failure can hand it back
    // (same trick the render side uses for the ring consumer).
    let cb_cell = Arc::new(std::sync::Mutex::new(Some(callback)));
    // WASAPI MTA objects are free-threaded; ownership crosses into the
    // thread as raw integers (into_raw/from_raw), keeping the closure
    // Send exactly like `lowlat` does for the render side.
    let (event_raw, done_raw) = (event.0 as isize, done.0 as isize);
    let client_raw = client.clone().into_raw() as isize;
    let capture_raw = capture.clone().into_raw() as isize;
    let thread = {
        let stop_t = stop.clone();
        let tid_t = tid.clone();
        let cell = cb_cell.clone();
        std::thread::Builder::new()
            .name("wasapi-ll-capture".into())
            .spawn(move || {
                crate::rt::ensure_rt_priority();
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                tid_t.store(
                    windows::Win32::System::Threading::GetCurrentThreadId(),
                    Ordering::Relaxed,
                );
                // Signal completion no matter how we leave (skip on
                // terminate — terminated threads run no guards).
                struct Done(HANDLE);
                impl Drop for Done {
                    fn drop(&mut self) {
                        unsafe {
                            let _ = SetEvent(self.0);
                        }
                    }
                }
                let _done_guard = Done(HANDLE(done_raw as *mut core::ffi::c_void));
                let event = HANDLE(event_raw as *mut core::ffi::c_void);
                let client = IAudioClient3::from_raw(client_raw as *mut core::ffi::c_void);
                let capture =
                    IAudioCaptureClient::from_raw(capture_raw as *mut core::ffi::c_void);
                let Some(mut callback) = cell.lock().unwrap().take() else {
                    return;
                };

                if client.Start().is_err() {
                    return;
                }
                let mut scratch = Vec::<f32>::new();
                loop {
                    if stop_t.load(Ordering::Relaxed) {
                        break;
                    }
                    let wait = WaitForSingleObject(event, 500);
                    if wait == WAIT_TIMEOUT {
                        continue;
                    }
                    if wait != WAIT_OBJECT_0 || stop_t.load(Ordering::Relaxed) {
                        break;
                    }
                    // Drain every packet this signal stands for. EVERY
                    // path below exits back to the event wait, so a
                    // desynced client (packets reported, buffer "empty")
                    // cannot turn into a spin — the fix cpal lacked.
                    loop {
                        let mut frames = match capture.GetNextPacketSize() {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        let mut buf: *mut u8 = std::ptr::null_mut();
                        let mut flags = 0u32;
                        let n = match capture.GetBuffer(&mut buf, &mut frames, &mut flags, None, None)
                        {
                            Ok(()) => frames,
                            // Buffer empty / device reconfig: back to the
                            // wait; the stream either recovers or the
                            // caller's watchdog rebuilds it.
                            Err(_) => break,
                        };
                        if n == 0 || buf.is_null() {
                            let _ = capture.ReleaseBuffer(0);
                            continue;
                        }
                        let len = n as usize * in_channels;
                        if scratch.len() < len {
                            scratch.resize(len, 0.0);
                        }
                        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            scratch[..len].fill(0.0);
                        } else {
                            std::ptr::copy_nonoverlapping(buf as *const f32, scratch.as_mut_ptr(), len);
                        }
                        let _ = capture.ReleaseBuffer(n);
                        callback(&scratch[..len], in_channels);
                    }
                }
                let _ = client.Stop();
            })
    };
    let thread = match thread {
        Ok(t) => t,
        Err(_) => {
            unsafe {
                let _ = CloseHandle(event);
                let _ = CloseHandle(done);
            }
            let cb_back = cb_cell
                .lock()
                .unwrap()
                .take()
                .expect("closure never ran; callback still in cell");
            bail!(cb_back, "capture thread spawn failed");
        }
    };

    Ok(LowLatencyCapture {
        client,
        event,
        done,
        stop,
        tid,
        thread: Some(thread),
        period: buffer_frames,
        shared,
    })
}
