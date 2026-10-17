//! Diagnostic probe: load a VST3 through the native host and report the
//! EXACT stage that fails at each setup_processing attempt.
//!
//! Usage (from the repo root):
//!   cargo run -p audio-engine --example vst3_probe -- "<plugin.vst3 path>"
//!
//! Prints, for stereo and mono attempts, the host's detailed last-error
//! (the app currently replaces it with a generic message, which is how
//! "setup_processing failed" arrives with no clue about the cause).

use std::path::PathBuf;

use vst_host_sys::{PluginFormat, PluginHost};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let plugin = PathBuf::from(
        args.next()
            .expect("usage: vst3_probe <plugin.vst3 path>"),
    );
    let host_dll = std::env::var("VST3_HOST_DLL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/release/Vst3HostNative.dll"));

    println!("host dll : {}", host_dll.display());
    println!("plugin   : {}", plugin.display());

    let mut host = match PluginHost::load(&host_dll, PluginFormat::Vst3, &plugin) {
        Ok(h) => h,
        Err(e) => {
            println!("LOAD FAILED: {e}");
            std::process::exit(2);
        }
    };
    println!("load     : OK");

    let mut configured = 0;
    for ch in [2, 1] {
        match host.setup_processing(48000.0, 512, ch) {
            Ok(()) => {
                println!("setup {ch}ch : OK");
                configured = ch;
                break;
            }
            Err(e) => println!("setup {ch}ch : FAILED -> {e}"),
        }
    }
    if configured == 0 {
        println!("RESULT: plugin cannot be configured at all");
        std::process::exit(3);
    }

    // Run a few blocks to prove the process path end-to-end.
    let frames = 512usize;
    let mut input = vec![0.0f32; frames * host.channels as usize];
    let mut output = vec![0.0f32; frames * host.channels as usize];
    for block in 0..3 {
        match host.process_f32(&input, &mut output) {
            Ok(()) => {
                let peak = output.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                println!("process #{block} : OK (peak {peak:.4})");
            }
            Err(e) => {
                println!("process #{block} : FAILED -> {e}");
                std::process::exit(4);
            }
        }
    }
    println!("RESULT: configured {configured}ch, 3 blocks processed clean");
}
