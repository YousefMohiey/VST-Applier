//! Streaming DeepFilterNet3-ll inference on ONNX Runtime.
//!
//! Mirrors `df::tract::DfTract`'s pipeline (analysis → ERB/DF features →
//! encoder → stage gating → ERB decoder gains + DF decoder coefficients →
//! deep filtering → post-filter → synthesis), but the three graphs run as
//! ONNX Runtime sessions whose conv delay lines and GRU hidden states are
//! graph inputs/outputs carried frame to frame (see `models/*.states.json`).
//!
//! The models are the "conv-stateful" exports validated against tract's
//! `PulsedModel` in `scratch/gpu_probe/REPORT.md`; the states are wired
//! output→input per the manifests, exactly as the Python validator does.

use std::collections::VecDeque;

use ort::session::{Session, SessionOutputs};
use ort::value::Tensor;

use crate::runtime::{self, EpKind, RuntimeStatus};
use crate::{as_slice_complex, as_slice_mut_complex, post_filter, Complex32, DFState};

// Model configuration, from the shipped DeepFilterNet3-ll `config.ini`.
const SR: usize = 48_000;
const FFT_SIZE: usize = 960;
const HOP: usize = 480;
const NB_ERB: usize = 32;
const NB_DF: usize = 96;
const DF_ORDER: usize = 5;
const MIN_NB_ERB_FREQS: usize = 2;
const CONV_LOOKAHEAD: usize = 0;
const DF_LOOKAHEAD: usize = 0;
const N_FREQS: usize = FFT_SIZE / 2 + 1; // 481
/// `calc_norm_alpha(48000, 480, norm_tau=1.0)` as libDF computes it.
const ALPHA: f32 = 0.990;

const ENC_ONNX: &[u8] = include_bytes!("../models/enc.onnx");
const ERB_DEC_ONNX: &[u8] = include_bytes!("../models/erb_dec.onnx");
const DF_DEC_ONNX: &[u8] = include_bytes!("../models/df_dec.onnx");
const ENC_STATES: &str = include_str!("../models/enc.states.json");
const ERB_DEC_STATES: &str = include_str!("../models/erb_dec.states.json");
const DF_DEC_STATES: &str = include_str!("../models/df_dec.states.json");

// ---------------------------------------------------------------------------
// Runtime parameters (same knobs and defaults as `df::tract::RuntimeParams`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RuntimeParams {
    pub n_ch: usize,
    pub post_filter: bool,
    pub post_filter_beta: f32,
    pub atten_lim_db: f32,
    pub min_db_thresh: f32,
    pub max_db_erb_thresh: f32,
    pub max_db_df_thresh: f32,
}

impl RuntimeParams {
    pub fn default_with_ch(channels: usize) -> Self {
        Self {
            n_ch: channels,
            post_filter: false,
            post_filter_beta: 0.02,
            atten_lim_db: 100.,
            min_db_thresh: -10.,
            max_db_erb_thresh: 30.,
            max_db_df_thresh: 20.,
        }
    }
    pub fn with_post_filter(mut self, beta: f32) -> Self {
        if beta > 0. {
            self.post_filter = true;
        }
        self.post_filter_beta = beta;
        self
    }
    pub fn with_atten_lim(mut self, db: f32) -> Self {
        self.atten_lim_db = db;
        self
    }
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self::default_with_ch(1)
    }
}

// ---------------------------------------------------------------------------
// State-carrying ONNX session
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Manifest {
    states: Vec<StateDesc>,
}

#[derive(serde::Deserialize)]
struct StateDesc {
    input: String,
    output: String,
    shape: Vec<usize>,
}

struct StateSlot {
    input: String,
    output: String,
    shape: Vec<usize>,
    data: Vec<f32>,
}

struct Model {
    session: Session,
    states: Vec<StateSlot>,
}

