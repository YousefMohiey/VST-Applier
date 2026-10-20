//! Steady-state cost of the ORT backend at the plugin's real cadence.
//!
//! The plugin calls `DfOrt::process` once per 10 ms hop from the audio
//! thread. Between hops the ONNX Runtime thread pools and the DirectML
//! pipeline are idle — but only if they are configured to be. This
//! example runs the exact 100 Hz cadence with a speech-like signal (so
//! every stage of the model is exercised, unlike a pure tone) and reports
//! the process's CPU time as a percentage of one core, plus per-hop cost.
//!
//!   DF_ORT_DIR=<dir with onnxruntime.dll> \
//!     cargo run -p df-ort --release --example hopload -- [seconds]
//!
//! `DF_ORT_FORCE_CPU=1` measures the CPU execution provider for
//! comparison. The audio is generated, never played.

use std::time::{Duration, Instant};

use df_ort::{DfOrt, RuntimeParams};

/// Total CPU seconds (kernel + user) this process has burned so far.
fn cpu_seconds() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }
    unsafe {
        let mut create = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetProcessTimes(GetCurrentProcess(), &mut create, &mut exit, &mut kernel, &mut user)
            .is_err()
        {
            return f64::NAN;
        }
        // FILETIME is 100 ns units.
        (to_u64(kernel) + to_u64(user)) as f64 * 100e-9
    }
}

/// Deterministic speech-like signal (same recipe as `smoke`): harmonic
/// stack + formants + syllable envelope + noise. Keeps the model in its
/// active LSNR range so gains and DF coefficients both run.
fn speechy(n: usize, noise_amp: f32, seed: u32) -> Vec<f32> {
    let sr = 48_000.0f32;
    let mut out = Vec::with_capacity(n);
    let mut s = seed;
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
        let noise = (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * noise_amp;
        out.push(v * 0.02 * syl + noise);
    }
    out
}

fn main() {
    let secs: f64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(10.0);

    let mut df = DfOrt::new(&RuntimeParams::default_with_ch(1)).expect("DfOrt::new");
    let hop = df.hop_size;
    df.warm_up(4);
    println!(
        "ep = {:?}  ({}), hop = {hop} samples, latency = {} samples",
        df.ep,
        df.status_line(),
        df.latency_samples()
    );

    // A few seconds of speech-like audio, looped.
    let audio = speechy(48_000 * 4, 0.01, 0x9E37_79B9);
    let mut pos = 0usize;
    let mut noisy = vec![0.0f32; hop];
    let mut enh = vec![0.0f32; hop];
    let mut per_hop_us: Vec<u64> = Vec::new();

    let cpu0 = cpu_seconds();
    let t0 = Instant::now();
    let mut next = Instant::now();
    let mut hops = 0u64;
    let mut lsnr_sum = 0.0f64;
    while t0.elapsed().as_secs_f64() < secs {
        for s in noisy.iter_mut() {
            *s = audio[pos];
            pos = (pos + 1) % audio.len();
        }
        let t = Instant::now();
        match df.process(&noisy, &mut enh) {
            Ok(lsnr) => lsnr_sum += lsnr as f64,
            Err(e) => {
                println!("process error: {e}");
                return;
            }
        }
        per_hop_us.push(t.elapsed().as_micros() as u64);
        hops += 1;
        // 10 ms hop cadence, like the plugin under a 48 kHz host.
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    let elapsed = t0.elapsed();
    let cpu = (cpu_seconds() - cpu0) / elapsed.as_secs_f64() * 100.0;

    per_hop_us.sort_unstable();
    let pick = |q: f64| -> f64 {
        if per_hop_us.is_empty() {
            0.0
        } else {
            per_hop_us[((per_hop_us.len() as f64 * q) as usize).min(per_hop_us.len() - 1)] as f64
                / 1000.0
        }
    };
    let mean = per_hop_us.iter().sum::<u64>() as f64 / per_hop_us.len().max(1) as f64 / 1000.0;
    println!(
        "hops = {hops} in {:.1} s  ->  cpu = {cpu:.1}% of one core",
        elapsed.as_secs_f64()
    );
    println!(
        "per hop: mean {mean:.3} ms, p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms (budget 10 ms)",
        pick(0.50),
        pick(0.95),
        per_hop_us.last().map(|v| *v as f64 / 1000.0).unwrap_or(0.0)
    );
    println!("mean lsnr = {:.1} dB", lsnr_sum / hops.max(1) as f64);
}
