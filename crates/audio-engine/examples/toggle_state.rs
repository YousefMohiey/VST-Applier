//! Proves enable/disable no longer wipes settings: start a route with
//! [reaeq enabled, reagate DISABLED carrying a fake state], stop,
//! and check the harvested blobs — slot 1 must come back intact.
use audio_engine::{ChainPlugin, start};
use vst_host_sys::PluginFormat;

fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let base = r"C:\Program Files\VST-Applier\Plugins\ReaPlugs";
    // Mirror try_start: only the enabled plugin attaches a host; the
    // disabled slot carries its state unattached.
    let mut p0 = ChainPlugin {
        name: "reaeq".into(),
        path: std::path::PathBuf::from(format!("{base}\\reaeq-standalone.dll")),
        format: PluginFormat::Vst2,
        enabled: true,
        host: None,
        saved_state: None,
    };
    p0.attach(&exe.join(PluginFormat::Vst2.dll_name())).expect("attach");
    let fake = vec![0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4];
    let p1 = ChainPlugin {
        name: "reagate".into(),
        path: std::path::PathBuf::from(format!("{base}\\reagate-standalone.dll")),
        format: PluginFormat::Vst2,
        enabled: false,
        host: None,
        saved_state: Some(fake.clone()),
    };
    let route = start(
        "Headset Microphone (Maonocaster G1 Neo)",
        "CABLE Input (VB-Audio Virtual Cable)",
        vec![p0, p1],
        512,
    )
    .expect("route start");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let blobs = route.stop();
    assert_eq!(blobs.len(), 2, "two slots harvested");
    assert!(blobs[0].as_ref().map(|b| !b.is_empty()).unwrap_or(false), "live plugin state harvested");
    assert_eq!(blobs[1], Some(fake), "disabled plugin state must survive, got {:?}", blobs[1].as_ref().map(|b| b.len()));
    println!("TOGGLE-STATE OK: disabled slot preserved");
}