impl Model {
    fn new(onnx: &[u8], manifest: &str, dml: bool, label: &str) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_str(manifest).map_err(|e| format!("{label}: bad manifest: {e}"))?;

        let mut builder =
            Session::builder().map_err(|e| format!("{label}: session builder: {e}"))?;
        // Power configuration for a session that is called once per 10 ms
        // hop rather than once per batch.
        //
        // v0.4.10 shipped ORT's defaults, which are tuned for throughput of
        // one long inference: an intra-op pool of one thread per core for
        // EACH of the three models, and both pools allowed to spin-wait for
        // their next work item between our calls. Measured on the plugin's
        // real cadence (crates/libdf-ort/examples/hopload.rs) the changes
        // below are worth a few percent of a core, and — more importantly —
        // they remove the spin-waits entirely, so no ORT thread is awake
        // while the audio thread is between hops:
        //
        //   * `allow_spinning = 0` for the intra-op and inter-op pools;
        //   * sequential graph execution (no inter-op pool at all);
        //   * denormals-are-zero, the right setting for audio DSP.
        //
        // What we deliberately did NOT change (each was A/B measured, see
        // docs/power.md):
        //   * the CPU arena / memory pattern: disabling them raised the
        //     per-hop cost from ~1.7 ms to ~2.1 ms (fresh allocations every
        //     hop) for ~20 MB of RSS;
        //   * the intra-op thread count: forcing 1 thread cost ~4 points of
        //     a core in the app (the encoder's nodes then run serially) to
        //     save ~20 MB of thread stacks.
        builder = builder
            .with_intra_op_spinning(false)
            .map_err(|e| format!("{label}: intra spinning: {e}"))?;
        builder = builder
            .with_inter_op_spinning(false)
            .map_err(|e| format!("{label}: inter spinning: {e}"))?;
        builder = builder
            .with_parallel_execution(false)
            .map_err(|e| format!("{label}: sequential execution: {e}"))?;
        builder = builder
            .with_flush_to_zero()
            .map_err(|e| format!("{label}: flush-to-zero: {e}"))?;

        if dml {
            builder = builder
                .with_execution_providers([ort::ep::DirectML::default()
                    .with_performance_preference(
                        ort::ep::directml::PerformancePreference::HighPerformance,
                    )
                    .build()])
                .map_err(|e| format!("{label}: DirectML EP: {e}"))?;
            // DirectML's CPU-side fence wait spins by default. At 100 calls
            // per second that is a busy-wait on the audio thread paying for
            // work that is already on the GPU; block instead.
            builder = builder
                .with_config_entry("ep.dml.enable_cpu_sync_spinning", "0")
                .map_err(|e| format!("{label}: dml spin: {e}"))?;
        } else {
            builder = builder
                .with_execution_providers([ort::ep::CPU::default().build()])
                .map_err(|e| format!("{label}: CPU EP: {e}"))?;
        }

        let session = builder
            .commit_from_memory(onnx)
            .map_err(|e| format!("{label}: load model: {e}"))?;

        let states = manifest
            .states
            .into_iter()
            .map(|s| {
                let len = s.shape.iter().product::<usize>();
                StateSlot {
                    input: s.input,
                    output: s.output,
                    shape: s.shape,
                    data: vec![0.0; len],
                }
            })
            .collect();

        Ok(Self { session, states })
    }

    /// Zero every carried state (start of stream).
    fn reset_states(&mut self) {
        for st in &mut self.states {
            st.data.fill(0.0);
        }
    }

    /// Run one frame: caller supplies the frame inputs by name; every state
    /// output is copied back into its slot before returning.
    fn run<'s>(
        &'s mut self,
        mut inputs: Vec<(String, Tensor<f32>)>,
    ) -> Result<SessionOutputs<'s>, String> {
        for st in &self.states {
            let t = Tensor::from_array((st.shape.clone(), st.data.clone()))
                .map_err(|e| format!("state `{}`: {e}", st.input))?;
            inputs.push((st.input.clone(), t));
        }
        let outs = self.session.run(inputs).map_err(|e| format!("run: {e}"))?;
        for st in &mut self.states {
            let (_, data) = outs[st.output.as_str()]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("state out `{}`: {e}", st.output))?;
            if data.len() != st.data.len() {
                return Err(format!(
                    "state `{}`: expected {} floats, got {}",
                    st.input,
                    st.data.len(),
                    data.len()
                ));
            }
            st.data.copy_from_slice(data);
        }
        Ok(outs)
    }
}

