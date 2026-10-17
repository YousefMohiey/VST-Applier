# v0.4.4 — 54 MB lighter, and start/stop is bulletproof

Two big things: the crashing stop/start loop is fixed for good, and the app itself shrank from 64 MB to 9.7 MB.

## Fixed: the start/stop crash
Repeatedly toggling routing (press start and stop a couple of times) could crash the app with the noise plugin in the chain. Root cause: on every stop we **unloaded the plugin DLL from memory** and on every start reloaded it — the ~4th unload/reload of a 54 MB plugin segfaulted inside its teardown state. Professional hosts (Reaper, Cubase, Ableton) never unload plugin modules; we now do the same — `GetModuleHandleExW(..., GET_MODULE_HANDLE_EX_FLAG_PIN)` loads the plugin exactly once per process, while plugin instances are still created and destroyed per route. Reproduced the segfault with a dedicated attach/drop harness (died on cycle 4), verified the fix with **20/20 clean cycles**, and the harness ships in the repo as a regression test.

## Smaller: 64 MB → 9.7 MB
The retired built-in AI denoise no longer compiles its model + inference runtime into the app exe — the **Yousef Noise Suppression** plugin owns denoising and keeps everything it needs. The installer (~100 MB, was 147) and the built-in updater's download (~50 MB, was 89) shrink with it, and the app starts a touch faster.

## Also included (everything from v0.4.3)
- The **in-app updater delivers the bundled plugin** along with the app.
- All of v0.4.3: updater plugin payload, and from v0.4.2: root-caused audio reliability fixes (resampler tail, silence-path cost, PI servo, own WASAPI capture, self-heal watchdog), DeepFilterNet3-ll denoise + plugin, launcher-style settings, UPDATE chip, rounded corners, Inter/Archivo/Geist Mono fonts, icon rendering fix.

## Update
- **From inside the app**: UPDATE chip next to your profile → Download → Install + Restart.
- **In place**: run **VST-Applier-Setup-v0.4.4.exe**.
- **Manual**: VST-Applier-v0.4.4-win64.zip (app + host DLLs + the bundled plugin).
