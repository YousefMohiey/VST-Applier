//! Live chain-edit verification: run a route and edit the chain WHILE
//! audio flows — push, toggle, swap, remove, clear. These are exactly
//! the operations the UI now performs on a live route (previously each
//! one tore the whole route down and rebuilt it, freezing the window
//! and crashing reload-sensitive plugins).
//!
//! Route: CABLE Output (loopback capture) -> [rnnoise] -> Speakers.
//! No tone is generated, so silence flows; the point is that the route
//! keeps running `running` through every live edit and never panics.

use audio_engine::{ChainPlugin, start};
use vst_host_sys::PluginFormat;

fn attach(name: &str, path: &str, format: PluginFormat) -> ChainPlugin {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut p = ChainPlugin {
        name: name.into(),
        path: std::path::PathBuf::from(path),
        format,
        enabled: true,
        host: None,
        saved_state: None,
    };
    let dll = exe.join(format.dll_name());
    p.attach(&dll).expect("attach");
    p
}

fn main() {
    let base = r"C:\Program Files\VST-Applier\Plugins";
    let cable_out = "CABLE Output (VB-Audio Virtual Cable)";
    let speakers = "Speakers (HyperX Cloud III S Wireless)";

    println!("== live chain-edit test ==");
    let chain = vec![attach(
        "rnnoise",
        &format!("{base}\\WermanRNNoise\\rnnoise_stereo.dll"),
        PluginFormat::Vst2,
    )];
    let mut route = start(cable_out, speakers, chain, 512).expect("route start");
    println!("[start] {}", route.status_line());

    let nap = |steps: usize, route: &mut audio_engine::RunningRoute, tag: &str| {
        for _ in 0..steps * 2 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            route.poll_preload();
        }
        println!("[{tag}] {}", route.status_line());
    };

    nap(2, &mut route, "1 plugin, 2s");

    // Push a VST2 live.
    let eq = attach(
        "reaeq",
        &format!("{base}\\ReaPlugs\\reaeq-standalone.dll"),
        PluginFormat::Vst2,
    );
    let idx = route.push_plugin(eq).expect("push vst2");
    println!("[push vst2] slot {idx} inserted");
    nap(2, &mut route, "2 plugins, 2s");

    // Push a VST3 live.
    let bertom = attach(
        "bertom",
        &format!("{base}\\BertomDenoiser\\Bertom Denoiser_x64.vst3"),
        PluginFormat::Vst3,
    );
    let idx = route.push_plugin(bertom).expect("push vst3");
    println!("[push vst3] slot {idx} inserted");
    nap(2, &mut route, "3 plugins, 2s");

    // Toggle slot 0 off and on.
    route.set_enabled(0, false);
    nap(1, &mut route, "slot 0 disabled, 1s");
    route.set_enabled(0, true);
    nap(1, &mut route, "slot 0 re-enabled, 1s");

    // Swap slots 0 and 2.
    route.swap_plugins(0, 2);
    nap(1, &mut route, "swap 0<->2, 1s");

    // Remove a slot and drop its host off-lock.
    let popped = route.remove_plugin(0);
    drop(popped);
    nap(1, &mut route, "slot 0 removed, 1s");

    // Clear everything; the route keeps flowing (passthrough).
    let drained = route.clear_plugins();
    drop(drained);
    nap(1, &mut route, "chain cleared, 1s");

    let final_line = route.status_line();
    println!("[final] {final_line}");
    assert!(
        final_line.contains("running"),
        "route stopped running after live edits: {final_line}"
    );
    println!("== PASS: live chain edits survived ==");
}
