//! "Hear yourself" monitor: a tee of the FINAL processed mono block into
//! an independent cpal output stream on a second device (headphones or
//! speakers), so levels can be set without another app.
//!
//! Design constraints:
//!  - The tee is a lock-free SPSC ring (`ringbuf`) written by the capture
//!    thread and drained by the monitor stream's callback. The route's
//!    own output ring is untouched: monitor underruns/drops are counted
//!    separately and can never stall or mute the cable feed.
//!  - The stream is opened at the CHAIN rate whenever the endpoint lists
//!    it (the common case: a 48 kHz cable and a 48 kHz headset), so there
//!    is no rate conversion at all. Otherwise `monitor_dsp::Pipe`
//!    band-limits with a windowed-sinc kernel — never the route's 2-tap
//!    linear interpolator, whose images are the buzz this module used to
//!    produce.
//!  - Occupancy is held at ~80 ms by a proportional servo that nudges the
//!    fractional resampling ratio (a fractional-delay trim), so clock
//!    drift becomes inaudible pitch offset instead of dropped samples.
//!  - The callback only `try_lock`s the consumer slot, never allocates,
//!    never logs and never blocks. Every buffer it touches was sized
//!    before `play()`.
//!
//! Safety: `device_problem` rejects the route's own output, the capture
//! input, and any virtual-cable endpoint. Monitoring into the cable would
//! be both pointless and a feedback risk.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use ringbuf::HeapCons;

use crate::monitor_dsp::{MAX_CHUNK, Pipe, Report};

/// Monitor ring capacity in milliseconds of chain-rate mono audio.
/// Enough slack for device-clock drift; the consumer parks at
/// `monitor_dsp::TARGET_MS` so latency can never grow past this.
pub(crate) const RING_MS: f64 = 500.0;

/// Unity gain: the monitor must hear exactly what the cable gets (the
/// chain's limiter owns the level). Keep a named constant so a future
/// volume control has one obvious home.
const GAIN: f32 = 1.0;

/// Ring frames (chain-rate mono) for a monitor ring at `src_rate`.
pub(crate) fn ring_capacity(src_rate: u32) -> usize {
    ((src_rate as f64 * RING_MS / 1000.0) as usize).max(1024)
}

/// True when the device name looks like a virtual cable render endpoint.
fn is_cable(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    low.contains("cable input") || low.contains("vb-audio") || low.contains("voicemeeter")
}

/// Why `device` must NOT be used as the monitor, or `None` when it is a
/// safe choice. `route_output` / `capture_input` are the exact names the
/// route is using (cpal names double as IDs).
pub fn device_problem(device: &str, route_output: &str, capture_input: &str) -> Option<String> {
    let d = device.trim();
    if d.is_empty() {
        return Some("no monitor device selected".to_string());
    }
    let eq = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
    if eq(d, route_output) {
        return Some(format!(
            "'{d}' is the route output — refusing to monitor into the cable"
        ));
    }
    if eq(d, capture_input) {
        return Some(format!(
            "'{d}' is the capture input — monitoring it would feed back"
        ));
    }
    if is_cable(d) {
        return Some(format!("'{d}' is a virtual-cable endpoint, not headphones"));
    }
    None
}

/// Counters + last status for the UI/probe. Kept separate from the
/// route's `Shared` so monitor trouble can never look like route
/// trouble (and vice versa).
#[derive(Debug, Default)]
pub struct MonitorStats {
    frames: AtomicU64,
    underruns: AtomicU64,
    drops: AtomicU64,
    peak: AtomicU32,
    failed: AtomicBool,
    /// `48.0 kHz native` / `44.1 kHz resampled` — what the stream runs at.
    detail: Mutex<String>,
    error: Mutex<String>,
    status: Mutex<String>,
}

