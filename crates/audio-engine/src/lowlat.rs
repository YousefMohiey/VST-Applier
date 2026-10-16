//! Low-latency shared-mode output (IAudioClient3).
//!
//! The Windows audio engine defaults every shared-mode stream to a 10 ms
//! period. A device may allow smaller periods — exposed through
//! `IAudioClient3::GetSharedModeEnginePeriod` — and the virtual cable we
//! render to reports a 128-frame minimum (~2.7 ms at 48 kHz). When the
//! device allows it, this module opens the render side with the smallest
//! supported period and drains the same lock-free ring the cpal path
//! uses (through `engine::drain_into`, same anti-lag + counters),
//! shaving ~7 ms of device-side latency off the route.
//!
//! Any failure — endpoint missing, no period benefit, request rejected —
//! hands the ring consumer back so the caller falls back to its regular
//! cpal output stream.
//!
//! Threading: WASAPI objects are created in the MTA and are free-threaded
//! (every real-time host relies on this; cpal does the same). The render
//! thread waits on the audio event and drains the ring per period.

// One long COM dance on purpose: the `unsafe fn` bodies below are called
// from a single guarded construction site, so per-operation unsafe blocks
// would add noise, not safety.
#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ringbuf::HeapCons;
use windows::core::Interface;
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    eRender, IAudioClient3, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

use crate::engine::{drain_into, Shared};

/// AUDCLNT_STREAMFLAGS_EVENTCALLBACK
const EVENTCALLBACK: u32 = 0x0004_0000;
/// WAVE_FORMAT_IEEE_FLOAT
const IEEE_FLOAT: u16 = 3;
/// WAVE_FORMAT_EXTENSIBLE
const EXTENSIBLE: u16 = 0xFFFE;

