# v0.4.9 — every editor rebuilt big & crisp, Applier branding, monitor fix

## All seven plugin editors, one big readable kit
Every editor is now the large 640×400 panel with big text — including the four older small ones (Noise Suppression CPU, 3-Band EQ, Expander, Limiter, previously 340 px wide). Text is rendered **DPI-native** (per-monitor aware, ClearType, no bitmap stretching), so it stays sharp at any display scaling — verified at 96, 120 and 144 DPI (640×400 / 800×500 / 960×600).

## The family is now **Applier**
All plugins are renamed from "Yousef …" to **Applier …** (editor titles, host names, file names, updater). Updates migrate automatically: old files are cleaned up, no duplicates.

## Hear-yourself monitor fixed
No more buzz: the monitor now opens your headphones at their native rate (no resampling), uses a proper band-limited path otherwise, trims smoothly and blocks DC. The MONITOR section got a tidy layout too. Verified: route dropouts stay at zero while monitoring.

## Denoiser is our own
Editor branding no longer references external projects — it reads "APPLIER DENOISE". Third-party components and licenses live in THIRD-PARTY-NOTICES.md.

## Also in this update
Everything from v0.4.8: Applier Noise Suppression GPU (DirectML), Compressor, Gate, and only Applier plugins bundled.

## Update
- **From inside the app**: UPDATE chip → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.9.exe**.
- **Manual**: VST-Applier-v0.4.9-win64.zip.
