# v0.4.2 — rock-solid audio + a fresh look

The biggest quality pass so far: the dropout/lag/freeze episodes were investigated to the root (measured, not guessed), noise suppression got a real engine, and the whole UI was rebuilt around a launcher-grade settings surface.

## Audio reliability — root causes found and fixed
- **Resampler tail growth in the noise plugin**: the tail-keep formula kept the *whole* buffer after a full emission, so the per-block memcpy grew quadratically at 44.1 kHz — 4–10 ms of work per 10 ms block after minutes, which is what starved the ring, flooded dropouts and froze the UI. Fixed: 44.1 kHz went from climbing 876→2504 µs to a flat ~15–300 µs.
- **Silence was the most expensive input**: the denoiser ran its full path on dead-silence (5–7× cost, denormals). Now: a −70/−60 dB silence gate plus FTZ/DAZ on every audio thread.
- **Own WASAPI capture** replaces the previous input path — capture teardown can no longer wedge; it now has a bounded stop (1.5 s, then the thread is terminated as a last resort) so stop/restart and route healing can never hang the app.
- **PI servo instead of P-only**: the controller now *learns* the exact mic↔cable clock drift (250–550 ppm on USB mics) instead of parking the ring at whatever ramps happened to cancel out.
- **Self-heal watchdog**: sustained starvation or a frozen render counter restarts the route automatically (at most once per 30 s), with a toast so you know it happened.
- Startup dropouts (boot churn) are now counted separately from live ones, so the counter reflects what you'd actually hear.

## Noise suppression
- Built-in AI denoise now runs the low-latency **DeepFilterNet3-ll** model, fed at true 48 kHz (with the fix above it is genuinely cheap on untouched signal).
- The denoiser also ships as a chainable **VST2 plugin — "Yousef Noise Suppression"** — with a rebuilt editor: flicker-free painting, correct slider dragging, and a credit footer that opens the project page.

## The new look
- **New Settings surface** in the launcher style: app icon header, five labeled sections, square checkboxes, theme selector, credits — compact (460 px), always dead-center over a dimmed backdrop. DONE closes it (Esc and clicking outside work too).
- **Update chip**: when a newer release exists, an **UPDATE** chip appears next to your profile in the header — one click opens Settings, where the download and install steps live. It also tracks *downloading* and *restart to update* states.
- **Rounded window corners** on Windows 11.
- **New typeface stack** — Inter for UI text, Archivo SemiBold for titles, Geist Mono for the small labels (embedded, OFL licensed).
- Fixed small icons that could render as letter lookalikes (a font collision), the brand logo now keeps its true aspect ratio, and the sidebar cable badge text is complete.

## Update
- In-place: run **VST-Applier-Setup-v0.4.2.exe** over your current install.
- The app's built-in updater (the new UPDATE chip) also handles it for you.

**Full payload:** VST-Applier-v0.4.2-win64.zip (vst-applier.exe + both host DLLs + the bundled plugin) for manual/portable updates.