fn tensor(shape: &[usize], data: &[f32]) -> Result<Tensor<f32>, String> {
    Tensor::from_array((shape.to_vec(), data.to_vec())).map_err(|e| format!("tensor: {e}"))
}

/// What one frame decided (mirrors the `(gains, coefs)` options of
/// `DfTract::process_raw`).
struct FrameOut {
    lsnr: f32,
    apply_gains: bool,
    apply_gain_zeros: bool,
    apply_df: bool,
}

// ---------------------------------------------------------------------------
// The pipeline
// ---------------------------------------------------------------------------

pub struct DfOrt {
    enc: Option<Model>,
    erb_dec: Option<Model>,
    df_dec: Option<Model>,

    pub ep: EpKind,
    pub status: RuntimeStatus,
    /// Why DirectML could not be used, when it couldn't (empty on the GPU path).
    pub fallback_reason: String,

    pub sr: usize,
    pub ch: usize,
    pub fft_size: usize,
    pub hop_size: usize,
    pub nb_erb: usize,
    pub min_nb_erb_freqs: usize,
    pub nb_df: usize,
    pub n_freqs: usize,
    pub df_order: usize,
    pub lookahead: usize,
    pub conv_lookahead: usize,
    pub df_lookahead: usize,

    pub post_filter: bool,
    pub post_filter_beta: f32,
    pub alpha: f32,
    pub min_db_thresh: f32,
    pub max_db_erb_thresh: f32,
    pub max_db_df_thresh: f32,
    pub atten_lim: Option<f32>,

    pub df_states: Vec<DFState>,
    /// Raw gains per channel, `[ch, nb_erb]` (erb_dec's `m`).
    gains: Vec<f32>,
    /// Mean-over-channels gains, `[nb_erb]`.
    gains_reduced: Vec<f32>,
    /// `[ch, 2, nb_df]` transposed DF spectrum (what the encoder expects).
    feat_spec: Vec<f32>,
    /// `[ch, nb_df, df_order, 2]` coefficients of the current frame.
    df_coefs: Vec<f32>,
    /// `[ch, nb_df, 2]` complex scratch for the DF accumulation.
    df_acc: Vec<f32>,
    spec_buf: Vec<f32>,
    erb_buf: Vec<f32>,
    cplx_buf: Vec<f32>,
    rolling_y: VecDeque<Vec<f32>>,
    rolling_x: VecDeque<Vec<f32>>,
    m_zeros: Vec<f32>,
    skip_counter: usize,
}

impl DfOrt {
    /// Build the pipeline. Tries DirectML first, falls back to the CPU EP on
    /// the same models; if ONNX Runtime is missing entirely the returned
    /// object is a pass-through (`ep == Missing`) instead of an error.
    pub fn new(rp: &RuntimeParams) -> Result<Self, String> {
        Self::with_dir(rp, None)
    }

    /// Same as [`DfOrt::new`], but lets the caller say where the ORT DLLs
    /// live (normally the plugin's own folder).
    pub fn with_dir(rp: &RuntimeParams, dir: Option<&std::path::Path>) -> Result<Self, String> {
        Self::with_dir_ep(rp, dir, false)
    }

