//! Numeric parity: the ORT pipeline vs the shipping tract engine.
//!
//! Both sides get the exact same deterministic signal hop by hop (48 kHz, 10 ms
//! hops) and the outputs are compared sample by sample. The ORT models are the
//! conv-stateful exports whose per-output agreement with tract's PulsedModel
//! was measured at <= 1.5e-5 (scratch/gpu_probe/REPORT.md); this test measures
//! the *whole* pipeline (features → 3 sessions → DF → post-filter → STFT).
//!
//! The signal is speech-like on purpose: pure white noise puts the model's LSNR
//! at the floor, the zero mask suppresses everything, and the comparison would
//! never touch the model's numerics.
//!
//! Needs `onnxruntime.dll` (+ `DirectML.dll` for the GPU path). Resolution:
//! `DF_ORT_DIR` env var, the deployed plugin folder, or the python
//! `onnxruntime` package directory on this machine. Run with `--nocapture` to
//! see the numbers.

use df::tract::{DfParams, DfTract, RuntimeParams as TractParams};
use df_ort::{DfOrt, RuntimeParams};
use ndarray::Array2;

/// See `examples/smoke.rs`: harmonic stack shaped by two formants,
/// syllabically modulated, plus broadband noise.
fn speechy(n: usize, noise_amp: f32, seed: u32) -> Vec<f32> {
    let sr = 48_000.0f32;
    let mut out = Vec::with_capacity(n);
    let mut s = seed;
    for i in 0..n {
        let t = i as f32 / sr;
        let f0 = 118.0 * (1.0 + 0.02 * (2.0 * std::f32::consts::PI * 0.8 * t).sin());
        let mut v = 0.0f32;
        for k in 1..=40 {
            let f = f0 * k as f32;
            if f > 7_000.0 {
                break;
            }
            let formants = 1.0 / (1.0 + ((f - 700.0) / 250.0).powi(2))
                + 0.6 / (1.0 + ((f - 1_220.0) / 300.0).powi(2))
                + 0.2 / (1.0 + ((f - 2_600.0) / 400.0).powi(2));
            v += formants / k as f32 * (2.0 * std::f32::consts::PI * f * t).sin();
        }
        let syl = (2.0 * std::f32::consts::PI * 3.5 * t).sin().max(0.0).powf(0.7);
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let noise = (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * noise_amp;
        out.push(v * 0.02 * syl + noise);
    }
    out
}

fn find_ort_dir() -> Option<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("DF_ORT_DIR") {
        return Some(std::path::PathBuf::from(d));
    }
    let candidates = [
        // deployed plugin folder
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../installer/app/Plugins/VST-Applier"
        ),
        // this machine's python onnxruntime-directml install
        "C:/Users/Administrator/AppData/Local/Programs/Python/Python312/Lib/site-packages/onnxruntime/capi",
    ];
    for c in candidates {
        let p = std::path::PathBuf::from(c);
        if p.join("onnxruntime.dll").is_file() {
            return Some(p);
        }
    }
    None
}

struct Diff {
    n: usize,
    max_abs: f32,
    max_rel: f32,
    rms_ref: f32,
    max_vs_dry: f32,
    lsnr_min: f32,
    lsnr_max: f32,
}