/// Debug counter: lowlat render loop iterations (spin bisection).
pub static LL_ITERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct LowLatencyOutput {
    client: IAudioClient3,
    render: IAudioRenderClient,
    event: HANDLE,
    buffer_frames: u32,
    channels: usize,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LowLatencyOutput {
    /// Period (in frames) the stream was opened with — for diagnostics.
    pub fn period_frames(&self) -> u32 {
        self.buffer_frames
    }

    /// Pre-fill one full period with silence, then start the stream.
    /// (Prefilling before `Start` is the standard WASAPI pattern: the
    /// first event must never find a partially empty buffer.)
    pub fn play(&self) -> Result<(), ()> {
        unsafe {
            let padding = self.client.GetCurrentPadding().map_err(|_| ())?;
            let free = self.buffer_frames.saturating_sub(padding);
            if free > 0 {
                let p = self.render.GetBuffer(free).map_err(|_| ())?;
                std::ptr::write_bytes(p, 0, free as usize * self.channels * 4);
                self.render.ReleaseBuffer(free, 0).map_err(|_| ())?;
            }
            self.client.Start().map_err(|_| ())
        }
    }
}

impl Drop for LowLatencyOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = SetEvent(self.event);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// Try to open `device_name` as a low-latency shared-mode render stream.
/// On failure the ring consumer comes back so the caller can fall back.
pub(crate) fn try_start(
    device_name: &str,
    out_rate: u32,
    out_channels: usize,
    cons: HeapCons<f32>,
    shared: Arc<Shared>,
    max_queue: usize,
    trim_to: usize,
) -> Result<LowLatencyOutput, (HeapCons<f32>, String)> {
    unsafe {
        try_start_inner(
            device_name,
            out_rate,
            out_channels,
            cons,
            shared,
            max_queue,
            trim_to,
        )
    }
}

unsafe fn try_start_inner(
    device_name: &str,
    out_rate: u32,
    out_channels: usize,
    cons: HeapCons<f32>,
    shared: Arc<Shared>,
    max_queue: usize,
    trim_to: usize,
) -> Result<LowLatencyOutput, (HeapCons<f32>, String)> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    macro_rules! bail {
        ($cons:ident, $($arg:tt)*) => { return Err(($cons, format!($($arg)*))) };
    }

    let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(
        &MMDeviceEnumerator,
        None,
        CLSCTX_ALL,
    ) else {
        bail!(cons, "device enumerator unavailable");
    };
    let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
        bail!(cons, "render endpoints unavailable");
    };
    let count = collection.GetCount().unwrap_or(0);

    // Find the device by its display name; among matches prefer one with
    // a genuine low-latency benefit (min period below default).
    let mut chosen: Option<(IAudioClient3, u32, u32)> = None; // client, min, default
    for i in 0..count {
        let Ok(dev) = collection.Item(i) else { continue };
        let display = device_display_name(&dev);
        if !name_matches(device_name, &display) {
            continue;
        }
        let Ok(client3) = dev.Activate::<IAudioClient3>(CLSCTX_ALL, None) else {
            continue;
        };
        let Ok(mix) = client3.GetMixFormat() else {
            continue;
        };
        let (sr, ch, is_float) = inspect_format(mix);
        let (mut def, mut _fund, mut min, mut _max) = (0u32, 0u32, 0u32, 0u32);
        let hr = client3.GetSharedModeEnginePeriod(mix, &mut def, &mut _fund, &mut min, &mut _max);
        CoTaskMemFree(Some(mix as *const _ as *const core::ffi::c_void));
        if hr.is_err() || sr != out_rate || ch != out_channels as u16 || !is_float {
            continue;
        }
        let better = chosen
            .as_ref()
            .map(|(_, best_min, _)| min < *best_min)
            .unwrap_or(true);
        let beneficial = min < def;
        if chosen.is_none() || better {
            chosen = Some((client3, min, def));
        }
        if beneficial {
            break; // first beneficial match wins
        }
    }
    let Some((client, min, def)) = chosen else {
        bail!(cons, "no low-latency-capable endpoint named '{device_name}'");
    };
    if min >= def {
        bail!(
            cons,
            "engine period locked to {:.2} ms — no benefit",
            1000.0 * def as f64 / out_rate as f64
        );
    }

    // Initialize the stream at the smallest period the device allows.
    let mix = match client.GetMixFormat() {
        Ok(m) => m,
        Err(e) => bail!(cons, "mix format: {e}"),
    };
    let init_result = client.InitializeSharedAudioStream(EVENTCALLBACK, min, mix, None);
    CoTaskMemFree(Some(mix as *const _ as *const core::ffi::c_void));
    if let Err(e) = init_result {
        bail!(cons, "InitializeSharedAudioStream rejected: {e}");
    }

    let buffer_frames = match client.GetBufferSize() {
        Ok(n) => n,
        Err(_) => bail!(cons, "buffer size unavailable"),
    };
    let Ok(event) = CreateEventW(None, false, false, None) else {
        bail!(cons, "event unavailable");
    };
    if client.SetEventHandle(event).is_err() {
        let _ = CloseHandle(event);
        bail!(cons, "SetEventHandle failed");
    }
    let Ok(render) = client.GetService::<IAudioRenderClient>() else {
        let _ = CloseHandle(event);
        bail!(cons, "render service unavailable");
    };

    // Render thread: wait per event, drain the ring into each period.
    // `cons` rides in a cell so a spawn failure can hand it back. The
    // event handle and both COM interfaces cross as raw integers:
    // WASAPI MTA objects are free-threaded, and ownership moves through
    // `into_raw` / `from_raw` (refcount), keeping the closure `Send`.
    let cons_cell = Arc::new(std::sync::Mutex::new(Some(cons)));
    let stop = Arc::new(AtomicBool::new(false));
    let event_raw: isize = event.0 as isize;
    let client_raw = client.clone().into_raw() as isize;
    let render_raw = render.clone().into_raw() as isize;
    let thread = {
        let stop_t = stop.clone();
        let cell = cons_cell.clone();
        let shared_t = shared;
        let ch = out_channels;
        std::thread::Builder::new()
            .name("wasapi-ll-render".into())
            .spawn(move || {
                crate::rt::ensure_rt_priority();
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let client = unsafe {
                    IAudioClient3::from_raw(client_raw as *mut core::ffi::c_void)
                };
                let render = unsafe {
                    IAudioRenderClient::from_raw(render_raw as *mut core::ffi::c_void)
                };
                let event = HANDLE(event_raw as *mut core::ffi::c_void);
                let Some(mut cons) = cell.lock().unwrap().take() else {
                    return;
                };
                loop {
                    LL_ITERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                    let padding = match client.GetCurrentPadding() {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    let free = buffer_frames.saturating_sub(padding);
                    if free == 0 {
                        continue;
                    }
                    let Ok(p) = render.GetBuffer(free) else { continue };
                    let data = std::slice::from_raw_parts_mut(p as *mut f32, free as usize * ch);
                    drain_into(&mut cons, &shared_t, data, max_queue, trim_to);
                    if render.ReleaseBuffer(free, 0).is_err() {
                        break;
                    }
                }
            })
    };
    let thread = match thread {
        Ok(t) => t,
        Err(e) => {
            let taken = cons_cell.lock().unwrap().take();
            unsafe {
                let _ = CloseHandle(event);
                // The thread never ran: take back the two interface refs.
                drop(IAudioClient3::from_raw(client_raw as *mut core::ffi::c_void));
                drop(IAudioRenderClient::from_raw(render_raw as *mut core::ffi::c_void));
            }
            // Spawn failed before the closure ever ran, so the cell must
            // still hold the consumer: hand it straight back.
            let c = taken.expect("render thread never started; ring consumer must still be here");
            return Err((c, format!("render thread spawn failed: {e}")));
        }
    };

    Ok(LowLatencyOutput {
        client,
        render,
        event,
        buffer_frames,
        channels: out_channels,
        stop,
        thread: Some(thread),
    })
}

