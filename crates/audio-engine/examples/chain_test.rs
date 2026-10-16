//! Full-loop self verification: 440 Hz tone -> CABLE Input (via a cpal
//! render stream) -> CABLE Output capture -> [ReaEQ, ReaGate, rnnoise] ->
//! Speakers. Prints peaks; proves the whole VST chain processes live audio.

use audio_engine::{ChainPlugin, start};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vst_host_sys::PluginFormat;

fn main() {
    let host = cpal::default_host();
    let cable_in = "CABLE Input (VB-Audio Virtual Cable)";
    let cable_out = "CABLE Output (VB-Audio Virtual Cable)";
    let speakers = "Speakers (HyperX Cloud III S Wireless)";

    // Tone generator into the cable.
    let out_dev = host
        .output_devices()
        .unwrap()
        .find(|d| d.name().ok().as_deref() == Some(cable_in))
        .expect("cable input");
    let cfg: cpal::StreamConfig = out_dev.default_output_config().unwrap().into();
    let rate = cfg.sample_rate.0 as f32;
    let ch = cfg.channels as usize;
    let mut phase = 0.0f32;
    let tone = out_dev
        .build_output_stream(
            &cfg,
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(ch) {
                    let s = (phase * 2.0 * std::f32::consts::PI / rate).sin() * 0.4;
                    phase = (phase + 440.0) % rate;
                    for x in frame.iter_mut() {
                        *x = s;
                    }
                }
            },
            |e| eprintln!("tone err: {e}"),
            None,
        )
        .unwrap();
    tone.play().unwrap();
    println!("tone playing into '{cable_in}'");

    // Chain with the real Main-profile plugins.
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let plug = |name: &str, file: &str, format: PluginFormat| {
        let mut p = ChainPlugin {
            name: name.into(),
            path: std::path::PathBuf::from(file),
            format,
            enabled: true,
            host: None,
            saved_state: None,
        };
        let dll = exe.join(format.dll_name());
        p.attach(&dll).expect("attach");
        p
    };
    let base = r"C:\Users\Administrator\AppData\Local\hermes\cache\scratch\VST-Applier\defaults\Plugins";
    let chain = vec![
        plug("reaeq", &format!("{base}\\ReaPlugs\\reaeq-standalone.dll"), PluginFormat::Vst2),
        plug("reagate", &format!("{base}\\ReaPlugs\\reagate-standalone.dll"), PluginFormat::Vst2),
        plug("rnnoise", &format!("{base}\\rnnoise\\vst\\rnnoise_stereo.dll"), PluginFormat::Vst2),
    ];
    println!("3 plugins attached");

    let mut route = start(cable_out, speakers, chain, 512, None).expect("route start");
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        route.poll_preload();
        println!(
            "{} | in={:.3} out={:.3}",
            route.status_line(),
            route.meters().input(),
            route.meters().output()
        );
    }
}