    /// Like [`DfOrt::with_dir`], with an explicit override to skip DirectML.
    /// `DF_ORT_FORCE_CPU=1` in the environment does the same (escape hatch for
    /// a broken GPU driver: same models, CPU execution provider).
    pub fn with_dir_ep(
        rp: &RuntimeParams,
        dir: Option<&std::path::Path>,
        force_cpu: bool,
    ) -> Result<Self, String> {
        let force_cpu = force_cpu || std::env::var_os("DF_ORT_FORCE_CPU").is_some();
        let status = runtime::init(dir);
        let ch = rp.n_ch.max(1);

        let mut ep = EpKind::Missing;
        let mut fallback_reason = String::new();
        let mut enc = None;
        let mut erb_dec = None;
        let mut df_dec = None;
        if status.available() {
            let mut dml_err = String::new();
            if !force_cpu {
                match (
                    Model::new(ENC_ONNX, ENC_STATES, true, "enc"),
                    Model::new(ERB_DEC_ONNX, ERB_DEC_STATES, true, "erb_dec"),
                    Model::new(DF_DEC_ONNX, DF_DEC_STATES, true, "df_dec"),
                ) {
                    (Ok(a), Ok(b), Ok(c)) => {
                        enc = Some(a);
                        erb_dec = Some(b);
                        df_dec = Some(c);
                        ep = EpKind::DirectMl;
                    }
                    (a, b, c) => {
                        dml_err = [a.err(), b.err(), c.err()]
                            .into_iter()
                            .flatten()
                            .next()
                            .unwrap_or_else(|| "DirectML session failed".to_string());
                    }
                }
            }
            if ep == EpKind::Missing {
                match (
                    Model::new(ENC_ONNX, ENC_STATES, false, "enc"),
                    Model::new(ERB_DEC_ONNX, ERB_DEC_STATES, false, "erb_dec"),
                    Model::new(DF_DEC_ONNX, DF_DEC_STATES, false, "df_dec"),
                ) {
                    (Ok(a), Ok(b), Ok(c)) => {
                        enc = Some(a);
                        erb_dec = Some(b);
                        df_dec = Some(c);
                        ep = EpKind::Cpu;
                        fallback_reason = if force_cpu {
                            "DF_ORT_FORCE_CPU set".to_string()
                        } else {
                            dml_err
                        };
                    }
                    _ => {
                        // Runtime present but the models won't load: stay
                        // pass-through rather than failing the plugin.
                        ep = EpKind::Missing;
                    }
                }
            }
        }

        let atten_lim_db = rp.atten_lim_db.abs();
        let atten_lim = if atten_lim_db >= 100. {
            None
        } else if atten_lim_db < 0.01 {
            Some(1.)
        } else {
            Some(10f32.powf(-atten_lim_db / 20.))
        };

        let spec_len = ch * N_FREQS * 2;
        let mut rolling_y = VecDeque::with_capacity(DF_ORDER + CONV_LOOKAHEAD);
        let mut rolling_x = VecDeque::with_capacity(DF_ORDER.max(CONV_LOOKAHEAD));
        for _ in 0..(DF_ORDER + CONV_LOOKAHEAD) {
            rolling_y.push_back(vec![0.0; spec_len]);
        }
        for _ in 0..DF_ORDER.max(lookahead()) {
            rolling_x.push_back(vec![0.0; spec_len]);
        }

        let mut df_state = DFState::new(SR, FFT_SIZE, HOP, NB_ERB, MIN_NB_ERB_FREQS);
        df_state.init_norm_states(NB_DF);
        let df_states: Vec<DFState> = (0..ch)
            .map(|i| {
                let mut s = if i == 0 {
                    df_state.clone()
                } else {
                    DFState::new(SR, FFT_SIZE, HOP, NB_ERB, MIN_NB_ERB_FREQS)
                };
                if i > 0 {
                    s.init_norm_states(NB_DF);
                }
                s
            })
            .collect();

        Ok(Self {
            enc,
            erb_dec,
            df_dec,
            ep,
            status,
            fallback_reason,
            sr: SR,
            ch,
            fft_size: FFT_SIZE,
            hop_size: HOP,
            nb_erb: NB_ERB,
            min_nb_erb_freqs: MIN_NB_ERB_FREQS,
            nb_df: NB_DF,
            n_freqs: N_FREQS,
            df_order: DF_ORDER,
            lookahead: lookahead(),
            conv_lookahead: CONV_LOOKAHEAD,
            df_lookahead: DF_LOOKAHEAD,
            post_filter: rp.post_filter,
            post_filter_beta: rp.post_filter_beta,
            alpha: ALPHA,
            min_db_thresh: rp.min_db_thresh,
            max_db_erb_thresh: rp.max_db_erb_thresh,
            max_db_df_thresh: rp.max_db_df_thresh,
            atten_lim,
            df_states,
            gains: vec![0.0; ch * NB_ERB],
            gains_reduced: vec![0.0; NB_ERB],
            feat_spec: vec![0.0; ch * 2 * NB_DF],
            df_coefs: vec![0.0; ch * NB_DF * DF_ORDER * 2],
            df_acc: vec![0.0; ch * NB_DF * 2],
            spec_buf: vec![0.0; spec_len],
            erb_buf: vec![0.0; ch * NB_ERB],
            cplx_buf: vec![0.0; ch * NB_DF * 2],
            rolling_y,
            rolling_x,
            m_zeros: vec![0.0; NB_ERB],
            skip_counter: 0,
        })
    }