/// The endpoint display name the way Windows/cpal spell it. At runtime
/// `PKEY_Device_FriendlyName` already carries the full display form
/// ("CABLE Input (VB-Audio Virtual Cable)"); the DeviceDesc ("CABLE
/// Input") is only the fallback.
pub(crate) unsafe fn device_display_name(dev: &IMMDevice) -> Option<String> {
    let store: IPropertyStore = dev.OpenPropertyStore(STGM_READ).ok()?;
    let friendly = prop_string(&store, &PKEY_Device_FriendlyName);
    let desc = prop_string(&store, &PKEY_Device_DeviceDesc);
    friendly.or(desc)
}

unsafe fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let pv = store.GetValue(key).ok()?;
    let s = PropVariantToStringAlloc(&pv).ok()?;
    let text = s.to_string().ok();
    CoTaskMemFree(Some(s.0 as *const core::ffi::c_void));
    text
}

/// Compare an endpoint display name against a wanted name in either of
/// the shapes it may arrive in ("Desc (Friendly)" or bare).
pub(crate) fn name_matches(wanted: &str, display: &Option<String>) -> bool {
    let Some(display) = display else { return false };
    if display == wanted {
        return true;
    }
    // "CABLE Input (VB-Audio Virtual Cable)" vs "VB-Audio Virtual Cable"
    if let Some(friendly) = display.split_once('(').map(|(_, rest)| rest.trim_end_matches(')')) {
        if friendly == wanted {
            return true;
        }
    }
    // Either side containing the other covers prefix/suffix decorations.
    let stripped = wanted.split_once('(').map(|(_, rest)| rest.trim_end_matches(')'));
    stripped.map(|f| f == display).unwrap_or(false)
}

/// (sample rate, channels, is-float-format) of a mix format.
pub(crate) unsafe fn inspect_format(p: *mut WAVEFORMATEX) -> (u32, u16, bool) {
    let w = *p;
    let float = if w.wFormatTag == EXTENSIBLE {
        let ext = p as *const WAVEFORMATEXTENSIBLE;
        (*ext).SubFormat.data1 == IEEE_FLOAT as u32
    } else {
        w.wFormatTag == IEEE_FLOAT
    };
    (w.nSamplesPerSec, w.nChannels, float)
}