impl MonitorStats {
    /// Device frames rendered since the stream started.
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
    /// Contiguous starvation gaps (one per gap, warm-up excluded).
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
    /// Input frames fast-forwarded to bound monitor latency.
    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
    /// Peak rendered since the stream started (0..1).
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }
    /// Sample rate + whether the stream needed rate conversion.
    pub fn detail(&self) -> String {
        self.detail.lock().map(|s| s.clone()).unwrap_or_default()
    }
    /// Last stream error, if the backend reported one.
    pub fn error(&self) -> Option<String> {
        if !self.failed.load(Ordering::Relaxed) {
            return None;
        }
        let e = self.error.lock().ok()?.clone();
        (!e.is_empty()).then_some(e)
    }
    /// Exact engine status line (`monitor: stream active on …`).
    pub fn status(&self) -> String {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "monitor: disabled (status unavailable)".into())
    }

    fn set_status(&self, text: impl Into<String>) {
        if let Ok(mut s) = self.status.lock() {
            *s = text.into();
        }
    }

    fn set_detail(&self, text: impl Into<String>) {
        if let Ok(mut d) = self.detail.lock() {
            *d = text.into();
        }
    }

    fn set_error(&self, text: impl Into<String>) {
        let text = text.into();
        if let Ok(mut e) = self.error.lock() {
            *e = text;
        }
        self.failed.store(true, Ordering::Relaxed);
    }

    /// Audio-thread accounting: no locks, only relaxed atomics.
    fn account(&self, frames: usize, rep: Report) {
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
        if rep.underrun {
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
        if rep.dropped > 0 {
            self.drops.fetch_add(rep.dropped, Ordering::Relaxed);
        }
        let mut cur = self.peak.load(Ordering::Relaxed);
        while rep.peak > f32::from_bits(cur) {
            match self.peak.compare_exchange_weak(
                cur,
                rep.peak.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(v) => cur = v,
            }
        }
    }
}

/// A live monitor stream. Dropping it stops the stream and reclaims the
/// ring consumer (see `MonitorStream::close`).
pub struct MonitorStream {
    device: String,
    stats: Arc<MonitorStats>,
    /// Consumer slot shared with the render callback. The callback only
    /// ever `try_lock`s it; enable/disable happens on the UI thread.
    slot: Arc<Mutex<Option<HeapCons<f32>>>>,
    stream: Option<cpal::Stream>,
}

impl MonitorStream {
    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn stats(&self) -> &Arc<MonitorStats> {
        &self.stats
    }

    /// Stop the stream and hand the ring consumer back (for a later
    /// re-enable or a device switch).
    pub fn close(mut self) -> Option<HeapCons<f32>> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> Option<HeapCons<f32>> {
        if let Some(s) = self.stream.take() {
            // cpal's WASAPI drop stops and joins the render thread.
            drop(s);
        }
        self.slot.lock().ok().and_then(|mut g| g.take())
    }
}

impl Drop for MonitorStream {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

/// Pick the stream config for `dev`: the chain rate when the endpoint
/// lists it (no resampling at all), otherwise whatever Windows calls the
/// default. Returns `(config, native)`.
fn pick_config(
    dev: &cpal::Device,
    want: u32,
) -> Result<cpal::SupportedStreamConfig, Box<dyn std::error::Error>> {
    let mut best: Option<(bool, u16, cpal::SupportedStreamConfigRange)> = None;
    if let Ok(ranges) = dev.supported_output_configs() {
        for r in ranges {
            if want < r.min_sample_rate().0 || want > r.max_sample_rate().0 {
                continue;
            }
            // Prefer f32 (no conversion), then the widest channel count.
            let rank = (r.sample_format() == SampleFormat::F32, r.channels());
            let better = best
                .as_ref()
                .map(|(f32_ok, ch, _)| rank > (*f32_ok, *ch))
                .unwrap_or(true);
            if better {
                best = Some((rank.0, rank.1, r));
            }
        }
    }
    match best {
        Some((_, _, r)) => Ok(r.with_sample_rate(cpal::SampleRate(want))),
        None => Ok(dev.default_output_config()?),
    }
}

/// Open `device` as a monitor output stream and start it. On failure the
/// ring consumer comes back untouched (nothing was consumed).
pub(crate) fn start(
    device: &str,
    src_rate: u32,
    cons: HeapCons<f32>,
) -> Result<MonitorStream, (HeapCons<f32>, String)> {
    let host = cpal::default_host();
    let Some(dev) = host
        .output_devices()
        .ok()
        .and_then(|mut it| it.find(|d| d.name().ok().as_deref() == Some(device)))
    else {
        return Err((cons, format!("output device '{device}' not found")));
    };
    let supported = match pick_config(&dev, src_rate) {
        Ok(v) => v,
        Err(e) => return Err((cons, format!("no usable output config: {e}"))),
    };
    let cfg = supported.config();
    let dst_rate = cfg.sample_rate.0.max(1);

    let stats = Arc::new(MonitorStats::default());
    let slot = Arc::new(Mutex::new(Some(cons)));
    let pipe = Pipe::new(src_rate, dst_rate, ring_capacity(src_rate));
    let native = pipe.native();

    let built = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::F64 => build::<f64>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::I8 => build::<i8>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::I16 => build::<i16>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::I32 => build::<i32>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::I64 => build::<i64>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::U8 => build::<u8>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::U16 => build::<u16>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::U32 => build::<u32>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        SampleFormat::U64 => build::<u64>(&dev, &cfg, pipe, slot.clone(), stats.clone()),
        other => {
            return Err((
                reclaim(&slot),
                format!("unsupported sample format {other:?}"),
            ));
        }
    };
    let stream = match built {
        Ok(s) => s,
        Err(e) => return Err((reclaim(&slot), format!("build stream: {e}"))),
    };
    if let Err(e) = stream.play() {
        drop(stream);
        return Err((reclaim(&slot), format!("play: {e}")));
    }

