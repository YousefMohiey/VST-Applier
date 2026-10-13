//! Engine overhead throughput: convert + resample path for 60 s of
//! 44.1 kHz mono audio in 512-frame blocks. No plugins, no devices.
use audio_engine::{convert, resample::resample_with_carry};

fn main() {
    let blocks = 44100 * 60 / 512;
    let input = vec![0.3f32; 512];
    let mut stereo = vec![0.0f32; 1024];
    let mut mono = vec![0.0f32; 512];
    let mut out = vec![0.0f32; 600];
    let mut route = vec![0.0f32; 1200];
    let mut carry = (0.0f64, 0.0f32, 0.0f32);
    let t0 = std::time::Instant::now();
    let mut total_out = 0usize;
    for _ in 0..blocks {
        convert::mono_to_stereo(&input, &mut stereo);
        convert::stereo_to_mono(&stereo, &mut mono);
        let p = convert::peak_level(&mono);
        std::hint::black_box(p);
        let got = resample_with_carry(&mut carry, &mono, 44100, &mut out, 48000, 1.0);
        convert::fan_out(&out[..got], 2, &mut route[..got * 2]);
        total_out += got;
    }
    let dt = t0.elapsed().as_secs_f64();
    println!("{blocks} blocks (60.0s audio) in {dt:.2}s = {:.2}x realtime, out={total_out}", 60.0 / dt);
}
