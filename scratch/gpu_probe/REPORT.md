# Streaming DeepFilterNet3-ll in plain ONNX Runtime — conv delay-line state surgery + tract-parity validation

Status: **DONE** — all three models (`enc`, `erb_dec`, `df_dec`) now reproduce tract's
`PulsedModel` streaming outputs from a plain ORT S=1 loop within f32 noise
(e0/c0 bit-exact); states are carried in the ONNX graphs themselves.

Artifacts (this directory):

| What | Path |
|---|---|
| **Streaming models (GRU + conv states)** | `tmp/conv_stateful/{enc,erb_dec,df_dec}.onnx` |
| State manifests (name/shape/init/wiring) | `tmp/conv_stateful/<model>.states.json` |
| Surgery script (produces the above from `tmp/stateful/*`) | `py/conv_state_surgery.py` |
| Validator (S=1 loop vs tract dumps, batch check, chunk check) | `py/validate_stream.py` |
| Full validation log | `py/validation_run.txt` |
| DirectML smoke test | `py/dml_smoke.py` |
| Batch==pulsed cross-check | `py/batch_vs_tract.py` |
| Pad-value instrumentation | `py/inspect_pads.py` (+ `py/enc_pads_S1.json`) |
| tract ground truth (probe) | `crates/ai-denoise/examples/pulse_probe2.rs` → dumps in `pulse_out2/` |

Regenerate the models with:

```
python py/conv_state_surgery.py tmp/stateful tmp/conv_stateful
```

(`tmp/stateful/*` = the earlier GRU-state surgery of `tmp/export/*`; the conv
surgery runs on top of it and leaves the GRU wiring untouched.)

---

## 1. What tract's PulsedModel actually computes

Reading tract-pulse 0.21.4 (`src/ops/cnn/pools.rs: pulsify_pooled_input`,
`src/ops/array/pad.rs: pulsify`):

* a time-axis `Conv` with kernel `k_s` and dilation 1 gets an internal
  **zero-initialised delay line** of `(k_s-1)` input frames (`Delay` op,
  `overlap = kernel_len + 1 - stride`);
* a pulse-axis `Pad` becomes a `PulsePad` that injects its constant (zeros)
  **before frame 0** and shifts the stream offset;
* the model-wide net delay is 0, so output frame *t* uses input frames ≤ *t*
  with **zeros for t < 0**.

**Measured confirmation** (py/batch_vs_tract.py): running the *unmodified*
exported graph in batch mode (S=64) on the exact frames the probe fed
reproduces the tract pulsed dumps with e0/c0 **bit-exact** and worst case
1.2e-5 elsewhere. I.e.

> tract-pulsed(frame t) ≡ batch model fed with zero left-context, frame-aligned.

That pins the target semantics exactly, and it means the states the S=1 loop
must carry are precisely the contents of the exporters' dynamic `Pad` nodes.

## 2. The exporter's "cache" pads

Every time-axis conv in the three graphs is preceded by a `Pad` whose pads
tensor is computed dynamically from `Shape()`. Instrumented runs
(`py/inspect_pads.py`) show it always evaluates to `[0,0,L,0, 0,0,0,0]` with

```
L = (k_s - 1) * dilation      (always dilation 1 here)
```

Measured values: `erb_conv0` k=3→2, `erb_conv1/2/3` k=2→1, `df_conv0` k=3→2,
`df_conv1` k=2→1 (enc); `convt3` k=2→1, `conv0_out` k=2→1 (erb_dec);
`df_convp.1` k=5→4 (df_dec). The convs themselves are all `valid` on S
(no S pad in the conv attrs) and stride-1 on S — asserted by the script.

## 3. Surgery (per time-axis conv, `py/conv_state_surgery.py`)

```
X --[Pad, L zeros on S]--> Xp --[Conv k_s, valid on S]--> Y

               STATE_IN (L frames, zero-init)
                 |
    X -----------[Concat, axis=S]--> X_eff --[Conv, attrs unchanged]--> Y
                                      |
                                      [Slice 1:] --> STATE_OUT (L frames)
```

* `STATE_IN` carries the last `L` frames of the **conv input**; zero-init
  reproduces the Pad's zeros byte-for-byte for `t < L`.
* Equivalence: batch-wise `out[t] = conv(x[t-L..t])` (zeros for t<0) — exactly
  what the padded graph computes; frame-wise it is the standard streaming conv.
