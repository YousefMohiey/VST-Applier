//! Prints the built-in denoiser's numbers: model load time, hop, the
//! inherent latency, measured steady-noise suppression, and the
//! per-hop processing cost as a share of the real-time budget.
//!
//!   cargo run -p ai-denoise --release --example probe

use ai_denoise::Denoiser;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn main() {
    let t0 = std::time::Instant::now();
    let mut d = Denoiser::new().expect("model");
    println!("model loaded in {} ms", t0.elapsed().as_millis());
    println!(
        "rate {} Hz, hop {} ({:.1} ms), latency {} samples ({:.1} ms)",
        d.sample_rate(),
        d.hop(),
        d.hop() as f64 / d.sample_rate() as f64 * 1000.0,
        d.latency_samples(),
        d.latency_ms(),
    );

    // Steady noise floor: 6 s of deterministic white noise.
    let n = 48_000 * 6;
    let mut s = 0x1234_5678u32;
    let input: Vec<f32> = (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 - 0.5) * 0.06
        })
        .collect();
    let mut out = vec![0.0f32; n];
    d.process(&input, &mut out);
    let (a, b) = (48_000 * 4, n);
    let (ni, no) = (rms(&input[a..b]), rms(&out[a..b]));
    println!(
        "steady noise: in {:.1} dBFS, out {:.1} dBFS -> {:.1} dB suppression",
        20.0 * ni.log10(),
        20.0 * no.max(1e-9).log10(),
        20.0 * (ni / no.max(1e-9)).log10(),
    );

    // Per-hop cost against the 10 ms real-time budget.
    let warm = vec![0.0f32; 480];
    let mut buf = vec![0.0f32; 480];
    for _ in 0..20 {
        d.process(&warm, &mut buf);
    }
    let t1 = std::time::Instant::now();
    let blocks = 300;
    for _ in 0..blocks {
        d.process(&warm, &mut buf);
    }
    let us = t1.elapsed().as_micros() as f64 / blocks as f64;
    println!(
        "process: {:.0} us per 480-frame hop ({:.1}% of the 10 ms budget)",
        us,
        us / 10_000.0 * 100.0
    );
}
