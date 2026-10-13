//! Measures per-block CPU cost of a plugin: 100 process() calls on
//! 512-frame stereo blocks, reports mean/max ms. Budget is ~10 ms
//! (one cpal period); anything near it will glitch under load.
fn main() {
    let path = std::env::args().nth(1).expect("usage: bench_vst3 <file-or-bundle>");
    let stereo = std::env::args().nth(2).unwrap_or_default() != "mono";
    let p = std::path::PathBuf::from(&path);
    let binary = if p.is_dir() {
        ["x86_64-win", "x86-win", "arm64-win"]
            .into_iter()
            .filter_map(|arch| std::fs::read_dir(p.join("Contents").join(arch)).ok())
            .flat_map(|r| r.flatten())
            .map(|e| e.path())
            .filter(|q| q.is_file())
            .next()
            .expect("bundle binary")
    } else {
        p
    };
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut host = vst_host_sys::PluginHost::load(
        &exe.join("Vst3HostNative.dll"),
        vst_host_sys::PluginFormat::Vst3,
        &binary,
    )
    .expect("load");
    let ch = if stereo { 2 } else { 1 };
    host.setup_processing(48000.0, 512, ch).expect("setup");
    let n = 512 * ch as usize;
    let input = vec![0.3f32; n];
    let mut output = vec![0.0f32; n];
    host.process_f32(&input, &mut output).expect("warmup");
    let t0 = std::time::Instant::now();
    let mut worst = 0u128;
    for _ in 0..100 {
        let t = std::time::Instant::now();
        host.process_f32(&input, &mut output).expect("process");
        worst = worst.max(t.elapsed().as_micros());
    }
    let mean = t0.elapsed().as_micros() as f64 / 100.0 / 1000.0;
    println!("mean={mean:.2}ms worst={:.2}ms", worst as f64 / 1000.0);
}
