//! Times the AI Denoise plugin per 512-frame stereo chunk through the
//! host, at both 44.1 kHz (internal resampler path) and 48 kHz.

use std::time::Instant;

fn bench(rate: f64, root: &std::path::Path) -> f64 {
    let mut host = vst_host_sys::PluginHost::load(
        &root.join("Vst2HostNative.dll"),
        vst_host_sys::PluginFormat::Vst2,
        &root.join("VSTApplierDenoise.dll"),
    )
    .expect("load");
    host.setup_processing(rate, 512, 2).expect("setup");
    let mut input = vec![0.0f32; 512 * 2];
    let mut seed = 0x1234_5678u32;
    for v in input.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (seed as f32 / u32::MAX as f32 - 0.5) * 0.1;
    }
    let mut output = vec![0.0f32; 512 * 2];
    for _ in 0..20 {
        host.process_f32(&input, &mut output).unwrap();
    }
    let n = 500;
    let t = Instant::now();
    for _ in 0..n {
        host.process_f32(&input, &mut output).unwrap();
    }
    let us = t.elapsed().as_micros() as f64 / n as f64;
    println!(
        "{rate:.0} Hz NOISE: {us:.0} us per 512-frame chunk ({:.1}% of the 11.6 ms budget)",
        us / 11_610.0 * 100.0
    );

    // Denormal test: the real mic in a quiet room is silence (in_peak
    // ~0.000). Feed zeros and a decaying probe; if this explodes, the
    // denoiser's recurrent state is decaying into denormal floats and
    // every MAC pays the x86 denormal penalty.
    let mut quiet = vec![0.0f32; 512 * 2];
    let t = Instant::now();
    for i in 0..n {
        if i % 50 == 0 {
            quiet[0] = 1e-8; // a whisper every ~0.5 s, decaying fast
        } else {
            quiet[0] *= 1e-4;
        }
        host.process_f32(&quiet, &mut output).unwrap();
    }
    let us_q = t.elapsed().as_micros() as f64 / n as f64;
    println!(
        "{rate:.0} Hz SILENCE: {us_q:.0} us per 512-frame chunk ({:.1}% of the 11.6 ms budget)",
        us_q / 11_610.0 * 100.0
    );

    // Level bisect: tiny but NORMAL floats vs tiny denormals vs zeros.
    for (label, level) in [
        ("1e-6-normal", 1e-6f32),
        ("1e-30-denormal", 1e-30f32),
        ("zeros", 0.0f32),
    ] {
        let buf = vec![level; 512 * 2];
        let t = Instant::now();
        for _ in 0..n {
            host.process_f32(&buf, &mut output).unwrap();
        }
        let us_l = t.elapsed().as_micros() as f64 / n as f64;
        println!("{rate:.0} Hz {label}: {us_l:.0} us per 512-frame chunk");
    }
    us_q
}

fn main() {
    // Same FTZ/DAZ the engine sets on its audio threads — the bench
    // should mirror the real thread environment to be meaningful.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
    let root = std::path::PathBuf::from("C:/Users/Administrator/vst-applier-rs/target/release");
    let _ = bench(48_000.0, &root);
    let _ = bench(44_100.0, &root);
}
