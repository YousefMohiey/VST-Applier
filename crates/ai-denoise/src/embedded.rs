//! Built-in AI noise suppression for VST-Applier.
//!
//! Wraps [DeepFilterNet3-ll] (MIT/Apache-2.0) with the model embedded
//! in the binary — no plugin slot, no external files, one toggle.
//! "ll" = the low-latency DFN3 variant: zero lookahead frames, so the
//! inherent delay is just the STFT window step (10 ms), versus 30 ms
//! for the regular DFN3 model.
//!
//! The model runs full-band at 48 kHz on 10 ms hops ([`Denoiser::hop`]).
//! It must be fed 48 kHz audio — at any other rate the spectrum shifts
//! and the output degrades badly. [`Denoiser::process`] is a 1:1 block
//! transform (same in/out length); the output is delayed by
//! [`Denoiser::latency_samples`]. The stage is allocation-free after
//! [`Denoiser::new`] and safe to call from the audio thread.
//!
//! [DeepFilterNet3-ll]: https://github.com/Rikorose/DeepFilterNet

use std::collections::VecDeque;

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

/// The DFN noise suppressor. One instance processes one stream; it is
/// not internally synchronised (the engine owns it behind its own lock).
pub struct Denoiser {
    df: DfTract,
    hop: usize,
    /// Input samples not yet forming a full hop.
    pending: Vec<f32>,
    /// Enhanced samples not yet handed out.
    ready: VecDeque<f32>,
    in_ch: Array2<f32>,
    out_ch: Array2<f32>,
    /// Silence gate state (see the gate in `process`).
    gated: bool,
}

// `DfTract` keeps `Rc` (tract tensors) inside, which is what makes it
// `!Send` by default. The value is only ever *used* by one thread at a
// time — the UI thread builds it, the audio thread runs it via the
// engine's mutex, and no internal reference ever escapes the object —
// so moving ownership between threads (creation → engine, swap-in /
// swap-out) is sound. These are the only transfers that happen.
unsafe impl Send for Denoiser {}

