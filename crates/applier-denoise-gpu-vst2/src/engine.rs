//! The denoiser as the audio thread sees it: a 1:1 block pump around the
//! 48 kHz, 10 ms-hop ONNX pipeline.
//!
//! Same contract as the shipping CPU denoiser (`ai-denoise`): hand it a block,
//! get back as many finished samples as are ready; the difference (bounded by
//! one hop) is carried, never zero-filled. The output lags the input by
//! `latency_samples()`.

use std::collections::VecDeque;

use df_ort::{DfOrt, EpKind, RuntimeParams};

pub struct GpuDenoiser {
    core: Option<DfOrt>,
    hop: usize,
    /// Input samples not yet forming a full hop.
    pending: Vec<f32>,
    /// Enhanced samples not yet handed out.
    ready: VecDeque<f32>,
    in_hop: Vec<f32>,
    out_hop: Vec<f32>,
    /// Silence gate state (see `process`).
    gated: bool,
    /// Last error from the model, if any (shown in the editor).
    err: Option<String>,
    init_ms: f64,
}

impl GpuDenoiser {
    /// Load ONNX Runtime + the three models. Takes ~1-2 s with DirectML
    /// session compilation — call off the audio thread.
    pub fn new() -> Result<Self, String> {
        let t0 = std::time::Instant::now();
        let mut core = DfOrt::new(&RuntimeParams::default_with_ch(1))?;
        // Compile the DirectML kernels now, at plugin load, not inside the
        // first audio block (measured ~0.6 s of first-run cost otherwise).
        if core.inference_available() {
            core.warm_up(4);
        }
        let init_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let hop = core.hop_size;
        Ok(Self {
            core: Some(core),
            hop,
            pending: Vec::with_capacity(hop * 8),
            ready: VecDeque::with_capacity(hop * 8),
            in_hop: vec![0.0; hop],
            out_hop: vec![0.0; hop],
            gated: false,
            err: None,
            init_ms,
        })
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Inherent latency of the *running* path: 0 when audio passes through
    /// untouched (no runtime), otherwise the model's window delay.
    pub fn latency_samples(&self) -> usize {
        match &self.core {
            Some(c) if c.ep != EpKind::Missing => c.latency_samples(),
            _ => 0,
        }
    }

    pub fn ep(&self) -> EpKind {
        self.core
            .as_ref()
            .map(|c| c.ep)
            .unwrap_or(EpKind::Missing)
    }

    pub fn status_line(&self) -> String {
        match &self.core {
            Some(c) if c.ep == EpKind::Cpu && c.fallback_reason.contains("FORCE_CPU") => {
                "CPU (DirectML skipped by DF_ORT_FORCE_CPU)".to_string()
            }
            Some(c) => c.status_line(),
            None => "initialising".to_string(),
        }
    }

    pub fn detail_line(&self) -> String {
        match &self.core {
            Some(c) => {
                // Keep this short enough to fit the card: version, load time,
                // latency. The DLL path lives in `status.detail` for logs.
                let ver = if c.status.ort_version.is_empty() {
                    "ONNX Runtime".to_string()
                } else {
                    format!("ONNX Runtime {}", c.status.ort_version)
                };
                let mut s = format!(
                    "{ver}  |  {:.0} ms init  |  {} smp latency",
                    self.init_ms,
                    c.latency_samples()
                );
                if let Some(e) = &self.err {
                    s.push_str("  |  ");
                    s.push_str(e);
                }
                s
            }
            None => String::new(),
        }
    }

    pub fn set_atten_lim_db(&mut self, db: f32) {
        if let Some(c) = self.core.as_mut() {
            c.set_atten_lim(db);
        }
    }

    pub fn set_post_filter_beta(&mut self, beta: f32) {
        if let Some(c) = self.core.as_mut() {
            c.set_pf_beta(beta);
        }
    }

    /// Process one block: append `input`, emit as many finished samples as are
    /// ready, return that count. Over time in == out; instantaneously the
    /// output can be short by up to one hop, which the caller carries.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        let Some(core) = self.core.as_mut() else {
            let n = output.len().min(input.len());
            output[..n].copy_from_slice(&input[..n]);
            return n;
        };
        self.pending.extend_from_slice(input);
        while self.pending.len() >= self.hop {
            self.in_hop.copy_from_slice(&self.pending[..self.hop]);
            // Silence gate: a hop below ~-70 dB RMS has nothing to denoise and
            // the model's low-level path is its most expensive one. Pass it
            // through dry; hysteresis (-70 dB in, -60 dB out) stops flapping.
            let rms = (self.in_hop.iter().map(|v| v * v).sum::<f32>() / self.hop as f32).sqrt();
            if self.gated {
                if rms > 1e-3 {
                    self.gated = false;
                }
            } else if rms < 3e-4 {
                self.gated = true;
            }
            if self.gated {
                self.ready.extend(self.in_hop.iter().copied());
                self.pending.drain(..self.hop);
                continue;
            }
            match core.process(&self.in_hop, &mut self.out_hop) {
                Ok(_) => {
                    self.err = None;
                    self.ready.extend(self.out_hop.iter().copied());
                }
                Err(e) => {
                    // Model hiccup: pass this hop through dry rather than
                    // injecting silence into the middle of the stream.
                    self.err = Some(e);
                    self.ready.extend(self.in_hop.iter().copied());
                }
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

// ---------------------------------------------------------------------------
// Windowed-sinc resampler (same 16-tap/128-phase design as the CPU plugin),
// used when the host runs the chain at a rate other than 48 kHz.
// ---------------------------------------------------------------------------

const RES_TAPS: usize = 16;
const RES_HALF: usize = RES_TAPS / 2;
const RES_PHASES: usize = 128;

pub struct Resampler {
    ratio: f64, // output_rate / input_rate
    pos: f64,
    tail: Vec<f32>,
    table: Vec<f32>,
}

fn kaiser(t: f64, beta: f64) -> f64 {
    let arg = beta * (1.0 - t * t).max(0.0).sqrt();
    let mut s = 1.0f64;
    let mut term = 1.0f64;
    for k in 1..24 {
        term *= (arg / 2.0) * (arg / 2.0) / (k as f64 * k as f64);
        s += term;
    }
    let mut s0 = 1.0f64;
    let mut term0 = 1.0f64;
    for k in 1..24 {
        term0 *= (beta / 2.0) * (beta / 2.0) / (k as f64 * k as f64);
        s0 += term0;
    }
    s / s0
}

impl Resampler {
    pub fn new(ratio: f64) -> Self {
        let beta = 8.0;
        let mut table = vec![0.0f32; RES_PHASES * RES_TAPS];
        for ph in 0..RES_PHASES {
            let frac = ph as f64 / RES_PHASES as f64;
            let mut sum = 0.0f64;
            for k in 0..RES_TAPS {
                let t = k as f64 - (RES_HALF as f64 - 1.0) - frac;
                let sinc = if t.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t)
                };
                let w = kaiser((t / RES_HALF as f64).abs().min(1.0), beta);
                table[ph * RES_TAPS + k] = (sinc * w) as f32;
            }
            for k in 0..RES_TAPS {
                sum += table[ph * RES_TAPS + k] as f64;
            }
            if sum.abs() > 1e-9 {
                for k in 0..RES_TAPS {
                    table[ph * RES_TAPS + k] = (table[ph * RES_TAPS + k] as f64 / sum) as f32;
                }
            }
        }
        Self {
            ratio,
            pos: 0.0,
            tail: Vec::new(),
            table,
        }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        if input.is_empty() {
            return 0;
        }
        let tail_len = self.tail.len();
        let total = tail_len + input.len();
        let get = |i: isize| -> f32 {
            if i < 0 || i as usize >= total {
                0.0
            } else if (i as usize) < tail_len {
                self.tail[i as usize]
            } else {
                input[i as usize - tail_len]
            }
        };
        let step = 1.0 / self.ratio;
        let mut out_n = 0usize;
        while self.pos + RES_HALF as f64 <= (total - RES_HALF) as f64 && out_n < output.len() {
            let center = self.pos.floor();
            let frac = self.pos - center;
            let ph = ((frac * RES_PHASES as f64) as usize).min(RES_PHASES - 1);
            let taps = &self.table[ph * RES_TAPS..(ph + 1) * RES_TAPS];
            let mut acc = 0.0f32;
            for k in 0..RES_TAPS {
                let idx = center as isize - (RES_HALF as isize - 1) + k as isize;
                acc += get(idx) * taps[k];
            }
            output[out_n] = acc;
            out_n += 1;
            self.pos += step;
        }
        // Keep only the samples the next call's windows can still need.
        let keep_from = (self.pos - RES_HALF as f64 - 1.0).max(0.0) as usize;
        let mut new_tail: Vec<f32> = Vec::with_capacity(total.saturating_sub(keep_from));
        for i in keep_from..total {
            new_tail.push(if i < tail_len {
                self.tail[i]
            } else {
                input[i - tail_len]
            });
        }
        self.pos -= keep_from as f64;
        self.tail = new_tail;
        out_n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_preserves_length_and_level() {
        let mut r = Resampler::new(48_000.0 / 44_100.0);
        let mut out = vec![0.0f32; 2048];
        let input: Vec<f32> = (0..2048)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect();
        let n = r.process(&input, &mut out);
        assert!(n > 0 && n <= out.len());
        assert!(out[..n].iter().all(|v| v.is_finite()));
    }
}
