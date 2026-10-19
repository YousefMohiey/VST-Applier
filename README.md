<div align="center">

# VST-Applier

**Run real VST2/VST3 plugins on your microphone. Free and open source.**

Pick your mic → add your plugins → press START. Discord, Zoom, OBS and Teams hear the processed voice,
because the audio comes out of a virtual cable instead of your raw microphone.

[![Latest release](https://img.shields.io/github/v/release/YousefMohiey/VST-Applier?style=flat-square&color=teal)](https://github.com/YousefMohiey/VST-Applier/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/YousefMohiey/VST-Applier/total?style=flat-square&color=teal)](https://github.com/YousefMohiey/VST-Applier/releases)
[![Stars](https://img.shields.io/github/stars/YousefMohiey/VST-Applier?style=flat-square&color=teal)](https://github.com/YousefMohiey/VST-Applier/stargazers)
[![License: MIT](https://img.shields.io/badge/license-MIT-teal?style=flat-square)](LICENSE)
[![Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%2F11-blue?style=flat-square)](#install)
[![Rust](https://img.shields.io/badge/100%25-Rust-orange?style=flat-square)](#building-from-source)

### [⬇ Download for Windows](https://github.com/YousefMohiey/VST-Applier/releases/latest)

</div>

![VST-Applier running a live plugin chain](docs/new-look.png)

---

## Why this exists

Every "pro mic" voice you hear on a stream or podcast is a chain of plugins — a gate, a compressor,
an EQ, a de-esser, a noise suppressor. Getting that normally means running a DAW, fighting ASIO loopback,
or paying a monthly fee for a voice changer with a fixed effect library.

VST-Applier is the small missing program: it hosts **your** VST plugins (the same ones you'd use in a DAW)
on your microphone in real time and pumps the result into a virtual audio cable. No DAW. No subscription.
No re-buying effects you already own.

## How it compares

| | VST-Applier | Voicemod | VoiceMeeter | Equalizer APO | DAW + loopback |
|---|---|---|---|---|---|
| Runs arbitrary **VST2/VST3** plugins | ✅ | closed effect library | ✅ (Potato) | ✅ via VST wrapper | ✅ |
| GUI chain: add / reorder / bypass live | ✅ | — | mixer strips | config file | ✅ |
| Remembers every plugin's knobs | ✅ auto-saved | server-side | ✅ | ✅ | ✅ |
| Free, no account, no telemetry | ✅ MIT | freemium | ✅ donationware | ✅ | depends |
| Focused "mic chain" app, not a full mixer | ✅ | ✅ | ❌ | ❌ | ❌ |

To be fair: **if you want a full virtual mixing desk with buses and per-app routing, use VoiceMeeter Potato** —
it is excellent at that. VST-Applier is deliberately narrower: one microphone, one plugin chain, one output,
nothing to configure.

## Features

- **Real-time VST2 + VST3 chain** on your mic, typically **~10–100 ms** round trip depending on buffer and devices.
- **Live chain editing** — add, remove, reorder, enable/disable plugins while audio is running. No cut, no restart.
- **21 plugins bundled** — RNNoise (mono/stereo), DeepFilterNet, Alt Denoiser, Bertom Denoiser, DeeGate,
  TDR Nova, and the REAPER ReaPlugs suite (ReaEQ, ReaGate, ReaComp, …). Point it at any VST folder to scan more.
- **Settings preserved** — every plugin keeps its knob positions across bypass, reorder and restarts.
  Your chain auto-saves on every exit, including during updates.
- **Profiles** — save and load named setups ("podcast", "game chat", "noisy room"); the active one restores at launch.
- **Safe engine** — a misbehaving plugin is contained by Windows SEH and reported as an error instead of taking
  the app down. Audio threads are panic-isolated, with a lock-free ring buffer, dropout counter, and honest
  measured latency / sample rate / buffer readout.
- **Quality of life** — system tray with single-instance wake, start-with-Windows, Dark / Light / System themes,
  folder picker for your plugin directory.
- **Self-updating** — checks GitHub Releases and stages the update in place.

## Install

> Requires Windows 10 or 11 (x64). No account, no signup, nothing to configure.

1. **Download** [`VST-Applier-Setup-vX.Y.Z.exe`](https://github.com/YousefMohiey/VST-Applier/releases/latest)
   and run it. The wizard bundles the two native host DLLs and the plugin pack, and can install the
   **VB-CABLE** virtual audio driver for you (recommended — it is skipped if you already have it; needs one reboot).
2. **Open VST-Applier** and set **input** = your microphone, **output** = `CABLE Input (VB-Audio Virtual Cable)`.
3. **Add plugins** from the PLUGINS panel, then press **START**.
4. In Discord / Zoom / OBS / Teams, set your **microphone** to `CABLE Output (VB-Audio Virtual Cable)`.

A portable zip (`VST-Applier-vX.Y.Z-win64.zip`) is attached to every release as well — it is the same payload
the in-app updater uses.

> **Windows SmartScreen will warn you on first run.** The installer is not code-signed (certificates cost money
> and this is a free project), so Windows reports an unrecognised app — click **More info → Run anyway**.
> The full source is in this repository and every release is built from it.

## Screenshots

| Dark (default) | Light |
|---|---|
| ![dark](docs/new-look.png) | ![light](docs/new-look-light.png) |

## How it works

```
mic ──► WASAPI capture ──► mono ──► VST2/VST3 chain (48 kHz) ──► resample ──► VB-CABLE ──► Discord/Zoom/OBS
```

- Capture and render run on separate streams joined by a lock-free ring buffer with an occupancy servo,
  so microphone and cable clock drift can never starve or flood the audio.
- The chain is one native plugin instance per slot, hosted through `Vst2HostNative.dll` / `Vst3HostNative.dll`.
- Latency is set by the buffer (128–4096 samples; 1024 is the default → snappy UI, stable audio).

## FAQ

**Does this count as a voice changer / will it get me banned?**
It is a microphone processor. It does not inject into games, hook processes, or touch anti-cheat.
It presents itself to your apps as a normal audio device.

**Is it really free?**
Yes — MIT licensed, no account, no telemetry, no "pro" tier. Bundled plugins keep their own licenses.

**Can I use plugins I already own?**
Yes. Point the plugin folder chip at your VST folder and hit SCAN. Anything that works in a DAW should host here.

**Why not just use Voicemod?**
Voicemod gives you its effect library on a subscription. This gives you any VST you want, forever, for free.

**Why not just use a DAW?**
You can — if you enjoy ASIO loopback, aggregate devices, and keeping a DAW open forever. This is one window
and a START button.

**Latency is too high / it crackles.**
Crackles mean the buffer is too small for your CPU; latency too high means it is too big. Move the buffer
slider and watch the measured readout in the status bar.

**Does it work with OBS / Discord / Zoom / Teams / Google Meet?**
Yes — any app that lets you pick an input device. That is the whole point of routing through a virtual cable.

## Tips

- Lowest latency: drop the buffer; if you hear crackles, raise it.
- Click the plugins-folder chip (bottom-right) to point at any VST folder, then hit **SCAN**.
- The bottom status bar shows buffer, sample rates and measured latency; hover it for full diagnostics.
- Updates: **SETTINGS → Check now**. Failures are logged to `%APPDATA%\VstApplier\pending\install.log`.

## Building from source

Requirements: Rust stable (MSVC toolchain) with `cargo` — that's it. Everything, including the two VST host
DLLs, builds from this workspace; there is no C/C++ source in the repo.

```powershell
cargo build --release
# -> target\release\vst-applier.exe
#    target\release\Vst2HostNative.dll
#    target\release\Vst3HostNative.dll
```

| Path | What it is |
|---|---|
| `src/` | egui desktop UI, tray, updater, single-instance, theming |
| `crates/audio-engine/` | WASAPI route: capture → chain → resample → cable, lock-free ring |
| `crates/vst-host-sys/` | FFI bindings to the host DLLs, state blobs, plugin calls |
| `crates/vst2-host-dll/` | the VST2 host DLL — 100% Rust cdylib (`Vst2HostNative.dll`) |
| `crates/vst3-host-dll/` | the VST3 host DLL — 100% Rust cdylib (`Vst3HostNative.dll`) |
| `crates/app-state/` | profiles, settings, session persistence, plugin scanning |
| `installer/` | Inno Setup script + staged payload for the setup |
| `examples/` (crate subdirs) | self-check binaries: chain hot-add, plugin verification, profile roundtrip |

### Self-checks

```powershell
cargo run -p audio-engine --example chain_hot_add   # live chain edits on a running route
cargo run -p app-state    --example profile_roundtrip
cargo test -p vst2-host-dll -p vst3-host-dll        # SEH guard: faults and panics are caught
```

## Third-party

- **[VB-CABLE](https://vb-audio.com/Cable/)** virtual audio driver by VB-Audio — bundleable installer, donationware.
- **[microseh](https://github.com/sonodima/microseh)** (MIT) — supplies the Windows SEH hardware-exception guard
  (a tiny `__try/__except` stub compiled at build time) that lets the host DLLs report a faulting plugin as an
  error instead of crashing — the same protection the former C++ host implemented by hand.
- Bundled plugins keep their own licenses: REAPER ReaPlugs (Cockos), RNNoise builds (Xiph/werman), DeepFilterNet,
  Alt Denoiser, Bertom Denoiser, DeeGate, TDR Nova (Tokyo Dawn Labs).

## Legacy

The original **C# implementation** (WinForms app + earlier native hosts) lives on the
[`csharp-legacy`](https://github.com/YousefMohiey/VST-Applier/tree/csharp-legacy) branch and in the v0.1–v0.3 tags.

## License

MIT — see [LICENSE](LICENSE).

---

<div align="center">

If this saved you a subscription, **a star helps the next person find it.**

</div>
