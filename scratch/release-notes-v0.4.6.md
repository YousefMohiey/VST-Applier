# v0.4.6 — three new plugins: **3-Band EQ**, **Expander**, **Limiter** + hang fixes

## New plugin: Yousef 3-Band EQ
High / Mid / Low (±12 dB) exactly like the panel you showed: drag the slider, click the ▲▼ steppers for 1 dB steps, dB readout per band. Low shelf 200 Hz, mid peak 1.2 kHz, high shelf 6 kHz. Zero latency, 150 KB.

## New plugin: Yousef Expander
The expander panel: **Threshold, Ratio, Attack, Release, Output Gain** with dB/ratio/ms readouts and steppers. Quiet signals below the threshold get pushed down (that's what an expander does — cleans up background hiss between phrases). Same zero-latency treatment.

## New plugin: Yousef Limiter
The limiter panel: **Threshold** (ceiling, default -6 dB) + **Release** (default 60 ms). Instant attack — nothing ever gets past the ceiling, so no more clipping in your voice chat. Stereo-linked, zero latency.

All three install into your plugins folder as **"Yousef 3-Band EQ" / "Yousef Expander" / "Yousef Limiter"** — SCAN, then ADD them to the chain like the noise suppression.

## Fixed: "Not Responding" while adding / removing / toggling plugins
Found it for real this time: some plugin loads were still happening **on the UI thread** — starting the route (a cold FunkBass 3 takes 16 seconds!) and flipping a chain slot's enable checkbox back on. Both now load in the background; the window stays responsive and the route/chain picks the plugin up the moment it's ready. The log also gained per-step timings (`chain-remove: … teardown=N ms`) so any future slowdown names itself in one line.

## Also in this update
- The in-app updater now delivers **all four bundled plugins**.
- Everything from v0.4.5: loads any VST including instruments (FunkBass 3 works), background plugin adds, silent crash diagnostics.

## Updating from v0.4.4 / v0.4.5

Heads-up for one hop: older versions' updaters only knew the plugins that existed back then, so the in-app update brings the new app but can skip the three new plugins. If Yousef 3-Band EQ / Expander / Limiter don't show up after updating from one of those versions, just run **VST-Applier-Setup-v0.4.6.exe** once (installs all four), or copy them from the update's staging folder (`%AppData%\VstApplier\pending`). From this version onward the updater delivers every bundled plugin the package carries — automatically.

## Update
- **From inside the app**: UPDATE chip next to your profile → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.6.exe**.
- **Manual**: VST-Applier-v0.4.6-win64.zip (app + host DLLs + all four bundled plugins).
