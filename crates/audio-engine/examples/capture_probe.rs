//! Capture-only probe: open JUST the mic via cpal (no engine, no lowlat
//! render, no plugins) and count frames. If this wedges/spins on its own,
//! the cpal capture path is the whole story; if it stays clean, the
//! interaction with our render side is involved.
//!
//!   cargo run --release -p audio-engine --example capture_probe
//!   VA_PROBE_SECS=240 cargo run ...   (run time, default 180)

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn main() {
    let secs: u64 = std::env::var("VA_PROBE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);
    let host = cpal::default_host();
    let dev = host
        .input_devices()
        .unwrap()
        .find(|d| {
            d.name()
                .ok()
                .as_deref()
                .map(|n| n.contains("Maonocaster"))
                .unwrap_or(false)
        })
        .expect("mic not found");
    let cfg = dev.default_input_config().expect("config");
    println!(
        "capture: {} @ {} Hz, {} ch, {:?}",
        dev.name().unwrap(),
        cfg.sample_rate().0,
        cfg.channels(),
        cfg.sample_format()
    );

    let frames = Arc::new(AtomicU64::new(0));
    let blocks = Arc::new(AtomicU64::new(0));
    let f = frames.clone();
    let b = blocks.clone();
    let nch = cfg.channels();
    let stream_cfg: cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            &stream_cfg,
            move |data: &[f32], _| {
                let n = data.len() as u64 / nch as u64;
                f.fetch_add(n, Ordering::Relaxed);
                b.fetch_add(1, Ordering::Relaxed);
            },
            |e| eprintln!("stream error: {e}"),
            None,
        ),
        other => panic!("format {other:?} not handled by probe"),
    }
    .expect("build stream");
    stream.play().expect("play");

    // Optional: also open a RENDER stream (silence) to the device named
    // by VA_PROBE_OUT — isolates "mic+cable in one process" from the
    // engine entirely.
    let _out_stream = if let Ok(want) = std::env::var("VA_PROBE_OUT") {
        let dev = host
            .output_devices()
            .unwrap()
            .find(|d| {
                d.name()
                    .ok()
                    .as_deref()
                    .map(|n| n.contains(&want))
                    .unwrap_or(false)
            })
            .expect("output device not found");
        let ocfg = dev.default_output_config().expect("out config");
        let och = ocfg.channels();
        let ostream = match ocfg.sample_format() {
            cpal::SampleFormat::F32 => dev
                .build_output_stream(
                    &ocfg.clone().into(),
                    move |data: &mut [f32], _| {
                        let _ = och;
                        for s in data.iter_mut() {
                            *s = 0.0;
                        }
                    },
                    |e| eprintln!("output stream error: {e}"),
                    None,
                )
                .expect("build output stream"),
            other => panic!("output format {other:?} not handled"),
        };
        ostream.play().expect("out play");
        println!("render stream open -> {want} (silence)");
        Some(ostream)
    } else {
        None
    };

    let t0 = std::time::Instant::now();
    let mut last = (0u64, 0u64, std::time::Instant::now());
    while t0.elapsed().as_secs() < secs {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let (fr, bl) = (frames.load(Ordering::Relaxed), blocks.load(Ordering::Relaxed));
        let dt = last.2.elapsed().as_secs_f64();
        let rate = (fr - last.0) as f64 / dt;
        let bps = (bl - last.1) as f64 / dt;
        println!(
            "[{}s] total {fr} frames / {bl} blocks | last window: {rate:.0} frames/s, {bps:.1} blocks/s",
            t0.elapsed().as_secs()
        );
        last = (fr, bl, std::time::Instant::now());
    }
    let (fr, bl) = (frames.load(Ordering::Relaxed), blocks.load(Ordering::Relaxed));
    println!(
        "PROBE DONE: {fr} frames in {secs}s = {:.1} frames/s avg",
        fr as f64 / secs as f64
    );
    drop(stream);
    println!("stream dropped cleanly");
}
