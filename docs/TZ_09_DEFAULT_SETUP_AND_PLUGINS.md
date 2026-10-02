# TZ_09: Bundled default plugins and first-launch default setup

VST-Applier v0.1.0 ships with the project's plugins and a ready-made default
setup, so a new user opens the app and finds everything already in place.

## Bundled plugins

The plugin payload lives in the repository under `defaults/Plugins` and is copied
into the published app by `publish/publish-self-contained.ps1`:

```text
Plugins/ReaPlugs/*.dll        Cockos ReaPlugs VST2 suite (+ JS/Data support files)
Plugins/rnnoise/vst/*.dll     rnnoise_mono / rnnoise_stereo (VST2)
Plugins/rnnoise/rnnoise.vst3  rnnoise VST3 bundle
```

The installer (Inno Setup) packs `publish/app/*` recursively, so both the
installer and the portable zip carry the same layout. On a fresh start the app
picks `<app folder>/Plugins` as the default plugin folder
(`GetDefaultPluginFolder`).

VST3 bundles: `VstPluginScanner` now resolves `.vst3` FOLDERS to the binary inside
them (`Contents/x86_64-win`) and skips the inner binary as a separate candidate,
so a bundle shows up exactly once and loads correctly (the native host needs the
binary path, not the folder).

Known quirk: `reacontrolmidi-standalone.dll` crashes inside its own dispatcher
when fed its own saved chunk. The native host catches the failure and the app's
per-plugin restore path keeps the chain item with defaults, so it is safe to
ship; `DefaultsPluginsTest` tracks it as a known exception.

## First-launch default setup

`defaults/user/` holds JSON templates: `settings.json`, `session.json` and
`profiles/Main.json`. They are the project's own setup:

- settings: auto-start microphone, close-to-tray, start with Windows, keep the
  cable clean;
- session + Main profile: ReaEQ -> ReaGate -> rnnoise chain with the tuned
  parameter states, 512 buffer, no device pinned (each machine has its own).

`DefaultSetupSeeder.EnsureSeeded()` runs in `Program.Main` before the main form.
It only seeds when there is no `settings.json` and no profile yet, so existing
users are never touched. Paths in the templates use an `{APP}` placeholder that is
replaced with the install folder; because the placeholder always sits inside JSON
strings, the replacement escapes backslashes for JSON

The device fields in the templates are null on purpose: the profile must not pin
one machine's audio endpoints. On first save the user's actual devices are stored.

## Verification

- `FormPersistenceTest` seeds the shipped templates into a temp directory with a
  fake app path and asserts the produced JSON parses, paths resolve, the chain has
  three plugins with state blobs, and the settings map correctly.
- `DefaultsPluginsTest` scans `defaults/Plugins` with the real scanner and loads
  every candidate through the native hosts: all 12 load and round-trip their state
  (reacontrolmidi tracked as a known exception).
- `fresh-user-test.ps1` (tests folder) wipes `%APPDATA%/VstApplier`, starts the
  published exe and asserts the seeded files appear with resolved paths; the UI
  capture then shows profile "Main", the three-plugin chain and routing running.
