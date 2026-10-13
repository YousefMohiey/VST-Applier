//! Cable audibility probe: play a loud 440 Hz sine into a render endpoint
//! for 3 s while capturing a capture endpoint, report the captured peak.
//! Usage: cable_probe <render-name-substring> <capture-name-substring>

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let render_want = args.get(1).map(|s| s.as_str()).unwrap_or("CABLE Input (VB");
    let capture_want = args.get(2).map(|s| s.as_str()).unwrap_or("CABLE Output");
    let host = cpal::default_host();

    // Capture first.
    let in_dev = host
        .input_devices()
        .unwrap()
        .find(|d| d.name().unwrap_or_default().contains(capture_want))
        .expect("capture device");
    let in_cfg: cpal::StreamConfig = in_dev.default_input_config().unwrap().into();
    println!(
        "capture: {:?} {}Hzx{}",
        in_dev.name().unwrap(),
        in_cfg.sample_rate.0,
        in_cfg.channels
    );
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let peak_cb = peak.clone();
    let ch = in_cfg.channels as usize;
    let in_stream = in_dev
        .build_input_stream(
            &in_cfg,
            move |data: &[f32], _| {
                let mut m = 0.0f32;
                for chunk in data.chunks(ch.max(1)) {
                    m = m.max(chunk[0].abs());
                }
                peak_cb.store(m.to_bits(), std::sync::atomic::Ordering::Relaxed);
            },
            |e| eprintln!("in err: {e}"),
            None,
        )
        .unwrap();
    in_stream.play().unwrap();

    // Then blast sine.
    let out_dev = host
        .output_devices()
        .unwrap()
        .find(|d| d.name().unwrap_or_default().contains(render_want))
        .expect("render device");
    let out_cfg: cpal::StreamConfig = out_dev.default_output_config().unwrap().into();
    println!(
        "render: {:?} {}Hzx{}",
        out_dev.name().unwrap(),
        out_cfg.sample_rate.0,
        out_cfg.channels
    );
    let och = out_cfg.channels as usize;
    let orate = out_cfg.sample_rate.0 as f32;
    let mut phase = 0.0f32;
    let out_stream = out_dev
        .build_output_stream(
            &out_cfg,
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(och.max(1)) {
                    let s = (phase * 2.0 * std::f32::consts::PI / orate).sin() * 0.9;
                    phase = (phase + 440.0) % orate;
                    for x in frame.iter_mut() {
                        *x = s;
                    }
                }
            },
            |e| eprintln!("out err: {e}"),
            None,
        )
        .unwrap();
    out_stream.play().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(3000));
    let p = f32::from_bits(peak.load(std::sync::atomic::Ordering::Relaxed));
    println!("captured peak after 3 s of 0.9 sine: {p:.3}");
}
