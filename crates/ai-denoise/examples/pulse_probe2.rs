//! Ground-truth probe #2: run the FULL DeepFilterNet3-ll streaming chain (enc ->
//! erb_dec -> df_dec) through tract exactly like `libDF/src/tract.rs` does
//! (`DfTract` in `deep_filter`): same input facts, same PulsedModel::new(...,
//! &1.to_dim()) hop-by-hop execution, same output wiring (df_dec output is
//! narrowed to `coefs` via with_output_names, erb_dec emits raw `m`).
//!
//! Feeds the same deterministic pseudo-random frames as `pulse_probe.rs`
//! (same LCG seed/order), so the dumps of the encoder outputs must be
//! bit-identical to the ones pulse_probe produced — a built-in consistency
//! check — while adding the decoder ground truth:
//!   * decin_{emb,e3,e2,e1,e0,c0}.raw - per-frame values fed to the decoders
//!   * tract_m.raw                     - erb_dec output per frame
//!   * tract_coefs.raw                 - df_dec output per frame
//!   * shapes.txt                      - per-output per-frame shapes
//!
//! Usage: cargo run --release -p ai-denoise --example pulse_probe2 -- <model_dir> <out_dir>
//!        (<model_dir> must contain enc.onnx, erb_dec.onnx, df_dec.onnx)

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use tract_core::prelude::*;
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::shapefactoid;
use tract_pulse::{internal::ToDim, model::*};

const T_FRAMES: usize = 64;
const NB_ERB: usize = 32;
const NB_DF: usize = 96;
const CONV_CH: usize = 64;

fn lcg(state: &mut u64) -> f32 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*state >> 33) as f32 / (1 << 31) as f32) - 0.5
}

fn dump(path: &str, data: &[f32]) {
    let mut f = fs::File::create(path).unwrap();
    for v in data {
        f.write_all(&v.to_le_bytes()).unwrap();
    }
}

fn load(model_dir: &str, name: &str) -> TractResult<InferenceModel> {
    let file = fs::read(PathBuf::from(model_dir).join(name)).expect("read onnx");
    tract_onnx::onnx()
        .with_ignore_output_shapes(true)
        .model_for_read(&mut file.as_slice())
}

/// Mirrors libDF `init_encoder_impl` (n_ch = 1).
fn init_enc(model_dir: &str) -> TractResult<TypedModel> {
    let mut m = load(model_dir, "enc.onnx")?;
    let s = m.symbol_table.sym("S");
    let feat_erb = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 1, s, NB_ERB));
    let feat_spec = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 2, s, NB_DF));
    m = m
        .with_input_fact(0, feat_erb)?
        .with_input_fact(1, feat_spec)?
        .with_input_names(["feat_erb", "feat_spec"])?
        .with_output_names(["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"])?;
    m.analyse(true)?;
    let mut m = m.into_typed()?;
    m.declutter()?;
    let pulsed = PulsedModel::new(&m, s, &1.to_dim())?;
    Ok(pulsed.into_typed()?.into_optimized()?)
}

/// Mirrors libDF `init_erb_decoder_impl` with mask_reduction = None (raw `m`).
fn init_erb_dec(model_dir: &str) -> TractResult<TypedModel> {
    let mut m = load(model_dir, "erb_dec.onnx")?;
    let s = m.symbol_table.sym("S");
    let n_hidden = CONV_CH * NB_ERB / 4;
    let emb = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, s, n_hidden));
    let e3 = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, CONV_CH, s, 8));
    let e2 = e3.clone();
    let e1 = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, CONV_CH, s, 16));
    let e0 = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, CONV_CH, s, NB_ERB));
    m = m
        .with_input_fact(0, emb)?
        .with_input_fact(1, e3)?
        .with_input_fact(2, e2)?
        .with_input_fact(3, e1)?
        .with_input_fact(4, e0)?
        .with_input_names(["emb", "e3", "e2", "e1", "e0"])?;
    m.analyse(true)?;
    let mut m = m.into_typed()?;
    m.declutter()?;
    let pulsed = PulsedModel::new(&m, s, &1.to_dim())?;
    let m = pulsed.into_typed()?.with_output_names(["m"])?;
    Ok(m.into_optimized()?)
}

/// Mirrors libDF `init_df_decoder_impl` (output narrowed to `coefs`).
fn init_df_dec(model_dir: &str) -> TractResult<TypedModel> {
    let mut m = load(model_dir, "df_dec.onnx")?;
    let s = m.symbol_table.sym("S");
    let n_hidden = CONV_CH * NB_ERB / 4;
    let emb = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, s, n_hidden));
    let c0 = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, CONV_CH, s, NB_DF));
    m = m
        .with_input_fact(0, emb)?
        .with_input_fact(1, c0)?
        .with_input_names(["emb", "c0"])?
        .with_output_names(["coefs"])?;
    m.analyse(true)?;
    let mut m = m.into_typed()?;
    m.declutter()?;
    let pulsed = PulsedModel::new(&m, s, &1.to_dim())?;
    Ok(pulsed.into_typed()?.into_optimized()?)
}

struct Accum {
    name: &'static str,
    shape: Vec<usize>,
    data: Vec<f32>,
}

