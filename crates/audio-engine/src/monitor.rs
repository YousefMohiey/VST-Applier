//! "Hear yourself" monitor: a tee of the FINAL processed mono block into
//! an independent cpal output stream on a second device (headphones or
//! speakers), so levels can be set without another app.
//!
//! Design constraints:
//!  - The tee is a lock-free SPSC ring (`ringbuf`) written by the capture
//!    thread and drained by the monitor stream's callback. The route's
//!    own output ring is untouched: monitor underruns/drops are counted
//!    separately and can never stall or mute the cable feed.
//!  - The monitor runs on the device's DEFAULT output config (whatever
//!    Windows exposes, often 44.1/48 kHz, any channel count / sample
//!    format) and linearly resamples from the chain rate. A slow
//!    occupancy servo absorbs clock drift between the cable and the
//!    monitor endpoint and bounds latency at ~80 ms.
//!  - Every failure path returns the ring consumer so the caller can
//!    retry, and only ever logs/auto-disables the monitor.
//!
//! Safety: [`device_problem`] rejects the route's own output, the
//! capture input, and any virtual-cable endpoint. Monitoring into the
//! cable would be both pointless and a feedback risk.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use ringbuf::{HeapCons, traits::*};

use crate::resample::{occupancy_servo, resample_with_carry};

/// Monitor ring capacity in milliseconds of chain-rate mono audio.
/// Enough slack for device-clock drift; the consumer trims back to
/// [`TARGET_MS`] so latency can never grow past this.
pub(crate) const RING_MS: f64 = 500.0;

/// Occupancy the consumer servo parks the ring at (audible latency of
/// the monitor path; the user is listening, not performing).
const TARGET_MS: f64 = 80.0;

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
    /// Frames skipped to bound monitor latency (ring overrun).
    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
    /// Peak of what the monitor stream actually rendered (0..1).
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
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

    fn set_error(&self, text: impl Into<String>) {
        let text = text.into();
        if let Ok(mut e) = self.error.lock() {
            *e = text;
        }
        self.failed.store(true, Ordering::Relaxed);
    }
}

/// A live monitor stream. Dropping it stops the stream and reclaims the
/// ring consumer (see [`MonitorStream::close`]).
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
    let supported = match dev.default_output_config() {
        Ok(c) => c,
        Err(e) => return Err((cons, format!("default config unavailable: {e}"))),
    };
    let cfg: StreamConfig = supported.config();

    let stats = Arc::new(MonitorStats::default());
    let slot = Arc::new(Mutex::new(Some(cons)));

    let built = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::F64 => build::<f64>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::I8 => build::<i8>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::I16 => build::<i16>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::I32 => build::<i32>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::I64 => build::<i64>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::U8 => build::<u8>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::U16 => build::<u16>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::U32 => build::<u32>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
        SampleFormat::U64 => build::<u64>(&dev, &cfg, src_rate, slot.clone(), stats.clone()),
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
    cfg: &StreamConfig,
    src_rate: u32,
    slot: Arc<Mutex<Option<HeapCons<f32>>>>,
    stats: Arc<MonitorStats>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32>,
{
    let dst_rate = cfg.sample_rate.0.max(1);
    let channels = cfg.channels.max(1) as usize;
    // Pre-size every buffer before the stream starts: the callback must
    // never allocate. 4096 frames covers any shared-mode period we've
    // seen; the grow fallback exists only for exotic drivers.
    let ratio = src_rate as f64 / dst_rate as f64;
    let mut src_buf = vec![0.0f32; ((4096.0 * ratio) as usize + 32).max(1024)];
    let mut mono_buf = vec![0.0f32; 4096];
    let mut carry = (0.0f64, 0.0f32, 0.0f32);
    let mut servo_i = 0.0f64;
    let mut primed = false;

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
            // render one block of silence, never block the audio thread.
            let Ok(mut guard) = slot.try_lock() else {
                data.fill(T::EQUILIBRIUM);
                return;
            };
            let Some(cons) = guard.as_mut() else {
                data.fill(T::EQUILIBRIUM);
                return;
            };

            // Pre-roll: never pull from a ring shallower than the target.
            // The tee fills it at the route's block rate, so starting
            // immediately would starve on the first callbacks and then
            // crackle on every scheduling hiccup. Render silence until
            // ~80 ms is buffered, then keep that lead with the servo.
            let target = (TARGET_MS / 1000.0 * src_rate as f64) as usize;
            let occ = cons.occupied_len();
            if occ < target {
                // Hysteresis: a shallow dip around the 80 ms target is
                // normal block quantization; only a half-empty ring is a
                // real gap (and a reason to re-prime).
                if !primed || occ < target / 2 {
                    if primed {
                        primed = false;
                        stats.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                    data.fill(T::EQUILIBRIUM);
                    stats.frames.fetch_add(frames as u64, Ordering::Relaxed);
                    return;
                }
            }
            primed = true;

            // Bound monitor latency: if the consumer fell behind (the
            // device stalled, or clocks drifted), drop back to target.
            let max_occ = target * 3;
            if occ > max_occ {
                let skipped = cons.skip(occ - target);
                stats.drops.fetch_add(skipped as u64, Ordering::Relaxed);
            }
            let occ = cons.occupied_len() as f64;
            let servo = occupancy_servo(&mut servo_i, occ, target as f64);

            // Pull exactly the source frames this device block needs
            // (± servo). Underrun -> silence, counted once per gap.
            let need = ((frames as f64 * ratio) * servo).ceil() as usize + 4;
            if src_buf.len() < need {
                src_buf.resize(need, 0.0);
            }
            let got = cons.pop_slice(&mut src_buf[..need]);
            if got < need {
                src_buf[got..need].fill(0.0);
                if primed {
                    primed = false;
                    stats.underruns.fetch_add(1, Ordering::Relaxed);
                }
            }

            if mono_buf.len() < frames {
                mono_buf.resize(frames, 0.0);
            }
            let n = resample_with_carry(
                &mut carry,
                &src_buf[..need],
                src_rate,
                &mut mono_buf[..frames],
                dst_rate,
                servo,
            );
            if n < frames {
                mono_buf[n..frames].fill(0.0);
            }
            let peak = crate::convert::peak_level(&mono_buf[..frames]);
            stats.peak.store(peak.to_bits(), Ordering::Relaxed);

            for (i, s) in mono_buf[..frames].iter().enumerate() {
                let v = *s * GAIN;
                for c in 0..channels {
                    data[i * channels + c] = T::from_sample(v);
                }
            }
            stats.frames.fetch_add(frames as u64, Ordering::Relaxed);
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