impl Denoiser {
    /// Load and prepare the embedded DFN3 model. Takes hundreds of
    /// milliseconds — call it off the audio thread (UI thread / startup).
    pub fn new() -> Result<Self, String> {
        let df = DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1))
            .map_err(|e| format!("DeepFilterNet init failed: {e}"))?;
        let hop = df.hop_size;
        Ok(Self {
            df,
            hop,
            pending: Vec::with_capacity(hop * 8),
            ready: VecDeque::with_capacity(hop * 8),
            in_ch: Array2::zeros((1, hop)),
            out_ch: Array2::zeros((1, hop)),
            gated: false,
        })
    }

    /// Hop size in samples (10 ms of audio at 48 kHz).
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Sample rate the model runs at (48 000).
    pub fn sample_rate(&self) -> usize {
        self.df.sr
    }

    /// Inherent algorithmic latency in samples at the model rate:
    /// the model lookahead frames plus the STFT reconstruction window.
    pub fn latency_samples(&self) -> usize {
        self.df.lookahead * self.hop + (self.df.fft_size - self.hop)
    }

    /// [`Self::latency_samples`] in milliseconds.
    pub fn latency_ms(&self) -> f64 {
        self.latency_samples() as f64 / self.df.sr as f64 * 1000.0
    }

    /// Maximum attenuation of the noise floor, in dB (100 = full gate,
    /// lower keeps some ambience). Applies to the next processed hop.
    pub fn set_atten_lim_db(&mut self, db: f32) {
        self.df.set_atten_lim(db);
    }

    /// Post-filter beta: `>0` slightly over-attenuates very noisy
    /// sections (`0` = off). Applies to the next processed hop.
    pub fn set_post_filter_beta(&mut self, beta: f32) {
        self.df.set_pf_beta(beta);
    }

    /// Process one block: append `input`, emit as many finished samples
    /// as are ready (model quantum = one hop), and return that count.
    /// The model consumes and produces exactly one hop per hop, so over
    /// time in == out; instantaneously the output can be a few hundred
    /// samples short of `output.len()` — the caller carries the slip
    /// (bounded by one hop) instead of us zero-filling the gap, which
    /// used to smear silence and let the internal queues grow forever
    /// (the drain cost then climbed every call — the 44.1 kHz "warm-up"
    /// slowdown). `output` lags `input` by [`Self::latency_samples`];
    /// during the first `latency_samples` of stream time the output is
    /// the algorithm priming its window (near-silence), never garbage.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        debug_assert_eq!(input.len(), output.len());
        self.pending.extend_from_slice(input);
        while self.pending.len() >= self.hop {
            {
                let mut row = self.in_ch.row_mut(0);
                row.as_slice_mut()
                    .expect("in_ch row contiguous")
                    .copy_from_slice(&self.pending[..self.hop]);
            }
            // Silence gate. A hop below ~-70 dB RMS has nothing to
            // denoise, and the model's low-level path is its most
            // expensive one (the recurrent state wanders near denormal
            // territory; measured 5-7x the cost of real signal, ~3 ms
            // per hop on a real mic sitting in a quiet room — a full
            // core, drained rings, "voice lag"). Pass the hop through
            // dry instead; hysteresis (-70 dB in, -60 dB out) keeps it
            // from flapping around the threshold.
            let rms = {
                let h = &self.pending[..self.hop];
                (h.iter().map(|v| v * v).sum::<f32>() / self.hop as f32).sqrt()
            };
            if self.gated {
                if rms > 1e-3 {
                    self.gated = false;
                }
            } else if rms < 3e-4 {
                self.gated = true;
            }
            if self.gated {
                let row = self.in_ch.row(0);
                let src = row.as_slice().expect("in_ch row contiguous");
                self.ready.extend(src.iter().copied());
                self.pending.drain(..self.hop);
                continue;
            }
            let ok = self
                .df
                .process(self.in_ch.view(), self.out_ch.view_mut())
                .is_ok();
            if ok {
                let row = self.out_ch.row(0);
                let src = row.as_slice().expect("out_ch row contiguous");
                self.ready.extend(src.iter().copied());
            } else {
                // Model hiccup: pass this hop through dry rather than
                // injecting silence into the middle of the stream.
                let row = self.in_ch.row(0);
                let src = row.as_slice().expect("in_ch row contiguous");
                self.ready.extend(src.iter().copied());
            }
            self.pending.drain(..self.hop);
        }
        let n = output.len().min(self.ready.len());
        for (o, v) in output.iter_mut().zip(self.ready.drain(..n)) {
            *o = v;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Deterministic white noise (xorshift), amplitude ~0.06 peak
    /// (≈ -30 dBFS) — steady noise is what the model suppresses best.
    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 0.06
            })
            .collect()
    }

    #[test]
    fn loads_model_at_48k() {
        let d = Denoiser::new().expect("model load");
        assert_eq!(d.sample_rate(), 48_000);
        assert_eq!(d.hop(), 480);
        // DFN3: lookahead frames + STFT window delta. Loose band so a
        // model tweak upstream doesn't break the test.
        let ms = d.latency_ms();
        assert!((5.0..60.0).contains(&ms), "latency {ms} ms out of band");
    }

    #[test]
    fn suppresses_steady_noise() {
        let mut d = Denoiser::new().expect("model load");
        let mut input = noise(48_000 * 8, 0x1234_5678);
        let mut out = vec![0.0f32; input.len()];
        d.process(&mut input, &mut out);
        // Steady-state windows: last 2 s in, measure the noise floor.
        let (a, b) = (48_000 * 6, 48_000 * 8);
        let nr_in = rms(&input[a..b]);
        let nr_out = rms(&out[a..b]);
        let atten_db = 20.0 * (nr_in / nr_out.max(1e-9)).log10();
        assert!(
            atten_db >= 6.0,
            "steady noise attenuated by only {atten_db:.1} dB"
        );
        // No NaN / blow-up anywhere.
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 10.0));
    }

    #[test]
    fn block_sizes_are_transparent() {
        // Odd block sizes (not multiples of the hop) must still come out
        // 1:1, finite everywhere, with the algorithmic delay where the
        // estimate says it is.
        let mut d = Denoiser::new().expect("model load");
        let total = 48_000 * 2;
        let input = noise(total, 0xDEAD_BEEF);
        let mut out = vec![0.0f32; total];
        let mut pos = 0;
        let sizes = [512usize, 128, 377, 1024];
        let mut k = 0;
        while pos < total {
            let n = sizes[k % sizes.len()].min(total - pos);
            d.process(&input[pos..pos + n], &mut out[pos..pos + n]);
            pos += n;
            k += 1;
        }
        assert!(out.iter().all(|v| v.is_finite()));
        let lag = d.latency_samples();
        let (a, b) = (48_000, 48_000 * 2 - lag);
        let mut best = (0usize, f32::MIN);
        for l in lag.saturating_sub(960)..(lag + 960).min(9_600) {
            let hi = b.min(total - l);
            let c: f32 = (a..hi).map(|i| input[i] * out[i + l]).sum();
            if c > best.1 {
                best = (l, c);
            }
        }
        let delta = best.0 as i64 - lag as i64;
        assert!(
            delta.abs() <= 960,
            "measured delay {} vs estimate {} (delta {} samples)",
            best.0,
            lag,
            delta
        );
    }
}
