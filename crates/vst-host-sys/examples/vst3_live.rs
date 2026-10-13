fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let host_dll = exe.join("Vst3HostNative.dll");
    let bundle = std::path::PathBuf::from(
        r"C:\Users\Administrator\AppData\Local\hermes\cache\scratch\VST-Applier\defaults\Plugins\rnnoise\rnnoise.vst3",
    );
    // Resolve the bundle binary the same way the scanner does.
    let binary = ["x86_64-win", "x86-win", "arm64-win"]
        .into_iter()
        .filter_map(|arch| std::fs::read_dir(bundle.join("Contents").join(arch)).ok())
        .flat_map(|r| r.flatten())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .next()
        .expect("bundle binary");
    println!("binary: {}", binary.display());
    let mut host = vst_host_sys::PluginHost::load(
        &host_dll,
        vst_host_sys::PluginFormat::Vst3,
        &binary,
    )
    .expect("load plugin");
    println!("loaded OK");
    host.setup_processing(48000.0, 512, 2).expect("setup");
    println!("setup OK");
    // Feed pseudo-random noise: a suppressor should output *something*.
    let mut seed = 0x12345678u32;
    let mut input = vec![0.0f32; 512 * 2];
    for s in input.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *s = ((seed >> 8) as f32 / 8388608.0) * 0.5;
    }
    let mut output = vec![0.0f32; 512 * 2];
    for block in 0..20 {
        // Fresh noise each block (advance the LCG).
        for s in input.iter_mut() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *s = ((seed >> 8) as f32 / 8388608.0) * 0.5;
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
