fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let host_dll = exe.join("Vst2HostNative.dll");
    let plugin = std::path::PathBuf::from(
        r"C:\Program Files\VST-Applier\Plugins\TDRNova\TDR Nova.dll",
    );
    println!("host: {}", host_dll.display());
    println!("plugin: {}", plugin.display());
    let mut host = vst_host_sys::PluginHost::load(
        &host_dll,
        vst_host_sys::PluginFormat::Vst2,
        &plugin,
    )
    .expect("load plugin");
    println!("loaded OK");
    host.setup_processing(48000.0, 512, 2).expect("setup");
    println!("setup OK");
    // 512 frames stereo: impulse in, capture out.
    let mut input = vec![0.0f32; 512 * 2];
    input[0] = 1.0;
    input[1] = 1.0;
    let mut output = vec![0.0f32; 512 * 2];
    host.process_f32(&input, &mut output).expect("process");
    let peak = output.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    println!("processed OK, out_peak={peak:.4}");
    match host.save_state().expect("save_state") {
        Some(blob) => println!("state: {} bytes, b64 len {}", blob.len(), vst_host_sys::encode_state_field(Some(&blob)).unwrap().len()),
        None => println!("state: none"),
    }
    println!("editor size: {:?}", host.editor_size());
    println!("ALL OK");
}
