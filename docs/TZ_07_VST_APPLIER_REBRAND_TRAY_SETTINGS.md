# TZ 07: VST-Applier Rebrand, Tray, Settings and Autostart

## Context

The project was rebranded from Snj Voice Changer to VST-Applier. All Snj-branded
identifiers, files and settings paths were renamed across the C# app and both
native VST host projects.

## Renames

| Old | New |
| --- | --- |
| `SnjVoiceChanger.sln` | `VstApplier.sln` |
| `SnjVoiceChanger/` (C# app, namespace `SnjVoiceChanger`) | `VstApplier/` (namespace `VstApplier`) |
| `SnjVstHostNative` (project, DLL) | `Vst3HostNative` |
| `SnjVst2HostNative` (project, DLL) | `Vst2HostNative` |
| Exports `SnjVstHost_*` / `SnjVst2Host_*` | `Vst3Host_*` / `Vst2Host_*` |
| `%APPDATA%\SnjVoiceChanger` | `%APPDATA%\VstApplier` |
| State blob magics `SNJS`/`SNJQ`/`SNJC`/`SNJP` | `VSTA`/`VSTP`/`VSTC`/`VSTD` |

State blobs written by earlier versions are still accepted on load (legacy
magics are recognized). The settings folder is migrated automatically on first
start: if `%APPDATA%\VstApplier` does not exist yet and the old
`%APPDATA%\SnjVoiceChanger` folder does, its contents are copied over.

The installer script (`publish/VstApplier.iss`) uses a new AppId, the new app
name and `VstApplier.exe`, and installs to `Program Files\VST-Applier`.

## New features

### Close to tray

- A tray icon is always present while the app runs (icon from `Assets/app.ico`).
- Closing the window (X button) hides the app to the tray instead of exiting,
  controlled by the `CloseToTray` setting (default on).
- Tray menu: `Open VST-Applier` (restore window) and `Exit` (real shutdown).
  Double-clicking the tray icon restores the window.
- First hide shows a balloon tip explaining the app is still running.
- Windows shutdown / task manager closes are not intercepted; the session is
  still saved on real exit.

### Settings dialog

A `Settings` button at the bottom of the main panel opens a dialog with:

- `Start microphone routing automatically when the app opens` (default on).
- `Keep running in the system tray when the window is closed` (default on).
- `Start with Windows (opens minimized to tray)` (default off).

Preferences live in `%APPDATA%\VstApplier\settings.json`.

### Start with Windows

The setting registers `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
value `VST-Applier` pointing at the executable with the `--tray` argument.
When the setting is enabled, the entry is rewritten on every start so a moved
executable self-heals. `--tray` starts the app hidden in the tray.

### Auto-start microphone routing

When enabled, the audio route starts automatically on launch (after the
session has been restored), so pressing Start is no longer needed. Failures
(no devices selected, no virtual cable) surface in the routing status label.

## Non-goals

- No installer code-signing.
- No tray "start/stop routing" shortcuts (window and Exit only).

## Verification

- Round-trip harness: all 12 bundled plugins save/restore state; legacy `SNJS`
  and `SNJC` magics still load.
- Form harness: startup chain restore, profile save, session save on close,
  close-to-tray hides instead of exiting, tray Exit disposes the form, settings
  round-trip.
- Published exe smoke tests: launch/close/session; chain restore round trip;
  close-to-tray keeps the process alive.
