//! Go/no-go probe: does tract's PULSED execution (the shipping semantics,
//! S=1 per hop) match a plain-ORT S=1 run that carries the GRU hidden
//! states across calls?
//!
//! Runs DeepFilterNet3-ll `enc.onnx` through tract exactly like
//! `libDF/src/tract.rs` does (same facts, same PulsedModel, same
//! SimpleState), feeding deterministic pseudo-random features one frame
//! at a time. Inputs and per-frame outputs are dumped as raw f32 so the
//! Python side (ONNX Runtime + externalized GRU states) can run the same
//! frames and compare.
//!
//! Usage: cargo run --release -p ai-denoise --example pulse_probe -- <enc.onnx> <out_dir>

use std::fs;
use std::io::Write;

use tract_core::prelude::*;
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::shapefactoid;
use tract_pulse::{internal::ToDim, model::*};

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

fn main() -> TractResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let onnx_path = args.get(1).expect("usage: pulse_probe <enc.onnx> <out_dir>");
    let out_dir = args.get(2).expect("usage: pulse_probe <enc.onnx> <out_dir>");
    fs::create_dir_all(out_dir).unwrap();

    let file = fs::read(onnx_path).expect("read onnx");
    let mut m = tract_onnx::onnx()
        .with_ignore_output_shapes(true)
        .model_for_read(&mut file.as_slice())?;

    let s = m.symbol_table.sym("S");
    let feat_erb = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 1, s, 32));
    let feat_spec = InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 2, s, 96));
    m = m
        .with_input_fact(0, feat_erb)?
        .with_input_fact(1, feat_spec)?
        .with_input_names(["feat_erb", "feat_spec"])?
        .with_output_names(["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"])?;
    m.analyse(true)?;
    let mut m = m.into_typed()?;
    m.declutter()?;
    let pulsed = PulsedModel::new(&m, s, &1.to_dim())?;
    let plan = pulsed.into_typed()?.into_optimized()?.into_runnable()?;
    let mut state = SimpleState::new(plan)?;

    let t_frames = 64usize;
    let mut rng: u64 = 0x1234_5678_9abc_def0;
    let mut all_erb = Vec::new();
    let mut all_spec = Vec::new();
    for _ in 0..(t_frames * 32) {
        all_erb.push(lcg(&mut rng) * 4.0);
    }
    for _ in 0..(t_frames * 2 * 96) {
        all_spec.push(lcg(&mut rng) * 4.0);
    }
    dump(&format!("{out_dir}/in_feat_erb.raw"), &all_erb);
    dump(&format!("{out_dir}/in_feat_spec.raw"), &all_spec);

    // Outputs: e0 [1,64,S,32], e1 [1,64,S,8]? (sizes per model), emb [1,S,512], c0 [1,64,S,96], lsnr [1,S,1]
    let mut outs_acc: Vec<Vec<f32>> = vec![Vec::new(); 7];
    let mut out_shapes: Vec<Vec<usize>> = vec![Vec::new(); 7];

    for t in 0..t_frames {
        let mut erb = unsafe { Tensor::uninitialized_dt(f32::datum_type(), &[1, 1, 1, 32])? };
        erb.as_slice_mut::<f32>()?
            .copy_from_slice(&all_erb[t * 32..(t + 1) * 32]);
        let mut spec = unsafe { Tensor::uninitialized_dt(f32::datum_type(), &[1, 2, 1, 96])? };
        spec.as_slice_mut::<f32>()?
            .copy_from_slice(&all_spec[t * 2 * 96..(t + 1) * 2 * 96]);
        let outs = state.run(tvec!(erb.into(), spec.into()))?;
        for (i, o) in outs.iter().enumerate() {
            let view = o.to_array_view::<f32>()?;
            out_shapes[i] = view.shape().to_vec();
            outs_acc[i].extend(view.iter().copied());
        }
    }

    let names = ["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"];
    for i in 0..7 {
        println!("out {}: shape {:?} len {}", names[i], out_shapes[i], outs_acc[i].len());
        dump(&format!("{out_dir}/tract_{}.raw", names[i]), &outs_acc[i]);
    }
    println!("done");
    Ok(())
}
