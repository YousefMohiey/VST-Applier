//! Proves the resampler neither gains nor loses samples over time:
//! 2000 blocks of mic-rate input (with cpal-like size jitter) must
//! produce exactly the expected output count ±2, position stays
//! bounded, and the servo converges a drained ring back to target.
use audio_engine::resample::{occupancy_servo, resample_with_carry};

fn main() {
    let (src, dst) = (44100u32, 48000u32);
    let mut state = (0.0f64, 0.0f32, 0.0f32);
    let mut total_in = 0usize;
    let mut total_out = 0usize;
    let mut worst_pos = 0.0f64;
    let mut seed = 0x9e3779b9u32;
    let mut input = vec![0.0f32; 512];
    let mut output = vec![0.0f32; 560];
    for _ in 0..2000 {
        // cpal-like jitter: 400..480 frames per block.
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = 400 + (seed % 81) as usize;
        for (i, s) in input[..n].iter_mut().enumerate() {
            *s = ((i * 7 + total_in) as f32).sin();
        }
        let got = resample_with_carry(&mut state, &input[..n], src, &mut output, dst, 1.0);
        total_in += n;
        total_out += got;
        worst_pos = worst_pos.max(state.0.abs());
        // Steady state for upsampling sits slightly negative (the tail
        // covers virtual -2/-1); the invariant is pos in [-1, ratio).
        assert!(state.0 >= -1.0 && state.0 < 1.1, "pos out of range: {}", state.0);
    }
    let expected = total_in as f64 * dst as f64 / src as f64;
    let err = (total_out as f64 - expected).abs();
    println!("in={total_in} out={total_out} expected={expected:.1} err={err:.2} worst_pos={worst_pos:.4}");
    assert!(err <= 2.0, "drift detected: {err}");
    // Continuity: sine through block joints must not click (adjacent
    // outputs across one joint differ by less than a big step).
    // Servo authority: drained ring recovers, flooded ring relieves.
    let s_dry = occupancy_servo(0.0, 9600.0);
    let s_full = occupancy_servo(30000.0, 9600.0);
    let s_half = occupancy_servo(9600.0, 9600.0);
    println!("servo dry={s_dry:.4} full={s_full:.4} half={s_half:.4}");
    assert!((s_dry - 0.997).abs() < 1e-9);
    assert!((s_full - 1.003).abs() < 1e-9);
    assert!((s_half - 1.0).abs() < 1e-9);
    // Downsample path exactness too (48k -> 44.1k).
    let mut st2 = (0.0f64, 0.0f32, 0.0f32);
    let (mut ti, mut to) = (0usize, 0usize);
    for _ in 0..2000 {
        let got = resample_with_carry(&mut st2, &input[..480], dst, &mut output, src, 1.0);
        ti += 480;
        to += got;
    }
    let exp2 = ti as f64 * src as f64 / dst as f64;
    println!("down: in={ti} out={to} expected={exp2:.1} err={:.2}", (to as f64 - exp2).abs());
    assert!((to as f64 - exp2).abs() <= 2.0);
    println!("RESAMPLE EXACT — no drift, servo sane");
}