* The Pad node is deleted (its only consumer is the conv); dead pad-offset
  chains are pruned. Batch usage (S>1) remains valid with zero states.
* GRU states are a straight copy of the earlier surgery (empty `initial_h`
  slot → named graph input, `Y_h` → graph output); wiring output→input is
  spelled out per state in the JSON manifests.

States added (shapes are per-frame, i.e. S=1):

| model | conv delay lines | GRU | total state |
|---|---|---|---|
| enc | erb_conv0 [1,1,2,32], erb_conv1 [1,64,1,32], erb_conv2 [1,64,1,16], erb_conv3 [1,64,1,8], df_conv0 [1,2,2,96], df_conv1 [1,64,1,96] | 1× [1,1,512] | 10 688 f32 ≈ 41.8 KB |
| erb_dec | convt3 [1,64,1,8], conv0_out [1,64,1,32] | 2× [1,1,512] | 3 584 f32 ≈ 14.0 KB |
| df_dec | df_convp [1,64,4,96] | 3× [1,1,512] | 26 112 f32 ≈ 102 KB |

## 4. Validation (py/validate_stream.py, 64 frames = the dumps' horizon)

Ground truth: tract `PulsedModel`, pulse=1, run exactly like `libDF
DfTract` (`crates/ai-denoise/examples/pulse_probe2.rs`; its encoder dumps are
bit-identical to the original `pulse_out/` dumps — self-check).

### S=1 ORT loop, carrying all states from zero (vs tract dumps)

| model | output | n | max_abs | max_abs / max&#124;tract&#124; | max_rel (&#124;tract&#124;>1e-3) | mean_abs |
|---|---|---|---|---|---|---|
| enc | e0 | 131 072 | **0.000e+00** | 0 | 0 | 0 |
| enc | e1 | 65 536 | 3.815e-06 | 1.05e-07 | 5.01e-04 | 7.5e-08 |
| enc | e2 | 32 768 | 3.815e-06 | 1.49e-07 | 1.97e-04 | 1.0e-07 |
| enc | e3 | 32 768 | 5.722e-06 | 1.94e-07 | 3.42e-04 | 1.1e-07 |
| enc | emb | 32 768 | 1.445e-05 | 7.16e-06 | 3.25e-03 | 2.0e-07 |
| enc | c0 | 393 216 | **0.000e+00** | 0 | 0 | 0 |
| enc | lsnr | 64 | 1.621e-05 | 4.83e-07 | 1.17e-06 | 2.0e-06 |
| erb_dec | m (full chain, ORT-enc inputs) | 2 048 | 2.980e-06 | 2.98e-06 | 1.60e-05 | 1.5e-07 |
| erb_dec | m (isolation, exact tract-fed inputs) | 2 048 | 9.537e-07 | 9.54e-07 | 1.37e-05 | 6.7e-08 |
| df_dec | coefs (full chain) | 61 440 | 2.295e-06 | 3.20e-06 | 2.86e-04 | 6.9e-08 |
| df_dec | coefs (isolation) | 61 440 | 1.281e-06 | 1.79e-06 | 2.06e-04 | 5.2e-08 |

"max_rel (|tract|>1e-3)" is the worst relative error restricted to elements
where the reference exceeds 1e-3 — the blown-up-looking values (e.g. 3.2e-3 on
enc/emb) are 3e-6 absolute errors on ~1e-3-sized elements, i.e. f32 noise; the
flat, scale-aware numbers are `max_abs` and `max_abs/max|tract|`.

### Additional equivalence checks (all pass)

* batch (S=64, states=0) vs tract dumps: e0/c0 bit-exact, worst 1.2e-5 (emb,
  same figures as the unmodified model) → the surgery preserves batch semantics.
* S=1 stream vs S=64 batch, per output, bit-identical: e0, e1, e2, e3, c0 =True;
  emb, lsnr =False (different reduction order in GRU/fc kernels, both within
  1.5e-5 of tract; no state-semantics difference).
* chunking invariance: 128-frame stream ≡ 64-frame stream followed by a second
  64-frame run initialised with the first run's final states — **bit-identical**
  for every output of all three models (state carry is exact/Markov, proven
  beyond the dumped horizon).
* DirectML EP smoke (py/dml_smoke.py, 8 frames, random input): all three
  models run under `DmlExecutionProvider`; CPU-vs-DML max abs ≤ 1.3e-5
  (bit-identical for e0 and several conv states). Per-node placement not
  profiled.

