//! Proof for the editor-stall fix: hold the chain lock for 250 ms while
//! the route runs (what a slow plugin editor open does to the audio
//! thread) and verify the stream never stops and nothing is counted as
//! a dropout.
use audio_engine::{start, ChainPlugin};
use std::time::Duration;
use vst_host_sys::PluginFormat;

fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut p = ChainPlugin {
        name: "reaeq".into(),
        path: std::path::PathBuf::from(
            r"C:\Program Files\VST-Applier\Plugins\ReaPlugs\reaeq-standalone.dll",
        ),
        format: PluginFormat::Vst2,
        enabled: true,
        host: None,
        saved_state: None,
    };
    p.attach(&exe.join(PluginFormat::Vst2.dll_name())).expect("attach");

    let mut route = start(
        "Headset Microphone (Maonocaster G1 Neo)",
        "CABLE Input (VB-Audio Virtual Cable)",
        vec![p],
        512,
        None,
    )
    .expect("route start");
    println!("route started");
    for i in 0..4 {
        std::thread::sleep(Duration::from_millis(500));
        route.poll_preload();
        let (xr, trims) = route.dropouts();
        println!("[{i}] {} | xr={xr} trim={trims}", route.status_line());
    }

    let (xr_before, _) = route.dropouts();
    let frames_before = route.output_frames();
    println!("simulating a 250 ms editor stall (chain lock held)…");
    let _ = route.with_plugin(0, |_| std::thread::sleep(Duration::from_millis(250)));
    std::thread::sleep(Duration::from_millis(1000));

    let (xr_after, _) = route.dropouts();
    let frames_after = route.output_frames();
    let advanced_ms = (frames_after - frames_before) as f64 / 48000.0 / 2.0 * 1000.0;
    println!(
        "stall result: new dropouts={} | audio advanced {:.0} ms during stall+1s",
        xr_after - xr_before,
        advanced_ms
    );
    if xr_after == xr_before && advanced_ms > 1000.0 {
        println!("== PASS: editor stall caused no dropouts; audio never stopped");
    } else {
        println!("== FAIL");
        std::process::exit(1);
    }
}
