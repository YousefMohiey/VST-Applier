# VST3 editor hosting handoff

## Goal

Implement the next milestone for Snj Voice Changer: open a real VST3 plugin editor in a separate WinForms window.

This milestone is editor-only. Keep the audio route as passthrough. Do not implement VST audio processing yet.

## Project context

- Workspace: `C:\Work\codex\snj-voice-changer`
- Main app: `SnjVoiceChanger` C# WinForms, .NET 9
- Native bridge: `SnjVstHostNative` C++ DLL
- VST3 SDK: `third_party\vst3sdk`
- Current native API already exports:
  - `SnjVstHost_OpenEditor(SnjVstHostHandle host, void* parentHwnd)`
  - `SnjVstHost_CloseEditor(SnjVstHostHandle host)`
- C# already creates `NativeVstHost`, calls `LoadPlugin(path)`, and has an `Editor` button in `Form1.cs`.

## Important constraints

- Do not touch any driver code or revive the old driver branch.
- Do not run builds or tests. The user builds manually in Visual Studio and reports results.
- Keep the C ABI stable unless there is no reasonable alternative.
- Keep `SetupProcessing` and `ProcessFloat32` as the current smoke passthrough.
- Scope edits tightly to:
  - `SnjVstHostNative\*`
  - `SnjVoiceChanger\Form1.cs` only if needed for the editor window UX.

## Native implementation target

Replace the current `OpenEditor` placeholder with a minimal real VST3 editor host:

1. Require that a VST3 plugin has already been loaded.
2. Create/initialize the plugin component/controller enough to obtain an editor view.
3. Create an `IPlugView` with:

   ```cpp
   controller->createView(Steinberg::Vst::ViewType::kEditor)
   ```

4. Implement a small native `IPlugFrame` for `IPlugView::setFrame`.
5. Attach the plugin view to the provided Win32 parent HWND:

   ```cpp
   plugView->attached(parentHwnd, Steinberg::kPlatformTypeHWND)
   ```

6. Read the plugin view size with `getSize` when possible and call `onSize`.
7. Handle plugin-driven resizing via `IPlugFrame::resizeView`.
8. Store editor state on `SnjVstHost` so it lives until `CloseEditor`.
9. `CloseEditor` must be safe and idempotent:
   - call `IPlugView::removed()` when attached;
   - release view/frame/controller/provider state;
   - clear native error.

Use the official SDK samples as guide:

- `third_party\vst3sdk\public.sdk\samples\vst-hosting\editorhost\source\editorhost.cpp`
- `third_party\vst3sdk\public.sdk\source\vst\hosting\plugprovider.h`
- `third_party\vst3sdk\public.sdk\source\vst\hosting\plugprovider.cpp`
- `third_party\vst3sdk\pluginterfaces\gui\iplugview.h`

The current native host loads modules via:

```cpp
VST3::Hosting::Module::create(...)
module->getFactory()
factory.classInfos()
```

Prefer using `VST3::Hosting::PlugProvider` if it avoids hand-rolling controller/component initialization.

## C# editor window target

`Form1.cs` currently creates a label before calling `plugin.Host.OpenEditor(editorForm.Handle)`. That label can cover the real plugin editor.

Adjust the UX so that:

- the editor window is empty or has a dedicated host panel while native attach succeeds;
- fallback text is shown only if native editor opening fails;
- default editor window is large enough for real plugin UIs, for example around `900x650`;
- `CloseEditor` is always called when the window closes after a successful `OpenEditor`.

It is acceptable to call `OpenEditor` from the editor form `Shown` event so the HWND is created and visible before native attach.

## Error behavior

Return clear native errors for:

- no plugin loaded;
- no edit controller/editor view;
- `setFrame` failed;
- `attached(HWND)` failed.

C# should continue showing the error in the editor window and in `pluginStatusLabel`.

## Expected manual verification by user

After the user rebuilds:

1. Launch Snj Voice Changer.
2. Confirm the default plugin folder still scans the 3 CANA VST3 plugins.
3. Add one plugin to the chain.
4. Click `Editor`.
5. Expected: a separate window opens with the real plugin UI and controls.
6. Audio route should remain passthrough.

## Non-goals

- No VST audio processing.
- No presets/state persistence.
- No parameter automation.
- No embedded editor inside the main form.
- No new installer work.
