# TZ 06: Profiles and Session Persistence

## Context

VST-Applier is a Windows C# WinForms app with a working user-mode audio route:

```text
real microphone -> VST-Applier -> VST2/VST3 chain -> CABLE Input -> CABLE Output -> Meet/Chrome
```

Before this stage the app forgot everything on exit: device selection, plugin
folder, buffer size and the whole plugin chain had to be rebuilt by hand on
every launch.

## Goal

1. The app remembers the previous session automatically. Closing and reopening
   the app restores input/output device selection, buffer size, plugin folder,
   the plugin chain (order, enabled flags) and each plugin's own parameter
   state.
2. Named profiles. The user can save the current setup as a named profile and
   switch between profiles from a dropdown. Loading a profile restores the
   full setup including plugin parameters.

## Storage layout

Everything lives under `%APPDATA%\VstApplier`:

```text
%APPDATA%\VstApplier\session.json           automatic session snapshot (written on close)
%APPDATA%\VstApplier\profiles\<name>.json   named profiles
```

`session.json` is written when the main form closes and read on startup.
Named profiles are written by the profile UI (Create / Save) and can be
deleted from the UI.

Profile JSON schema (version 1):

```json
{
  "SchemaVersion": 1,
  "Name": "Streaming",
  "ActiveProfile": "Streaming",
  "PluginFolder": "C:\\...\\common\\VST",
  "InputDeviceId": "{0.0.1.00000000}.{...}",
  "InputDeviceName": "Microphone (USB Audio)",
  "OutputDeviceId": "{...}",
  "OutputDeviceName": "CABLE Input (VB-Audio Virtual Cable)",
  "BufferSize": 512,
  "Plugins": [
    {
      "Name": "Graillon 3",
      "Path": "C:\\...\\Auburn Sounds Graillon 3-64.dll",
      "Format": "Vst2",
      "Enabled": true,
      "State": "<base64 state blob>"
    }
  ]
}
```

Device selection matches by endpoint id first and falls back to the device
friendly name, so profiles survive devices being re-plugged with new ids.

## Plugin state capture

Plugin parameter state is captured through native host APIs:

- `Vst3Host_SaveState` / `Vst3Host_LoadState` (VST3)
- `Vst2Host_SaveState` / `Vst2Host_LoadState` (VST2)

Both use a two-call protocol: first call with a null buffer returns the
required byte count (or a negative error code), second call fills the buffer.
The app stores the blob as base64 inside the profile JSON.

VST3 blob format (magic `VSTA`, version 1):

```text
[uint32 magic 'VSTA'][uint32 version][uint32 componentSize][component state]
[uint32 controllerSize][controller state]
```

Restore calls `IComponent::setState`, then `IEditController::setComponentState`
with the same component stream, then `IEditController::setState`.

If `IComponent::getState` fails or the component is missing, the host falls
back to a controller parameter dump (magic `VSTP`): every parameter id plus its
normalized value. Restore uses `setParamNormalized` and forwards the change to
the audio processor through the host's component handler.

VST2 blob format:

- Chunk state (magic `VSTC`): `[magic][version][uint32 chunkSize][chunk bytes]`.
  Save resolves bank chunk (index 0) first, then program chunk (index 1).
  Restore resolves the index the same way and calls `effSetChunk`.
- Parameter fallback (magic `VSTD`): `[magic][version][uint32 count][float * count]`
  captured with `getParameter`, restored with `setParameter`. Used for plugins
  that do not implement chunk opcodes.

Blobs written by earlier app versions use older magic values; the loaders
still recognize them so existing profiles keep working.

State capture is best effort: a plugin whose state cannot be read is still
persisted by path, order and enabled flag.

## UI

A "Profile" row was added to the main panel:

- Profile dropdown: selecting a profile loads it immediately. If the current setup has changes that were not saved to the equipped profile, a prompt asks to save them before switching (Yes / No / Cancel), so edits are never dropped silently.
- `Create`: asks for a name (small dialog, suggests the first unused "New profile" name) and creates an EMPTY profile (zero plugins) equipped for use. Devices, buffer size and plugin folder stay as they are; the chain is cleared so the new profile starts fresh. Nothing is stored until Save is pressed.
- `Save`: saves the current setup (including the current plugin chain) to the equipped (selected) profile.
- `Delete`: deletes the selected profile after a confirmation.

The main panel layout was shifted down to make room; the found-plugins and
plugin-chain boxes were tightened slightly. The window size is unchanged.

## Non-Goals

- No cloud sync, no import/export of profiles in this stage.
- No automatic per-plugin preset management (plugin editor presets still work
  as before).
- No changes to the audio route itself.

## Verification

- Native round-trip harness: every bundled VST2 and VST3 plugin saves state,
  restores it and re-saves an identical blob (byte-for-byte).
- Form harness: session.json with two plugin entries is restored into the
  chain on startup (including enabled/disabled flags); Save profile writes the
  full state; closing the form re-writes session.json with the active profile
  name and state blobs intact.
