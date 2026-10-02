# Snj Voice Changer - C# Native VST Interop Handoff

This document is the handoff for a C# worker thread. The goal is to connect the
existing WinForms app to the native smoke ABI exported by `SnjVstHostNative.dll`.

The user runs builds/tests manually. Do not run builds/tests from Codex.

## Current State

Solution:

- `SnjVoiceChanger/` - C# WinForms app.
- `SnjVstHostNative/` - C++ DLL copied beside `SnjVoiceChanger.exe`.

Native smoke ABI exists and builds:

- `SnjVstHost_GetApiVersion()`
- `SnjVstHost_Create()`
- `SnjVstHost_Destroy(...)`
- `SnjVstHost_LoadPlugin(...)`
- `SnjVstHost_SetupProcessing(...)`
- `SnjVstHost_ProcessFloat32(...)`
- `SnjVstHost_OpenEditor(...)`
- `SnjVstHost_CloseEditor(...)`
- `SnjVstHost_GetLastError(...)`

The app already has right-panel VST UI:

- plugin folder textbox;
- Browse;
- Scan;
- found `.vst3` list;
- Add plugin;
- plugin chain list;
- Remove;
- Editor.

Default plugin folder is copied to output:

```text
common\VST
```

Test plugins:

```text
common\VST\CANA-Epic-Vocals(64)\*.vst3
```

## Goal

Implement C# P/Invoke interop to the native smoke ABI and connect it to the
existing plugin UI.

This milestone is not real VST processing yet. The native bridge still only
validates `.vst3` paths and processes float buffers as passthrough. This step is
only to prove:

```text
C# WinForms -> P/Invoke -> SnjVstHostNative.dll
```

## Ownership / Write Scope

Allowed:

- `SnjVoiceChanger/*.cs`
- `SnjVoiceChanger/Form1.cs`
- `SnjVoiceChanger/Form1.Designer.cs` only if a tiny UI label/status is needed

Do not edit:

- `SnjVstHostNative/*`
- `.sln`
- `common/*`
- root docs except appending notes if absolutely necessary

Do not revert other people's edits. This repo may have uncommitted work.

## Required Design

Create a small managed wrapper, suggested files:

- `NativeVstHost.cs`
- `NativeVstHostApi.cs`
- optionally `NativeVstHostException.cs`

Use `DllImport("SnjVstHostNative", CallingConvention = CallingConvention.Cdecl)`.

Use `CharSet.Unicode` for `wchar_t*` paths/errors.

Recommended wrapper shape:

```csharp
public sealed class NativeVstHost : IDisposable
{
    public static int ApiVersion { get; }

    public void LoadPlugin(string pluginPath);
    public void SetupProcessing(double sampleRate, int maxBlockSize, int inputChannels, int outputChannels);
    public void ProcessFloat32(ReadOnlySpan<float> inputInterleaved, Span<float> outputInterleaved, int frameCount);
    public void OpenEditor(IntPtr parentHwnd);
    public void CloseEditor();
}
```

For MVP, one host per chain entry is fine. A simpler one-host prototype is also
acceptable if the code leaves a clear path to one-host-per-plugin.

## UI Integration Requirements

When the user clicks `Add plugin`:

1. Create a native host for the selected `.vst3`.
2. Call `LoadPlugin(plugin.Path)`.
3. If success, add an object to `pluginChainListBox`.
4. If failure, show native error in `pluginStatusLabel` and do not add it.

The chain item should keep both:

- display name;
- plugin path;
- native host handle/wrapper.

Do not add duplicate protection unless it is trivial. Duplicates are okay for
this milestone.

When the user clicks `Remove`:

1. Dispose the associated native host.
2. Remove the item from the chain list.

When the user clicks `Editor`:

1. Open the same placeholder editor window as today or a small host window.
2. Call `OpenEditor(editorForm.Handle)`.
3. Since native smoke returns not implemented, show the returned native error in
   the window or `pluginStatusLabel`.
4. Do not treat this as a crash/fatal error.

On form close:

- Dispose all chain plugin native hosts.

## Smoke Check

On app startup, call `SnjVstHost_GetApiVersion()` and surface it in a small
status label or `pluginStatusLabel`, for example:

```text
Native VST API v1. Found 3 VST3 plugin(s)
```

If the native DLL cannot be loaded, the app must not crash. It should show a
clear plugin status:

```text
Native VST host unavailable: ...
```

The rest of the audio passthrough app should keep working even when the native
VST host is unavailable.

## Error Handling

Native API returns `0` for success, negative for failure.

On failure:

1. Call `SnjVstHost_GetLastError(host)` when a host exists.
2. Use a fallback message for null-host/API-load failures.
3. Show the error in `pluginStatusLabel`.

Avoid throwing exceptions from UI event handlers unless caught locally.

## Acceptance Criteria

Manual user verification after build:

1. App starts even if no plugin is selected.
2. Plugin status mentions native API v1 when DLL is available.
3. Selecting `CANA Epic Bender (64)` and clicking Add adds it to chain.
4. If a fake/nonexistent path is used, UI shows a clean error.
5. Remove disposes and removes the selected chain item.
6. Editor opens a window and reports native "not implemented" instead of
   crashing.
7. Existing audio passthrough still works.

## Notes

Do not integrate native processing into `AudioRoutingService` in this milestone.
That is the next stage after C# interop is proven.
