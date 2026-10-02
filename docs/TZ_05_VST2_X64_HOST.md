# TZ 05: VST2 x64 Host, Future Stage

## Context

Snj Voice Changer is a Windows C# WinForms app with a working user-mode audio route:

```text
real microphone -> Snj Voice Changer -> VST3 chain -> CABLE Input -> CABLE Output -> Meet/Chrome
```

Current VST3 hosting is implemented through a native C++ host DLL and C# wrapper. This stage must add VST2 x64 support without destabilizing the working VST3 route.

Important project constraints:

- Do not touch the old driver branch/files.
- Do not run builds/tests unless the user explicitly changes the standing rule.
- The user builds manually and reports results.
- Existing VST3 processing, editor hosting, plugin add/remove/reorder/enable UI must keep working.
- ReaPitch at `C:\Program Files\REAPER (x64)\Plugins\FX\reapitch.dll` was inspected earlier and identified as x64 VST2: AMD64 PE, exported `VSTPluginMain`.

## Goal

Add support for loading and processing 64-bit VST2 plugins in the same plugin chain as existing VST3 plugins.

Target chain examples:

```text
VST3 pitch plugin -> VST2 ReaPitch -> VST3 EQ
```

or:

```text
VST2 ReaPitch -> VST3 compressor
```

## Non-Goals

- No 32-bit VST2 bridge in this stage.
- No separate helper process or IPC bridge in this stage.
- No plugin preset/state persistence in this stage.
- No full parameter automation UI in this stage.
- No replacement of the existing VST3 host.
- No ASIO or exclusive-mode audio rewrite.

## Architecture

### Preferred Approach

Create a separate native project/DLL for VST2:

```text
SnjVst2HostNative.dll
```

Keep the current VST3 DLL intact:

```text
SnjVstHostNative.dll
```

In C#, introduce a small common plugin-host abstraction so the audio route does not care whether a chain item is backed by VST3 or VST2.

Suggested interface:

```csharp
public interface IAudioPluginHost : IDisposable
{
    string DisplayName { get; }
    string PluginPath { get; }

    void SetupProcessing(double sampleRate, int maxBlockSize, int inputChannels, int outputChannels);
    void ProcessFloat32(ReadOnlySpan<float> inputInterleaved, Span<float> outputInterleaved, int frameCount);

    void OpenEditor(IntPtr parentWindowHandle);
    void CloseEditor();
}
```

Then make chain items hold `IAudioPluginHost` instead of a concrete VST3 host.

```text
VstPluginChainItem
    -> IAudioPluginHost
        -> Vst3PluginHostAdapter
        -> Vst2PluginHostAdapter
```

This lets `AudioRoutingService` keep the current processing loop shape.

### Scanner

Extend plugin scanning to discover:

- VST3: existing `.vst3` bundles/files.
- VST2: `.dll` files that are x64 and export either:
  - `VSTPluginMain`
  - or legacy `main`

Scanner output should include plugin type:

```csharp
public enum VstPluginFormat
{
    Vst3,
    Vst2,
}
```

Candidate display:

```text
ReaPitch (VST2 x64)
CANA Epic Bender (VST3)
```

### Loading VST2

Native VST2 host needs to:

1. Load the plugin DLL.
2. Locate `VSTPluginMain` or `main`.
3. Provide an `audioMasterCallback`.
4. Receive `AEffect*`.
5. Validate magic value.
6. Open plugin with `effOpen`.
7. Configure:
   - sample rate via `effSetSampleRate`
   - block size via `effSetBlockSize`
   - mains on/off via `effMainsChanged`
8. Process float audio with `processReplacing`.
9. Close with `effMainsChanged(0)`, `effClose`, then unload DLL.

### Editor Hosting

VST2 editor support is separate from processing:

- Check `effFlagsHasEditor`.
- Open with `effEditOpen`.
- Close with `effEditClose`.
- Optionally query size through `effEditGetRect`.

If editor hosting fails, processing should still remain possible when the plugin supports processing.

### Channel Strategy

Keep the current app-level voice pipeline:

```text
capture input -> downmix to mono -> duplicate to 2-channel plugin buffer -> plugin chain -> downmix to mono -> route output format
```

For this VST2 stage, use 2 in / 2 out plugin processing, matching current VST3 behavior.

If a VST2 plugin reports unsupported input/output counts, bypass only that plugin or fail loading it gracefully.

## C# Integration Plan

1. Add common plugin host interface.
2. Wrap existing VST3 host in an adapter without changing native VST3 implementation.
3. Add VST2 native wrapper class:

```csharp
public sealed class NativeVst2Host : IAudioPluginHost
```

4. Update `VstPluginChainItem` to store `IAudioPluginHost`.
5. Update scanner candidate model to include `VstPluginFormat`.
6. In `addPluginButton_Click`, instantiate host by format:

