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
        "{rate:.0} Hz: {us:.0} us per 512-frame chunk ({:.1}% of the 11.6 ms budget)",
        us / 11_610.0 * 100.0
    );
    us
}

fn main() {
    let root = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\release");
    let _ = bench(48_000.0, &root);
    let _ = bench(44_100.0, &root);
}
