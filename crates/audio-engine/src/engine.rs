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
use ringbuf::{traits::*, HeapRb};
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
    status: Mutex<String>,
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
}

impl Shared {
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
    _input: cpal::Stream,
    output: Option<OutputBackend>,
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
        }
    }

    pub fn meters(&self) -> &Arc<Meters> {
        &self.meters
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
        let mut plugins = self.chain.lock().unwrap();
        plugins.push(cp);
        Ok(plugins.len() - 1)
    }

    /// Apply the start-path processing setup to one slot's host:
    /// stereo first, mono fallback for mono-only plugins.
    fn setup_slot(&self, cp: &mut ChainPlugin) -> Result<()> {
        if let Some(h) = cp.host.as_mut() {
            if h.setup_processing(self.in_rate as f64, self.plugin_block as i32, 2)
                .is_err()
                && h.setup_processing(self.in_rate as f64, self.plugin_block as i32, 1)
                    .is_err()
            {
                return Err(EngineError::Plugin(vst_host_sys::HostError::CallFailed {
                    code: -1,
                    message: "setup_processing failed".into(),
                }));
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
        let mut s = format!(
            "Running • {} → {} • {}-sample blocks • ~{:.0} ms latency",
            khz(d.input_rate),
            khz(d.output_rate),
            d.plugin_block_size,
            d.buffered_ms,
        );
        if d.low_latency {
            s.push_str(" • low-latency output");
        }
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

    /// `q/o/c/p`-style readout like the C# `UpdateLatencyStatus`.
    /// `xr` counts render callbacks that starved (audible gaps);
    /// `trim` counts samples fast-forwarded under overload.
    /// Shown as the status bar's hover tooltip, not its text.
    pub fn status_line(&self) -> String {
        let d = self.diagnostics();
        let (b, inf, outf) = self.shared.counters();
        let (xr, trim) = self.shared.dropouts();
        format!(
            "q {:.0}ms / {}Hzx{}->{}Hzx{} / blk {} / inb {} inf {} outf {} / xr {} trim {} / {}",
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
            d.status
        )
    }

    /// Stop the route, harvesting each plugin's state blob in chain order.
    /// Best-effort per plugin (`None` = defaults), mirroring
    /// `CaptureCurrentSetup`. Streams are dropped first so no audio
    /// callback can touch a host while its state is read.
    pub fn stop(self) -> Vec<Option<Vec<u8>>> {
        let Self { _input, output, chain, .. } = self;
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
    {
        let mut plugins = chain.lock().unwrap();
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

    // Resample to output rate, then fan out. The occupancy servo
    // holds the ring near the low-latency preload target so mic/cable
    // clock drift can never starve or flood the render side.
    let est = (frames as f64 * out_rate as f64 / in_rate as f64).ceil() as usize + 4;
    if sc.resampled.len() < est {
        sc.resampled.resize(est, 0.0);
    }
    let target = preload_secs(block, out_rate) * out_rate as f64 * out_channels as f64;
    let servo = crate::resample::occupancy_servo(shared.level.load(Ordering::Relaxed) as f64, target);
    let got = resample_with_carry(
        &mut sc.res_carry,
        &sc.mono[..frames],
        in_rate,
        &mut sc.resampled[..est],
        out_rate,
        servo,
    );
    let need_route = got * out_channels;
    if sc.route.len() < need_route {
        sc.route.resize(need_route, 0.0);
    }
    convert::fan_out(
        &sc.resampled[..got],
        out_channels,
        &mut sc.route[..need_route],
    );
    shared
        .meters
        .set_output(convert::peak_level(&sc.route[..need_route]));
    need_route
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
        shared.underruns.fetch_add(1, Ordering::Relaxed);
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
) -> Result<RunningRoute> {
    let block = normalize_block_size(block_size) as usize;
    let host = cpal::default_host();

    let in_dev = host
        .input_devices()?
        .find(|d| d.name().ok().as_deref() == Some(input_name))
        .ok_or_else(|| EngineError::DeviceNotFound(input_name.into()))?;
    let out_dev = host
        .output_devices()?
        .find(|d| d.name().ok().as_deref() == Some(output_name))
        .ok_or_else(|| EngineError::DeviceNotFound(output_name.into()))?;

    let in_supported = in_dev.default_input_config()?;
    let in_cfg: cpal::StreamConfig = in_supported.config().clone();
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
    let shared = Arc::new(Shared {
        level: AtomicI64::new(0),
        capacity_ms: 1000.0,
        out_rate,
        out_channels,
        meters: meters.clone(),
        status: Mutex::new("running".to_string()),
        last_block_us: AtomicU64::new(0),
        underruns: AtomicU64::new(0),
        trim_drops: AtomicU64::new(0),
        in_blocks: AtomicU64::new(0),
        in_frames: AtomicU64::new(0),
        out_frames: AtomicU64::new(0),
        low_latency: AtomicBool::new(false),
    });
    let scratch = Scratch {
        mono: Vec::new(),
        mono_out: Vec::new(),
        plug_a: Vec::new(),
        plug_b: Vec::new(),
        resampled: Vec::new(),
        route: Vec::new(),
        res_carry: (0.0, 0.0, 0.0),
    };

    // Lock-free SPSC ring: the producer half lives on the capture thread,
    // the consumer half on the render thread. No mutex is shared between
    // the audio threads, so a big multichannel pull can never stall capture.
    // (The UI reads the level through an atomic estimate instead.)
    let (prod, cons) = HeapRb::<f32>::new(cap_samples.max(1024)).split();

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
    let output = match crate::lowlat::try_start(
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
    };

    // Input callback shared by all sample formats: de-interleave to mono f32,
    // run the DSP, sanitize, and push lock-free. `scratch` is owned by
    // this single capture thread outright — no mutex in the hot path
    // (the only other lock, `chain`, is uncontended except for rare UI
    // editor operations).
    let make_input_cb = |shared: Arc<Shared>,
                         mut scratch: Scratch,
                         chain: Arc<Mutex<Vec<ChainPlugin>>>,
                         mut prod: ringbuf::HeapProd<f32>| {
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

    macro_rules! spawn_input {
        ($sample:ty) => {{
            let mut cb = make_input_cb(shared.clone(), scratch, chain.clone(), prod);
            // Reused staging buffer: no per-callback heap allocation on
            // the real-time capture thread (grow-only).
            let mut deint = Vec::<f32>::new();
            in_dev.build_input_stream(
                &in_cfg,
                move |data: &[$sample], _: &cpal::InputCallbackInfo| {
                    let frames = data.len() / in_channels;
                    if deint.len() < frames {
                        deint.resize(frames, 0.0);
                    }
                    // Multiply by the reciprocal: one division per block,
                    // not one per sample.
                    let inv = 1.0f32 / in_channels as f32;
                    for (i, m) in deint[..frames].iter_mut().enumerate() {
                        let mut sum = 0.0f32;
                        for c in 0..in_channels {
                            sum += data[i * in_channels + c].to_float_sample() as f32;
                        }
                        *m = sum * inv;
                    }
                    cb(&deint[..frames]);
                },
                |e| eprintln!("input stream error: {e}"),
                None,
            )?
        }};
    }

    let in_stream: cpal::Stream = match in_supported.sample_format() {
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

    in_stream.play()?;

    Ok(RunningRoute {
        _input: in_stream,
        output: Some(output),
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
