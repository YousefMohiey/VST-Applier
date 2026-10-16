//! 90-second soak: the AI Denoise VST2 plugin as the only chain item on
//! the real capture path, status printed every 2 s (watch q + dropouts).
//!
//!   cargo run --release -p audio-engine --example chain_applier

use audio_engine::{ChainPlugin, start};
use vst_host_sys::PluginFormat;

fn main() {
    let dll = std::path::PathBuf::from(
        r"C:\Program Files\VST-Applier\Plugins\VST-Applier\AI Denoise.dll",
    );
    let mut p = ChainPlugin {
        name: "AI Denoise".into(),
        path: dll.clone(),
        format: PluginFormat::Vst2,
        enabled: true,
        host: None,
        saved_state: None,
    };
    let host_dll = std::path::PathBuf::from("C:/Users/Administrator/vst-applier-rs/target/release")
        .join(PluginFormat::Vst2.dll_name());
    p.attach(&host_dll).expect("attach AI Denoise");
    println!("attached via {}", host_dll.display());

    let mut route = start(
        "Headset Microphone (Maonocaster G1 Neo)",
        "CABLE Input (VB-Audio Virtual Cable)",
        vec![p],
        std::env::var("VA_BLOCK")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(512),
        None,
    )
    .expect("route start");
    println!("route started");

    let t0 = std::time::Instant::now();
    let mut worst_xr: u64 = 0;
    let secs: u64 = std::env::var("VA_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    while t0.elapsed().as_secs() < secs {
        std::thread::sleep(std::time::Duration::from_millis(2000));
        route.poll_preload();
        let line = route.status_line();
        let blk_ms = route.diagnostics().last_capture_block_ms;
        let c = cpal::dbg_iters();
        println!(
            "[{}s] {line} | iters: cpal[in={} pkt={} out={} pout={}] ll={}",
            t0.elapsed().as_secs(),
            c[0],
            c[1],
            c[2],
            c[3],
            audio_engine::ll_iters()
        );
        println!("    last capture block: {blk_ms:.2} ms");
        let (xr, _) = route.dropouts();
        worst_xr = worst_xr.max(xr);
    }
    let (xr, trim) = route.dropouts();
    println!("SOAK DONE: dropouts={xr} trim={trim}");
}
