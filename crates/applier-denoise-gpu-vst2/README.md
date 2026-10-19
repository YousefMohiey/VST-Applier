# Yousef Noise Suppression GPU (VST2)

DeepFilterNet3-ll noise suppression running on the **GPU** through ONNX Runtime
+ DirectML, packaged as a VST2 plugin for the VST-Applier chain
(mic → plugin chain → VB-CABLE → Discord/OBS).

* DLL: `VSTApplierDenoiseGpu.dll` (built here), deployed as
  `Yousef Noise Suppression GPU.dll`.
* Same model as the CPU plugin (`Yousef Noise Suppression.dll`), but the three
  graphs run as ONNX Runtime sessions with the recurrent state carried inside
  the graph — no tract, no pulsed model. Numeric parity with the tract engine is
  measured at ~3e-8 absolute (see `../libdf-ort/README.md`).
* Params: **Attenuation limit** (0–100 dB, 100 = full gate) and
  **Post-filter** (0–0.2, 0 = off). State rides in a 12-byte `ADG1` chunk.
* Editor: **640 × 400** logical px (reported via `effEditGetRect`), title
  `YOUF DENOISE GPU`, two sliders with 13 px mono labels / 16 px value readouts,
  and an INFERENCE card showing `DirectML <adapter>` (amber `GPU` badge) or the
  CPU fallback with a red `OFF` badge if the runtime is missing.

## Deployment layout

The plugin folder must contain all four files (one folder, side by side):

```
Plugins/VST-Applier/
  Yousef Noise Suppression GPU.dll     <- target/release/VSTApplierDenoiseGpu.dll
  onnxruntime.dll                      <- ORT 1.24 (DirectML build)
  DirectML.dll
  onnxruntime_providers_shared.dll
```

`installer/deploy-gpu-plugin.ps1` builds and copies them (ASCII-only script,
defaults to the pip `onnxruntime-directml` files on this machine).

## Runtime behaviour

* Load (in `effOpen`) creates the three DirectML sessions and runs four silent
  warm-up hops, then resets the state. Measured ~1.2–1.4 s, once per instance;
  the first audio block is then ~2–3 ms instead of a ~0.6 s kernel-compile
  stall (that measurement is why the warm-up exists).
* Steady state on an RX 9070 XT: ~2.0–3.0 ms per 512-sample block at 48 kHz
  (10.7 ms of audio), i.e. ~4× realtime, one block of latency budget left.
* DirectML unavailable → the same models on the CPU EP (`CPU` badge, white
  status line). ONNX Runtime missing → pass-through audio plus a red `OFF`
  badge; the plugin never refuses to load.
* `DF_ORT_FORCE_CPU=1` skips DirectML on purpose. `ADG_TRACE=1` prints
  per-block counters to stderr.

## Tests

```
cargo test -p applier-denoise-gpu-vst2            # param/chunk/editor/rect + resampler
cargo run -p df-ort --release --example smoke     # EP, per-hop timing, LSNR range
cargo run -p applier-denoise-gpu-vst2 --release --example manual_host -- \
    "installer/app/Plugins/VST-Applier/Yousef Noise Suppression GPU.dll" --editor
```

`manual_host` mimics the app's VST2 host: open, 48 kHz, 512-frame blocks, 60
blocks of deterministic noise, SEH-guarded process calls (faults must be 0),
then `effEditGetRect`, `effEditOpen`, a slider click and a `PrintWindow` grab
(written as BMP; set `ADG_SHOT` to choose the path).
