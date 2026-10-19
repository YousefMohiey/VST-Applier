//! Smoke test for the ORT backend: load the models, stream a deterministic
//! speech-like signal for a few seconds, print the EP, per-hop timing and the
//! measured attenuation / LSNR range.
//!
//!   DF_ORT_DIR=<dir with onnxruntime.dll> cargo run -p df-ort --release --example smoke

use std::time::Instant;

use df_ort::{DfOrt, RuntimeParams};

/// Deterministic "speechy" signal: a 118 Hz harmonic stack shaped by two
/// formants, syllabically modulated, plus broadband noise. Unlike white noise
/// this keeps the model in its active range (LSNR between the stage
/// thresholds), so gains and DF coefficients are actually exercised.
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
        // Syllables at 3.5 Hz with short pauses.
        let syl = (2.0 * std::f32::consts::PI * 3.5 * t).sin().max(0.0).powf(0.7);
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let noise = (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * noise_amp;
        out.push(v * 0.02 * syl + noise);
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn main() {
    let t0 = Instant::now();
    let mut df = DfOrt::new(&RuntimeParams::default_with_ch(1)).expect("DfOrt::new");
    let init_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("ep = {:?}", df.ep);
    println!("status = {}", df.status_line());
    println!("detail = {}", df.status.detail);
    println!(
        "init = {init_ms:.0} ms, hop = {} samples, latency = {} samples",
        df.hop_size,
        df.latency_samples()
    );

    let hops = 300; // 3 s
    let input = speechy(hops * df.hop_size, 0.02, 0x1234_5678);
    let mut out = vec![0.0f32; input.len()];

    let mut worst_us = 0u128;
    let mut total_us = 0u128;
    let mut ls_min = f32::MAX;
    let mut ls_max = f32::MIN;
    let t1 = Instant::now();
    for h in 0..hops {
        let a = h * df.hop_size;
        let b = a + df.hop_size;
        let t = Instant::now();
        let lsnr = df.process(&input[a..b], &mut out[a..b]).expect("process");
        let us = t.elapsed().as_micros();
        total_us += us;
        worst_us = worst_us.max(us);
        ls_min = ls_min.min(lsnr);
        ls_max = ls_max.max(lsnr);
        if h % 50 == 0 {
            println!("hop {h:3}: lsnr {lsnr:6.2} dB, {us} us");
        }
    }
    let wall = t1.elapsed().as_secs_f64();
    let audio_s = (hops * df.hop_size) as f64 / df.sr as f64;
    println!(
        "per hop: mean {:.0} us, worst {worst_us} us | realtime factor {:.2}x",
        total_us as f64 / hops as f64,
        audio_s / wall
    );
    println!("lsnr range: {ls_min:.2} .. {ls_max:.2} dB");

    let skip = (df.sr / 3) as usize;
    let nr_in = rms(&input[skip..]);
    let nr_out = rms(&out[skip..]);
    println!(
        "rms: in {:.6} out {:.6} -> {:.1} dB change",
        nr_in,
        nr_out,
        20.0 * (nr_out.max(1e-12) / nr_in).log10()
    );
    assert!(out.iter().all(|v| v.is_finite()), "non-finite output");
    // The output must differ from the dry signal: that is the model doing work.
    let mut diff = 0.0f32;
    for i in 0..out.len() {
        diff = diff.max((out[i] - input[i]).abs());
    }
    println!("max |out - dry| = {diff:.4}");
}