impl Accum {
    fn push(&mut self, t: &TValue) -> TractResult<()> {
        let view = t.to_array_view::<f32>()?;
        self.shape = view.shape().to_vec();
        self.data.extend(view.iter().copied());
        Ok(())
    }
}

fn main() -> TractResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let model_dir = args.get(1).expect("usage: pulse_probe2 <model_dir> <out_dir>");
    let out_dir = args.get(2).expect("usage: pulse_probe2 <model_dir> <out_dir>");
    fs::create_dir_all(out_dir).unwrap();

    let mut enc = SimpleState::new(init_enc(model_dir)?.into_runnable()?)?;
    let mut erb_dec = SimpleState::new(init_erb_dec(model_dir)?.into_runnable()?)?;
    let mut df_dec = SimpleState::new(init_df_dec(model_dir)?.into_runnable()?)?;

    // Same LCG stream as pulse_probe.rs (seed + consumption order).
    let mut rng: u64 = 0x1234_5678_9abc_def0;
    let mut all_erb = Vec::new();
    let mut all_spec = Vec::new();
    for _ in 0..(T_FRAMES * NB_ERB) {
        all_erb.push(lcg(&mut rng) * 4.0);
    }
    for _ in 0..(T_FRAMES * 2 * NB_DF) {
        all_spec.push(lcg(&mut rng) * 4.0);
    }
    dump(&format!("{out_dir}/in_feat_erb.raw"), &all_erb);
    dump(&format!("{out_dir}/in_feat_spec.raw"), &all_spec);

    let mut accs = vec![
        Accum { name: "e0", shape: vec![], data: vec![] },
        Accum { name: "e1", shape: vec![], data: vec![] },
        Accum { name: "e2", shape: vec![], data: vec![] },
        Accum { name: "e3", shape: vec![], data: vec![] },
        Accum { name: "emb", shape: vec![], data: vec![] },
        Accum { name: "c0", shape: vec![], data: vec![] },
        Accum { name: "lsnr", shape: vec![], data: vec![] },
    ];
    let mut m_acc = Accum { name: "m", shape: vec![], data: vec![] };
    let mut coefs_acc = Accum { name: "coefs", shape: vec![], data: vec![] };
    let mut feed = ["emb", "e3", "e2", "e1", "e0", "c0"]
        .iter()
        .map(|n| Accum { name: n, shape: vec![], data: vec![] })
        .collect::<Vec<_>>();

    for t in 0..T_FRAMES {
        let mut erb = unsafe { Tensor::uninitialized_dt(f32::datum_type(), &[1, 1, 1, NB_ERB])? };
        erb.as_slice_mut::<f32>()?
            .copy_from_slice(&all_erb[t * NB_ERB..(t + 1) * NB_ERB]);
        let mut spec = unsafe { Tensor::uninitialized_dt(f32::datum_type(), &[1, 2, 1, NB_DF])? };
        spec.as_slice_mut::<f32>()?
            .copy_from_slice(&all_spec[t * 2 * NB_DF..(t + 1) * 2 * NB_DF]);
        let outs = enc.run(tvec!(erb.into(), spec.into()))?;
        // outs[0..7] = e0, e1, e2, e3, emb, c0, lsnr
        for (a, o) in accs.iter_mut().zip(outs.iter()) {
            a.push(o)?;
        }
        let e0 = outs[0].clone();
        let e1 = outs[1].clone();
        let e2 = outs[2].clone();
        let e3 = outs[3].clone();
        let emb = outs[4].clone();
        let c0 = outs[5].clone();

        // erb_dec input order: emb, e3, e2, e1, e0 (libDF order)
        let dec_in = tvec!(emb.clone(), e3, e2, e1, e0);
        for (a, v) in feed.iter_mut().zip(dec_in.iter()) {
            a.push(v)?;
        }
        let m = erb_dec.run(dec_in)?;
        m_acc.push(&m[0])?;

        let df_in = tvec!(emb, c0);
        // feed[0] ("emb") was already recorded above (same tensor); record c0.
        feed[5].push(&df_in[1])?;
        let coefs = df_dec.run(df_in)?;
        coefs_acc.push(&coefs[0])?;
    }

    let mut shapes = String::new();
    for a in accs.iter().chain([&m_acc, &coefs_acc].into_iter()) {
        let file = format!("tract_{}", a.name);
        dump(&format!("{out_dir}/{file}.raw"), &a.data);
        shapes.push_str(&format!(
            "{file}: shape(frames={}, per-frame={:?}) len={}\n",
            T_FRAMES,
            a.shape,
            a.data.len()
        ));
    }
    for a in feed.iter() {
        let file = format!("decin_{}", a.name);
        dump(&format!("{out_dir}/{file}.raw"), &a.data);
        shapes.push_str(&format!(
            "{file}: shape(frames={}, per-frame={:?}) len={}\n",
            T_FRAMES,
            a.shape,
            a.data.len()
        ));
    }
    fs::write(format!("{out_dir}/shapes.txt"), &shapes).unwrap();
    print!("{shapes}");
    println!("done");
    Ok(())
}