### Residual-error diagnosis (why not literally 0.0 everywhere)

The residual is ORT-vs-tract floating-point accumulation, **not** missing
state: (a) outputs whose op sequence is identical to the batch reference are
bit-exact (e0, c0); (b) the same residuals appear when comparing *any* ORT run
against tract, including the stateless batch run of the original graph;
(c) the chunking check is bit-exact, so state evolution is exact. There is no
evidence of any remaining hidden streaming state in these three models.

## 5. Caveats

* Horizon of the ground-truth comparison is 64 frames (640 ms). Extending is
  trivial: raise `T_FRAMES` in `pulse_probe2.rs`, rerun, re-validate (the LCG
  input stream is deterministic).
* `df_dec` has a second graph output (`Sigmoid`, tensor `302`) that libDF
  drops (`with_output_names(["coefs"])`); it is not validated and not needed.
* The engine's ERB mask reduction (Mean over channel) is applied downstream of
  `m` in libDF; the validated model emits raw `m` ([1,1,1,32] per frame).
  With n_ch=1 mean==identity, but keep applying it for multi-channel.
* `pulse_probe2.rs` mirrors libDF with `mask_reduction=None`; nothing else from
  `DfTract` (analysis/synthesis, DF application, stage gating) is inside these
  graphs.
* DirectML: smoke-level evidence only (runs, matches CPU within f32 noise);
  not profiled for node placement or performance.

## 6. Next steps — Rust / DirectML integration

**Produce step** (offline, once per model revision):

```
python scratch/gpu_probe/py/conv_state_surgery.py scratch/gpu_probe/tmp/stateful scratch/gpu_probe/tmp/conv_stateful
```

Outputs: `tmp/conv_stateful/{enc,erb_dec,df_dec}.onnx` (39 MB total) and
`<model>.states.json` (machine-readable: per state — `input` name, `output`
name, `shape`, `init:"zeros"`, `kind: "gru_hidden"|"conv_delay"`).

**Runtime contract** (what to carry and feed):

* Per model, create the ORT session and allocate one f32 buffer per state
  (`states.json`, zero-initialised at stream start).
* Per 10 ms hop: `session.run(outputs, {…frame inputs…, …current states…})`;
  every state *output* tensor becomes the corresponding state *input* for the
  next hop (wiring is in the manifest; output names are the raw graph tensor
  names, keep them).
* Frame I/O (S=1, all shapes static):
  * `enc`: in `feat_erb [1,1,1,32]`, `feat_spec [1,2,1,96]` (dim1 = re/im);
    out `e0 [1,64,1,32] … c0 [1,64,1,96]`, `emb [1,1,512]`, `lsnr [1,1,1]`.
  * `erb_dec`: in `emb [1,1,512]`, `e3 [1,64,1,8]`, `e2 [1,64,1,8]`,
    `e1 [1,64,1,16]`, `e0 [1,64,1,32]`; out `m [1,1,1,32]` → gains [32]
    (then engine's mask reduce).
  * `df_dec`: in `emb [1,1,512]`, `c0 [1,64,1,96]`; out `coefs [1,1,96,10]`
    → per-frame reshape `[1, 96, 5, 2]` = `[ch, nb_df, df_order, re/im]`
    (nb_df=96, df_order=5; same convention libDF sets with
    `set_shape(&[ch, nb_df, df_order, 2])`).
* Total state per frame: enc ≈ 42 KB, erb_dec ≈ 14 KB, df_dec ≈ 102 KB;
  zero-alloc steady state (swap two preallocated buffers per state, or use
  IOBinding on DML).
* The S=1 form has **no dynamic shapes and no Pad/Shape-dependent op**: per
  time-axis conv it is a fixed-shape `Concat + Conv(k_s×k_f, S-valid) + Slice`.
  If DML per-op launch overhead dominates the ~15 convs/frame, the obvious
  follow-ups are (a) ORT session options / IOBinding to avoid copies,
  (b) fusing Concat+Slice away by rotating contiguous state buffers (state
  buffer as ring: conv input = memoized concat view; only worth it if
  profiling says so).
* Keep the exact numeric path: CPU and DML both agree with tract in the same
  f32-noise class (≤1.5e-5 abs); no f16 conversions anywhere.