    pub fn inference_available(&self) -> bool {
        self.enc.is_some()
    }

    /// Return to the start-of-stream state: model states zeroed, DSP state
    /// re-initialised, spec history cleared. Used after the warm-up runs.
    pub fn reset(&mut self) {
        for m in [self.enc.as_mut(), self.erb_dec.as_mut(), self.df_dec.as_mut()]
            .into_iter()
            .flatten()
        {
            m.reset_states();
        }
        for s in &mut self.df_states {
            let mut fresh = DFState::new(SR, FFT_SIZE, HOP, NB_ERB, MIN_NB_ERB_FREQS);
            fresh.init_norm_states(NB_DF);
            *s = fresh;
        }
        for b in self.rolling_y.iter_mut() {
            b.fill(0.0);
        }
        for b in self.rolling_x.iter_mut() {
            b.fill(0.0);
        }
        self.spec_buf.fill(0.0);
        self.skip_counter = 0;
    }

    /// Run a few silent hops so the execution provider compiles its kernels
    /// now (at plugin load) instead of on the first audio block, then reset.
    pub fn warm_up(&mut self, hops: usize) {
        let mut z = vec![0.0f32; self.ch * self.hop_size];
        let mut out = vec![0.0f32; z.len()];
        for _ in 0..hops {
            let _ = self.process(&z, &mut out);
        }
        self.reset();
        z.fill(0.0);
    }

    /// The line the editor shows on the GPU status row.
    pub fn status_line(&self) -> String {
        self.status.status_line(self.ep)
    }

    /// Inherent latency in samples at the model rate: lookahead frames plus
    /// the STFT reconstruction window.
    pub fn latency_samples(&self) -> usize {
        self.lookahead * self.hop_size + (self.fft_size - self.hop_size)
    }

    pub fn set_pf_beta(&mut self, beta: f32) {
        self.post_filter_beta = beta;
        if beta > 0. {
            self.post_filter = true;
        } else if beta == 0. {
            self.post_filter = false;
        } else {
            self.post_filter = false;
            self.post_filter_beta = 0.;
        }
    }

    pub fn set_atten_lim(&mut self, db: f32) {
        let lim = db.abs();
        self.atten_lim = if lim >= 100. {
            None
        } else if lim < 0.01 {
            Some(1.)
        } else {
            Some(10f32.powf(-lim / 20.))
        };
    }

