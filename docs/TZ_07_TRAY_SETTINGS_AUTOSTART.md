# TZ 07: Tray, Settings and Autostart

## Context

VST-Applier v1.4 adds quality-of-life behavior so the app can stay running and
start working without manual steps: close-to-tray, a settings dialog,
start-with-Windows and automatic microphone routing on launch.

## Close to tray

- A tray icon is always present while the app runs (icon from `Assets/app.ico`).
- Closing the window (X button) hides the app to the tray instead of exiting,
  controlled by the `CloseToTray` setting (default on).
- Tray menu: `Open VST-Applier` (restore window) and `Exit` (real shutdown).
  Double-clicking the tray icon restores the window.
- First hide shows a balloon tip explaining the app is still running.
- Task manager closes and Windows shutdown are not intercepted; the session is
  still saved on real exit.

Implementation note: WinForms reports a bare WM_CLOSE (as posted by Task
Manager / `Process.CloseMainWindow`) as `CloseReason.TaskManagerClosing`, while
a real X-button click arrives as `CloseReason.UserClosing` via
`WM_SYSCOMMAND`/`SC_CLOSE`. Only the user-click path is intercepted.

## Settings dialog

A `Settings` button at the bottom of the main panel opens a dialog with:

- `Start microphone routing automatically when the app opens` (default on).
- `Keep running in the system tray when the window is closed` (default on).
- `Start with Windows (opens minimized to tray)` (default off).

Preferences live in `%APPDATA%\VstApplier\settings.json`.

## Start with Windows

The setting registers `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
value `VST-Applier` pointing at the executable with the `--tray` argument.
When the setting is enabled, the entry is rewritten on every start so a moved
executable self-heals. `--tray` starts the app hidden in the tray.

## Auto-start microphone routing

When enabled, the audio route starts automatically on launch (after the
session has been restored), so pressing Start is no longer needed. Failures
(no devices selected, no virtual cable) surface in the routing status label.

## Non-goals

- No installer code-signing.
- No tray "start/stop routing" shortcuts (window and Exit only).

## Verification

- Form harness: close-to-tray hides instead of exiting, tray Exit disposes the
  form, settings round-trip, settings dialog mapping.
- Published exe smoke tests: close-to-tray keeps the process alive after a
  simulated X-click; launch/close/session and chain restore still pass.
