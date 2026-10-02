# TZ 01: Plugin Chain Auto-Restart

## Goal

Make the app feel continuously running after the user presses `Start` once. Plugin-chain changes may briefly stop and restart the audio route internally, but the user should not need to press `Start` again after adding, removing, reordering, or enabling/disabling plugins.

## Scope

- C# WinForms UI/controller layer only.
- Prefer changes in `SnjVoiceChanger/Form1.cs`.
- Reuse existing `AudioRoutingService.Start(...)` and `AudioRoutingService.Stop()`.
- Do not change native VST host code.
- Do not change the audio route core unless absolutely required.

## Current Behavior

`addPluginButton_Click` and `removePluginButton_Click` call `StopAudioRoute(...)` when the route is running. This leaves the app stopped until the user presses `Start` again.

## Required Behavior

- If route is stopped:
  - Add/remove/reorder/enable changes only update the chain UI.
  - Do not auto-start route.
- If route is running:
  - Apply the requested chain mutation.
  - Internally restart the route with the same selected input device, output device, buffer size, and updated active chain snapshot.
  - Keep `Start` disabled and `Stop` enabled.
  - Status should remain in a running state after restart, for example `Running - VST active...`.
- A short silence during restart is acceptable.
- If restart fails:
  - Keep the app safe and stopped.
  - Show the failure in route/plugin status.
  - Re-enable `Start` and disable `Stop`.

## Proposed Design

Add helper methods to `MainForm`:

```csharp
private bool TryRestartAudioRouteAfterChainChange(string actionStatus)
private bool TryStartAudioRouteFromCurrentSelection()
```

Suggested flow:

1. Capture `wasRunning = _audioRoutingService.IsRunning` before chain mutation.
2. If `wasRunning`, call `_audioRoutingService.Stop()` directly instead of `StopAudioRoute(...)`, because `StopAudioRoute(...)` intentionally updates UI into manual stopped state and restarts the idle input meter.
3. Apply mutation.
4. If `wasRunning`, call the normal route start helper with current selections and updated `GetPluginChainSnapshot()`.
5. Keep input monitor behavior consistent with existing `StartButton_Click`: route capture owns the input meter while running.

## Acceptance Criteria

- Press `Start` once.
- Add plugin: route restarts automatically and ends running.
- Remove plugin: route restarts automatically and ends running.
- No manual `Start` click is needed after chain mutation.
- If a plugin setup/process error happens, app does not crash and the status explains the bypass/failure.
- Existing explicit `Stop` still stops the route and does not auto-restart.

## Risk Notes

- This feature must not change audio processing internals.
- Most risk is UI-state drift: `Start`/`Stop` buttons and input meter can become inconsistent if restart helper bypasses existing button logic incorrectly.
- Keep restart logic centralized to avoid subtly different behavior in Add/Remove/Up/Down/Enable.