    /// Stage gating, identical to `DfTract::apply_stages`.
    pub fn apply_stages(&self, lsnr: f32) -> (bool, bool, bool) {
        if lsnr < self.min_db_thresh {
            (false, true, false)
        } else if lsnr > self.max_db_erb_thresh {
            (false, false, false)
        } else if lsnr > self.max_db_df_thresh {
            (true, false, false)
        } else {
            (true, false, true)
        }
    }

    /// Process one hop (`ch * hop_size` samples); writes `ch * hop_size`.
    ///
    /// This is the streaming contract of `DfTract::process`: the output is
    /// delayed by [`Self::latency_samples`] and every call advances the model
    /// state by exactly one hop.
    pub fn process(&mut self, noisy: &[f32], enh: &mut [f32]) -> Result<f32, String> {
        let (ch, hop) = (self.ch, self.hop_size);
        assert_eq!(noisy.len(), ch * hop);
        assert_eq!(enh.len(), ch * hop);

        let e = noisy.iter().map(|x| x * x).sum::<f32>();
        let rms = e / noisy.len() as f32;
        if rms < 1e-7 {
            self.skip_counter += 1;
        } else {
            self.skip_counter = 0;
        }
        if self.skip_counter > 5 {
            enh.fill(0.);
            return Ok(-15.);
        }

        self.rolling_y.pop_front();
        self.rolling_x.pop_front();
        for c in 0..ch {
            let out = &mut self.spec_buf[c * N_FREQS * 2..(c + 1) * N_FREQS * 2];
            self.df_states[c].analysis(&noisy[c * hop..(c + 1) * hop], as_slice_mut_complex(out));
        }
        self.rolling_y.push_back(self.spec_buf.clone());
        self.rolling_x.push_back(self.spec_buf.clone());
        if self.atten_lim.unwrap_or_default() == 1. {
            enh.copy_from_slice(noisy);
            return Ok(35.);
        }

        let fo = match self.process_raw() {
            Ok(fo) => fo,
            Err(e) => {
                // Inference hiccup: pass the hop through dry. Never inject
                // silence into the stream on an error.
                enh.copy_from_slice(noisy);
                return Err(e);
            }
        };

        // Mask stage, on the frame that is about to become the output.
        let mask_idx = self.df_order - 1;
        if fo.apply_gains {
            let y = &mut self.rolling_y[mask_idx];
            let st = &self.df_states[0];
            for c in 0..ch {
                let range = c * N_FREQS * 2..(c + 1) * N_FREQS * 2;
                st.apply_mask(as_slice_mut_complex(&mut y[range]), &self.gains_reduced);
            }
            self.skip_counter = 0;
        } else if fo.apply_gain_zeros {
            let y = &mut self.rolling_y[mask_idx];
            let st = &self.df_states[0];
            for c in 0..ch {
                let range = c * N_FREQS * 2..(c + 1) * N_FREQS * 2;
                st.apply_mask(as_slice_mut_complex(&mut y[range]), &self.m_zeros);
            }
            self.skip_counter = 0;
        } else {
            self.skip_counter += 1;
        }

        // Upper bins come from the (masked) frame; DF overwrites bins < nb_df.
        self.spec_buf.clone_from(&self.rolling_y[mask_idx]);
        if fo.apply_df {
            self.apply_df()?;
        }

        let noisy_idx = self.lookahead.max(self.df_order) - self.lookahead - 1;

        // Post filter (noisy vs enhanced), only when stage 1 ran.
        if fo.apply_gains && self.post_filter {
            post_filter(
                as_slice_complex(&self.rolling_x[noisy_idx]),
                as_slice_mut_complex(&mut self.spec_buf),
                self.post_filter_beta,
            );
        }

        // Attenuation limit: mix a little of the noisy signal back in.
        if let Some(lim) = self.atten_lim {
            for (e, n) in self.spec_buf.iter_mut().zip(self.rolling_x[noisy_idx].iter()) {
                *e = *e * (1. - lim) + lim * *n;
            }
        }

        {
            let Self {
                df_states,
                spec_buf,
                hop_size,
                ..
            } = self;
            let hop_size = *hop_size;
            for c in 0..ch {
                let range = c * N_FREQS * 2..(c + 1) * N_FREQS * 2;
                df_states[c].synthesis(
                    as_slice_mut_complex(&mut spec_buf[range]),
                    &mut enh[c * hop_size..(c + 1) * hop_size],
                );
            }
        }

        Ok(fo.lsnr)
    }

