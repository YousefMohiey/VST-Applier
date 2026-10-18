# v0.4.7 — tray CPU fix (a full core reclaimed)

## Fixed: closing the app to the tray no longer burns a CPU core
Found and fixed a real one: when the window was hidden to the tray (or minimized), the windowing layer kept spinning at ~100% of one CPU core — measured **92.6% → 0.1–0.3%** after the fix. Root cause: a hidden window never receives paint messages, and the redraw request that reset the loop could never arrive, so the event loop free-ran below the app's own code. Fixed at the windowing layer (vendored eframe patch: drop pending redraw requests for invisible windows) plus the app requesting nothing while hidden. Visible and minimized behavior unchanged; tray restore re-verified.

## Also in this update
- **Yousef 3-Band EQ / Expander / Limiter** — the three plugins from v0.4.6, now delivered by the updater automatically (wildcard plugin delivery: every "Yousef …" plugin in the package is copied — no more name-list)
- Everything from v0.4.6: any-VST loading (FunkBass 3 works), all plugin loads off the UI thread (no more "Not Responding"), silent crash diagnostics, the updater fix

## Update
- **From inside the app**: UPDATE chip next to your profile → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.7.exe**.
- **Manual**: VST-Applier-v0.4.7-win64.zip (app + host DLLs + all four bundled plugins).
