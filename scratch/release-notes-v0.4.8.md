# v0.4.8 — GPU noise suppression, Compressor & Gate, hear-yourself monitor

## New: **Yousef Noise Suppression GPU**
The DeepFilterNet denoiser now runs **on your graphics card** via DirectML (works on AMD, NVIDIA and Intel GPUs; falls back to CPU automatically if no GPU runtime is present). Measured across the whole pipeline: **max deviation from the CPU engine 2.8e-8** (float noise — identical sound), ~6.8× realtime, ~1.5 ms per 10 ms of audio. Ships with its own runtime (onnxruntime + DirectML), no installs needed. Its editor is the new large one (640×400) and shows which engine is active — DirectML with your adapter name, CPU, or OFF.

## New: **Yousef Compressor** and **Yousef Gate**
Native replacements for the third-party compressor/gate you were using: real DSP (soft-knee, attack/release, 4 dB gate hysteresis + hold), 20/20 unit tests, zero latency, and the same big 640×400 editor. Your chain can now be 100 % Yousef.

## New: hear-yourself monitor
A MONITOR toggle under Levels plays your processed voice on your headphones so you can set levels without another app. Off by default, it refuses to monitor into the virtual cable and disables itself if the device misbehaves — the main route is never affected (verified: zero dropouts while toggling).

## Cleaner plugins folder: only the Yousef family
The bundled third-party plugins from older versions are retired — this update removes AltDenoiser, Bertom Denoiser, DeeGate, DeepFilterNet, ReaPlugs, TDR Nova and RNNoise from the install and ships **only the Yousef plugins**. Plugins **you added yourself are never touched**. (New installs: just the Yousef family.)

| Plugin | What it does |
|---|---|
| Yousef Noise Suppression | AI denoise (CPU) |
| Yousef Noise Suppression GPU | AI denoise on the GPU (DirectML) |
| Yousef 3-Band EQ | High / Mid / Low |
| Yousef Expander | Downward expander |
| Yousef Limiter | Peak limiter |
| Yousef Compressor | Soft-knee compressor |
| Yousef Gate | Noise gate |

## Update
- **From inside the app**: UPDATE chip → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.8.exe**.
- **Manual**: VST-Applier-v0.4.8-win64.zip.
