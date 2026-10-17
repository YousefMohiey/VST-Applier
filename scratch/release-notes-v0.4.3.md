# v0.4.3 — the updater now brings the plugin too

A small, focused release that closes the one gap from v0.4.2: updating from inside the app now delivers the bundled plugin as well, not just the app files.

## What changed
- **The in-app updater installs the bundled plugin.** The release zip now also carries **Yousef Noise Suppression.dll**; when you press Install + Restart, it lands in `Plugins\VST-Applier\` together with the app and host DLLs (same elevated path as everything else — one UAC prompt if needed). The staged-update line in Settings tells you when the plugin is part of the update.
- Older app-only zips still apply: the plugin step is skipped and only the app files are swapped, so nothing breaks mid-flight.
- Everything from v0.4.2 is included — root-caused audio reliability fixes (resampler tail, silence-path cost, PI servo, own WASAPI capture, self-heal), DeepFilterNet3-ll denoise + the plugin, the launcher-style settings surface, the UPDATE chip, rounded corners, the Inter/Archivo/Geist Mono stack, and the icon rendering fix.

## Update
- **From inside the app**: the UPDATE chip next to your profile → Download → Install + Restart. This now brings the plugin with it.
- **In place**: run **VST-Applier-Setup-v0.4.3.exe** over your current install.
- **Portable / manual**: VST-Applier-v0.4.3-win64.zip (vst-applier.exe + both native host DLLs + the bundled plugin).
