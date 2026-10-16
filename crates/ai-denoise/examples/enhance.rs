//! Enhance a 48 kHz mono 16-bit PCM WAV with the built-in denoiser and
//! write the result — used to produce A/B listening demos.
//!
//!   cargo run -p ai-denoise --release --example enhance -- in.wav out.wav

use ai_denoise::Denoiser;
use std::io::{Read, Write};

fn read_wav(path: &str) -> (Vec<f32>, u32, u16) {
    let mut f = std::fs::File::open(path).expect("open wav");
    let mut b = Vec::new();
    f.read_to_end(&mut b).expect("read wav");
    assert_eq!(&b[0..4], b"RIFF", "not a wav");
    let mut pos = 12;
    let (mut rate, mut ch, mut bits) = (48000u32, 1u16, 16u16);
    let mut data = Vec::new();
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let body = &b[pos + 8..(pos + 8 + size).min(b.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = body.to_vec();
        }
        pos += 8 + size + (size & 1);
    }
    assert_eq!(bits, 16, "only 16-bit PCM supported");
    let samples: Vec<f32> = data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect();
    (samples, rate, ch)
}

fn write_wav(path: &str, samples: &[f32], rate: u32, ch: u16) {
    let mut data = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(ch * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::File::create(path)
        .expect("create wav")
        .write_all(&out)
        .expect("write wav");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (inp, outp) = (args[1].clone(), args[2].clone());
    let (samples, rate, ch) = read_wav(&inp);
    if rate != 48_000 && std::env::var("VA_DEMO_44").is_err() {
        panic!("the model expects 48 kHz input (set VA_DEMO_44=1 to simulate the old bug)");
    }
    assert_eq!(ch, 1, "mono input expected");
    let t0 = std::time::Instant::now();
    let mut d = Denoiser::new().expect("model load");
    println!(
        "model in {} ms — sample rate {} Hz, latency {:.0} ms",
        t0.elapsed().as_millis(),
        d.sample_rate(),
        d.latency_ms()
    );
    let mut out = vec![0.0f32; samples.len()];
    d.process(&samples, &mut out);
    write_wav(&outp, &out, rate, ch);
    println!("wrote {outp}");
}
