# df-ort — DeepFilterNet3-ll DSP + ONNX Runtime streaming backend

Vendored from libDF (`deep_filter` crate, MIT/Apache-2.0, rev `d375b2d`,
https://github.com/Rikorose/DeepFilterNet):

* `src/lib.rs` — the DSP half, a **verbatim copy** of libDF's `DFState`
  (STFT analysis/synthesis with the Vorbis window), the ERB filter bank and
  band features, the interpolated band gain, and the post-filter. Copied rather
  than reimplemented so the numbers match the shipping tract engine bit for bit.
* `src/backend.rs` — **new**: the streaming inference pipeline, mirroring
  `df::tract::DfTract::process` (stage gating, mask, deep filtering, post-filter,
  attenuation limit) but driving three ONNX Runtime sessions whose recurrent
  state lives in the graphs.
* `src/runtime.rs` — **new**: dynamic loading of `onnxruntime.dll` +
  `DirectML.dll`, execution-provider reporting, DXGI adapter name.

Everything under `models/` comes from the state-surgery work in
`scratch/gpu_probe` (see `scratch/gpu_probe/REPORT.md`):

| file | what |
|---|---|
| `enc.onnx`, `erb_dec.onnx`, `df_dec.onnx` | DeepFilterNet3-ll graphs with every time-axis conv delay line and GRU hidden state externalised |
| `*.states.json` | per model: state input name, state output name, per-frame shape |

Regenerate with:

```
python scratch/gpu_probe/py/conv_state_surgery.py scratch/gpu_probe/tmp/stateful crates/libdf-ort/models
```

## Contract

* Sample rate 48 kHz, hop 480, FFT 960, 32 ERB bands, 96 DF bins, DF order 5,
  0 lookahead frames — i.e. the shipped `DeepFilterNet3_ll` config. The values
  are constants in `backend.rs` (a `const` mismatch would silently change the
  model's meaning; they are checked by `models/*.states.json` shapes at load).
* Frame I/O per hop (all shapes static, batch = 1):
  * `enc`: in `feat_erb [1,1,1,32]`, `feat_spec [1,2,1,96]`; out `e0..e3`, `emb`, `c0`, `lsnr`
  * `erb_dec`: in `emb [1,1,512]`, `e3/e2 [1,64,1,8]`, `e1 [1,64,1,16]`, `e0 [1,64,1,32]`; out `m [1,1,1,32]`
  * `df_dec`: in `emb [1,1,512]`, `c0 [1,64,1,96]`; out `coefs [1,1,96,10]` (5 taps × re/im interleaved)
* State carry: every state **output** tensor is copied into its **input**
  buffer for the next hop, zero-initialised at stream start. `enc` carries 7
  states (10 688 f32), `erb_dec` 4 (3 584), `df_dec` 4 (26 112).

## ONNX Runtime binaries

`ort` is built with `load-dynamic`, so nothing is linked at build time. At
runtime `runtime::init` looks for `onnxruntime.dll` in:

1. `DF_ORT_DIR` (env var, a directory),
2. the caller's hint directory (the plugin passes its own folder),
3. the folder of the module containing `runtime.rs` (the plugin DLL, or the
   test binary under `cargo test`),
4. the running executable's folder.

`onnxruntime.dll` is loaded by **absolute path**, which sidesteps the Win32
DLL search order (a DLL loaded by full path does *not* get its own folder
searched for its dependencies). `DirectML.dll` is pre-loaded by absolute path
before the first DirectML session: ONNX Runtime's DML provider calls
`LoadLibraryA("DirectML.dll")` itself, and Windows resolves that to the
already-loaded module of the same base name.

The binaries that work here ship with the pip package
`onnxruntime-directml==1.24.4`:
`.../site-packages/onnxruntime/capi/{onnxruntime.dll,DirectML.dll,onnxruntime_providers_shared.dll}`.
`ort` is pinned to `=2.0.0-rc.12`, which wraps the ONNX Runtime **1.24** C API
(`api-24`) — matching that DLL.

If DirectML cannot be brought up, the same three models are re-opened on the
CPU execution provider. If the DLL is missing entirely, `DfOrt` still
constructs: `inference_available()` is false and `process` returns
`Err(...)` without touching the audio, so callers pass audio through.
`DF_ORT_FORCE_CPU=1` skips DirectML on purpose (diagnostics / broken driver).

## Numeric parity vs the shipping engine

`tests/parity.rs` runs the whole pipeline next to `df::tract::DfTract` on the
same deterministic speech-like signal (400 hops = 4 s, 48 kHz):

| case | samples | max abs diff | max rel (|ref|>1e-3) |
|---|---|---|---|
| default (atten 100 dB, pf off), DirectML | 187 200 | 2.79e-8 | 8.7e-6 |
| atten 20 dB + post-filter 0.02, DirectML | 187 200 | 2.42e-8 | 1.3e-5 |
| low-noise speech, DirectML | 187 200 | 3.49e-9 | 3.1e-6 |
| same cases on the CPU EP | 187 200 | 1.58e-8 / 1.49e-8 | ~9e-6 |

Reference RMS is ~1e-3, so 2.8e-8 absolute is ~3e-5 relative — f32 noise, well
inside the 1e-4 target. The tests also assert `max|ref - dry| > 1e-2`, so a
comparison that never actually engaged the model cannot pass.

Run them with:

```
DF_ORT_DIR=<dir with onnxruntime.dll> cargo test -p df-ort --test parity -- --nocapture
```

## Limits

* Mono (`n_ch = 1`); the mask reduction is implemented (mean over channels) but
  the plugin only ever instantiates one channel.
* DirectML node placement is not profiled; ORT falls back to CPU per node where
  DML has no kernel, which is normal and invisible here.
* The models are 39 MB, embedded in the DLL (`include_bytes!`), so plugin load
  reads them from memory rather than disk.
