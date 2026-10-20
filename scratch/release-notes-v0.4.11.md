# v0.4.11 — less CPU, less waste in the tray

## Tray is cheaper now
Hiding to the tray closes any open plugin editors and pauses the hear-yourself monitor (it resumes when you show the window) — you're not listening in the tray, so that work stops. With the route stopped, tray idle measures **0.0% CPU**.

## GPU denoiser: spin removed
Tuned the GPU inference path for a plugin that runs every 10 ms: ONNX Runtime session spinning is off (sequential, spin-free sessions) and DirectML's CPU fence wait — which was genuinely spinning — is disabled. Measured: the denoiser costs ~1.4–2 ms per 10 ms block; a full GPU chain (denoise → gate → compressor) measures **~27–30% of one core**, without the GPU plugin **~11%**, route stopped **~1%**. New `ADG_TRACE` diagnostics print per-block denoiser cost if you ever need to pinpoint a load.

## Honest limits
- Plugins in an active chain stay loaded in memory by design (that's what keeps your mic live); a GPU chain runs ~360 MB.
- Tray CPU with a live chain is the audio route itself doing its job — that part doesn't idle.

## Update
- **From inside the app**: UPDATE chip → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.11.exe**.
- **Manual**: VST-Applier-v0.4.11-win64.zip.
