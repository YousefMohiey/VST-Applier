# v0.4.5 — snappier plugin adds, any-plugin loading + quiet crash protection

## New: the app now loads instruments and stubborn plugins
Previously the app rejected plugins that don't have a (stereo) audio input — which is **every instrument** (synths, samplers, bass machines like **FunkBass 3**): they generate sound instead of processing it, so they have no input at all. That's a perfectly valid plugin and now loads and runs correctly. Plugins that refuse the app's requested channel layout but work on their own defaults are no longer turned away either. And when something genuinely can't load, the message now says the real reason (e.g. "output bus width does not match") instead of a generic error.

## Smoother: adding plugins no longer freezes the app
Adding a plugin to a running chain used to load it **on the UI thread** — with the bundled denoiser (which carries its AI model) that froze the window for up to ~2 seconds. The plugin now loads **in the background**: the window stays responsive, and the plugin snaps into the live chain the moment it's ready. Removing plugins was already instant and stays that way. This also removed a hidden crash risk in the old code path (adding a plugin while routing was stopped could hit a bad unwrap).

## Safer: crash diagnostics, completely invisible
The start/stop crash fixed in v0.4.4 stays fixed. On top of that, if the app ever dies unexpectedly for any other reason it now **records a small diagnostic file on your machine** — no window, no message, nothing sent anywhere. If you ever report "it crashed", that file makes it directly diagnosable instead of a guessing game. Old diagnostic files are cleaned up automatically (newest 3 kept).

## Under the hood
- Every internal routing restart is now tagged in the log (`START (boot|button|space|…)`) — if a restart ever happens on its own, the log names exactly what triggered it.
- Internal build cleanups and test hooks (including a `vst3_probe` plugin-loading harness).

## Update
- **From inside the app**: UPDATE chip next to your profile → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.5.exe**.
- **Manual**: VST-Applier-v0.4.5-win64.zip (app + host DLLs + the bundled plugin).

