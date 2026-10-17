//! Reload-cycle repro: attach/drop the Yousef Noise Suppression VST2
//! plugin repeatedly in ONE process — the exact sequence the app hits on
//! stop→start with the plugin in the chain (crashed on the 2nd attach).
//!
//!   VA_CYCLES=6 cargo run --release -p audio-engine --example reload_cycle

use audio_engine::ChainPlugin;
use vst_host_sys::PluginFormat;

fn main() {
    let dll = std::path::PathBuf::from(
        r"C:\Program Files\VST-Applier\Plugins\VST-Applier\Yousef Noise Suppression.dll",
    );
    let host_dll = std::path::PathBuf::from("C:/Users/Administrator/vst-applier-rs/target/release")
        .join(PluginFormat::Vst2.dll_name());
    let cycles: u32 = std::env::var("VA_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    println!("plugin: {}", dll.display());
    println!("host:   {}", host_dll.display());
    for i in 0..cycles {
        let t0 = std::time::Instant::now();
        let mut p = ChainPlugin {
            name: "Yousef Noise Suppression".into(),
            path: dll.clone(),
            format: PluginFormat::Vst2,
            enabled: true,
            host: None,
            saved_state: None,
        };
        match p.attach(&host_dll) {
            Ok(()) => println!(
                "cycle {i}: attached ok ({} ms)",
                t0.elapsed().as_millis()
            ),
            Err(e) => {
                println!("cycle {i}: attach failed: {e} — stopping");
                return;
            }
        }
        drop(p);
        println!("cycle {i}: dropped ok ({} ms)", t0.elapsed().as_millis());
    }
    println!("ALL {cycles} CYCLES OK");
}
