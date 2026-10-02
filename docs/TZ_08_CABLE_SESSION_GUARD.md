# TZ_08: Cable session guard

VST-Applier v0.1.0 adds an option that keeps the virtual cable clean while the app
runs. It automates what users normally do by hand in the Windows volume mixer
(sndvol): muting other applications on the cable device.

## The problem

Anything that plays audio into the virtual cable becomes part of the "microphone"
that other apps record from CABLE Output. If another app (for example Discord)
outputs to the cable, its audio leaks into the mic signal: call audio echoes back
to the other side, stream audio gets captured, and so on. The manual fix is to open
the volume mixer for CABLE Input and mute that app.

## The option

Settings dialog, new checkbox: "Mute other apps on the virtual cable".
On by default. While the app runs and the option is enabled:

- The guard checks the selected output device every 2 seconds.
- It only acts when the device name looks like a virtual cable
  ("cable", "virtual" or "voicemeeter", case-insensitive). Real playback devices
  are never touched.
- Every audio session on that device that belongs to another process gets muted,
  which is exactly what the volume mixer mute button does.
- Sessions that were already muted by the user are left alone (and are not
  unmuted later).
- Sessions muted by the guard are unmuted again when the guard stops: app exit,
  option turned off, or the output device is switched to a non-cable device.

New app sessions (for example Discord starting later) are caught by the next poll.

## Implementation

`VstApplier/CableSessionGuard.cs`. A background thread polls the Windows Core
Audio session API through NAudio (`MMDeviceEnumerator` -> `MMDevice` ->
`AudioSessionManager.Sessions` -> `SimpleAudioVolume.Mute`). Muted sessions are
tracked by instance id and volume interface so they can be restored. The provider
function returns the device currently selected in the output dropdown, so the
guard follows device changes automatically.

## Verification

`CableGuardTest` in the tests folder, run against real Windows audio sessions:

- A helper process (`CableGuardPlayer`) plays a sine tone to the test device.
- The guard mutes the helper's session (checked via the same session API the
  volume mixer uses).
- The test process's own session is never muted.
- Disposing the guard unmutes the helper's session again.