```csharp
Vst3 -> NativeVstHost / Vst3 adapter
Vst2 -> NativeVst2Host
```

7. Keep `AudioRoutingService` mostly unchanged.

## Risk Controls

- Implement VST2 as a separate native DLL.
- Do not edit `SnjVstHostNative` unless a tiny shared output-copy rule requires it.
- Keep VST2 disabled from processing until load/open/setup succeeds.
- On any VST2 host error, show status and bypass/fail only that plugin, not the whole app.
- Keep existing VST3 plugins as the first regression test after every step.

## Suggested Milestones

### Milestone 1: VST2 Discovery Only

- Scan `.dll` plugins.
- Detect x64 PE.
- Detect exported `VSTPluginMain`/`main`.
- Show candidates in UI with `(VST2 x64)`.
- Do not load/process yet.

Implementation notes for this repo:

- `common\VST` is expected to contain format subfolders:
  - `common\VST\vst3`
  - `common\VST\vst2`
- The folder structure must be preserved by normal build/publish/installer flows.
- `SnjVoiceChanger.csproj` already copies `..\common\**\*`, so this should keep
  the subfolder layout in build output and installer output.
- `VstPluginScanner` must scan recursively from the selected folder.
- VST3 candidates come from `.vst3` files/bundles.
- VST2 candidates come from `.dll` files only when both conditions are true:
  - PE machine is AMD64/x64.
  - export table contains `VSTPluginMain` or legacy `main`.
- First implementation should show VST2 candidates but reject Add with a clear
  "VST2 host is not implemented yet" message. This avoids accidentally passing
  a VST2 DLL into the existing VST3 host.

### Milestone 2: VST2 Load Smoke

- Load DLL.
- Call entrypoint.
- Validate `AEffect`.
- Read name/vendor where possible.
- Close safely.

Implementation notes for this repo:

- Add a separate Visual C++ dynamic library project:
  - `SnjVst2HostNative\SnjVst2HostNative.vcxproj`
  - output beside `SnjVoiceChanger.exe`
- Export a C ABI with `SnjVst2Host_*` names. Do not reuse the VST3
  `SnjVstHost_*` names.
- The first C# integration should be smoke-only:
  - selecting a VST2 candidate and pressing `Add plugin` calls
    `NativeVst2Host.LoadPlugin(path)`;
  - if load succeeds, show `VST2 load OK: <name>`;
  - do not add the plugin to the audio chain yet.
- `publish\publish-self-contained.ps1` must copy `SnjVst2HostNative.dll` into
  `publish\app`, just like `SnjVstHostNative.dll`.
- Stop here for manual user build/testing before implementing editor hosting.

### Milestone 3: VST2 Editor Window

- Add plugin to chain.
- Open VST2 editor in modeless window.
- Processing can still be bypass/pass-through during this milestone.

### Milestone 4: VST2 Processing

- Configure sample rate/block size/channels.
- Run `processReplacing`.
- Mix VST2 and VST3 in one chain.

### Milestone 5: Stability Pass

- Remove plugin while editor is open.
- Reorder mixed VST2/VST3 chain.
- Enable/bypass checkboxes preserve plugin state.
- Start/Stop route repeatedly.
- Remove all plugins and confirm pass-through remains stable.

## Open Questions

- Which VST2 header/API source is legally acceptable for this repository?
- Should VST2 support rely on Steinberg legacy headers supplied by the developer, or a minimal clean-room struct definition?
- Should unsupported VST2 plugins be hidden from the list or shown with a disabled/error status?
- Do we need to copy REAPER plugins into build output, or only load them from their installed location?

## Acceptance Criteria

- Existing VST3 chain still works.
- ReaPitch x64 VST2 can be discovered.
- ReaPitch x64 VST2 can be loaded without crashing the app.
- ReaPitch editor opens in a modeless window.
- Audio can pass through ReaPitch in the same chain with VST3 plugins.
- Removing/reordering/enabling/bypassing mixed VST2/VST3 plugins does not crash.
- No 32-bit bridge is required for ReaPitch x64.

## Isolated Subagent Prompt For Next Stage

Use this prompt in a new clean thread if delegating without parent-chat context:

```text
You are working on Snj Voice Changer at C:\Work\codex\snj-voice-changer.
Do not run builds or tests; the user builds manually.
Do not touch any driver branch/files.
Do not spawn subagents.

Read docs\TZ_05_VST2_X64_HOST.md first.

Current goal: implement the next VST2 x64 milestone only.
Preserve the working VST3 route.
Prefer small scoped changes and stop when user build/testing is needed.

Milestone boundary:
- If Milestone 1 discovery is already implemented, do not rewrite it broadly.
- For Milestone 2, add a separate SnjVst2HostNative DLL smoke host.
- Do not modify SnjVstHostNative except tiny output-copy/publish rules if needed.
- Do not implement audio processing until VST2 load/open smoke is manually verified.
```
