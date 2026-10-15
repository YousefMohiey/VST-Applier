//! Repro for the Bertom crash: real engine path (mic -> [Bertom VST3,
//! mono bus] -> CABLE Input), 6 seconds. Prints peaks; crashes where
//! the app crashes.
use audio_engine::{ChainPlugin, start};
use vst_host_sys::PluginFormat;

fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut p = ChainPlugin {
        name: "bertom".into(),
        path: std::path::PathBuf::from(
            r"C:\Program Files\VST-Applier\Plugins\BertomDenoiser\Bertom Denoiser_x64.vst3",
        ),
        format: PluginFormat::Vst3,
        enabled: true,
        host: None,
        saved_state: None,
    };
    p.attach(&exe.join(PluginFormat::Vst3.dll_name())).expect("attach");
    println!("attached");
    let mut route = start(
        "Headset Microphone (Maonocaster G1 Neo)",
        "CABLE Input (VB-Audio Virtual Cable)",
        vec![p],
        std::env::var("VA_BLOCK")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(512),
    )
    .expect("route start");
    println!("route started");
    for i in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        route.poll_preload();
        println!(
            "[{i}] {} | in={:.3} out={:.3}",
            route.status_line(),
            route.meters().input(),
            route.meters().output()
        );
    }
    println!("SURVIVED");
}