    /// Feature extraction + three model calls.
    fn process_raw(&mut self) -> Result<FrameOut, String> {
        if self.enc.is_none() || self.erb_dec.is_none() || self.df_dec.is_none() {
            return Err("inference unavailable".to_string());
        }
        let (ch, nb_erb, nb_df) = (self.ch, self.nb_erb, self.nb_df);

        // Per-channel ERB + complex features.
        for c in 0..ch {
            let spec = as_slice_complex(&self.spec_buf[c * N_FREQS * 2..(c + 1) * N_FREQS * 2]);
            let erb = &mut self.erb_buf[c * nb_erb..(c + 1) * nb_erb];
            self.df_states[c].feat_erb(spec, self.alpha, erb);
            let cplx = &mut self.cplx_buf[c * nb_df * 2..(c + 1) * nb_df * 2];
            self.df_states[c].feat_cplx(&spec[..nb_df], self.alpha, as_slice_mut_complex(cplx));
        }
        // feat_spec is [ch, 2, 1, nb_df]: re block then im block per channel.
        for c in 0..ch {
            let src = &self.cplx_buf[c * nb_df * 2..(c + 1) * nb_df * 2];
            let dst = &mut self.feat_spec[c * 2 * nb_df..(c + 1) * 2 * nb_df];
            for f in 0..nb_df {
                dst[f] = src[2 * f];
                dst[nb_df + f] = src[2 * f + 1];
            }
        }

        let out = {
            let enc = self.enc.as_mut().unwrap();
            let feat_erb = tensor(&[ch, 1, 1, nb_erb], &self.erb_buf)?;
            let feat_spec = tensor(&[ch, 2, 1, nb_df], &self.feat_spec)?;
            let outs = enc.run(vec![
                ("feat_erb".to_string(), feat_erb),
                ("feat_spec".to_string(), feat_spec),
            ])?;
            EncOut::extract(&outs)?
        };

        let (apply_gains, apply_gain_zeros, apply_df) = self.apply_stages(out.lsnr);

        if apply_gains {
            let m_gains = {
                let erb_dec = self.erb_dec.as_mut().unwrap();
                let emb = tensor(&[ch, 1, 512], &out.emb)?;
                let e3 = tensor(&[ch, 64, 1, nb_erb / 4], &out.e3)?;
                let e2 = tensor(&[ch, 64, 1, nb_erb / 4], &out.e2)?;
                let e1 = tensor(&[ch, 64, 1, nb_erb / 2], &out.e1)?;
                let e0 = tensor(&[ch, 64, 1, nb_erb], &out.e0)?;
                let outs = erb_dec.run(vec![
                    ("emb".to_string(), emb),
                    ("e3".to_string(), e3),
                    ("e2".to_string(), e2),
                    ("e1".to_string(), e1),
                    ("e0".to_string(), e0),
                ])?;
                let (_, m) = outs["m"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("erb_dec m: {e}"))?;
                if m.len() != ch * nb_erb {
                    return Err(format!(
                        "erb_dec m: expected {} floats, got {}",
                        ch * nb_erb,
                        m.len()
                    ));
                }
                let mut g = vec![0.0f32; ch * nb_erb];
                g.copy_from_slice(m);
                g
            };
            self.gains.copy_from_slice(&m_gains);
            for b in 0..nb_erb {
                let mut sum = 0.0;
                for c in 0..ch {
                    sum += self.gains[c * nb_erb + b];
                }
                self.gains_reduced[b] = sum / ch as f32;
            }
        } else if apply_gain_zeros {
            self.gains_reduced.fill(0.0);
        }

        if apply_df {
            let coefs = {
                let df_dec = self.df_dec.as_mut().unwrap();
                let emb = tensor(&[ch, 1, 512], &out.emb)?;
                let c0 = tensor(&[ch, 64, 1, nb_df], &out.c0)?;
                let outs =
                    df_dec.run(vec![("emb".to_string(), emb), ("c0".to_string(), c0)])?;
                let (_, c) = outs["coefs"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("df_dec coefs: {e}"))?;
                c.to_vec()
            };
            if coefs.len() != ch * nb_df * DF_ORDER * 2 {
                return Err(format!(
                    "df coefs: expected {}, got {}",
                    ch * nb_df * DF_ORDER * 2,
                    coefs.len()
                ));
            }
            self.df_coefs.copy_from_slice(&coefs);
        }

        Ok(FrameOut {
            lsnr: out.lsnr,
            apply_gains,
            apply_gain_zeros,
            apply_df,
        })
    }