/// Compare both engines on the same signal.
fn compare(atten_db: f32, pf_beta: f32, noise_amp: f32, hops: usize) -> Diff {
    let dir = find_ort_dir().expect(
        "onnxruntime.dll not found: set DF_ORT_DIR to the folder that has onnxruntime.dll \
         (e.g. .../site-packages/onnxruntime/capi) or deploy the plugin to \
         installer/app/Plugins/VST-Applier first",
    );

    let rp = RuntimeParams::default_with_ch(1)
        .with_atten_lim(atten_db)
        .with_post_filter(pf_beta);
    let tp = TractParams::default_with_ch(1)
        .with_atten_lim(atten_db)
        .with_post_filter(pf_beta);

    let mut ort = DfOrt::with_dir(&rp, Some(&dir)).expect("ORT pipeline");
    let mut tract = DfTract::new(DfParams::default(), &tp).expect("tract pipeline");
    println!("ORT EP: {:?} ({})", ort.ep, ort.status_line());

    assert_eq!(ort.hop_size, tract.hop_size, "hop size");
    let hop = ort.hop_size;

    let input = speechy(hops * hop, noise_amp, 0x1234_5678);
    let mut out_ort = vec![0.0f32; hops * hop];
    let mut out_tract = vec![0.0f32; hops * hop];
    let mut in2 = Array2::<f32>::zeros((1, hop));
    let mut out2 = Array2::<f32>::zeros((1, hop));

    let (mut lsnr_min, mut lsnr_max) = (f32::MAX, f32::MIN);
    for h in 0..hops {
        let a = h * hop;
        let b = a + hop;
        let lsnr = ort.process(&input[a..b], &mut out_ort[a..b]).expect("ort process");
        lsnr_min = lsnr_min.min(lsnr);
        lsnr_max = lsnr_max.max(lsnr);
        in2.row_mut(0)
            .as_slice_mut()
            .unwrap()
            .copy_from_slice(&input[a..b]);
        tract
            .process(in2.view(), out2.view_mut())
            .expect("tract process");
        out_tract[a..b].copy_from_slice(out2.row(0).as_slice().unwrap());
    }

    // Skip the first 100 ms (window/state priming) in the comparison.
    let skip = (ort.sr / 10).min(hops * hop / 4);
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    let mut max_vs_dry = 0.0f32;
    let mut n = 0usize;
    for i in skip..hops * hop {
        let x = out_tract[i];
        let y = out_ort[i];
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let d = (x - y).abs();
        max_abs = max_abs.max(d);
        if x.abs() > 1e-3 {
            max_rel = max_rel.max(d / x.abs());
        }
        // How far the reference is from the dry input: proves the model ran.
        let dry = input[i];
        max_vs_dry = max_vs_dry.max((x - dry).abs());
        n += 1;
    }
    let s: f32 = out_tract[skip..].iter().map(|v| v * v).sum();
    Diff {
        n,
        max_abs,
        max_rel,
        rms_ref: (s / n as f32).sqrt(),
        max_vs_dry,
        lsnr_min,
        lsnr_max,
    }
}

#[test]
fn parity_default_settings() {
    let d = compare(100.0, 0.0, 0.02, 400);
    println!(
        "default (atten 100 dB, pf off): n={} max_abs={:.3e} max_rel(|ref|>1e-3)={:.3e} \
         rms_ref={:.3e} max|ref-dry|={:.3e} lsnr {:.2}..{:.2} dB",
        d.n, d.max_abs, d.max_rel, d.rms_ref, d.max_vs_dry, d.lsnr_min, d.lsnr_max
    );
    assert!(d.rms_ref > 1e-4, "reference output is silent");
    assert!(
        d.max_vs_dry > 1e-2,
        "the model did not change the signal (max|ref-dry|={:.3e}); the comparison would be vacuous",
        d.max_vs_dry
    );
    assert!(
        d.max_abs < 1e-4,
        "ORT vs tract max abs diff {:.3e} is above float noise",
        d.max_abs
    );
}

#[test]
fn parity_post_filter_and_atten_limit() {
    let d = compare(20.0, 0.02, 0.02, 400);
    println!(
        "atten 20 dB + pf 0.02: n={} max_abs={:.3e} max_rel(|ref|>1e-3)={:.3e} \
         rms_ref={:.3e} max|ref-dry|={:.3e} lsnr {:.2}..{:.2} dB",
        d.n, d.max_abs, d.max_rel, d.rms_ref, d.max_vs_dry, d.lsnr_min, d.lsnr_max
    );
    assert!(d.rms_ref > 1e-4, "reference output is silent");
    assert!(
        d.max_vs_dry > 1e-2,
        "the model did not change the signal (max|ref-dry|={:.3e})",
        d.max_vs_dry
    );
    assert!(
        d.max_abs < 1e-3,
        "ORT vs tract max abs diff {:.3e} is above float noise",
        d.max_abs
    );
}

/// Low-noise speech: the model keeps the mask close to 1 and LSNR climbs to
/// ~19 dB, so the gains (not the zero mask) carry the output — the opposite
/// end of the range from the noisy cases above.
#[test]
fn parity_low_noise() {
    let d = compare(100.0, 0.0, 0.0005, 400);
    println!(
        "low noise (atten 100 dB, pf off): n={} max_abs={:.3e} max_rel(|ref|>1e-3)={:.3e} \
         rms_ref={:.3e} max|ref-dry|={:.3e} lsnr {:.2}..{:.2} dB",
        d.n, d.max_abs, d.max_rel, d.rms_ref, d.max_vs_dry, d.lsnr_min, d.lsnr_max
    );
    assert!(d.rms_ref > 1e-5, "reference output is silent");
    assert!(
        d.max_vs_dry > 1e-2,
        "the model did not change the signal (max|ref-dry|={:.3e})",
        d.max_vs_dry
    );
    assert!(
        d.max_abs < 1e-4,
        "ORT vs tract max abs diff {:.3e} is above float noise",
        d.max_abs
    );
}
