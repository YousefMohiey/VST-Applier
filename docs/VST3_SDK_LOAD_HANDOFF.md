# Snj Voice Changer - VST3 SDK Load-Only Handoff

This document is the handoff for a separate C++ worker thread.

The goal is the next native milestone after the smoke ABI:

```text
SnjVstHostNative.dll can really load a VST3 module and identify an audio effect class.
```

No audio processing and no editor hosting in this milestone.

The user runs builds manually. Do not run builds/tests from Codex.

## Current State

Solution:

- `SnjVoiceChanger/` - C# WinForms app.
- `SnjVstHostNative/` - C++ native bridge DLL.

The native smoke ABI already exists:

- `SnjVstHost_GetApiVersion`
- `SnjVstHost_Create`
- `SnjVstHost_Destroy`
- `SnjVstHost_LoadPlugin`
- `SnjVstHost_SetupProcessing`
- `SnjVstHost_ProcessFloat32`
- `SnjVstHost_OpenEditor`
- `SnjVstHost_CloseEditor`
- `SnjVstHost_GetLastError`

The C# app can already call the native smoke ABI and add `.vst3` files to the
plugin chain.

Test plugin path in build output:

```text
SnjVoiceChanger\bin\Debug\net9.0-windows\common\VST\CANA-Epic-Vocals(64)
```

Source test plugin path:

```text
common\VST\CANA-Epic-Vocals(64)
```

Use only `.vst3` files for this milestone:

```text
CANA Epic Bender (64).vst3
CANA Epic Tune (64).vst3
CANA Epic Vocals (64).vst3
```

## SDK Requirement

Real VST3 hosting requires the official Steinberg VST3 SDK.

Preferred location:

```text
third_party\vst3sdk
```

Alternative acceptable location:

```text
external\vst3sdk
```

If the SDK is not present, do not download it, do not vendor random code, and do
not fake a real VST3 load. Report clearly that the SDK must be added first.

Use official Steinberg sources as references:

- `https://github.com/steinbergmedia/vst3sdk`
- `https://steinbergmedia.github.io/vst3_dev_portal/`
- SDK samples:
  - `public.sdk/samples/vst-hosting/audiohost`
  - `public.sdk/samples/vst-hosting/editorhost`

## Ownership / Write Scope

Allowed:

- `SnjVstHostNative/*`
- `SnjVstHostNative/SnjVstHostNative.vcxproj`
- `SnjVstHostNative/SnjVstHostNative.vcxproj.filters`
- optional `SnjVstHostNative/README.md` if notes are needed

Do not edit:

- `SnjVoiceChanger/*`
- `.sln`
- `common/*`
- root docs
- SDK contents

Do not revert other people's edits. The repo may have many uncommitted files.

## Goal

Replace the smoke-only body of `SnjVstHost_LoadPlugin` with real VST3 module
loading while preserving the current C ABI.

Required behavior:

1. Validate the path is non-empty and ends with `.vst3`.
2. Load the VST3 module using SDK-supported mechanisms.
3. Read the plugin factory/classes.
4. Find an audio effect class.
5. Store enough native state on `SnjVstHost` for later setup/process milestones.
6. Return success if an audio effect class was found and can be instantiated or
   prepared for instantiation.
7. Return a clear native error if:
   - the file does not exist;
   - it is not a VST3 module;
   - no factory is available;
   - no audio effect class exists;
   - SDK initialization/loading fails.

For this milestone, `SetupProcessing` and `ProcessFloat32` may remain smoke
passthrough. Do not wire real processing yet.

`OpenEditor` may remain not implemented.

## API Compatibility

Do not change exported function names or signatures from `SnjVstHostNative.h`.

C# already depends on these exact exports.

Return convention remains:

- `0` success
- negative error code failure
- last error available via `SnjVstHost_GetLastError`

## Suggested Native Design

Keep SDK-specific implementation behind internal classes/functions. Do not leak
VST3 SDK types into the exported C ABI.

Suggested internal shape:

```cpp
struct SnjVstHost
{
    std::wstring pluginPath;
    std::wstring lastError;

    // smoke processing fields already present
    double sampleRate;
    int maxBlockSize;
    int inputChannels;
    int outputChannels;
    bool processingConfigured;

    // VST3 load-only state, exact types depend on SDK helpers used
    // module/factory references
    // selected effect class id
    // component/controller placeholders
};
```

Use RAII for SDK module/factory lifetime. `SnjVstHost_Destroy` must unload/free
native resources safely.

## Project Configuration

If SDK exists under `third_party\vst3sdk` or `external\vst3sdk`, update only
`SnjVstHostNative.vcxproj` with include/library/source references needed for
load-only hosting.

Keep:

- x64
- DynamicLibrary
- C++20
- `/MD`
- output beside the C# app

Do not change solution configs unless absolutely required.

## Acceptance Criteria

Manual user verification after build:

1. App starts and still reports `Native VST API v1`.
2. Found CANA `.vst3` plugins still appear in the right panel.
3. Clicking `Add plugin` on a real CANA `.vst3` succeeds only if the native
   bridge loaded it through the SDK and found an audio effect.
4. If a non-VST3/fake path is added later, native error is clean and visible.
5. Editor still reports not implemented.
6. Existing audio passthrough still works.

## Important Non-Goals

- Do not implement real audio processing yet.
- Do not implement editor attach yet.
- Do not add VST2 support.
- Do not create a separate process host yet.
- Do not run builds/tests.

## If Blocked

If the SDK is missing, stop after inspection and report:

```text
VST3 SDK is missing. Add official Steinberg SDK under third_party\vst3sdk or external\vst3sdk.
```

Do not attempt an unofficial workaround.