    /// Deep filtering: overwrite bins `< nb_df` with the filtered sum.
    fn apply_df(&mut self) -> Result<(), String> {
        let (ch, nb_df, df_order, n_freqs) = (self.ch, self.nb_df, self.df_order, self.n_freqs);
        for c in 0..ch {
            let acc = &mut self.df_acc[c * nb_df * 2..(c + 1) * nb_df * 2];
            acc.fill(0.0);
            for t in 0..df_order {
                let frame = &self.rolling_x[t];
                let s = as_slice_complex(&frame[c * n_freqs * 2..(c + 1) * n_freqs * 2]);
                let cf = &self.df_coefs;
                for f in 0..nb_df {
                    let cr = cf[((c * nb_df + f) * df_order + t) * 2];
                    let ci = cf[((c * nb_df + f) * df_order + t) * 2 + 1];
                    let o = &mut acc[2 * f..2 * f + 2];
                    o[0] += s[f].re * cr - s[f].im * ci;
                    o[1] += s[f].re * ci + s[f].im * cr;
                }
            }
        }
        for c in 0..ch {
            let out = &mut self.spec_buf[c * n_freqs * 2..(c + 1) * n_freqs * 2];
            let o = as_slice_mut_complex(out);
            let acc = &self.df_acc[c * nb_df * 2..(c + 1) * nb_df * 2];
            for f in 0..nb_df {
                o[f] = Complex32::new(acc[2 * f], acc[2 * f + 1]);
            }
        }
        Ok(())
    }
}

/// `max(conv_lookahead, df_lookahead)` for the DFN3-ll model (both 0).
fn lookahead() -> usize {
    CONV_LOOKAHEAD.max(DF_LOOKAHEAD)
}

/// Encoder outputs for one frame.
struct EncOut {
    e0: Vec<f32>,
    e1: Vec<f32>,
    e2: Vec<f32>,
    e3: Vec<f32>,
    emb: Vec<f32>,
    c0: Vec<f32>,
    lsnr: f32,
}

impl EncOut {
    fn extract(outs: &SessionOutputs<'_>) -> Result<Self, String> {
        let get = |name: &str| -> Result<Vec<f32>, String> {
            let (_, d) = outs[name]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("enc `{name}`: {e}"))?;
            Ok(d.to_vec())
        };
        let lsnr = {
            let (_, d) = outs["lsnr"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("enc lsnr: {e}"))?;
            d.first().copied().unwrap_or(0.0)
        };
        Ok(Self {
            e0: get("e0")?,
            e1: get("e1")?,
            e2: get("e2")?,
            e3: get("e3")?,
            emb: get("emb")?,
            c0: get("c0")?,
            lsnr,
        })
    }
}
