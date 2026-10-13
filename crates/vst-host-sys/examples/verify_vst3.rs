fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: verify_vst3 <plugin-file-or-bundle>");
    let p = std::path::PathBuf::from(&path);
    // Bundle dir -> resolve binary like the scanner; loose file -> use it.
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
    println!("binary: {}", binary.display());
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut host = vst_host_sys::PluginHost::load(
        &exe.join("Vst3HostNative.dll"),
        vst_host_sys::PluginFormat::Vst3,
        &binary,
    )
    .expect("load plugin");
    println!("loaded OK");
    host.setup_processing(48000.0, 512, 2).expect("setup");
    println!("setup OK");
    // 440Hz tone + noise: a denoiser/gate should pass tone, cut noise floor.
    let mut seed = 0x12345678u32;
    let mut input = vec![0.0f32; 512 * 2];
    let mut output = vec![0.0f32; 512 * 2];
    for block in 0..20 {
        for (i, s) in input.iter_mut().enumerate() {
            let t = (block * 512 + i / 2) as f32 / 48000.0;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let nz = ((seed >> 8) as f32 / 8388608.0) * 0.05;
            *s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.4 + nz;
        }
        host.process_f32(&input, &mut output).expect("process");
        if block % 5 == 4 {
            let peak = output.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            println!("block {block}: out_peak={peak:.4}");
        }
    }
    match host.save_state().expect("save_state") {
        Some(blob) => println!("state: {} bytes", blob.len()),
        None => println!("state: none"),
    }
    println!("ALL OK");
}
