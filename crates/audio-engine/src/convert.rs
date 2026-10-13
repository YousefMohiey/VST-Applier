//! Sample-format conversion (mirrors `AudioSampleConverter`).
//!
//! Supported capture formats: 16-bit PCM, 24-bit PCM (packed 3 bytes),
//! 32-bit PCM, 32-bit IEEE float. Everything converts to f32 in [-1, 1].

/// Convert interleaved source samples into f32.
///
/// `src` holds `frames * channels` samples of `bits` size; for 24-bit each
/// sample is 3 little-endian bytes.
pub fn to_f32(src: &[u8], bits: u16, is_float: bool, dst: &mut [f32]) {
    match (bits, is_float) {
        (32, true) => {
            for (d, chunk) in dst.iter_mut().zip(src.chunks_exact(4)) {
                let v = f32::from_le_bytes(chunk.try_into().unwrap());
                *d = sanitize(v);
            }
        }
        (16, false) => {
            for (d, chunk) in dst.iter_mut().zip(src.chunks_exact(2)) {
                let v = i16::from_le_bytes(chunk.try_into().unwrap()) as f32 / 32768.0;
                *d = v;
            }
        }
        (32, false) => {
            for (d, chunk) in dst.iter_mut().zip(src.chunks_exact(4)) {
                let v = i32::from_le_bytes(chunk.try_into().unwrap()) as f32 / 2147483648.0;
                *d = v;
            }
        }
        (24, false) => {
            for (d, chunk) in dst.iter_mut().zip(src.chunks_exact(3)) {
                let raw = (chunk[0] as i32)
                    | ((chunk[1] as i32) << 8)
                    | ((chunk[2] as i32) << 16);
                // Sign-extend from 24 bits.
                let v = (raw << 8 >> 8) as f32 / 8388608.0;
                *d = v;
            }
        }
        _ => {
            dst.fill(0.0);
        }
    }
}

/// Convert f32 samples into interleaved bytes of the requested format.
pub fn from_f32(src: &[f32], bits: u16, is_float: bool, dst: &mut [u8]) {
    match (bits, is_float) {
        (32, true) => {
            for (s, chunk) in src.iter().zip(dst.chunks_exact_mut(4)) {
                chunk.copy_from_slice(&sanitize(*s).to_le_bytes());
            }
        }
        (16, false) => {
            for (s, chunk) in src.iter().zip(dst.chunks_exact_mut(2)) {
                let v = (sanitize(*s).clamp(-1.0, 1.0) * 32767.0).round() as i16;
                chunk.copy_from_slice(&v.to_le_bytes());
            }
        }
        (32, false) => {
            for (s, chunk) in src.iter().zip(dst.chunks_exact_mut(4)) {
                let v = (sanitize(*s).clamp(-1.0, 1.0) * 2147483647.0).round() as i32;
                chunk.copy_from_slice(&v.to_le_bytes());
            }
        }
        (24, false) => {
            for (s, chunk) in src.iter().zip(dst.chunks_exact_mut(3)) {
                let v = (sanitize(*s).clamp(-1.0, 1.0) * 8388607.0).round() as i32;
                let bytes = v.to_le_bytes();
                chunk.copy_from_slice(&bytes[..3]);
            }
        }
        _ => {}
    }
}

#[inline]
fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Peak level of a mono f32 span, clamped to [0, 1].
/// NaN-safe without a finiteness branch: NaN > peak is always false.
pub fn peak_level(mono: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    for &s in mono {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    peak.min(1.0)
}

/// Average channels of interleaved `src` into mono `dst`.
pub fn downmix_to_mono(src: &[f32], channels: usize, dst: &mut [f32]) {
    if channels == 1 {
        dst.copy_from_slice(&src[..dst.len()]);
        return;
    }
    let inv = 1.0 / channels as f32;
    for (i, d) in dst.iter_mut().enumerate() {
        let mut sum = 0.0;
        for c in 0..channels {
            sum += src[i * channels + c];
        }
        *d = sum * inv;
    }
}

/// Duplicate mono into interleaved stereo.
pub fn mono_to_stereo(mono: &[f32], stereo: &mut [f32]) {
    for (i, &s) in mono.iter().enumerate() {
        stereo[i * 2] = s;
        stereo[i * 2 + 1] = s;
    }
}

/// Average interleaved stereo back to mono.
pub fn stereo_to_mono(stereo: &[f32], mono: &mut [f32]) {
    for (i, m) in mono.iter_mut().enumerate() {
        *m = (stereo[i * 2] + stereo[i * 2 + 1]) * 0.5;
    }
}

/// Fan mono out to `channels` interleaved channels.
pub fn fan_out(mono: &[f32], channels: usize, dst: &mut [f32]) {
    for (i, &s) in mono.iter().enumerate() {
        for c in 0..channels {
            dst[i * channels + c] = s;
        }
    }
}
