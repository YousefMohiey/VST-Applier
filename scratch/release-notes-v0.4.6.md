# v0.4.6 — the new **Yousef 3-Band EQ** plugin

## New plugin: Yousef 3-Band EQ
A three-band equalizer for your chain, built with the High / Mid / Low layout you asked for:

- **High** — high shelf (6 kHz), **Mid** — peaking (1.2 kHz), **Low** — low shelf (200 Hz), each **±12 dB**
- Drag the slider, click the **▲▼ steppers** for 1 dB steps, or type-free dB readout box per band — exactly the "High / Mid / Low" panel style
- **Zero latency** (plain biquads, no model, no resampling — works at 44.1 and 48 kHz natively), tiny (150 KB)
- Same family style as the noise suppression plugin: dark editor, mono labels, credit footer, state saved in your profiles like any other plugin

Where to find it after updating: it installs into your plugins folder as **"Yousef 3-Band EQ"** → hit **SCAN** in the app if it doesn't appear → **ADD** it to the chain next to the noise suppression.

## Also in this update
- The **in-app updater now delivers both bundled plugins** (noise suppression + the EQ) along with the app.
- Everything from v0.4.5: loads any VST including instruments (FunkBass 3 works now), plugin adds don't freeze the window, silent crash diagnostics.

## Update
- **From inside the app**: UPDATE chip next to your profile → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.6.exe**.
- **Manual**: VST-Applier-v0.4.6-win64.zip (app + host DLLs + both bundled plugins).
