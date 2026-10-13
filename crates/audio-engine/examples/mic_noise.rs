//! Noise-floor probe: capture a device for 3 s, report the max peak with
//! full precision plus how many blocks contained ANY nonzero sample.
//! A live mic always shows preamp noise (>0). Exact 0.0 across 3 s means
//! the stream is digitally muted upstream of us.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

fn main() {
    let want = std::env::args().nth(1).unwrap_or("Maonocaster".into());
    let host = cpal::default_host();
    let dev = host
        .input_devices()
        .unwrap()
        .find(|d| d.name().unwrap_or_default().contains(want.as_str()))
        .expect("device");
    let cfg: cpal::StreamConfig = dev.default_input_config().unwrap().into();
    println!("capture: {:?} {}Hzx{}", dev.name().unwrap(), cfg.sample_rate.0, cfg.channels);
    let peak = std::sync::Arc::new(AtomicU32::new(0));
    let nonzero_blocks = std::sync::Arc::new(AtomicU64::new(0));
    let blocks = std::sync::Arc::new(AtomicU64::new(0));
    let ch = cfg.channels.max(1) as usize;
    let peak_c = peak.clone();
    let nz_c = nonzero_blocks.clone();
    let blk_c = blocks.clone();
    let stream = dev
        .build_input_stream(
            &cfg,
            move |data: &[f32], _| {
                blk_c.fetch_add(1, Ordering::Relaxed);
                let mut m = 0.0f32;
                for s in data.iter() {
                    m = m.max(s.abs());
                }
                if m > 0.0 {
                    nz_c.fetch_add(1, Ordering::Relaxed);
                }
                let cur = f32::from_bits(peak_c.load(Ordering::Relaxed));
                if m > cur {
                    peak_c.store(m.to_bits(), Ordering::Relaxed);
                }
                let _ = ch;
            },
            |e| eprintln!("err: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3000));
    println!(
        "blocks={} nonzero_blocks={} max_peak={:.9}",
        blocks.load(Ordering::Relaxed),
        nonzero_blocks.load(Ordering::Relaxed),
        f32::from_bits(peak.load(Ordering::Relaxed))
    );
}
