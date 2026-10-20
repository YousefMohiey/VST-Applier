//! Real-time routing engine (mirrors `AudioRoutingService`).
//!
//! Pipeline per capture block, on the cpal input-stream thread:
//! f32 frames -> downmix mono -> stereo dup -> serial VST chain
//! (chunked to `block_size`, ping-pong buffers) -> downmix mono ->
//! resample to output rate -> fan-out -> lock-free ring.
//!
//! The cpal output-stream thread drains the ring; underrun reads silence.
//! Anti-lag budget: if the ring runs away past 3× the preload (min 120 ms),
//! the oldest audio is [dropped] back to the preload. The output stream
//! starts after a short preload (two DSP blocks + 10 ms). Capture blocks
//! are re-chunked so no single
//! plugin call exceeds `block_size` frames.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use ringbuf::{traits::*, HeapCons, HeapRb};
use thiserror::Error;
use vst_host_sys::{HostError, PluginFormat, PluginHost};

use crate::convert;
use crate::resample::resample_with_carry;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("stream error: {0}")]
    Stream(#[from] cpal::BuildStreamError),
    #[error("play error: {0}")]
    Play(#[from] cpal::PlayStreamError),
    #[error("plugin error: {0}")]
    Plugin(#[from] HostError),
    #[error("device error: {0}")]
    Devices(#[from] cpal::DevicesError),
    #[error("default config error: {0}")]
    DefaultConfig(#[from] cpal::DefaultStreamConfigError),
    #[error("unsupported sample format")]
    UnsupportedFormat,
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// One link in the processing chain (mirrors `VstPluginChainItem`).
pub struct ChainPlugin {
    pub name: String,
    pub path: std::path::PathBuf,
    pub format: PluginFormat,
    pub enabled: bool,
    pub host: Option<PluginHost>,
    pub saved_state: Option<Vec<u8>>,
}

impl ChainPlugin {
    /// Attach the native host for this plugin. `host_dll` is the
    /// `Vst{2,3}HostNative.dll` matching `format`.
    pub fn attach(&mut self, host_dll: &Path) -> Result<()> {
        let mut host = PluginHost::load(host_dll, self.format, &self.path)?;
        if let Some(blob) = self.saved_state.take() {
            // State failure keeps the plugin with defaults (as in C#).
            let _ = host.load_state(&blob);
        }
        self.host = Some(host);
        Ok(())
    }
}

/// Compact latency diagnostics (mirrors `AudioRouteDiagnostics`).
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub buffered_ms: f64,
    pub capacity_ms: f64,
    pub plugin_block_size: u32,
    pub last_capture_block_ms: f64,
    pub input_rate: u32,
    pub output_rate: u32,
    pub output_channels: usize,
    pub status: String,
    /// Output stream runs at the device's smallest engine period.
    pub low_latency: bool,
    /// Built-in AI denoise stage active.
    pub denoise: bool,
    /// Inherent latency of the active AI denoise stage (0 when off).
    pub denoise_ms: f64,
}

pub const ALLOWED_BLOCKS: &[u32] = &[128, 256, 512, 1024, 2048, 4096];

pub fn normalize_block_size(n: u32) -> u32 {
    if n.is_power_of_two() && (64..=4096).contains(&n) {
        n
    } else {
        1024
    }
}

/// Low-latency pipeline fill for a given DSP block size, in seconds:
/// two blocks plus 10 ms of jitter margin (floor 15 ms). Used both as
/// the start preload and as the occupancy-servo target so the resampler
/// path parks its queue at the same low level.
pub fn preload_secs(block_frames: usize, out_rate: u32) -> f64 {
    let block_secs = block_frames as f64 / out_rate as f64;
    (block_secs * 2.0 + 0.010).max(0.015)
}

/// Shared meter state between audio threads and UI.
#[derive(Debug, Default)]
pub struct Meters {
    input_peak: AtomicU32,
    output_peak: AtomicU32,
}

impl Meters {
    pub fn set_input(&self, v: f32) {
        self.input_peak.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_output(&self, v: f32) {
        self.output_peak.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn input(&self) -> f32 {
        f32::from_bits(self.input_peak.load(Ordering::Relaxed))
    }
    pub fn output(&self) -> f32 {
        f32::from_bits(self.output_peak.load(Ordering::Relaxed))
    }
}

pub(crate) struct Shared {
    /// Lock-free level estimate (producer adds, consumer subtracts).
    level: AtomicI64,
    capacity_ms: f64,
    out_rate: u32,
    out_channels: usize,
    meters: Arc<Meters>,
    pub(crate) status: Mutex<String>,
    /// Microseconds per capture block (atomic: written on the RT thread,
    /// read by the UI — no mutex in the hot path).
    last_block_us: AtomicU64,
    /// Render callbacks that found the ring empty (audible gaps).
    underruns: AtomicU64,
    /// Ring samples discarded by the anti-lag trim (overload fast-forward).
    trim_drops: AtomicU64,
    in_blocks: AtomicU64,
    in_frames: AtomicU64,
    out_frames: AtomicU64,
    /// True when the output runs on the low-latency WASAPI backend.
    low_latency: AtomicBool,
    /// True while the render side is starving (no data in the ring).
    /// Dropouts are counted once per contiguous gap, not once per wake.
    starving: AtomicBool,
    /// "Hear yourself" monitor enabled: the capture thread tees the final
    /// processed mono block into the monitor ring only while this is true
    /// (one relaxed load per block when off).
    monitor_on: AtomicBool,
    /// Built-in AI denoise: on/off flag + the live model (created by
    /// the UI thread; the audio thread only `try_lock`s it, so a swap
    /// can never stall a callback).
    denoise: AtomicBool,
    /// f32 bits: inherent latency of the active denoise stage, in ms.
    denoise_ms: AtomicU32,
    denoiser: Mutex<Option<ai_denoise::Denoiser>>,
}

impl Shared {
    /// Fresh routing state. Split out of `start` so tests can build the
    /// exact same `Shared` without opening any device.
    fn new(
        out_rate: u32,
        out_channels: usize,
        meters: Arc<Meters>,
        denoise: Option<ai_denoise::Denoiser>,
    ) -> Self {
        Self {
            level: AtomicI64::new(0),
            capacity_ms: 1000.0,
            out_rate,
            out_channels,
            meters,
            status: Mutex::new("running".to_string()),
            last_block_us: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            trim_drops: AtomicU64::new(0),
            in_blocks: AtomicU64::new(0),
            in_frames: AtomicU64::new(0),
            out_frames: AtomicU64::new(0),
            low_latency: AtomicBool::new(false),
            starving: AtomicBool::new(false),
            monitor_on: AtomicBool::new(false),
            denoise: AtomicBool::new(denoise.is_some()),
            denoise_ms: AtomicU32::new(
                denoise
                    .as_ref()
                    .map(|d| d.latency_ms() as f32)
                    .unwrap_or(0.0)
                    .to_bits(),
            ),
            denoiser: Mutex::new(denoise),
        }
    }

    fn buffered_samples(&self) -> usize {
        self.level.load(Ordering::Relaxed).max(0) as usize
    }

    fn buffered_ms(&self) -> f64 {
        self.buffered_samples() as f64 / (self.out_rate as f64 * self.out_channels as f64) * 1000.0
    }

    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.in_blocks.load(Ordering::Relaxed),
            self.in_frames.load(Ordering::Relaxed),
            self.out_frames.load(Ordering::Relaxed),
        )
    }

    pub fn dropouts(&self) -> (u64, u64) {
        (
            self.underruns.load(Ordering::Relaxed),
            self.trim_drops.load(Ordering::Relaxed),
        )
    }
}

/// A running route. Dropping stops both streams.
pub struct RunningRoute {
    _input: InputBackend,
    output: Option<OutputBackend>,
    /// Consumer half of the monitor ring. Lives here on the UI thread
    /// while the monitor is off; handed to the monitor stream when on.
    tee: Option<HeapCons<f32>>,
    /// Live monitor output stream (headphones). Never the route output.
    monitor_stream: Option<crate::monitor::MonitorStream>,
    output_name: String,
    input_name: String,
    preload_samples: usize,
    started: bool,
    chain: Arc<Mutex<Vec<ChainPlugin>>>,
    shared: Arc<Shared>,
    meters: Arc<Meters>,
    plugin_block: u32,
    in_rate: u32,
    in_channels: usize,
}

impl RunningRoute {
    pub fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            buffered_ms: self.shared.buffered_ms(),
            capacity_ms: self.shared.capacity_ms,
            plugin_block_size: self.plugin_block,
            last_capture_block_ms: self.shared.last_block_us.load(Ordering::Relaxed) as f64 / 1000.0,
            input_rate: self.in_rate,
            output_rate: self.shared.out_rate,
            output_channels: self.shared.out_channels,
            status: self.shared.status.lock().unwrap().clone(),
            low_latency: self.shared.low_latency.load(Ordering::Relaxed),
            denoise: self.shared.denoise.load(Ordering::Relaxed),
            denoise_ms: f32::from_bits(self.shared.denoise_ms.load(Ordering::Relaxed)) as f64,
        }
    }

    pub fn meters(&self) -> &Arc<Meters> {
        &self.meters
    }

    /// Swap the built-in AI denoise stage at runtime. The model is
    /// created on the UI thread (hundreds of ms) and handed over here;
    /// pass `None` to switch the stage off. The audio thread only ever
    /// `try_lock`s the slot, so the swap cannot stall a callback.
    pub fn set_denoise(&self, denoise: Option<ai_denoise::Denoiser>) {
        let on = denoise.is_some();
        let ms = denoise
            .as_ref()
            .map(|d| d.latency_ms() as f32)
            .unwrap_or(0.0);
        if let Ok(mut g) = self.shared.denoiser.lock() {
            *g = denoise;
        }
        self.shared.denoise_ms.store(ms.to_bits(), Ordering::Relaxed);
        self.shared.denoise.store(on, Ordering::Relaxed);
    }

    /// Run `f` against one chain plugin (e.g. editor open/idle/close).
    /// Indices match UI order; disabled or host-less slots are still
    /// present. Returns None when the index is out of range.
    pub fn with_plugin<R>(&self, index: usize, f: impl FnOnce(&mut ChainPlugin) -> R) -> Option<R> {
        self.chain.lock().unwrap().get_mut(index).map(f)
    }

    /// Non-blocking twin of [`with_plugin`]: returns None instead of
    /// waiting when the audio thread currently owns the chain. Use for
    /// per-frame chores (editor idle pumps) so the UI can never stall —
    /// and be stalled by — the real-time path under CPU load.
    pub fn try_with_plugin<R>(
        &self,
        index: usize,
        f: impl FnOnce(&mut ChainPlugin) -> R,
    ) -> Option<R> {
        self.chain.try_lock().ok()?.get_mut(index).map(f)
    }

    /// Insert an already-attached plugin into the LIVE chain. The host
    /// FIRST gets the same per-plugin setup the start path applies
    /// (sample rate, block size, stereo bus with mono fallback) —
    /// without it the audio callback fails the fresh slot and the rest
    /// of the chain goes dry ("VST bypassed"). Then the slot appears
    /// under the chain lock and the callback picks it up next block.
    /// No stream teardown, no restart, editors stay open — this is what
    /// keeps "Add" from freezing/hanging the app.
    pub fn push_plugin(&self, mut cp: ChainPlugin) -> Result<usize> {
        self.setup_slot(&mut cp)?;
        self.warm_up_slot(&mut cp);
        let mut plugins = self.chain.lock().unwrap();
        plugins.push(cp);
        Ok(plugins.len() - 1)
    }

    /// Run one silent block through a freshly set-up host BEFORE it enters
    /// the live chain — plugins do their lazy allocations on the first
    /// `process` call, and doing that later would stall the capture thread
    /// and count a dropout on every add.
    fn warm_up_slot(&self, cp: &mut ChainPlugin) {
        let Some(h) = cp.host.as_mut() else { return };
        let frames = self.plugin_block as usize;
        let silence = vec![0.0f32; frames * 2];
        let mut out = vec![0.0f32; frames * 2];
        if h.channels == 2 {
            let _ = h.process_f32(&silence, &mut out);
        } else {
            let _ = h.process_f32(&silence[..frames], &mut out[..frames]);
        }
    }

    /// Apply the start-path processing setup to one slot's host:
    /// stereo first, mono fallback for mono-only plugins.
    fn setup_slot(&self, cp: &mut ChainPlugin) -> Result<()> {
        if let Some(h) = cp.host.as_mut() {
            if h.setup_processing(self.in_rate as f64, self.plugin_block as i32, 2)
                .is_err()
            {
                if let Err(mono_err) = h.setup_processing(self.in_rate as f64, self.plugin_block as i32, 1)
                {
                    // Keep the host's specific stage detail ("does not
                    // provide an audio input bus", "arrangement not
                    // supported", …) — the old generic text made every
                    // failure unanswerable.
                    let detail = match &mono_err {
                        vst_host_sys::HostError::CallFailed { message, .. } => message.clone(),
                        other => other.to_string(),
                    };
                    return Err(EngineError::Plugin(vst_host_sys::HostError::CallFailed {
                        code: -1,
                        message: format!("setup_processing failed: {detail}"),
                    }));
                }
            }
        }
        Ok(())
    }

    /// Remove one slot from the live chain. The returned plugin (and
    /// its host) is dropped by the CALLER, outside the lock, so a slow
    /// plugin unload can't stall the audio thread.
    pub fn remove_plugin(&self, index: usize) -> Option<ChainPlugin> {
        let mut plugins = self.chain.lock().unwrap();
        if index < plugins.len() {
            Some(plugins.remove(index))
        } else {
            None
        }
    }

    /// Reorder two slots in the live chain.
    pub fn swap_plugins(&self, i: usize, j: usize) {
        let mut plugins = self.chain.lock().unwrap();
        if i < plugins.len() && j < plugins.len() {
            plugins.swap(i, j);
        }
    }

    /// Enable/disable one slot live (the callback skips disabled slots
    /// on the next block).
    pub fn set_enabled(&self, index: usize, enabled: bool) {
        let mut plugins = self.chain.lock().unwrap();
        if let Some(p) = plugins.get_mut(index) {
            p.enabled = enabled;
        }
    }

    /// Swap in a freshly attached host for one slot (re-enabling a slot
    /// that started disabled). Same setup as [`push_plugin`]. The
    /// previous slot value is returned for the caller to drop, outside
    /// the lock.
    pub fn replace_plugin(&self, index: usize, mut cp: ChainPlugin) -> Result<Option<ChainPlugin>> {
        self.setup_slot(&mut cp)?;
        self.warm_up_slot(&mut cp);
        let old = {
            let mut plugins = self.chain.lock().unwrap();
            plugins
                .get_mut(index)
                .map(|slot| std::mem::replace(slot, cp))
        };
        Ok(old)
    }

    /// Drop every slot from the live chain. The slots are returned so
    /// the CALLER drops the hosts outside the lock (close editors
    /// first!). No stream teardown.
    pub fn clear_plugins(&self) -> Vec<ChainPlugin> {
        let mut plugins = self.chain.lock().unwrap();
        plugins.drain(..).collect()
    }

    /// Compact human-readable status for the bottom bar. Every value
    /// here is a real measurement: queue depth in ms, device sample
    /// rates, DSP block size, dropout counter. The full diagnostic
    /// line (`status_line`) is shown as a hover tooltip instead.
    pub fn human_line(&self) -> String {
        let d = self.diagnostics();
        let (xr, _) = self.shared.dropouts();
        fn khz(hz: u32) -> String {
            if hz % 1000 == 0 {
                format!("{} kHz", hz / 1000)
            } else {
                format!("{:.1} kHz", hz as f32 / 1000.0)
            }
        }
        let tags = match (d.low_latency, d.denoise) {
            (true, true) => " (low-latency, AI denoise)",
            (true, false) => " (low-latency)",
            (false, true) => " (AI denoise)",
            (false, false) => "",
        };
        let mut s = format!(
            "Running{} • {} → {} • {}-sample blocks • ~{:.0} ms latency",
            tags,
            khz(d.input_rate),
            khz(d.output_rate),
            d.plugin_block_size,
            d.buffered_ms + d.denoise_ms,
        );
        if xr > 0 {
            s.push_str(&format!(
                " • {} dropout{}",
                xr,
                if xr == 1 { "" } else { "s" }
            ));
        }
        s
    }

    /// Render starvation + trim counters for UI (dropouts, trims).
    pub fn dropouts(&self) -> (u64, u64) {
        self.shared.dropouts()
    }

    /// Total frames the render side has produced (diagnostics/tests).
    pub fn output_frames(&self) -> u64 {
        self.shared.out_frames.load(Ordering::Relaxed)
    }

    /// Enable the "hear yourself" monitor: the final processed mono block
    /// is teed into an independent output stream on `device`. The device
    /// must not be the route output, the capture input, or a virtual
    /// cable endpoint. Failure leaves the route untouched and returns the
    /// reason (also logged as `monitor: disabled (<reason>)`).
    pub fn enable_monitor(&mut self, device: &str) -> std::result::Result<(), String> {
        if let Some(m) = self.monitor_stream.as_ref() {
            if m.device() == device {
                return Ok(());
            }
            self.disable_monitor();
        }
        if let Some(why) = crate::monitor::device_problem(device, &self.output_name, &self.input_name)
        {
            eprintln!("monitor: disabled ({why})");
            return Err(why);
        }
        let Some(mut cons) = self.tee.take() else {
            let why = "monitor ring unavailable".to_string();
            eprintln!("monitor: disabled ({why})");
            return Err(why);
        };
        // Drop anything left from a previous monitor session (disable
        // does not drain the ring) so re-enabling never replays stale
        // audio or starts with a runaway queue.
        cons.clear();
        match crate::monitor::start(device, self.shared.out_rate, cons) {
            Ok(stream) => {
                self.monitor_stream = Some(stream);
                self.shared.monitor_on.store(true, Ordering::Relaxed);
                Ok(())
            }
            Err((cons, why)) => {
                self.tee = Some(cons);
                eprintln!("monitor: disabled ({why})");
                Err(why)
            }
        }
    }

    /// Stop the monitor stream. The route keeps running untouched; the
    /// ring consumer comes back for a later re-enable.
    pub fn disable_monitor(&mut self) {
        self.shared.monitor_on.store(false, Ordering::Relaxed);
        if let Some(m) = self.monitor_stream.take() {
            let device = m.device().to_string();
            self.tee = m.close();
            eprintln!("monitor: disabled (off) — was '{device}'");
        }
    }

    /// Suspend the monitor stream because the window went to the tray.
    ///
    /// Same teardown as [`RunningRoute::disable_monitor`] — the WASAPI
    /// render device, its callback thread and the resampler state all go
    /// away — but logged as a pause: the user's MONITOR preference is
    /// unchanged and the stream comes back when the window is shown.
    pub fn pause_monitor(&mut self) {
        self.shared.monitor_on.store(false, Ordering::Relaxed);
        if let Some(m) = self.monitor_stream.take() {
            let device = m.device().to_string();
            self.tee = m.close();
            eprintln!("monitor: paused (tray) — was '{device}'");
        }
    }

    /// Device the monitor is currently playing on (`None` = off).
    pub fn monitor_device(&self) -> Option<&str> {
        self.monitor_stream.as_ref().map(|m| m.device())
    }

    /// Exact engine status line for the monitor (`monitor: stream active
    /// on …`), when a stream is live.
    pub fn monitor_status(&self) -> Option<String> {
        self.monitor_stream.as_ref().map(|m| m.stats().status())
    }

    /// Sample rate + resampling mode of the live monitor stream
    /// (`48.0 kHz native` / `44.1 kHz resampled`).
    pub fn monitor_detail(&self) -> Option<String> {
        self.monitor_stream.as_ref().map(|m| m.stats().detail())
    }

    /// Last monitor backend error (the UI auto-disables on this).
    pub fn monitor_error(&self) -> Option<String> {
        self.monitor_stream.as_ref().and_then(|m| m.stats().error())
    }

    /// `(frames, underruns, drops, peak)` of the monitor stream.
    pub fn monitor_counters(&self) -> Option<(u64, u64, u64, f32)> {
        let m = self.monitor_stream.as_ref()?;
        Some((
            m.stats().frames(),
            m.stats().underruns(),
            m.stats().drops(),
            m.stats().peak(),
        ))
    }

    /// `q/o/c/p`-style readout like the C# `UpdateLatencyStatus`.
    /// `xr` counts render callbacks that starved (audible gaps);
    /// `trim` counts samples fast-forwarded under overload.
    /// Shown as the status bar's hover tooltip, not its text.
    pub fn status_line(&self) -> String {
        let d = self.diagnostics();
        let (b, inf, outf) = self.shared.counters();
        let (xr, trim) = self.shared.dropouts();
        format!(
            "q {:.0}ms / {}Hzx{}->{}Hzx{} / blk {} / inb {} inf {} outf {} / xr {} trim {} / dn {:.0}ms / {}",
            d.buffered_ms,
            d.input_rate,
            self.in_channels,
            d.output_rate,
            d.output_channels,
            d.plugin_block_size,
            b,
            inf,
            outf,
            xr,
            trim,
            d.denoise_ms,
            d.status
        )
    }

    /// Stop the route, harvesting each plugin's state blob in chain order.
    /// Best-effort per plugin (`None` = defaults), mirroring
    /// `CaptureCurrentSetup`. Streams are dropped first so no audio
    /// callback can touch a host while its state is read.
    pub fn stop(self) -> Vec<Option<Vec<u8>>> {
        let Self { _input, output, monitor_stream, chain, .. } = self;
        // Monitor first: its stream must never outlive the chain teardown,
        // and dropping it reclaims the ring consumer.
        drop(monitor_stream);
        drop(_input);
        drop(output);
        let mut plugins = chain.lock().unwrap();
        plugins
            .iter_mut()
            .map(|p| {
                // Live hosts harvest current state; host-less (disabled)
                // slots keep the state they carried in — otherwise every
                // restart would wipe bypassed plugins back to defaults.
                p.host
                    .as_mut()
                    .and_then(|h| h.save_state().ok())
                    .flatten()
                    .or_else(|| p.saved_state.clone())
            })
            .collect()
    }

    /// Called from the UI thread each frame: starts the paused output
    /// stream once the preload is buffered. (The output handle
    /// stays on the UI thread because cpal streams are not `Send`.)
    pub fn poll_preload(&mut self) {
        if !self.started && self.shared.buffered_samples() >= self.preload_samples {
            if let Some(o) = self.output.as_ref() {
                if o.play().is_ok() {
                    self.started = true;
                }
            }
        }
    }
}

// Scratch buffers reused every block (grow-only; the hot path stages one
// small copy per chain stage — see `process_frames`).
struct Scratch {
    mono: Vec<f32>,
    mono_out: Vec<f32>,
    plug_a: Vec<f32>,
    plug_b: Vec<f32>,
    resampled: Vec<f32>,
    route: Vec<f32>,
    res_carry: (f64, f32, f32),
    /// PI occupancy servo integral (learns the mic/cable clock offset).
    servo_i: f64,
    /// AI-denoised mic block (built-in stage output).
    den: Vec<f32>,
}

/// VA_PROF=1: accumulate per-stage microseconds and dump every 100 blocks.
mod prof {
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static CHAIN_US: AtomicU64 = AtomicU64::new(0);
    pub static RES_US: AtomicU64 = AtomicU64::new(0);
    pub static PUSH_US: AtomicU64 = AtomicU64::new(0);
    pub static BLOCKS: AtomicU64 = AtomicU64::new(0);
    pub fn on() -> bool {
        std::env::var_os("VA_PROF").is_some()
    }
    pub fn tick(chain: u64, res: u64, push: u64) {
        if !on() {
            return;
        }
        let c = CHAIN_US.fetch_add(chain, Ordering::Relaxed) + chain;
        let r = RES_US.fetch_add(res, Ordering::Relaxed) + res;
        let p = PUSH_US.fetch_add(push, Ordering::Relaxed) + push;
        let n = BLOCKS.fetch_add(1, Ordering::Relaxed) + 1;
        if n % 100 == 0 {
            eprintln!(
                "PROF: {n} blocks | chain {:.0}us/blk res {:.0}us/blk push {:.0}us/blk | total {:.0}us/blk",
                c as f64 / n as f64,
                r as f64 / n as f64,
                p as f64 / n as f64,
                (c + r + p) as f64 / n as f64
            );
        }
    }
}

fn process_frames(
    mono_in: &[f32],
    in_rate: u32,
    out_rate: u32,
    out_channels: usize,
    block: usize,
    chain: &Mutex<Vec<ChainPlugin>>,
    sc: &mut Scratch,
    shared: &Shared,
    mon_prod: &mut ringbuf::HeapProd<f32>,
) -> usize {
    let frames = mono_in.len();
    shared.meters.set_input(convert::peak_level(mono_in));
    shared.in_blocks.fetch_add(1, Ordering::Relaxed);
    shared.in_frames.fetch_add(frames as u64, Ordering::Relaxed);

    // Serial VST chain on a stereo lane with ping-pong buffers.
    let need_st = frames * 2;
    if sc.plug_a.len() < need_st {
        sc.plug_a.resize(need_st, 0.0);
        sc.plug_b.resize(need_st, 0.0);
    }
    convert::mono_to_stereo(mono_in, &mut sc.plug_a[..need_st]);
    // Mono bridge buffers (used by mono-only plugins mid-chain) are
    // sized here — before the plugin loop, not after it.
    if sc.mono.len() < frames {
        sc.mono.resize(frames, 0.0);
    }
    if sc.mono_out.len() < frames {
        sc.mono_out.resize(frames, 0.0);
    }
    // `cur_is_a == true` means the freshest data is in plug_a.
    let mut cur_is_a = true;
    let t_chain = std::time::Instant::now();
    // NEVER wait on the chain lock from the audio thread: while the UI
    // holds it (editor open/close — a plugin can spend hundreds of ms
    // building its window — or chain edits), this block runs DRY:
    // cur_is_a stays true, so the downmix reads the input copy. A brief
    // unprocessed blip instead of a silence gap plus dropout counts on
    // every editor click.
    if let Ok(mut plugins) = chain.try_lock() {
        for p in plugins.iter_mut() {
            if !p.enabled {
                continue;
            }
            let Some(host) = p.host.as_mut() else {
                continue;
            };
            let mut done = 0;
            let mut failed = false;
            while done < frames {
                let n = (frames - done).min(block);
                let (src, dst) = if cur_is_a {
                    (
                        &sc.plug_a[done * 2..(done + n) * 2],
                        &mut sc.plug_b[done * 2..(done + n) * 2],
                    )
                } else {
                    (
                        &sc.plug_b[done * 2..(done + n) * 2],
                        &mut sc.plug_a[done * 2..(done + n) * 2],
                    )
                };
                // NOTE: branch on `channels` first — calling the stereo
                // path on a mono host would trip its length assert.
                if host.channels == 1 {
                    // Mono-only plugin on the stereo lane: collapse to
                    // mono, process, expand back. `sc.mono`/`mono_out`
                    // are disjoint fields, safe beside the lane borrows.
                    convert::stereo_to_mono(src, &mut sc.mono[..n]);
                    if host
                        .process_f32(&sc.mono[..n], &mut sc.mono_out[..n])
                        .is_err()
                    {
                        failed = true;
                        break;
                    }
                    convert::mono_to_stereo(&sc.mono_out[..n], dst);
                } else if host.process_f32(src, dst).is_err() {
                    failed = true;
                    break;
                }
                done += n;
            }
            if failed {
                // Mid-chain failure: bypass the rest dry.
                *shared.status.lock().unwrap() = "VST bypassed".to_string();
                break;
            }
            cur_is_a = !cur_is_a;
        }
    }
    let finished_is_a = cur_is_a;
    if sc.mono.len() < frames {
        sc.mono.resize(frames, 0.0);
    }
    // Disjoint field borrows: no copy needed on the hot path.
    if finished_is_a {
        convert::stereo_to_mono(&sc.plug_a[..need_st], &mut sc.mono[..frames]);
    } else {
        convert::stereo_to_mono(&sc.plug_b[..need_st], &mut sc.mono[..frames]);
    }

    let chain_us = t_chain.elapsed().as_micros() as u64;
    let t_res = std::time::Instant::now();
    // Resample to output rate, then fan out. The occupancy servo
    // holds the ring near the low-latency preload target so mic/cable
    // clock drift can never starve or flood the render side.
    let est = (frames as f64 * out_rate as f64 / in_rate as f64).ceil() as usize + 4;
    if sc.resampled.len() < est {
        sc.resampled.resize(est, 0.0);
    }
    let target = preload_secs(block, out_rate) * out_rate as f64 * out_channels as f64;
    let servo = crate::resample::occupancy_servo(
        &mut sc.servo_i,
        shared.level.load(Ordering::Relaxed) as f64,
        target,
    );
    let got = resample_with_carry(
        &mut sc.res_carry,
        &sc.mono[..frames],
        in_rate,
        &mut sc.resampled[..est],
        out_rate,
        servo,
    );
    // Built-in AI denoise runs at the model's native 48 kHz — i.e. on
    // the stream AFTER the resampler. Feeding it capture-rate samples
    // (44.1 k) shifts every frequency ~9% and wrecks the voice; skip
    // the stage outright when the route isn't running at 48 kHz. The
    // model lives behind a mutex the UI thread swaps; a try_lock miss
    // just lets this block through dry and the next one catches up.
    let mut denoised = false;
    if shared.denoise.load(Ordering::Relaxed) && out_rate == 48_000 {
        if let Ok(mut g) = shared.denoiser.try_lock() {
            if let Some(d) = g.as_mut() {
                if sc.den.len() < got {
                    sc.den.resize(got, 0.0);
                }
                let produced = d.process(&sc.resampled[..got], &mut sc.den[..got]);
                // Short produced runs are normal (model quantum vs block
                // size); carry the slip and dry-fill only this block's
                // sliver instead of zero-filling.
                if produced < got {
                    sc.den[produced..got].copy_from_slice(&sc.resampled[produced..got]);
                }
                denoised = true;
            }
        }
    }
    let res_us = t_res.elapsed().as_micros() as u64;
    let need_route = got * out_channels;
    if sc.route.len() < need_route {
        sc.route.resize(need_route, 0.0);
    }
    let mono48: &[f32] = if denoised {
        &sc.den[..got]
    } else {
        &sc.resampled[..got]
    };
    convert::fan_out(mono48, out_channels, &mut sc.route[..need_route]);
    // "Hear yourself" tee: best-effort push of the exact final mono block
    // (post-chain / post-denoise) into the monitor ring. A full ring just
    // loses the newest samples for the monitor; the cable ring and its
    // counters are untouched.
    if shared.monitor_on.load(Ordering::Relaxed) {
        let _ = mon_prod.push_slice(mono48);
    }
    shared
        .meters
        .set_output(convert::peak_level(&sc.route[..need_route]));
    prof::tick(chain_us, res_us, 0);
    need_route
}

/// Input sink for the route: our own WASAPI capture (fully owned —
/// bounded teardown, cannot spin; see `lowlat_capture`) with cpal's
/// input stream only as the fallback if the device can't open there.
// Each variant is a live stream handle: never read, dropped to stop.
#[allow(dead_code)]
enum InputBackend {
    LowLat(crate::lowlat_capture::LowLatencyCapture),
    Cpal(cpal::Stream),
    /// `VA_FAKE_CAPTURE=1` probes/tests: synthetic in-process source.
    Fake(FakeInput),
}

/// Synthetic capture for `VA_FAKE_CAPTURE` (test/probe only), generated in
/// 10 ms blocks exactly like a 48 kHz mono mic. Two signals:
///
/// * `1` / `tone` — a quiet 440 Hz sine (what the route/monitor tests use);
/// * `speech` — a speech-like signal (harmonic stack + formants + syllable
///   envelope + noise). The denoiser's stage gating keys off the model's
///   LSNR estimate, so a pure tone sends it down the cheap path; a probe
///   that measures the plugin's real cost needs something voice-shaped.
///
/// Never enabled by the app itself.
struct FakeInput {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Deterministic speech-like mono block source (48 kHz), looped.
fn speech_loop(n: usize) -> Vec<f32> {
    let sr = 48_000.0f32;
    let mut out = Vec::with_capacity(n);
    let mut s = 0x9E37_79B9u32;
    for i in 0..n {
        let t = i as f32 / sr;
        let f0 = 118.0 * (1.0 + 0.02 * (2.0 * std::f32::consts::PI * 0.8 * t).sin());
        let mut v = 0.0f32;
        for k in 1..=40 {
            let f = f0 * k as f32;
            if f > 7_000.0 {
                break;
            }
            let formants = 1.0 / (1.0 + ((f - 700.0) / 250.0).powi(2))
                + 0.6 / (1.0 + ((f - 1_220.0) / 300.0).powi(2))
                + 0.2 / (1.0 + ((f - 2_600.0) / 400.0).powi(2));
            v += formants / k as f32 * (2.0 * std::f32::consts::PI * f * t).sin();
        }
        let syl = (2.0 * std::f32::consts::PI * 3.5 * t).sin().max(0.0).powf(0.7);
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let noise = (s as f32 / u32::MAX as f32 - 0.5) * 0.02;
        out.push(v * 0.06 * syl + noise);
    }
    out
}

impl FakeInput {
    fn spawn<F>(mut cb: F, rate: u32, speech: bool) -> Self
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = stop.clone();
        let rate = rate.max(1);
        let block = (rate as usize / 100).max(1);
        let step = 440.0f64 / rate as f64;
        let looped = if speech {
            speech_loop(rate as usize * 4)
        } else {
            Vec::new()
        };
        let thread = std::thread::Builder::new()
            .name("fake-capture".into())
            .spawn(move || {
                let mut buf = vec![0.0f32; block];
                let mut phase = 0.0f64;
                let mut pos = 0usize;
                let start = std::time::Instant::now();
                let mut emitted: u64 = 0;
                while !stop_t.load(Ordering::Relaxed) {
                    // Emit exactly the audio the wall clock says should
                    // exist since the start: Windows sleep granularity
                    // (10-16 ms) otherwise makes a naive sleep(10ms) loop
                    // under-produce and starves the route on its own.
                    let target = (start.elapsed().as_secs_f64() * rate as f64) as u64;
                    let mut deficit = target.saturating_sub(emitted) as usize;
                    while deficit > 0 {
                        let n = deficit.min(block);
                        if speech {
                            for s in buf[..n].iter_mut() {
                                *s = looped[pos];
                                pos += 1;
                                if pos == looped.len() {
                                    pos = 0;
                                }
                            }
                        } else {
                            for s in buf[..n].iter_mut() {
                                *s = (0.03 * (phase * std::f64::consts::TAU).sin()) as f32;
                                phase += step;
                                if phase >= 1.0 {
                                    phase -= 1.0;
                                }
                            }
                        }
                        cb(&buf[..n]);
                        emitted += n as u64;
                        deficit -= n;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
            })
            .expect("spawn fake capture thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Output sink for the route: either a cpal stream (Windows default
/// periods) or a low-latency WASAPI stream opened by [`crate::lowlat`].
enum OutputBackend {
    Cpal(cpal::Stream),
    LowLatency(crate::lowlat::LowLatencyOutput),
}

impl OutputBackend {
    fn play(&self) -> std::result::Result<(), ()> {
        match self {
            OutputBackend::Cpal(s) => s.play().map_err(|_| ()),
            OutputBackend::LowLatency(l) => l.play(),
        }
    }
}

/// Consumer half shared by both output backends: anti-lag trim, ring
/// pop, silence on starvation, and the level / out-frames accounting.
pub(crate) fn drain_into(
    cons: &mut ringbuf::HeapCons<f32>,
    shared: &Shared,
    data: &mut [f32],
    max_queue: usize,
    trim_to: usize,
) {
    let occ = cons.occupied_len();
    let mut dropped = 0;
    if occ > max_queue {
        dropped = cons.skip(occ - trim_to);
        shared.trim_drops.fetch_add(dropped as u64, Ordering::Relaxed);
    }
    let n = cons.pop_slice(data);
    if n < data.len() {
        data[n..].fill(0.0);
        // Count one dropout per contiguous gap, not per render wake:
        // a single 40 ms stall used to tally ~15 with small periods.
        if !shared.starving.swap(true, Ordering::Relaxed) {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        shared.starving.store(false, Ordering::Relaxed);
    }
    shared
        .level
        .fetch_sub((dropped + n) as i64, Ordering::Relaxed);
    shared
        .out_frames
        .fetch_add(data.len() as u64, Ordering::Relaxed);
}

/// Start routing: capture `input_name`, process through enabled `chain`,
/// render to `output_name`.
pub fn start(
    input_name: &str,
    output_name: &str,
    mut chain: Vec<ChainPlugin>,
    block_size: u32,
    denoise: Option<ai_denoise::Denoiser>,
) -> Result<RunningRoute> {
    let block = normalize_block_size(block_size) as usize;
    let host = cpal::default_host();

    // Debug/probe hook: VA_FAKE_CAPTURE=1 replaces the microphone with an
    // in-process synthetic source (quiet 440 Hz sine). Route/monitor
    // tests use it so they never need the real mic, and a monitor probe
    // can never create a real acoustic loop.
    let fake_capture = std::env::var_os("VA_FAKE_CAPTURE").is_some();

    let in_dev = if fake_capture {
        None
    } else {
        Some(
            host.input_devices()?
                .find(|d| d.name().ok().as_deref() == Some(input_name))
                .ok_or_else(|| EngineError::DeviceNotFound(input_name.into()))?,
        )
    };
    let out_dev = host
        .output_devices()?
        .find(|d| d.name().ok().as_deref() == Some(output_name))
        .ok_or_else(|| EngineError::DeviceNotFound(output_name.into()))?;

    let in_supported = match &in_dev {
        Some(d) => Some(d.default_input_config()?),
        None => None,
    };
    let in_cfg: cpal::StreamConfig = match &in_supported {
        Some(s) => s.config().clone(),
        None => cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(48_000),
            buffer_size: cpal::BufferSize::Default,
        },
    };
    let out_cfg: cpal::StreamConfig = out_dev.default_output_config()?.into();

    // Keep every chain slot so UI indices stay aligned with engine
    // indices; disabled or host-less plugins are skipped at runtime.
    for p in &mut chain {
        if let Some(h) = p.host.as_mut() {
            // Stereo first; mono-only plugins (classic denoisers) fall
            // back to a mono bus and are bridged per block below.
            if h
                .setup_processing(in_cfg.sample_rate.0 as f64, block as i32, 2)
                .is_err()
            {
                let _ = h.setup_processing(in_cfg.sample_rate.0 as f64, block as i32, 1);
            }
        }
    }
    let chain = Arc::new(Mutex::new(chain));

    let in_channels = in_cfg.channels.max(1) as usize;
    let in_rate = in_cfg.sample_rate.0;
    let out_rate = out_cfg.sample_rate.0;
    let out_channels = out_cfg.channels.max(1) as usize;

    let cap_samples = (out_rate as f64 * out_channels as f64 * 1.0) as usize; // 1000 ms
    let meters = Arc::new(Meters::default());
    let shared = Arc::new(Shared::new(out_rate, out_channels, meters.clone(), denoise));
    let scratch = Scratch {
        mono: Vec::new(),
        mono_out: Vec::new(),
        plug_a: Vec::new(),
        plug_b: Vec::new(),
        den: Vec::new(),
        resampled: Vec::new(),
        route: Vec::new(),
        res_carry: (0.0, 0.0, 0.0),
        servo_i: 0.0,
    };

    // Lock-free SPSC ring: the producer half lives on the capture thread,
    // the consumer half on the render thread. No mutex is shared between
    // the audio threads, so a big multichannel pull can never stall capture.
    // (The UI reads the level through an atomic estimate instead.)
    let (prod, mut cons) = HeapRb::<f32>::new(cap_samples.max(1024)).split();
    // Monitor tee ring: same lock-free SPSC pattern, separate from the
    // route ring so monitor trouble can never touch the cable feed.
    let (mon_prod, mon_cons) =
        HeapRb::<f32>::new(crate::monitor::ring_capacity(out_rate)).split();

    // Output stream, created paused; the UI thread starts it via
    // `poll_preload` once the preload is buffered. Low latency: preload =
    // two DSP blocks + 10 ms of jitter margin (≈31 ms at 512-sample
    // blocks, ≈15 ms at 128) instead of a flat 100 ms.
    let preload = preload_secs(block, out_rate);
    let preload_samples = (out_rate as f64 * out_channels as f64 * preload) as usize;
    // Anti-lag budget: if the queue runs away past 3× the preload (at
    // least 120 ms), drop the oldest audio back down to the preload.
    let max_queue_secs = (preload * 3.0).max(0.120);
    let max_queue = (max_queue_secs * out_rate as f64 * out_channels as f64) as usize;
    let trim_to = preload_samples + block * out_channels;
    // Output: prefer a low-latency shared stream (IAudioClient3, the
    // smallest engine period the device allows — ~2.7 ms on the virtual
    // cable) and fall back to cpal's default ~10 ms stream. Both paths
    // drain the ring through `drain_into` (same anti-lag + counters).
    // Debug knob: VA_NO_LOWLAT=1 forces the plain cpal output path
    // (bisecting route-vs-lowlat behavior in soak runs).
    let force_cpal = std::env::var_os("VA_NO_LOWLAT").is_some();
    let output = if force_cpal {
        eprintln!("output: VA_NO_LOWLAT set — using default output stream");
        let shared_o = shared.clone();
        let stream = out_dev.build_output_stream(
            &out_cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                crate::rt::ensure_rt_priority();
                drain_into(&mut cons, &shared_o, data, max_queue, trim_to);
            },
            |e| eprintln!("output stream error: {e}"),
            None,
        )?;
        OutputBackend::Cpal(stream)
    } else {
        match crate::lowlat::try_start(
            output_name,
            out_rate,
            out_channels,
            cons,
            shared.clone(),
            max_queue,
            trim_to,
        ) {
            Ok(low) => {
                shared.low_latency.store(true, Ordering::Relaxed);
                eprintln!(
                    "output: low-latency shared stream (buffer {} frames)",
                    low.period_frames()
                );
                OutputBackend::LowLatency(low)
            }
            Err((mut cons, why)) => {
                eprintln!("low-latency output unavailable ({why}); using default output stream");
                let shared_o = shared.clone();
                let stream = out_dev.build_output_stream(
                    &out_cfg,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        crate::rt::ensure_rt_priority();
                        drain_into(&mut cons, &shared_o, data, max_queue, trim_to);
                    },
                    |e| eprintln!("output stream error: {e}"),
                    None,
                )?;
                OutputBackend::Cpal(stream)
            }
        }
    };

    // Input callback shared by all sample formats: de-interleave to mono f32,
    // run the DSP, sanitize, and push lock-free. `scratch` is owned by
    // this single capture thread outright — no mutex in the hot path
    // (the only other lock, `chain`, is uncontended except for rare UI
    // editor operations).
    let make_input_cb = |shared: Arc<Shared>,
                         mut scratch: Scratch,
                         chain: Arc<Mutex<Vec<ChainPlugin>>>,
                         mut prod: ringbuf::HeapProd<f32>,
                         mut mon_prod: ringbuf::HeapProd<f32>| {
        move |mono: &[f32]| {
            crate::rt::ensure_rt_priority();
            shared
                .last_block_us
                .store((mono.len() as f64 / in_rate as f64 * 1_000_000.0) as u64, Ordering::Relaxed);
            // A DSP panic must never take down the process: WASAPI
            // threads cannot unwind into C++, so an uncaught panic
            // aborts everything. Isolate it, mute the block, flag it.
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process_frames(
                    mono,
                    in_rate,
                    out_rate,
                    out_channels,
                    block,
                    &chain,
                    &mut scratch,
                    &shared,
                    &mut mon_prod,
                )
            }));
            let n = match res {
                Ok(n) => n,
                Err(_) => {
                    // Poison-tolerant: never panic in the handler itself.
                    if let Ok(mut st) = shared.status.lock() {
                        *st = "DSP panic — block muted".to_string();
                    }
                    0
                }
            };
            if n == 0 {
                // Skip the push: the ring coasts, xr counts one block,
                // and the servo recovers. Pushing guess-length silence
                // would corrupt the stream clock worse than a gap.
                return;
            }
            for s in scratch.route[..n].iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }
            let pushed = prod.push_slice(&scratch.route[..n]);
            shared.level.fetch_add(pushed as i64, Ordering::Relaxed);
        }
    };

    // Capture callback: interleaved f32 in, mono out (reciprocal-scaled
    // downmix), then the engine's per-block callback. Built once and
    // handed to whichever backend opens — ours first, cpal as the
    // fallback — so the DSP path is identical either way.
    let mut cb = make_input_cb(shared.clone(), scratch, chain.clone(), prod, mon_prod);

    let input = if fake_capture {
        let mode = std::env::var("VA_FAKE_CAPTURE").unwrap_or_default();
        let speech = mode.eq_ignore_ascii_case("speech");
        eprintln!(
            "input: synthetic test capture (VA_FAKE_CAPTURE={mode}, {})",
            if speech { "speech-like" } else { "440 Hz" }
        );
        InputBackend::Fake(FakeInput::spawn(cb, in_rate, speech))
    } else {
        let in_dev = in_dev
            .as_ref()
            .expect("non-fake start resolved an input device");
        let in_supported = in_supported
            .as_ref()
            .expect("non-fake start resolved an input config");
        let mut deint = Vec::<f32>::new();
        let feed = move |data: &[f32], ch: usize| {
            let frames = data.len() / ch;
            if deint.len() < frames {
                deint.resize(frames, 0.0);
            }
            // Multiply by the reciprocal: one division per block, not one
            // per sample.
            let inv = 1.0f32 / ch as f32;
            for (i, m) in deint[..frames].iter_mut().enumerate() {
                let mut sum = 0.0f32;
                for c in 0..ch {
                    sum += data[i * ch + c];
                }
                *m = sum * inv;
            }
            cb(&deint[..frames]);
        };

        // Prefer our own WASAPI capture: fully owned loop, every error path
        // returns to the event wait (it cannot busy-spin like the cpal
        // thread did), and teardown is bounded — stop/restart can never
        // freeze behind a wedged COM call. cpal's input stream is only the
        // fallback for devices our path cannot open.
        match crate::lowlat_capture::try_start(
            input_name,
            in_rate,
            in_channels,
            shared.clone(),
            feed,
        ) {
            Ok(cap) => {
                eprintln!(
                    "input: own WASAPI capture (buffer {} frames)",
                    cap.period_frames()
                );
                InputBackend::LowLat(cap)
            }
            Err((mut feed, why)) => {
                eprintln!("own capture unavailable ({why}); using cpal input stream");
                // Reused staging buffer: no per-callback heap allocation on
                // the real-time capture thread (grow-only).
                let mut inter = Vec::<f32>::new();
                macro_rules! spawn_input {
                    ($sample:ty) => {{
                        in_dev.build_input_stream(
                            &in_cfg,
                            move |data: &[$sample], _: &cpal::InputCallbackInfo| {
                                if inter.len() < data.len() {
                                    inter.resize(data.len(), 0.0);
                                }
                                for (d, s) in inter.iter_mut().zip(data.iter()) {
                                    *d = s.to_float_sample() as f32;
                                }
                                feed(&inter[..data.len()], in_channels);
                            },
                            |e| eprintln!("input stream error: {e}"),
                            None,
                        )?
                    }};
                }

                let stream: cpal::Stream = match in_supported.sample_format() {
                    cpal::SampleFormat::F32 => spawn_input!(f32),
                    cpal::SampleFormat::F64 => spawn_input!(f64),
                    cpal::SampleFormat::I8 => spawn_input!(i8),
                    cpal::SampleFormat::I16 => spawn_input!(i16),
                    cpal::SampleFormat::I32 => spawn_input!(i32),
                    cpal::SampleFormat::I64 => spawn_input!(i64),
                    cpal::SampleFormat::U8 => spawn_input!(u8),
                    cpal::SampleFormat::U16 => spawn_input!(u16),
                    cpal::SampleFormat::U32 => spawn_input!(u32),
                    cpal::SampleFormat::U64 => spawn_input!(u64),
                    _ => return Err(EngineError::UnsupportedFormat),
                };

                stream.play()?;
                InputBackend::Cpal(stream)
            }
        }
    };

    Ok(RunningRoute {
        _input: input,
        output: Some(output),
        tee: Some(mon_cons),
        monitor_stream: None,
        output_name: output_name.to_string(),
        input_name: input_name.to_string(),
        preload_samples,
        started: false,
        chain,
        shared,
        meters,
        plugin_block: block as u32,
        in_rate,
        in_channels,
    })
}

/// Device enumeration helpers for the UI.
/// cpal exposes no stable endpoint IDs, so the name doubles as the ID
/// (friendly-name fallback behavior, matching the C# app).
pub fn input_devices() -> Vec<(String, String)> {
    cpal::default_host()
        .input_devices()
        .map(|it| {
            it.filter_map(|d| d.name().ok().map(|n| (n.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn output_devices() -> Vec<(String, String)> {
    cpal::default_host()
        .output_devices()
        .map(|it| {
            it.filter_map(|d| d.name().ok().map(|n| (n.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

/// Windows default render endpoint name (friendly name), if any. Used as
/// the initial "hear yourself" monitor device.
pub fn default_output_device_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor_dsp::Pipe;
    use ringbuf::traits::Split;

    fn scratch() -> Scratch {
        Scratch {
            mono: Vec::new(),
            mono_out: Vec::new(),
            plug_a: Vec::new(),
            plug_b: Vec::new(),
            den: Vec::new(),
            resampled: Vec::new(),
            route: Vec::new(),
            res_carry: (0.0, 0.0, 0.0),
            servo_i: 0.0,
        }
    }

    /// The whole point of the monitor: streaming the post-chain block to a
    /// second device must not cost the cable feed a single dropout, even
    /// though both run off different clocks. Driven in virtual time through
    /// the real `process_frames` + a real `Pipe` on the tee ring.
    #[test]
    fn monitor_streaming_keeps_route_dropouts_at_zero() {
        let rate = 48_000u32;
        let block = 480usize; // 10 ms at 48 kHz
        let shared = Arc::new(Shared::new(rate, 2, Arc::new(Meters::default()), None));
        shared.monitor_on.store(true, Ordering::Relaxed);
        let chain = Mutex::new(Vec::<ChainPlugin>::new());
        let (mut prod, mut cons) = ringbuf::HeapRb::<f32>::new(24_000).split();
        // Deliberately a different device rate (44.1 kHz) so the test covers
        // the band-limited path that the old linear interpolator mangled.
        let mut pipe = Pipe::new(rate, 44_100, 24_000);
        let mut sc = scratch();
        let mut block_in = vec![0.0f32; block];
        let mut out = vec![0.0f32; 441];
        let mut phase = 0.0f64;
        let mut rendered = Vec::new();

        // 4 s of virtual time: 400 route blocks of 10 ms, 400 device blocks.
        for step in 0..400 {
            for s in block_in.iter_mut() {
                *s = (0.3 * phase.sin()) as f32;
                phase += std::f64::consts::TAU * 1000.0 / rate as f64;
            }
            process_frames(
                &block_in, rate, rate, 2, block, &chain, &mut sc, &shared, &mut prod,
            );
            let rep = pipe.render(&mut cons, 441, &mut out);
            if step > 30 {
                assert_eq!(rep.written, 441, "monitor block {step} short");
                rendered.extend_from_slice(&out);
            }
        }

        let (xr, trim) = shared.dropouts();
        assert_eq!(xr, 0, "route dropouts with the monitor streaming");
        assert_eq!(trim, 0, "route trims with the monitor streaming");
        assert_eq!(shared.in_blocks.load(Ordering::Relaxed), 400);
        assert!(shared.in_frames.load(Ordering::Relaxed) >= 400 * block as u64);

        // And the monitor signal itself is intact: every sample-to-sample
        // step stays inside a 1 kHz sine's own slope (no clicks, no drops).
        let ideal = (std::f64::consts::TAU * 1000.0 / 44_100.0 * 0.3) as f32;
        let worst = rendered
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "monitor tee: {} output frames, route dropouts {xr}, trims {trim}, worst step {worst:.4} (ideal {ideal:.4})",
            rendered.len()
        );
        assert!(worst < ideal * 1.2, "monitor path stepped: {worst}");
        assert!(rendered.iter().any(|v| v.abs() > 0.2), "no audio on the tee");
    }
}