    stats.set_detail(format!(
        "{:.1} kHz {}",
        dst_rate as f64 / 1000.0,
        if native { "native" } else { "resampled" }
    ));
    eprintln!(
        "monitor: open {device} @ {dst_rate} Hz ({})",
        if native {
            "native, no resampling"
        } else {
            "band-limited resampler"
        }
    );
    let status = format!("monitor: stream active on {device}");
    stats.set_status(status.clone());
    eprintln!("{status}");
    Ok(MonitorStream {
        device: device.to_string(),
        stats,
        slot,
        stream: Some(stream),
    })
}

/// Take the consumer back out of the slot (the callback never keeps it).
fn reclaim(slot: &Arc<Mutex<Option<HeapCons<f32>>>>) -> HeapCons<f32> {
    slot.lock()
        .expect("monitor consumer slot poisoned")
        .take()
        .expect("monitor callback never takes the consumer")
}

fn build<T>(
    dev: &cpal::Device,
    cfg: &cpal::StreamConfig,
    mut pipe: Pipe,
    slot: Arc<Mutex<Option<HeapCons<f32>>>>,
    stats: Arc<MonitorStats>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = cfg.channels.max(1) as usize;
    // Pre-size the mono scratch before the stream starts: the callback must
    // never allocate, and `Pipe` finishes a block instead of growing.
    let mut mono = vec![0.0f32; MAX_CHUNK];

    let err_stats = stats.clone();
    let err_fn = move |e: cpal::StreamError| {
        eprintln!("monitor: stream error: {e}");
        err_stats.set_error(e.to_string());
        err_stats.set_status(format!("monitor: disabled ({e})"));
    };

    dev.build_output_stream(
        cfg,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            crate::rt::ensure_rt_priority();
            let frames = data.len() / channels;
            if frames == 0 {
                return;
            }
            // A busy slot means the UI is switching devices right now:
            // render silence, never block the audio thread.
            let Ok(mut guard) = slot.try_lock() else {
                data.fill(T::EQUILIBRIUM);
                return;
            };
            let Some(cons) = guard.as_mut() else {
                data.fill(T::EQUILIBRIUM);
                return;
            };

            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(mono.len());
                let rep = pipe.render(cons, n, &mut mono[..n]);
                stats.account(n, rep);
                for (i, s) in mono[..n].iter().enumerate() {
                    let v = *s * GAIN;
                    let base = (done + i) * channels;
                    for c in 0..channels {
                        data[base + c] = T::from_sample(v);
                    }
                }
                done += n;
            }
        },
        err_fn,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_device_safety_rules() {
        assert!(device_problem("", "CABLE Input", "Microphone").is_some());
        assert!(device_problem("CABLE Input", "CABLE Input", "Microphone").is_some());
        assert!(device_problem("Microphone (USB)", "CABLE Input", "Microphone (USB)").is_some());
        assert!(
            device_problem("CABLE Input (VB-Audio Virtual Cable)", "Speakers", "Mic").is_some()
        );
        assert!(
            device_problem("Headphones (Realtek(R) Audio)", "CABLE Input", "Microphone").is_none()
        );
        assert!(
            device_problem("Speakers (Realtek(R) Audio)", "CABLE Input", "Microphone").is_none()
        );
    }

    #[test]
    fn ring_covers_half_a_second() {
        assert_eq!(ring_capacity(48_000), 24_000);
        assert!(ring_capacity(0) >= 1024);
    }
}
