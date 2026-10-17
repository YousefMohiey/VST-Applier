//! Built-in AI noise suppression for VST-Applier.
//!
//! Wraps [DeepFilterNet3-ll] (MIT/Apache-2.0). With the `embed-model`
//! feature (default) the model is embedded in the binary — no plugin
//! slot, no external files. Without it (the desktop app build, whose
//! built-in stage is retired in favour of the Yousef Noise Suppression
//! plugin) the model + tract runtime are compiled out entirely and
//! [`Denoiser::new`] returns an error — the app exe sheds ~50 MB.
//!
//! "ll" = the low-latency DFN3 variant: zero lookahead frames, so the
//! inherent delay is just the STFT window step (10 ms), versus 30 ms
//! for the regular DFN3 model.
//!
//! [DeepFilterNet3-ll]: https://github.com/Rikorose/DeepFilterNet

#[cfg(feature = "embed-model")]
mod embedded;

#[cfg(feature = "embed-model")]
pub use embedded::Denoiser;

/// Placeholder used when the `embed-model` feature is off: same API, no
/// model, no runtime — [`Denoiser::new`] always fails, which every
/// caller already handles as "built-in denoise unavailable".
#[cfg(not(feature = "embed-model"))]
mod stub {
    /// See the crate docs for the feature story.
    pub struct Denoiser {
        _priv: (),
    }

    impl Denoiser {
        /// Always fails in this build.
        pub fn new() -> Result<Self, String> {
            Err("built-in denoise is not embedded in this build (the Yousef Noise Suppression plugin owns denoising)".into())
        }

        /// Hop size in samples (10 ms of audio at 48 kHz).
        pub fn hop(&self) -> usize {
            480
        }

        /// Sample rate the model would run at (48 000).
        pub fn sample_rate(&self) -> usize {
            48_000
        }

        /// No model, no delay.
        pub fn latency_samples(&self) -> usize {
            0
        }

        /// No model, no delay.
        pub fn latency_ms(&self) -> f64 {
            0.0
        }

        /// No-op in this build.
        pub fn set_atten_lim_db(&mut self, _db: f32) {}

        /// No-op in this build.
        pub fn set_post_filter_beta(&mut self, _beta: f32) {}

        /// Pass-through (never reached — instances cannot be created).
        pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
            let n = input.len().min(output.len());
            output[..n].copy_from_slice(&input[..n]);
            for o in output[n..].iter_mut() {
                *o = 0.0;
            }
            n
        }
    }
}

#[cfg(not(feature = "embed-model"))]
pub use stub::Denoiser;
